//! Prism desktop shell (Tauri 2).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod i18n;
mod integration;
#[cfg(target_os = "linux")]
mod sound;
mod tray;
mod updates;
mod windows;

use prism_core::events::{CaptureSource, CoreEvent};
use prism_core::manager::{Core, CoreOptions, NewItem};
use prism_core::settings::Settings;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
use tauri_plugin_notification::NotificationExt;

pub(crate) static QUITTING: AtomicBool = AtomicBool::new(false);
/// Started with `--background` / `--minimized` (autostart at login passes `--background`).
pub(crate) static STARTED_IN_BACKGROUND: AtomicBool = AtomicBool::new(false);

fn main() {
    // Linux: WebKitGTK picks its renderer itself (#36, #41, #44). Prism 1.1.8–1.2.2 forced the
    // previous one (WEBKIT_DISABLE_DMABUF_RENDERER=1) for less memory, which left windows empty,
    // half drawn or no longer redrawn on KDE Plasma (Intel), Hyprland (AMD) and XWayland. A
    // WEBKIT_DISABLE_DMABUF_RENDERER set by the user still applies, as for any WebKitGTK app.
    limit_malloc_arenas();
    let args: Vec<String> = std::env::args().collect();
    let background = args
        .iter()
        .any(|a| a == "--background" || a == "--minimized");
    STARTED_IN_BACKGROUND.store(background, std::sync::atomic::Ordering::Relaxed);
    let _log_guard = init_logging();
    match prism_core::platform::desktop_session() {
        Some(session) => tracing::info!(%session, "Prism {} starting", prism_core::APP_VERSION),
        None => tracing::info!("Prism {} starting", prism_core::APP_VERSION),
    }
    // Release builds have no console: make sure a panic always reaches the log file.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(
            "panic: {info}
{}",
            std::backtrace::Backtrace::force_capture()
        );
        default_hook(info);
    }));

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            let background = argv.iter().any(|a| a == "--background");
            handle_args(app, &argv);
            if !background && open_targets(&argv).is_empty() {
                windows::show_main(app);
            }
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--background"]),
        ))
        .plugin(
            // Position, size and maximized only: restoring "visible" would show the main window
            // at login or with Start minimized whenever it was open when Prism last quit.
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::all()
                        - tauri_plugin_window_state::StateFlags::VISIBLE,
                )
                .with_filter(|label| label == "main" || label.starts_with("details-"))
                .build(),
        )
        .setup(move |app| {
            let handle = app.handle().clone();
            let core = tauri::async_runtime::block_on(Core::start(CoreOptions::default()))
                .map_err(|e| format!("Prism failed to start: {e:#}"))?;
            app.manage(core.clone());
            tray::create(&handle)?;
            #[cfg(target_os = "macos")]
            mac_menu(&handle)?;
            spawn_event_forwarder(handle.clone(), core.clone());

            let settings = core.settings();
            if let Some(main) = app.get_webview_window("main") {
                apply_window_material(&main, &settings);
                let show = !background && !settings.general.start_minimized;
                if show && windows::SHOW_ON_CREATE {
                    // macOS: WKWebView doesn't paint into a hidden window. Linux (#30): a hidden
                    // WebKitGTK window sometimes stays blank when shown. The transparent window is
                    // shown now and paints itself in.
                    let _ = main.show();
                }
                if show {
                    // The UI shows the window after its first paint; this is only a fallback.
                    let w = main.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                        if !w.is_visible().unwrap_or(true) {
                            let _ = w.show();
                        }
                    });
                } else {
                    // Waiting in the tray: no need to keep the window's web view in memory.
                    windows::update_dock(&handle, false);
                    windows::release_main_later(&handle);
                }
            }
            // Self-heal browser integration on every start (cheap, idempotent).
            std::thread::spawn(|| {
                if let Err(e) = integration::register() {
                    tracing::warn!("native messaging registration failed: {e}");
                }
            });
            // macOS and AppImage: bring the extension copy the browser loads up to this version.
            let h = handle.clone();
            std::thread::spawn(move || integration::refresh_extension_copies(&h));
            handle_args(&handle, &std::env::args().collect::<Vec<_>>());
            core.arm_wake_timers();
            // Fetch yt-dlp/deno (and ffmpeg if there is no system copy) in the background so the
            // first "Download video" click doesn't wait for a tool download.
            if settings.video.auto_update {
                let c = core.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = c.ensure_tools().await {
                        tracing::warn!("video tools not ready: {e:#}");
                    }
                    let _ = c.update_ytdlp().await;
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Windows switched between light and dark: with the theme following it, the Mica
            // behind the window switches too (it was picked only when the window opened).
            if window.label() == "main"
                && let WindowEvent::ThemeChanged(_) = event
                && let Some(main) = window.app_handle().get_webview_window("main")
            {
                apply_window_material(&main, &window.state::<Core>().settings());
            }
            if window.label() == "main"
                && let WindowEvent::CloseRequested { api, .. } = event
                && !QUITTING.load(Ordering::SeqCst)
            {
                api.prevent_close();
                if window.state::<Core>().settings().general.close_to_tray {
                    let _ = window.hide();
                    windows::update_dock(window.app_handle(), false);
                    windows::release_main_later(window.app_handle());
                } else {
                    // "Close button hides to the tray" is off: the close button quits Prism (it
                    // used to close just the window and leave Prism running with no way back).
                    let app = window.app_handle().clone();
                    tauri::async_runtime::spawn(async move { quit(&app).await });
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::update_settings,
            commands::toggle_speed_limiter,
            commands::add_url,
            commands::get_capture,
            commands::confirm_capture,
            commands::cancel_capture,
            commands::reprobe_capture,
            commands::zip_contents,
            commands::extract_zip_file,
            commands::cancel_zip_file,
            commands::batch_from_pattern,
            commands::batch_from_text,
            commands::get_batch,
            commands::confirm_batch,
            commands::cancel_batch,
            commands::get_video,
            commands::open_video_url,
            commands::probe_video,
            commands::confirm_video,
            commands::cancel_video,
            commands::add_stream,
            commands::inspect_torrent,
            commands::read_torrent_file,
            commands::add_torrent,
            commands::hf_context,
            commands::hf_repo,
            commands::hf_search,
            commands::hf_present,
            commands::hf_add,
            commands::media_inspect,
            commands::media_plan,
            commands::media_start,
            commands::media_cancel,
            commands::media_jobs,
            commands::media_clear,
            commands::open_tools,
            commands::open_file,
            commands::reveal_path,
            commands::start_items,
            commands::pause_items,
            commands::pause_all,
            commands::resume_all,
            commands::delete_items,
            commands::restore_items,
            commands::empty_trash,
            commands::remove_completed,
            commands::redownload,
            commands::update_item,
            commands::item_details,
            commands::open_item,
            commands::reveal_item,
            commands::open_path,
            commands::assign_to_queue,
            commands::move_in_queue,
            commands::upsert_queue,
            commands::delete_queue,
            commands::start_queue,
            commands::stop_queue,
            commands::next_schedule_runs,
            commands::cancel_post_action,
            commands::upsert_category,
            commands::delete_category,
            commands::grabber_projects,
            commands::start_grabber,
            commands::stop_grabber,
            commands::delete_grabber_project,
            commands::download_grabbed,
            commands::export_list,
            commands::import_list,
            commands::export_settings,
            commands::import_settings,
            commands::export_diagnostics,
            commands::tool_status,
            commands::ensure_tools,
            commands::update_ytdlp,
            commands::integration_status,
            commands::repair_integration,
            commands::extension_dir,
            commands::extension_folder,
            commands::open_browser,
            commands::open_extension_store,
            commands::play_sound,
            commands::open_details,
            commands::open_window,
            commands::toggle_mini,
            commands::toggle_dropzone,
            commands::show_main,
            commands::main_window_starts_visible,
            commands::native_material,
            commands::app_run_id,
            commands::set_ui_language,
            commands::remove_deleted_files,
            commands::set_autostart,
            commands::quit_app,
            commands::free_space,
            commands::data_dir,
            commands::clipboard_read,
            commands::clipboard_write,
            updates::check_update,
            updates::install_update,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Prism");

    app.run(|app, event| {
        // Prism lives in the tray: closing the last window must not exit.
        if let RunEvent::ExitRequested { api, code, .. } = &event
            && code.is_none()
            && !QUITTING.load(Ordering::SeqCst)
        {
            api.prevent_exit();
        }
        // macOS: clicking the Dock icon (or `open -a Prism`) brings back the hidden main window.
        #[cfg(target_os = "macos")]
        if let RunEvent::Reopen { .. } = &event {
            windows::show_main(app);
        }
        let _ = (app, event);
    });
}

/// macOS app menu: the standard menu plus "Show Prism" (⌘0), so the main window can always be
/// reached even when the menu-bar icon is hidden.
#[cfg(target_os = "macos")]
fn mac_menu(app: &AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    let menu = Menu::default(app)?;
    if let Some(first) = menu.items()?.into_iter().next()
        && let Some(sub) = first.as_submenu()
    {
        let show = MenuItem::with_id(app, "show-main", "Show Prism", true, Some("CmdOrCtrl+0"))?;
        sub.insert(&show, 1)?;
        sub.insert(&PredefinedMenuItem::separator(app)?, 2)?;
    }
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        if event.id() == "show-main" {
            windows::show_main(app);
        }
    });
    Ok(())
}

