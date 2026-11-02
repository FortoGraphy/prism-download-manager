//! System tray icon with a quick menu and live state (idle / downloading / paused).

use crate::i18n::tr;
use prism_core::manager::Core;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

pub const TRAY_ID: &str = "prism-tray";

// Windows/Linux trays are usually dark: white glyphs. macOS uses template images
// (black + alpha) that the menu bar tints for light/dark automatically.
#[cfg(not(target_os = "macos"))]
static ICON_IDLE: &[u8] = include_bytes!("../icons/tray/tray-idle.png");
#[cfg(not(target_os = "macos"))]
static ICON_ACTIVE: &[u8] = include_bytes!("../icons/tray/tray-downloading.png");
#[cfg(target_os = "macos")]
static ICON_IDLE: &[u8] = include_bytes!("../icons/tray/tray-idle-template.png");
#[cfg(target_os = "macos")]
static ICON_ACTIVE: &[u8] = include_bytes!("../icons/tray/tray-downloading-template.png");

pub struct TrayState {
    pub limiter: CheckMenuItem<Wry>,
    /// Menu items with the English text they show (relabelled when the UI language changes).
    items: Vec<(MenuItem<Wry>, &'static str)>,
}

pub fn create(app: &AppHandle) -> tauri::Result<TrayIcon> {
    let item = |id: &str, text: &str| MenuItem::with_id(app, id, tr(text, &[]), true, None::<&str>);
    let show = item("show", "Open Prism")?;
    let add = item("add", "Add URL…")?;
    let pause = item("pause_all", "Pause all")?;
    let resume = item("resume_all", "Resume all")?;
    let limiter_on = app
        .state::<Core>()
        .settings()
        .downloads
        .speed_limiter_enabled;
    let limiter = CheckMenuItem::with_id(
        app,
        "limiter",
        tr("Speed limiter", &[]),
        true,
        limiter_on,
        None::<&str>,
    )?;
    let mini = item("mini", "Mini window")?;
    let drop = item("dropzone", "Drop zone")?;
    let hide = item("hide_tray", "Hide tray icon")?;
    let quit = item("quit", "Quit Prism")?;
    let sep = || PredefinedMenuItem::separator(app);
    let menu = Menu::with_items(
        app,
        &[
            &show,
            &add,
            &sep()?,
            &pause,
            &resume,
            &limiter,
            &sep()?,
            &mini,
            &drop,
            &sep()?,
            &hide,
            &quit,
        ],
    )?;
    app.manage(TrayState {
        limiter: limiter.clone(),
        items: vec![
            (show.clone(), "Open Prism"),
            (add.clone(), "Add URL…"),
            (pause.clone(), "Pause all"),
            (resume.clone(), "Resume all"),
            (mini.clone(), "Mini window"),
            (drop.clone(), "Drop zone"),
            (hide.clone(), "Hide tray icon"),
            (quit.clone(), "Quit Prism"),
        ],
    });

    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(ICON_IDLE)?)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Prism")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let app = app.clone();
            match event.id().as_ref() {
                "show" => crate::windows::show_main(&app),
                "add" => {
                    let _ = crate::windows::open(&app, "addurl", None);
                }
                "pause_all" => app.state::<Core>().pause_all(),
                "resume_all" => app.state::<Core>().resume_all(),
                "limiter" => {
                    let _ = app.state::<Core>().toggle_speed_limiter();
                    refresh(&app);
                }
                "mini" => {
                    let _ = crate::windows::toggle_mini(&app);
                }
                "dropzone" => {
                    let _ = crate::windows::toggle_dropzone(&app);
                }
                "hide_tray" => {
                    let _ = app.state::<Core>().update_settings(&serde_json::json!({
                        "general": { "hideTrayIcon": true }
                    }));
                    // The menu that offered this is gone now: open the window so Prism is not left
                    // running with nothing on screen to click.
                    if !crate::windows::main_is_open(&app) {
                        crate::windows::show_main(&app);
                    }
                    sync_visibility(&app, true);
                }
                "quit" => {
                    tauri::async_runtime::spawn(async move { crate::quit(&app).await });
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                crate::windows::show_main(tray.app_handle());
            }
        })
        .build(app)?;
    if app.state::<Core>().settings().general.hide_tray_icon {
        sync_visibility(app, true);
    }
    Ok(tray)
}

/// Show or hide the tray icon ("Hide the tray icon" in the settings or in the tray menu).
pub fn sync_visibility(app: &AppHandle, hidden: bool) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_visible(!hidden);
    }
}

/// Sync the limiter check mark with settings.
pub fn refresh(app: &AppHandle) {
    let on = app
        .state::<Core>()
        .settings()
        .downloads
        .speed_limiter_enabled;
    if let Some(state) = app.try_state::<TrayState>() {
        let _ = state.limiter.set_checked(on);
    }
}

/// Show the menu in the current UI language.
pub fn relabel(app: &AppHandle) {
    if let Some(state) = app.try_state::<TrayState>() {
        for (item, text) in &state.items {
            let _ = item.set_text(tr(text, &[]));
        }
        let _ = state.limiter.set_text(tr("Speed limiter", &[]));
    }
}

/// Update icon + tooltip from aggregate progress (called on each progress event).
pub fn update(app: &AppHandle, active: usize, speed: u64, icon_changed: bool) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let tooltip = if active == 0 {
        tr("Prism — idle", &[])
    } else {
        tr(
            "Prism — {count} downloading · {speed}",
            &[
                ("count", &active.to_string()),
                ("speed", &human_speed(speed)),
            ],
        )
    };
    let _ = tray.set_tooltip(Some(tooltip));
    if !icon_changed {
        return;
    }
    let bytes = if active == 0 { ICON_IDLE } else { ICON_ACTIVE };
    if let Ok(img) = Image::from_bytes(bytes) {
        let _ = tray.set_icon(Some(img));
        let _ = tray.set_icon_as_template(cfg!(target_os = "macos"));
    }
}

fn human_speed(bps: u64) -> String {
    const UNITS: [&str; 4] = ["B/s", "KB/s", "MB/s", "GB/s"];
    let mut v = bps as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{v:.0} {}", UNITS[i])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}
