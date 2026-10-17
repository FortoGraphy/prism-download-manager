//! Prism desktop shell (Tauri 2).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod integration;
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let background = args
        .iter()
        .any(|a| a == "--background" || a == "--minimized");
    let _log_guard = init_logging();
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
            tauri_plugin_window_state::Builder::default()
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
                #[cfg(target_os = "macos")]
                if show {
                    // WKWebView doesn't paint into a hidden window: show the (transparent) window now.
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
            if window.label() == "main"
                && let WindowEvent::CloseRequested { api, .. } = event
            {
                let close_to_tray = window.state::<Core>().settings().general.close_to_tray;
                if close_to_tray && !QUITTING.load(Ordering::SeqCst) {
                    api.prevent_close();
                    let _ = window.hide();
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
            commands::open_details,
            commands::open_window,
            commands::toggle_mini,
            commands::toggle_dropzone,
            commands::show_main,
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
    tray::refresh(app);
}

fn apply_window_material(window: &tauri::WebviewWindow, settings: &Settings) {
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
            let dark = !matches!(
                settings.appearance.theme,
                prism_core::settings::Theme::Light
            );
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
        .cloned()
        .collect()
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
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            };
            side_effects(&app, &core, &event, &mut was_active, &mut last_tooltip, &mut opened_progress);
            let _ = app.emit("core", &event);
        }
    });
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
        if settings.general.show_progress_window && opened_progress.insert(item.id.clone()) {
            if let Err(e) = windows::open_progress(app, &item.id) {
                tracing::error!("could not open the progress window: {e}");
            }
        }
    } else {
        if opened_progress.remove(&item.id) && item.status == prism_core::model::Status::Completed {
            // The completion dialog takes over; no reason to leave the popup open too.
            windows::close_progress(app, &item.id);
        }
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
            if settings.general.show_completion_dialog {
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
        CoreEvent::QueueFinished { name, .. } => {
            notify("Queue finished", &format!("{name} has no more downloads."))
        }
        CoreEvent::Notify { title, body } => notify(title, body),
        CoreEvent::ShowMain => windows::show_main(app),
        CoreEvent::ExitRequested => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move { quit(&app).await });
        }
        CoreEvent::ClipboardUrls(urls) if settings.clipboard.monitor => {
            if urls.len() == 1 {
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