/// Graceful exit: checkpoint every download, then quit.
pub async fn quit(app: &AppHandle) {
    if QUITTING.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(core) = app.try_state::<Core>() {
        core.shutdown().await;
    }
    app.exit(0);
}

/// Apply settings that live in the shell (autostart, window material).
pub fn apply_shell_settings(app: &AppHandle, settings: &Settings) {
    use tauri_plugin_autostart::ManagerExt;
    let al = app.autolaunch();
    let enabled = al.is_enabled().unwrap_or(false);
    if settings.general.launch_at_startup != enabled {
        let _ = if settings.general.launch_at_startup {
            al.enable()
        } else {
            al.disable()
        };
    }
    if let Some(main) = app.get_webview_window("main") {
        apply_window_material(&main, settings);
    }
    windows::update_dock(app, windows::main_is_open(app));
    tray::refresh(app);
}

/// Whether the OS puts a material behind the window: Mica on Windows 11, vibrancy on macOS. On
/// Linux and Windows 10 there is none, and the window keeps its own background.
pub(crate) fn native_material_supported() -> bool {
    #[cfg(target_os = "windows")]
    {
        // Mica needs Windows 11 (build 22000). Unknown: assume it's there, as before.
        windows_build().is_none_or(|b| b >= 22000)
    }
    #[cfg(target_os = "macos")]
    {
        true
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        false
    }
}

