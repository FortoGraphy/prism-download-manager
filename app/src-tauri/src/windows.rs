//! Secondary windows: capture dialogs, details, completion, mini mode, drop zone.
//!
//! Every window loads the same SPA; the hash route (`#/capture/<id>`) selects the view.

use prism_core::manager::Core;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

struct Spec {
    width: f64,
    height: f64,
    always_on_top: bool,
    resizable: bool,
    title: &'static str,
    /// Smallest the user can resize to. `None` falls back to the generic
    /// `width.min(420) × height.min(240)` — fine for simple forms, but windows with a fixed
    /// footer (buttons that must not get squeezed against its own edges) or a multi-section
    /// layout (content that can overlap the footer if the window gets too short) need a real
    /// floor instead.
    min_size: Option<(f64, f64)>,
}

fn spec(kind: &str) -> Spec {
    let s = |width, height, always_on_top, resizable, title| Spec {
        width,
        height,
        always_on_top,
        resizable,
        title,
        min_size: None,
    };
    match kind {
        "capture" => s(600.0, 440.0, true, false, "Download File"),
        "batch" => s(820.0, 600.0, true, true, "Download Links"),
        "video" => s(680.0, 640.0, true, true, "Download Video"),
        "complete" => s(500.0, 300.0, true, false, "Download Complete"),
        "details" => Spec {
            min_size: Some((640.0, 480.0)),
            ..s(760.0, 600.0, false, true, "Download Details")
        },
        // The auto-opened progress popup: header + connections only, no speed chart/stats —
        // matches IDM's simpler always-on progress dialog, so it's shorter than "details".
        "progress" => Spec {
            min_size: Some((640.0, 340.0)),
            ..s(700.0, 420.0, false, true, "Downloading")
        },
        "torrent" => s(680.0, 600.0, true, true, "Add Torrent"),
        "model" => Spec {
            min_size: Some((600.0, 520.0)),
            ..s(760.0, 680.0, false, true, "Download Model")
        },
        // Convert / Make smaller: file cards, the choices and the jobs below.
        "tools" => Spec {
            min_size: Some((560.0, 540.0)),
            ..s(640.0, 600.0, false, true, "Tools")
        },
        "addurl" => s(600.0, 300.0, true, false, "Add URL"),
        "mini" => s(360.0, 220.0, true, false, "Prism Mini"),
        "dropzone" => s(112.0, 112.0, true, false, "Prism Drop Zone"),
        _ => s(720.0, 560.0, false, true, "Prism"),
    }
}

/// Windows (#92): dialogs that open where they were last closed instead of in the middle of the
/// screen. Only their position comes back: their size is the app's (a later version may change
/// it). Elsewhere they open centered as before (shown at once, they would jump).
pub const REMEMBER_POSITION: &[&str] = if cfg!(windows) {
    &[
        "capture", "batch", "video", "complete", "progress", "torrent", "model", "tools", "addurl",
    ]
} else {
    &[]
};

/// The window-state plugin keeps one entry per kind of window: `capture-<id>` → `capture`.
/// (Before, every Download Details window kept its own entry, forever.)
pub fn state_label(label: &str) -> &str {
    match label.split_once('-') {
        Some((kind, _)) if kind == "details" || REMEMBER_POSITION.contains(&kind) => kind,
        _ => label,
    }
}

/// Windows are shown as soon as they are created, not after the page's first paint:
/// * macOS: WKWebView doesn't render into a hidden window.
/// * Linux (#30): a WebKitGTK window that goes from hidden to shown sometimes stays blank on KDE
///   Plasma (#18 was the same for a window shown again from the tray). The windows are
///   transparent, so one shown before its page paints has nothing to flash.
///
/// Windows (WebView2) keeps them hidden until the first paint, so no white frame shows.
pub const SHOW_ON_CREATE: bool = cfg!(any(target_os = "macos", target_os = "linux"));

/// Labels of windows that are being built right now. Tauri checks a label before it creates the
/// window but registers it only afterwards, so two requests at the same moment both built one
/// (#37: the core's `BatchRequested` and the page's `open_window`); the second one replaced the
/// first in Tauri's list, and the first stayed on screen frozen and impossible to close.
static OPENING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Takes a label off `OPENING` when the build is over, whether it worked or not.
struct Opening(String);

impl Drop for Opening {
    fn drop(&mut self) {
        let mut opening = OPENING.lock().unwrap_or_else(|e| e.into_inner());
        opening.retain(|l| l != &self.0);
    }
}

