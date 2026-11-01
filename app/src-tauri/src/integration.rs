//! Browser integration: native messaging host registration and status.

use prism_native_host::registration::{self, Browser, Locations, RegisterOptions};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// The host executable shipped next to the app binary.
pub fn bundled_host_path() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    let name = if cfg!(windows) {
        "prism-native-host.exe"
    } else {
        "prism-native-host"
    };
    exe.parent()
        .map(|d| d.join(name))
        .unwrap_or_else(|| PathBuf::from(name))
}

/// Host path that browsers should launch. Inside an AppImage the mount point changes on
/// every run, so the host is copied to a stable location in the data dir and the
/// AppImage path is recorded in `<data dir>/app-path` for the host to start the app.
pub fn host_path() -> PathBuf {
    let bundled = bundled_host_path();
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        let data = prism_core::paths::data_dir();
        let stable = data.join("bin").join("prism-native-host");
        let copy = || -> std::io::Result<()> {
            std::fs::create_dir_all(data.join("bin"))?;
            let needs_copy = std::fs::metadata(&stable).map(|m| m.len()).ok()
                != std::fs::metadata(&bundled).map(|m| m.len()).ok();
            if needs_copy {
                let tmp = stable.with_extension("tmp");
                std::fs::copy(&bundled, &tmp)?;
                std::fs::rename(&tmp, &stable)?;
            }
            std::fs::write(data.join("app-path"), appimage.to_string_lossy().as_bytes())
        };
        match copy() {
            Ok(()) => return stable,
            Err(e) => tracing::warn!("could not install stable native host copy: {e}"),
        }
    }
    bundled
}

/// Register the host for every supported browser (idempotent).
pub fn register() -> Result<serde_json::Value, String> {
    let host = host_path();
    if !host.exists() {
        return Err(format!("Native host not found at {}", host.display()));
    }
    registration::register(&RegisterOptions::new(&host)).map_err(|e| e.to_string())?;
    Ok(status())
}

pub fn status() -> serde_json::Value {
    let host = host_path();
    let browsers = registration::status(&Browser::ALL, &host, &Locations::default());
    serde_json::json!({
        "hostPath": host,
        "hostExists": host.exists(),
        "browsers": browsers,
        "extensionId": prism_native_host::CHROME_EXTENSION_ID,
        "firefoxExtensionId": prism_native_host::FIREFOX_EXTENSION_ID,
    })
}

/// Folder with the unpacked extension builds (for "Load unpacked" instructions).
pub fn extension_dir(app: &AppHandle) -> Option<PathBuf> {
    let res = app.path().resource_dir().ok()?.join("extension");
    if res.exists() {
        return Some(res);
    }
    // Development checkout.
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../extension/build");
    dev.exists().then(|| dev.canonicalize().unwrap_or(dev))
}