#[cfg(target_os = "windows")]
fn windows_build() -> Option<u32> {
    let key = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .ok()?;
    key.get_value::<String, _>("CurrentBuildNumber")
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Dark or light Mica behind the window. With the theme following the system, it follows what
/// Windows uses (#21: a light Windows used to get the dark Mica under the light theme).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn mica_dark(theme: &prism_core::settings::Theme, system: Option<tauri::Theme>) -> bool {
    match theme {
        prism_core::settings::Theme::Light => false,
        prism_core::settings::Theme::Dark | prism_core::settings::Theme::Dimmed => true,
        prism_core::settings::Theme::System => !matches!(system, Some(tauri::Theme::Light)),
        prism_core::settings::Theme::Custom(_) => true,
    }
}

pub(crate) fn apply_window_material(window: &tauri::WebviewWindow, settings: &Settings) {
    #[cfg(target_os = "windows")]
    {
        // Windows 11 rounds undecorated windows (and their shadow) only when asked to.
        if let Ok(hwnd) = window.hwnd() {
            use windows_sys::Win32::Graphics::Dwm::{
                DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
            };
            let pref = DWMWCP_ROUND;
            // SAFETY: valid window handle; the attribute value is a 4-byte enum.
            unsafe {
                DwmSetWindowAttribute(
                    hwnd.0 as _,
                    DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                    &pref as *const _ as *const core::ffi::c_void,
                    std::mem::size_of_val(&pref) as u32,
                );
            }
        }
        if settings.appearance.window_material {
            let dark = mica_dark(&settings.appearance.theme, window.theme().ok());
            let _ = window_vibrancy::apply_mica(window, Some(dark));
        } else {
            let _ = window_vibrancy::clear_mica(window);
        }
    }
    #[cfg(target_os = "macos")]
    {
        use window_vibrancy::{NSVisualEffectMaterial, apply_vibrancy};
        if settings.appearance.window_material {
            let _ = apply_vibrancy(
                window,
                NSVisualEffectMaterial::UnderWindowBackground,
                None,
                Some(12.0),
            );
        }
    }
    #[cfg(target_os = "linux")]
    {
        let _ = (window, settings);
    }
}