/// Open (or focus) a window for `kind` with an optional entity id.
pub fn open(app: &AppHandle, kind: &str, id: Option<&str>) -> tauri::Result<()> {
    let label = match id {
        Some(id) => format!("{kind}-{id}"),
        None => kind.to_string(),
    };
    let Some(_opening) = claim(app, kind, &label) else {
        return Ok(());
    };
    // A `#` inside WebviewUrl::App is percent-encoded into the path, so the page is loaded
    // plainly and the hash route is set before any app script runs.
    let route = match id {
        Some(id) => format!("/{kind}/{id}"),
        None => format!("/{kind}"),
    };
    create(app, kind, &label, &route)
}

/// Brings `label`'s window forward if it exists; otherwise the right to build it, or `None` when
/// another request is building it right now (that one shows and focuses it).
fn claim(app: &AppHandle, kind: &str, label: &str) -> Option<Opening> {
    let mut opening = OPENING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(w) = app.get_webview_window(label) {
        drop(opening);
        bring_app_forward(app, kind);
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return None;
    }
    if opening.iter().any(|l| l == label) {
        return None;
    }
    opening.push(label.to_string());
    Some(Opening(label.to_string()))
}

/// The Tools window (one, whatever it was opened for): an open one gets the request as a
/// `tools-request` event and comes forward; a new one reads it from its route.
pub fn open_tools(app: &AppHandle, request: &serde_json::Value) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window("tools") {
        let _ = app.emit_to("tools", "tools-request", request);
        bring_app_forward(app, "tools");
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    let tab = match request.get("tab").and_then(|t| t.as_str()) {
        Some("shrink") => "shrink",
        _ => "convert",
    };
    let has_files = request
        .get("paths")
        .and_then(|p| p.as_array())
        .is_some_and(|p| !p.is_empty());
    let route = if has_files {
        let hex: String = request
            .to_string()
            .bytes()
            .map(|b| format!("{b:02x}"))
            .collect();
        format!("/tools/{tab}/{hex}")
    } else {
        format!("/tools/{tab}")
    };
    let Some(_opening) = claim(app, "tools", "tools") else {
        return Ok(());
    };
    create(app, "tools", "tools", &route)
}

fn create(app: &AppHandle, kind: &str, label: &str, route: &str) -> tauri::Result<()> {
    let sp = spec(kind);
    let (min_w, min_h) = sp
        .min_size
        .unwrap_or((sp.width.min(420.0), sp.height.min(240.0)));
    let init = format!("if (!location.hash) history.replaceState(null, '', '#{route}');");
    let builder = WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
        .initialization_script(&init)
        .title(sp.title)
        .inner_size(sp.width, sp.height)
        .min_inner_size(min_w, min_h)
        .resizable(sp.resizable)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(sp.always_on_top)
        .skip_taskbar(matches!(kind, "mini" | "dropzone"))
        // Windows: hidden until the page has painted (no white flash). macOS and Linux: shown
        // right away (see `SHOW_ON_CREATE`). Corner windows are positioned first and shown below.
        .visible(SHOW_ON_CREATE && !matches!(kind, "mini" | "dropzone"))
        .focused(!matches!(kind, "mini" | "dropzone"));
    // HTML5 drag & drop (links from the browser) instead of Tauri's file-only handler, except in
    // the Tools window: files dropped there need their paths, which only Tauri's handler gives.
    let builder = if kind == "tools" {
        builder
    } else {
        builder.disable_drag_drop_handler()
    };
    let builder = if matches!(kind, "mini" | "dropzone") {
        builder
    } else {
        builder.center()
    };
    let window = builder.build()?;
    if REMEMBER_POSITION.contains(&kind) {
        // Where this kind of dialog was last closed, if that monitor is still there. On the main
        // thread: the plugin holds its state's lock while it moves the window, and its own event
        // handlers take that lock on the main thread, so from here the two waited on each other.
        let w = window.clone();
        let _ = app.run_on_main_thread(move || {
            use tauri_plugin_window_state::{StateFlags, WindowExt};
            let _ = w.restore_state(StateFlags::POSITION);
        });
    }
    if matches!(kind, "mini" | "dropzone") {
        place_bottom_right(&window, kind == "dropzone");
        if SHOW_ON_CREATE {
            let _ = window.show();
        }
    }
    bring_app_forward(app, kind);
    // The page calls `show()` itself once it has rendered (no white flash). A webview that
    // never paints while hidden (WKWebView can pause animation frames for invisible windows)
    // must not leave the dialog invisible, so show it anyway after a moment.
    let fallback = window.clone();
    let what = label.to_string();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        if !fallback.is_visible().unwrap_or(true) {
            tracing::warn!("window {what} did not show itself; showing it");
            let _ = fallback.show();
            let _ = fallback.set_focus();
        }
    });
    tracing::info!("opened window {label}");
    Ok(())
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
    open(app, "details", Some(id))
}

