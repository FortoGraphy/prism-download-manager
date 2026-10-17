//! Secondary windows: capture dialogs, details, completion, mini mode, drop zone.
//!
//! Every window loads the same SPA; the hash route (`#/capture/<id>`) selects the view.

use tauri::{
    AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

struct Spec {
    width: f64,
    height: f64,
    always_on_top: bool,
    resizable: bool,
    title: &'static str,
}

fn spec(kind: &str) -> Spec {
    let s = |width, height, always_on_top, resizable, title| Spec {
        width,
        height,
        always_on_top,
        resizable,
        title,
    };
    match kind {
        "capture" => s(600.0, 440.0, true, false, "Download File"),
        "batch" => s(820.0, 600.0, true, true, "Download Links"),
        "video" => s(680.0, 640.0, true, true, "Download Video"),
        "complete" => s(500.0, 300.0, true, false, "Download Complete"),
        "details" => s(760.0, 600.0, false, true, "Download Details"),
        // The auto-opened progress popup: header + connections only, no speed chart/stats —
        // matches IDM's simpler always-on progress dialog, so it's shorter than "details".
        "progress" => s(620.0, 420.0, false, true, "Downloading"),
        "torrent" => s(680.0, 600.0, true, true, "Add Torrent"),
        "addurl" => s(600.0, 300.0, true, false, "Add URL"),
        "mini" => s(360.0, 220.0, true, false, "Prism Mini"),
        "dropzone" => s(112.0, 112.0, true, false, "Prism Drop Zone"),
        _ => s(720.0, 560.0, false, true, "Prism"),
    }
}

/// Open (or focus) a window for `kind` with an optional entity id.
pub fn open(app: &AppHandle, kind: &str, id: Option<&str>) -> tauri::Result<WebviewWindow> {
    let label = match id {
        Some(id) => format!("{kind}-{id}"),
        None => kind.to_string(),
    };
    if let Some(w) = app.get_webview_window(&label) {
        bring_app_forward(app, kind);
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(w);
    }
    // A `#` inside WebviewUrl::App is percent-encoded into the path, so the page is loaded
    // plainly and the hash route is set before any app script runs.
    let route = match id {
        Some(id) => format!("/{kind}/{id}"),
        None => format!("/{kind}"),
    };
    let sp = spec(kind);
    let init = format!("if (!location.hash) history.replaceState(null, '', '#{route}');");
    let builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
        .initialization_script(&init)
        .title(sp.title)
        .inner_size(sp.width, sp.height)
        .min_inner_size(sp.width.min(420.0), sp.height.min(240.0))
        .resizable(sp.resizable)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        // A solid fallback behind the transparent/glass surface: without this, a region the
        // webview hasn't painted yet (or a compositor hiccup during resize/creation) can show
        // whatever was on screen behind the window instead of this app, looking like ghosted
        // page content bleeding through the dialog's edges.
        .background_color(tauri::window::Color(7, 8, 11, 255))
        .always_on_top(sp.always_on_top)
        .skip_taskbar(matches!(kind, "mini" | "dropzone"))
        // Windows/Linux: hidden until the page has painted (no white flash). macOS: WKWebView
        // doesn't render into a hidden window, so the transparent window is shown right away.
        // (Corner windows are positioned first and shown below.)
        .visible(cfg!(target_os = "macos") && !matches!(kind, "mini" | "dropzone"))
        // HTML5 drag & drop (links from the browser) instead of Tauri's file-only handler.
        .disable_drag_drop_handler()
        .focused(!matches!(kind, "mini" | "dropzone"));
    let builder = if matches!(kind, "mini" | "dropzone") {
        builder
    } else {
        builder.center()
    };
    let window = builder.build()?;
    if matches!(kind, "mini" | "dropzone") {
        place_bottom_right(&window, kind == "dropzone");
        #[cfg(target_os = "macos")]
        let _ = window.show();
    }
    bring_app_forward(app, kind);
    // The page calls `show()` itself once it has rendered (no white flash). A webview that
    // never paints while hidden (WKWebView can pause animation frames for invisible windows)
    // must not leave the dialog invisible, so show it anyway after a moment.
    let fallback = window.clone();
    let what = label.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        if !fallback.is_visible().unwrap_or(true) {
            tracing::warn!("window {what} did not show itself; showing it");
            let _ = fallback.show();
            let _ = fallback.set_focus();
        }
    });
    tracing::info!("opened window {label}");
    Ok(window)
}

/// On macOS a background app's new window opens behind the browser; activate the app so
/// dialogs (Download File, video picker, …) come to the front.
fn bring_app_forward(app: &AppHandle, kind: &str) {
    #[cfg(target_os = "macos")]
    if !matches!(kind, "mini" | "dropzone") {
        let _ = app.show();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, kind);
}

fn place_bottom_right(window: &WebviewWindow, higher: bool) {
    if let (Ok(Some(monitor)), Ok(size)) = (window.current_monitor(), window.outer_size()) {
        let area = monitor.work_area();
        let margin = (24.0 * monitor.scale_factor()) as i32;
        let x = area.position.x + area.size.width as i32 - size.width as i32 - margin;
        let lift = if higher {
            (260.0 * monitor.scale_factor()) as i32
        } else {
            0
        };
        let y = area.position.y + area.size.height as i32 - size.height as i32 - margin - lift;
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
}

pub fn open_details(app: &AppHandle, id: &str) -> tauri::Result<()> {
    open(app, "details", Some(id)).map(|_| ())
}

/// The auto-opened progress popup — a distinct window kind from `open_details` so it never
/// collides with (or gets reused by) a manually-opened full details window for the same item.
pub fn open_progress(app: &AppHandle, id: &str) -> tauri::Result<()> {
    open(app, "progress", Some(id)).map(|_| ())
}

/// Closes the auto-opened progress popup for `id`, if one is open (used when the download
/// finishes — the completion dialog takes over, so the simpler popup doesn't need to linger).
pub fn close_progress(app: &AppHandle, id: &str) {
    if let Some(w) = app.get_webview_window(&format!("progress-{id}")) {
        let _ = w.close();
    }
}

fn toggle(app: &AppHandle, kind: &str) -> tauri::Result<bool> {
    if let Some(w) = app.get_webview_window(kind) {
        w.close()?;
        Ok(false)
    } else {
        open(app, kind, None)?;
        Ok(true)
    }
}

pub fn toggle_mini(app: &AppHandle) -> tauri::Result<bool> {
    toggle(app, "mini")
}

pub fn toggle_dropzone(app: &AppHandle) -> tauri::Result<bool> {
    toggle(app, "dropzone")
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}