/// URLs / files passed on the command line (file associations, magnet handler, "Open with").
fn open_targets(argv: &[String]) -> Vec<String> {
    argv.iter()
        .skip(1)
        .filter(|a| !a.starts_with("--"))
        .filter(|a| {
            a.starts_with("magnet:")
                || a.contains("://")
                || a.to_ascii_lowercase().ends_with(".torrent")
        })
        .map(|a| local_path(a).unwrap_or_else(|| a.clone()))
        .collect()
}

/// Linux desktops may hand a file over as a `file://` URL (the launcher's `Exec=prism %U`).
fn local_path(arg: &str) -> Option<String> {
    if !arg.starts_with("file://") {
        return None;
    }
    let path = tauri::Url::parse(arg).ok()?.to_file_path().ok()?;
    Some(path.to_string_lossy().into_owned())
}

fn handle_args(app: &AppHandle, argv: &[String]) {
    let core = app.state::<Core>().inner().clone();
    for target in open_targets(argv) {
        let is_torrent_file = target.to_ascii_lowercase().ends_with(".torrent")
            && std::path::Path::new(&target).exists();
        if is_torrent_file || target.starts_with("magnet:") {
            let _ = windows::open(app, "torrent", Some(&encode_path(&target)));
        } else {
            core.capture(
                NewItem {
                    url: target,
                    ..Default::default()
                },
                CaptureSource::Manual,
            );
        }
    }
}

/// Linux (#40): glibc gives threads that allocate their own memory arenas, and memory freed in
/// them rarely goes back to the system, so a download with 32 connections left Prism holding a
/// few times what it used. Two arenas are plenty (measured: same speed, half the peak).
fn limit_malloc_arenas() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: mallopt only changes an allocator setting; it runs before any thread starts.
    unsafe {
        libc::mallopt(libc::M_ARENA_MAX, 2);
    }
}

/// Linux (#40): when the last download stops, hand the memory it freed back to the system. On a
/// thread of its own: the windows' events don't wait for it.
fn release_free_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    std::thread::spawn(|| {
        // SAFETY: malloc_trim only returns free heap pages to the system.
        unsafe {
            libc::malloc_trim(0);
        }
    });
}

/// Hex-encode an argument so it can travel inside a window label/route.
fn encode_path(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

fn spawn_event_forwarder(app: AppHandle, core: Core) {
    let mut rx = core.subscribe();
    tauri::async_runtime::spawn(async move {
        let mut was_active = None::<bool>;
        let mut last_tooltip = std::time::Instant::now();
        // Ids an auto-opened progress window already covers for their current active run.
        // Lives here (not in the frontend) because every open window would otherwise react
        // to the same event independently and race to open duplicate windows.
        let mut opened_progress = std::collections::HashSet::<String>::new();
        loop {
            let event = match rx.recv().await {
                Ok(e) => e,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    // The windows missed events, perhaps a download that finished (#41): they get
                    // the whole list again (stopped downloads drop their live progress).
                    tracing::warn!("the windows missed {n} events; sending them the list again");
                    let _ = app.emit("core", &CoreEvent::ItemsAdded(core.bootstrap().items));
                    continue;
                }
                Err(_) => break,
            };
            side_effects(
                &app,
                &core,
                &event,
                &mut was_active,
                &mut last_tooltip,
                &mut opened_progress,
            );
            let _ = app.emit("core", &event);
        }
    });
}

/// A file of a Hugging Face model downloading in its queue (no window per file).
fn in_model_queue(item: &prism_core::model::DownloadItem) -> bool {
    item.queue_id.as_deref() == Some(prism_core::huggingface::QUEUE_ID)
}

/// Opens the item's progress window the first time it's seen active, when the setting is on.
/// `opened_progress.insert` both checks and records "already handled" in one step.
fn maybe_open_progress_window(
    app: &AppHandle,
    settings: &Settings,
    opened_progress: &mut std::collections::HashSet<String>,
    item: &prism_core::model::DownloadItem,
) {
    if item.status.is_active() {
        if settings.general.show_progress_window
            && !in_model_queue(item)
            && opened_progress.insert(item.id.clone())
            && let Err(e) = windows::open_progress(app, &item.id)
        {
            tracing::error!("could not open the progress window: {e}");
        }
    } else if opened_progress.remove(&item.id)
        && item.status == prism_core::model::Status::Completed
    {
        // The completion dialog takes over; no reason to leave the popup open too.
        windows::close_progress(app, &item.id);
    }
}