/// The folder a user picks in the browser's "Load unpacked" dialog (`.../extension/chromium`,
/// or the Firefox build), as a plain path that can be pasted into a file dialog.
pub fn extension_folder(app: &AppHandle, firefox: bool) -> Option<PathBuf> {
    let sub = if firefox { "firefox" } else { "chromium" };
    let dir = extension_dir(app)?.join(sub);
    let dir = match extension_copy_root() {
        Some(root) => {
            let copy = root.join(sub);
            match sync_dir(&dir, &copy) {
                Ok(()) => copy,
                Err(e) => {
                    tracing::warn!("could not copy the extension to {}: {e}", copy.display());
                    dir
                }
            }
        }
        None => dir,
    };
    let s = dir.to_string_lossy();
    // `canonicalize` on Windows yields `\?\C:\...`, which file dialogs don't accept.
    Some(PathBuf::from(s.strip_prefix(r"\?\").unwrap_or(&s)))
}

/// Where the browser loads the extension from when not straight from the app's resources:
/// * macOS: a folder inside Prism.app can't be picked comfortably in the browser's file dialog;
/// * AppImage: the resources sit under the AppImage's mount point (`/tmp/.mount_XXXXXX`), which
///   changes on every run, so the browser would lose the extension once Prism restarts.
///
/// Both keep a copy in the data dir that the app refreshes when an update changes it.
fn extension_copy_root() -> Option<PathBuf> {
    let copy = cfg!(target_os = "macos")
        || (cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_some());
    copy.then(|| prism_core::paths::data_dir().join("extension"))
}

/// Refresh the extension copies that exist (see `extension_copy_root`) on every start, so an
/// update brings the new extension along (the browser loads it on its next start).
pub fn refresh_extension_copies(app: &AppHandle) {
    let Some(root) = extension_copy_root() else {
        return;
    };
    for firefox in [false, true] {
        let sub = if firefox { "firefox" } else { "chromium" };
        if root.join(sub).exists() {
            let _ = extension_folder(app, firefox);
        }
    }
}

/// Starts (or focuses) the browser with a new window. Browsers refuse to open their internal
/// pages (`chrome://extensions`) from the command line, so the UI copies that address for the
/// user to paste.
pub fn open_browser(browser: &str) -> Result<(), String> {
    launch(browser, None).map_err(|e| format!("Could not start the browser: {e}"))
}

/// Prism's extension in the Chrome Web Store (Chrome, Edge, Brave, Opera, Vivaldi, Chromium).
pub const EXTENSION_STORE_URL: &str = "https://chromewebstore.google.com/detail/prism-download-manager/mmcdeiglkbfbicdjboocmhehodogilfl";

/// Prism's add-on in Firefox Add-ons (Firefox, Zen, Waterfox).
pub const FIREFOX_ADDON_URL: &str =
    "https://addons.mozilla.org/firefox/addon/prism-download-manager/";

/// Where `browser` gets Prism's extension: Firefox Add-ons for Firefox, Zen and Waterfox, else the Chrome
/// Web Store.
pub fn store_url(browser: &str) -> &'static str {
    if matches!(browser, "firefox" | "zen" | "waterfox") {
        FIREFOX_ADDON_URL
    } else {
        EXTENSION_STORE_URL
    }
}

/// Opens the extension's store page in `browser`.
pub fn open_extension_store(browser: &str) -> Result<(), String> {
    launch(browser, Some(store_url(browser)))
        .map_err(|e| format!("Could not start the browser: {e}"))
}

#[cfg(windows)]
fn launch(browser: &str, url: Option<&str>) -> std::io::Result<()> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    // Browsers register their executable under "App Paths" (what the Run dialog uses).
    let app_path = |exe: &str| {
        let key = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{exe}");
        [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE]
            .into_iter()
            .find_map(|root| {
                let v: String = RegKey::predef(root)
                    .open_subkey(&key)
                    .ok()?
                    .get_value("")
                    .ok()?;
                let p = PathBuf::from(v.trim_matches('"'));
                p.exists().then_some(p)
            })
    };
    let under = |var: &str, rel: &str| {
        std::env::var_os(var)
            .map(|d| PathBuf::from(d).join(rel))
            .filter(|p| p.exists())
    };
    let path = match browser {
        // Helium registers no App Paths (its chrome.exe would clash with Chrome's): it installs
        // to imput\Helium\Application (imputnet/helium-windows installer/helium.nsi).
        "helium" => under("LOCALAPPDATA", r"imput\Helium\Application\chrome.exe")
            .or_else(|| under("ProgramFiles", r"imput\Helium\Application\chrome.exe")),
        "zen" => app_path("zen.exe").or_else(|| under("ProgramFiles", r"Zen Browser\zen.exe")),
        "edge" => app_path("msedge.exe"),
        "brave" => app_path("brave.exe"),
        "vivaldi" => app_path("vivaldi.exe"),
        "opera" => app_path("opera.exe"),
        "firefox" => app_path("firefox.exe"),
        "waterfox" => app_path("waterfox.exe")
            .or_else(|| under("ProgramFiles", r"Waterfox\waterfox.exe")),
        _ => app_path("chrome.exe"),
    }
    .ok_or_else(|| std::io::Error::other(format!("{browser} is not installed")))?;
    std::process::Command::new(path)
        .args(url)
        .spawn()
        .map(|_| ())
}

#[cfg(target_os = "macos")]
fn launch(browser: &str, url: Option<&str>) -> std::io::Result<()> {
    let apps: &[&str] = match browser {
        "edge" => &["Microsoft Edge"],
        "brave" => &["Brave Browser"],
        "vivaldi" => &["Vivaldi"],
        "opera" => &["Opera"],
        "chromium" => &["Chromium"],
        "helium" => &["Helium"],
        "firefox" => &["Firefox"],
        "waterfox" => &["Waterfox"],
        // Zen.app (older releases: "Zen Browser.app").
        "zen" => &["Zen", "Zen Browser"],
        _ => &["Google Chrome"],
    };
    for &app in apps {
        let status = std::process::Command::new("open")
            .args(["-a", app])
            .args(url)
            .status()?;
        if status.success() {
            return Ok(());
        }
    }
    Err(std::io::Error::other(format!("{browser} not found")))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn launch(browser: &str, url: Option<&str>) -> std::io::Result<()> {
    let candidates: &[&str] = match browser {
        "edge" => &["microsoft-edge", "microsoft-edge-stable"],
        "brave" => &["brave-browser", "brave"],
        "vivaldi" => &["vivaldi", "vivaldi-stable"],
        "opera" => &["opera"],
        "chromium" => &["chromium", "chromium-browser"],
        "helium" => &["helium", "helium-browser"],
        "firefox" => &["firefox"],
        "waterfox" => &["waterfox", "waterfox-g"],
        "zen" => &["zen-browser", "zen"],
        _ => &["google-chrome", "google-chrome-stable"],
    };
    let mut last = std::io::Error::from(std::io::ErrorKind::NotFound);
    for exe in candidates {
        match std::process::Command::new(exe)
            .args(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(mut child) => {
                // Reap it (a browser that hands the URL to a running instance exits at once).
                std::thread::spawn(move || child.wait());
                return Ok(());
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Mirrors `src` into `dst` when the extension build differs (new app version).
fn sync_dir(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    // The start-up refresh and the setup screen may sync at the same time.
    static SYNC: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = SYNC.lock().unwrap_or_else(|e| e.into_inner());
    let stamp = |d: &std::path::Path| {
        ["manifest.json", "background.js"].map(|f| std::fs::read(d.join(f)).ok())
    };
    let current = stamp(src);
    if current[0].is_some() && current == stamp(dst) {
        return Ok(());
    }
    let tmp = dst.with_extension("tmp");
    let _ = std::fs::remove_dir_all(&tmp);
    copy_dir(src, &tmp)?;
    let _ = std::fs::remove_dir_all(dst);
    std::fs::rename(&tmp, dst)
}

fn copy_dir(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{EXTENSION_STORE_URL, FIREFOX_ADDON_URL, store_url, sync_dir};
    use std::fs;
    use std::path::Path;

    #[test]
    fn firefox_and_zen_get_firefox_add_ons() {
        assert_eq!(store_url("firefox"), FIREFOX_ADDON_URL);
        assert_eq!(store_url("zen"), FIREFOX_ADDON_URL);
        assert_eq!(store_url("waterfox"), FIREFOX_ADDON_URL);
        for b in [
            "chrome", "edge", "brave", "opera", "vivaldi", "helium", "chromium",
        ] {
            assert_eq!(store_url(b), EXTENSION_STORE_URL, "{b}");
        }
    }

    fn write_build(dir: &Path, version: &str, icon: &str) {
        fs::create_dir_all(dir.join("icons")).unwrap();
        fs::write(
            dir.join("manifest.json"),
            format!(r#"{{"version":"{version}"}}"#),
        )
        .unwrap();
        fs::write(dir.join("background.js"), format!("// {version}")).unwrap();
        fs::write(dir.join("icons").join(icon), "png").unwrap();
    }

    fn version(dir: &Path) -> String {
        fs::read_to_string(dir.join("manifest.json")).unwrap()
    }

    /// An AppImage is mounted at a new `/tmp/.mount_XXXXXX` on every run and unmounted on exit.
    /// The folder the browser loads the extension from has to outlive the mount, stay put
    /// across runs and pick up the extension of an updated AppImage.
    #[test]
    fn extension_copy_outlives_the_mount_and_follows_updates() {
        let tmp = tempfile::tempdir().unwrap();
        let mount = |id: &str| {
            tmp.path()
                .join(format!(".mount_{id}/usr/lib/Prism/extension/chromium"))
        };
        let copy = tmp.path().join("data/extension/chromium");

        write_build(&mount("aaaaaa"), "1.0.4", "old.png");
        sync_dir(&mount("aaaaaa"), &copy).unwrap();
        fs::remove_dir_all(tmp.path().join(".mount_aaaaaa")).unwrap();
        assert_eq!(version(&copy), r#"{"version":"1.0.4"}"#);
        assert!(copy.join("background.js").exists() && copy.join("icons/old.png").exists());

        // Next run of the same AppImage: the copy is left as it is.
        write_build(&mount("bbbbbb"), "1.0.4", "old.png");
        fs::write(copy.join("untouched"), "").unwrap();
        sync_dir(&mount("bbbbbb"), &copy).unwrap();
        assert!(copy.join("untouched").exists());

        // Updated AppImage: the copy becomes the new build, without the files it dropped.
        write_build(&mount("cccccc"), "1.0.5", "new.png");
        sync_dir(&mount("cccccc"), &copy).unwrap();
        assert_eq!(version(&copy), r#"{"version":"1.0.5"}"#);
        assert!(copy.join("icons/new.png").exists());
        assert!(!copy.join("icons/old.png").exists() && !copy.join("untouched").exists());
        assert!(!copy.with_extension("tmp").exists());
    }
}