/// The auto-opened progress popup — a distinct window kind from `open_details` so it never
/// collides with (or gets reused by) a manually-opened full details window for the same item.
pub fn open_progress(app: &AppHandle, id: &str) -> tauri::Result<()> {
    open(app, "progress", Some(id))
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

/// How long the main window waits hidden in the tray before it is closed to free its web view's
/// memory (#11: WebView2 keeps 100+ MB that downloads don't need). Opening Prism creates a new
/// one; a quick close and reopen keeps the same window.
#[cfg(not(target_os = "macos"))]
const RELEASE_HIDDEN_MAIN_AFTER: Duration = Duration::from_secs(60);

/// Set once the user opens the main window, so a newly created one shows itself after its first
/// paint (see `commands::main_window_starts_visible`).
pub static MAIN_WANTED: AtomicBool = AtomicBool::new(false);
/// A new main window is on its way; another "Open Prism" meanwhile has nothing to do.
static CREATING_MAIN: AtomicBool = AtomicBool::new(false);
/// Counts trips to the tray and reopenings: only the latest trip's timer may close the window.
#[cfg(not(target_os = "macos"))]
static TRAY_TRIP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn show_main(app: &AppHandle) {
    MAIN_WANTED.store(true, Ordering::SeqCst);
    update_dock(app, true);
    #[cfg(not(target_os = "macos"))]
    TRAY_TRIP.fetch_add(1, Ordering::SeqCst);
    if CREATING_MAIN.load(Ordering::SeqCst) {
        return;
    }
    match app.get_webview_window("main") {
        // Linux (#18): a transparent WebKitGTK window that was hidden can come back without
        // painting anything — invisible, though the desktop shows it as open (KDE Plasma). A new
        // window always paints, so a hidden main window is replaced instead of shown again.
        Some(w) if cfg!(target_os = "linux") && !w.is_visible().unwrap_or(true) => {
            let _ = w.destroy();
            create_main(app);
        }
        Some(w) => {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
        }
        None => create_main(app),
    }
}

/// A window's rectangle in physical pixels (only Windows fits windows into the work area).
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl Rect {
    fn overlap(&self, other: &Rect) -> u64 {
        let w =
            (self.x + self.width as i32).min(other.x + other.width as i32) - self.x.max(other.x);
        let h =
            (self.y + self.height as i32).min(other.y + other.height as i32) - self.y.max(other.y);
        if w > 0 && h > 0 {
            w as u64 * h as u64
        } else {
            0
        }
    }
}

/// Shrinks `window` until it fits `work` and moves it inside, so its bottom edge can't end up
/// under the taskbar, nor its top above the screen.
#[cfg_attr(not(windows), allow(dead_code))]
fn fit_into_work_area(window: Rect, work: Rect) -> Rect {
    let width = window.width.min(work.width);
    let height = window.height.min(work.height);
    let right = work.x + (work.width - width) as i32;
    let bottom = work.y + (work.height - height) as i32;
    Rect {
        x: window.x.clamp(work.x, right.max(work.x)),
        y: window.y.clamp(work.y, bottom.max(work.y)),
        width,
        height,
    }
}

/// Windows (#53): where the main window was just before it was closed to free memory, so the
/// window created in its place comes back as it was (see [`restore_main`]).
#[cfg(windows)]
static MAIN_RECT: Mutex<Option<Rect>> = Mutex::new(None);

/// Windows (#53): the window-state plugin gives the main window its saved size before the
/// window's frame is final. The size then counts a title bar's frame, the window ends up with a
/// shadow's, and it came back 30 px taller (at 100 %) on every start or reopening from the tray.
/// The frame has settled by now, so the saved rectangle is applied again, inside the work area
/// of the monitor it's mostly on. A maximized window is left alone. Idea and measurements from
/// PR #61 by karayelxyz.
pub fn restore_main(app: &AppHandle, main: &WebviewWindow) {
    #[cfg(windows)]
    {
        if main.is_maximized().unwrap_or(false) || main.is_fullscreen().unwrap_or(false) {
            return;
        }
        let remembered = MAIN_RECT.lock().unwrap_or_else(|e| e.into_inner()).take();
        let Some(saved) = remembered.or_else(|| saved_main_rect(app)) else {
            return;
        };
        // The saved position is the outer corner and the shadow sits around the content: the
        // content is what has to stay inside the work area.
        let (left, top) = match (main.outer_position(), main.inner_position()) {
            (Ok(outer), Ok(inner)) => (inner.x - outer.x, inner.y - outer.y),
            _ => (0, 0),
        };
        let content = Rect {
            x: saved.x + left,
            y: saved.y + top,
            ..saved
        };
        let Some(work) = main.available_monitors().ok().and_then(|monitors| {
            monitors
                .iter()
                .map(|m| {
                    let a = m.work_area();
                    Rect {
                        x: a.position.x,
                        y: a.position.y,
                        width: a.size.width,
                        height: a.size.height,
                    }
                })
                .max_by_key(|work| content.overlap(work))
                .filter(|work| content.overlap(work) > 0)
        }) else {
            return;
        };
        let fitted = fit_into_work_area(content, work);
        let _ = main.set_position(PhysicalPosition::new(fitted.x - left, fitted.y - top));
        let _ = main.set_size(tauri::PhysicalSize::new(fitted.width, fitted.height));
    }
    #[cfg(not(windows))]
    let _ = (app, main);
}

/// Windows (#53): remembers the main window before it's closed to free memory.
#[cfg(not(target_os = "macos"))]
fn remember_main(window: &WebviewWindow) {
    #[cfg(windows)]
    {
        if window.is_maximized().unwrap_or(false) || window.is_fullscreen().unwrap_or(false) {
            return;
        }
        if let (Ok(position), Ok(size)) = (window.outer_position(), window.inner_size()) {
            *MAIN_RECT.lock().unwrap_or_else(|e| e.into_inner()) = Some(Rect {
                x: position.x,
                y: position.y,
                width: size.width,
                height: size.height,
            });
        }
    }
    #[cfg(not(windows))]
    let _ = window;
}

/// The main window's size and position as the plugin last saved them (its cache is private, so
/// from the file it writes, in its own format). Nothing when there's none or it looks wrong.
#[cfg(windows)]
fn saved_main_rect(app: &AppHandle) -> Option<Rect> {
    let path = app
        .path()
        .app_config_dir()
        .ok()?
        .join(tauri_plugin_window_state::DEFAULT_FILENAME);
    let state: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let main = state.get("main")?;
    if main.get("maximized").and_then(|v| v.as_bool()) == Some(true) {
        return None;
    }
    let number = |key: &str| main.get(key).and_then(|v| v.as_i64());
    let (width, height) = (number("width")?, number("height")?);
    if width <= 0 || height <= 0 {
        return None;
    }
    Some(Rect {
        x: i32::try_from(number("x")?).ok()?,
        y: i32::try_from(number("y")?).ok()?,
        width: u32::try_from(width).ok()?,
        height: u32::try_from(height).ok()?,
    })
}

/// Creates the main window again from its configuration (it was closed while in the tray).
fn create_main(app: &AppHandle) {
    if CREATING_MAIN.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    // Off the caller's thread: building a window inside an event handler can deadlock WebView2.
    std::thread::spawn(move || {
        // A window that is being destroyed keeps its label for a moment.
        for _ in 0..60 {
            if app.get_webview_window("main").is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let built = (|| -> tauri::Result<WebviewWindow> {
            let config = app
                .config()
                .app
                .windows
                .iter()
                .find(|c| c.label == "main")
                .cloned()
                .ok_or(tauri::Error::WindowNotFound)?;
            WebviewWindowBuilder::from_config(&app, &config)?.build()
        })();
        CREATING_MAIN.store(false, Ordering::SeqCst);
        match built {
            Ok(window) => {
                tracing::info!("opened the main window again");
                crate::apply_window_material(&window, &app.state::<Core>().settings());
                restore_main(&app, &window);
                // macOS and Linux show it now (see `SHOW_ON_CREATE`).
                if SHOW_ON_CREATE {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
                // The page shows the window after its first paint; this is only a fallback.
                std::thread::sleep(Duration::from_millis(1500));
                if !window.is_visible().unwrap_or(true) {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            Err(e) => tracing::error!("could not open the main window: {e}"),
        }
    });
}

/// macOS (#34): the Dock icon as last set — 0 shown (the default), 1 hidden.
#[cfg(target_os = "macos")]
static DOCK_HIDDEN: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// macOS (#34): with "Hide the Dock icon" on, Prism has no Dock icon while it waits in the menu
/// bar; while the main window is open the icon is there, so Cmd+Tab and the menu bar work.
pub fn update_dock(app: &AppHandle, main_open: bool) {
    #[cfg(target_os = "macos")]
    {
        let hide = !main_open && app.state::<Core>().settings().general.hide_dock_icon;
        if hide {
            DOCK_HIDDEN.store(1, Ordering::SeqCst);
            let _ = app.set_dock_visibility(false);
            // macOS ignores hiding within a second of showing (it would leave a stray icon), so
            // a window closed right after it opened is checked again.
            let app = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(1200));
                if DOCK_HIDDEN.load(Ordering::SeqCst) == 1 && !main_is_open(&app) {
                    let _ = app.set_dock_visibility(false);
                }
            });
        } else if DOCK_HIDDEN.swap(0, Ordering::SeqCst) == 1 {
            let _ = app.set_dock_visibility(true);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, main_open);
}

/// Whether the main window is on screen.
pub fn main_is_open(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .is_some_and(|w| w.is_visible().unwrap_or(false))
}

/// The main window went to the tray, or Prism started there: after a while the hidden window is
/// closed to free its memory (#11), and `show_main` creates it again. macOS keeps it.
pub fn release_main_later(app: &AppHandle) {
    #[cfg(not(target_os = "macos"))]
    {
        let trip = TRAY_TRIP.fetch_add(1, Ordering::SeqCst) + 1;
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(RELEASE_HIDDEN_MAIN_AFTER);
            if TRAY_TRIP.load(Ordering::SeqCst) != trip || CREATING_MAIN.load(Ordering::SeqCst) {
                return;
            }
            if let Some(w) = app.get_webview_window("main")
                && !w.is_visible().unwrap_or(true)
            {
                tracing::info!("closed the hidden main window to free its memory");
                remember_main(&w);
                let _ = w.destroy();
            }
        });
    }
    #[cfg(target_os = "macos")]
    let _ = app;
}

#[cfg(test)]
mod tests {
    use super::{Rect, fit_into_work_area};

    const WORK: Rect = Rect {
        x: 0,
        y: 0,
        width: 2560,
        height: 1392,
    };

    #[test]
    fn a_window_that_fits_is_left_alone() {
        let window = Rect {
            x: 100,
            y: 100,
            width: 1320,
            height: 1000,
        };
        assert_eq!(fit_into_work_area(window, WORK), window);
    }

    #[test]
    fn a_window_that_grew_is_pulled_back_inside() {
        // Taller than the work area (#53): shrunk, its bottom edge on the taskbar line.
        let grown = Rect {
            x: 290,
            y: 149,
            width: 1980,
            height: 1703,
        };
        assert_eq!(
            fit_into_work_area(grown, WORK),
            Rect {
                x: 290,
                y: 0,
                width: 1980,
                height: 1392
            }
        );
        // Fits, but hangs under the taskbar: moved up.
        let low = Rect {
            x: 100,
            y: 900,
            width: 1320,
            height: 1000,
        };
        assert_eq!(fit_into_work_area(low, WORK).y, 392);
        // Wider than the work area and left of it.
        let wide = Rect {
            x: -500,
            y: -40,
            width: 3000,
            height: 700,
        };
        assert_eq!(
            fit_into_work_area(wide, WORK),
            Rect {
                x: 0,
                y: 0,
                width: 2560,
                height: 700
            }
        );
    }

    #[test]
    fn one_saved_state_per_kind_of_window() {
        use super::state_label;
        assert_eq!(state_label("main"), "main");
        assert_eq!(state_label("details-82a9a65b"), "details");
        assert_eq!(state_label("mini"), "mini");
        if cfg!(windows) {
            assert_eq!(state_label("capture-1f2e"), "capture");
            assert_eq!(state_label("video-1f2e"), "video");
            assert_eq!(state_label("addurl"), "addurl");
        } else {
            assert_eq!(state_label("capture-1f2e"), "capture-1f2e");
        }
    }

    #[test]
    fn the_monitor_a_window_is_mostly_on_wins() {
        let left = Rect {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1040,
        };
        // Mostly on the right monitor, a little on the left one.
        let window = Rect {
            x: -100,
            y: 50,
            width: 1300,
            height: 800,
        };
        assert!(window.overlap(&WORK) > window.overlap(&left));
        assert_eq!(window.overlap(&Rect { x: 5000, ..WORK }), 0);
    }
}