fn side_effects(
    app: &AppHandle,
    core: &Core,
    event: &CoreEvent,
    was_active: &mut Option<bool>,
    last_tooltip: &mut std::time::Instant,
    opened_progress: &mut std::collections::HashSet<String>,
) {
    let settings = core.settings();
    let notify = |title: &str, body: &str| {
        if settings.general.notifications {
            let title = i18n::tr(title, &[]);
            let _ = app.notification().builder().title(title).body(body).show();
        }
    };
    match event {
        CoreEvent::ItemsAdded(items) => {
            for item in items {
                maybe_open_progress_window(app, &settings, opened_progress, item);
            }
        }
        CoreEvent::ItemUpdated(item) => {
            maybe_open_progress_window(app, &settings, opened_progress, item);
        }
        CoreEvent::CaptureRequested(info) => {
            if let Err(e) = windows::open(app, "capture", Some(&info.id)) {
                // Never lose a download because its dialog could not open: start it directly.
                tracing::error!("could not open the Download File window: {e}");
                match core.confirm_capture_default(&info.id) {
                    Ok(name) => notify("Download added", &name),
                    Err(e) => tracing::error!("could not start captured download: {e:#}"),
                }
            }
        }
        CoreEvent::BatchRequested(info) => {
            if let Err(e) = windows::open(app, "batch", Some(&info.id)) {
                tracing::error!("could not open the Download Links window: {e}");
            }
        }
        CoreEvent::VideoRequested(info) => {
            if let Err(e) = windows::open(app, "video", Some(&info.id)) {
                tracing::error!("could not open the Download Video window: {e}");
            }
        }
        CoreEvent::Completed(item) => {
            if in_model_queue(item) {
                // A model's files finish one after another: one notification when the queue is
                // done instead of a dialog for every config file and weights part.
            } else if settings.general.show_completion_dialog {
                let _ = windows::open(app, "complete", Some(&item.id));
            } else {
                notify("Download complete", &item.file_name);
            }
        }
        CoreEvent::Failed(item) => {
            notify(
                "Download failed",
                &format!(
                    "{}\n{}",
                    item.file_name,
                    item.error.clone().unwrap_or_default()
                ),
            );
        }
        CoreEvent::QueueFinished { name, .. } => notify(
            "Queue finished",
            &i18n::tr("{name} has no more downloads.", &[("name", name)]),
        ),
        CoreEvent::Notify { title, body } => notify(title, body),
        CoreEvent::LeftToBrowser { host } => notify(
            "Downloading in the browser",
            &i18n::tr(
                "{host} lets only a web browser download this file, so the browser is downloading it.",
                &[("host", host)],
            ),
        ),
        CoreEvent::OpenTorrent { source } => {
            if let Err(e) = windows::open(app, "torrent", Some(&encode_path(source))) {
                tracing::error!("could not open the Add torrent window: {e}");
            }
        }
        CoreEvent::OpenModel { source } => {
            let id = (!source.is_empty()).then(|| encode_path(source));
            if let Err(e) = windows::open(app, "model", id.as_deref()) {
                tracing::error!("could not open the model window: {e}");
            }
        }
        CoreEvent::MediaJob(job) => {
            // A long conversion that finished while nobody was looking at Prism.
            use prism_core::media::{JobState, MediaOp};
            let took = job.finished_at.zip(job.started_at).map(|(b, a)| b - a);
            let looking = app
                .webview_windows()
                .values()
                .any(|w| w.is_focused().unwrap_or(false));
            if took.is_some_and(|ms| ms >= 10_000) && !looking {
                let output = job
                    .output
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| job.name.clone());
                match (job.state, &job.op) {
                    (JobState::Done, MediaOp::Shrink { .. }) => {
                        notify("Smaller file ready", &output)
                    }
                    (JobState::Done, MediaOp::Zip) => notify("ZIP file ready", &output),
                    (JobState::Done, _) => notify("Conversion finished", &output),
                    (JobState::Failed, _) => notify("Conversion failed", &job.name),
                    _ => {}
                }
            }
        }
        // Linux plays sounds here, once, instead of in every window (#41).
        #[cfg(target_os = "linux")]
        CoreEvent::PlaySound { kind } if settings.sounds.enabled => sound::play(
            kind,
            &settings.sounds.complete_sound,
            settings.sounds.volume,
        ),
        CoreEvent::ShowMain => windows::show_main(app),
        CoreEvent::ExitRequested => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move { quit(&app).await });
        }
        CoreEvent::ClipboardUrls(urls) if settings.clipboard.monitor => {
            if !settings.clipboard.ask {
                // Quietly into the list ("Download later"), with a notification instead of a window.
                match core.add_copied_links(urls) {
                    Ok(items) if !items.is_empty() => {
                        let mut names: Vec<&str> =
                            items.iter().take(3).map(|i| i.file_name.as_str()).collect();
                        if items.len() > 3 {
                            names.push("…");
                        }
                        notify("Added to the list", &names.join("\n"));
                    }
                    Ok(_) => {}
                    Err(e) => tracing::error!("could not add the copied links: {e:#}"),
                }
            } else if urls.len() == 1 {
                core.capture(
                    NewItem {
                        url: urls[0].clone(),
                        ..Default::default()
                    },
                    CaptureSource::Clipboard,
                );
            } else {
                let entries = urls
                    .iter()
                    .map(|u| prism_core::events::BatchEntry {
                        url: u.clone(),
                        ..Default::default()
                    })
                    .collect();
                core.open_batch("Links from clipboard".into(), None, entries);
            }
        }
        CoreEvent::Progress(list) => {
            let active = list.len();
            let speed: u64 = list.iter().map(|p| p.speed).sum();
            let is_active = active > 0;
            if *was_active != Some(is_active) || last_tooltip.elapsed().as_secs() >= 1 {
                if *was_active == Some(true) && !is_active {
                    release_free_memory();
                }
                tray::update(app, active, speed, *was_active != Some(is_active));
                *was_active = Some(is_active);
                *last_tooltip = std::time::Instant::now();
            }
        }
        _ => {}
    }
    // When nothing runs any more the progress stream stops; reset the tray.
    if matches!(
        event,
        CoreEvent::Completed(_) | CoreEvent::Failed(_) | CoreEvent::ItemUpdated(_)
    ) {
        let active = core.active_count();
        if active == 0 && *was_active == Some(true) {
            release_free_memory();
            tray::update(app, 0, 0, true);
            *was_active = Some(false);
        }
    }
}

fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};
    let dir = prism_core::paths::logs_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("prism")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&dir)
        .ok()?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_env("PRISM_LOG").unwrap_or_else(|_| {
        EnvFilter::new(
            "info,prism_core=info,prism_engine=info,hyper=warn,reqwest=warn,librqbit=warn",
        )
    });
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(writer).with_ansi(false))
        .with(cfg!(debug_assertions).then(|| fmt::layer().with_writer(std::io::stderr)))
        .init();
    Some(guard)
}

#[cfg(test)]
mod tests {
    use super::{mica_dark, open_targets};
    use prism_core::settings::Theme;

    #[test]
    fn mica_follows_the_theme() {
        assert!(!mica_dark(&Theme::Light, Some(tauri::Theme::Dark)));
        assert!(mica_dark(&Theme::Dark, Some(tauri::Theme::Light)));
        assert!(mica_dark(&Theme::Dimmed, Some(tauri::Theme::Light)));
        assert!(mica_dark(&Theme::Custom("nord".into()), Some(tauri::Theme::Light)));
        // #21: following a light Windows gives the light Mica (it was always dark).
        assert!(!mica_dark(&Theme::System, Some(tauri::Theme::Light)));
        assert!(mica_dark(&Theme::System, Some(tauri::Theme::Dark)));
        assert!(mica_dark(&Theme::System, None), "unknown: dark, as before");
    }

    fn args(list: &[&str]) -> Vec<String> {
        std::iter::once("prism")
            .chain(list.iter().copied())
            .map(String::from)
            .collect()
    }

    #[test]
    fn links_and_torrent_files_from_the_command_line() {
        assert_eq!(
            open_targets(&args(&[
                "--background",
                "magnet:?xt=urn:btih:abc",
                "https://example.com/a.zip",
                "notes.txt"
            ])),
            ["magnet:?xt=urn:btih:abc", "https://example.com/a.zip"]
        );
        assert_eq!(
            open_targets(&args(&["Linux ISO.torrent"])),
            ["Linux ISO.torrent"]
        );
    }

    #[test]
    fn a_file_url_becomes_a_path() {
        let (url, path) = if cfg!(windows) {
            (
                "file:///C:/Users/me/Linux%20ISO.torrent",
                r"C:\Users\me\Linux ISO.torrent",
            )
        } else {
            (
                "file:///home/me/Linux%20ISO.torrent",
                "/home/me/Linux ISO.torrent",
            )
        };
        assert_eq!(open_targets(&args(&[url])), [path]);
    }
}
