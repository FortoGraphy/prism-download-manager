//! Native messaging host registration for every supported browser.
//!
//! Reused by the app core ("repair browser integration") — keep the API small:
//! [`register`], [`unregister`], [`status`]. All functions are idempotent and
//! fast (files/registry values are only written when they differ), so the app
//! can call [`register`] on every start to self-heal.
//!
//! Where things go:
//!
//! * **Windows** — two manifest files in `<data dir>/native-messaging/`
//!   (`dev.prism.host.chromium.json` with `allowed_origins`,
//!   `dev.prism.host.firefox.json` with `allowed_extensions`), and per-user
//!   registry keys `HKCU\Software\<vendor>\NativeMessagingHosts\dev.prism.host`
//!   whose default value is the manifest path:
//!   Chrome/Brave/Vivaldi/Opera → `Google\Chrome`, Chromium and Chrome for Testing →
//!   `Chromium`, Edge → `Microsoft\Edge`, Firefox → `Mozilla`.
//! * **Linux** — `dev.prism.host.json` in each browser's `NativeMessagingHosts`
//!   dir under `$XDG_CONFIG_HOME` (default `~/.config`), Firefox in
//!   `~/.mozilla/native-messaging-hosts`. Flatpak browsers are detected and
//!   reported as unsupported.
//! * **macOS** — `~/Library/Application Support/<browser>/NativeMessagingHosts`,
//!   Firefox in `~/Library/Application Support/Mozilla/NativeMessagingHosts`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use crate::{CHROME_EXTENSION_ID, HOST_DESCRIPTION, HOST_NAME, paths};

/// A browser (family member) we can register the host for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Browser {
    Chrome,
    ChromeForTesting,
    Chromium,
    Edge,
    Brave,
    Vivaldi,
    Opera,
    /// Helium (imput), Chromium-based.
    Helium,
    Firefox,
    /// Zen Browser, Firefox-based: Firefox's extension and Firefox's native messaging locations.
    Zen,
}

impl Browser {
    pub const ALL: [Browser; 10] = [
        Browser::Chrome,
        Browser::Edge,
        Browser::Brave,
        Browser::Vivaldi,
        Browser::Opera,
        Browser::Helium,
        Browser::Chromium,
        Browser::ChromeForTesting,
        Browser::Firefox,
        Browser::Zen,
    ];

    /// Stable lowercase id (`chrome`, `chrome-for-testing`, ...).
    pub fn id(self) -> &'static str {
        match self {
            Browser::Chrome => "chrome",
            Browser::ChromeForTesting => "chrome-for-testing",
            Browser::Chromium => "chromium",
            Browser::Edge => "edge",
            Browser::Brave => "brave",
            Browser::Vivaldi => "vivaldi",
            Browser::Opera => "opera",
            Browser::Helium => "helium",
            Browser::Firefox => "firefox",
            Browser::Zen => "zen",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Browser::Chrome => "Google Chrome",
            Browser::ChromeForTesting => "Chrome for Testing",
            Browser::Chromium => "Chromium",
            Browser::Edge => "Microsoft Edge",
            Browser::Brave => "Brave",
            Browser::Vivaldi => "Vivaldi",
            Browser::Opera => "Opera",
            Browser::Helium => "Helium",
            Browser::Firefox => "Mozilla Firefox",
            Browser::Zen => "Zen Browser",
        }
    }

    /// Firefox-family: Firefox's manifest format and extension ID.
    pub fn is_firefox(self) -> bool {
        matches!(self, Browser::Firefox | Browser::Zen)
    }
}

impl fmt::Display for Browser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

impl FromStr for Browser {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim().to_ascii_lowercase();
        Ok(match s.as_str() {
            "chrome" | "google-chrome" => Browser::Chrome,
            "chrome-for-testing" | "cft" => Browser::ChromeForTesting,
            "chromium" => Browser::Chromium,
            "edge" | "msedge" | "microsoft-edge" => Browser::Edge,
            "brave" => Browser::Brave,
            "vivaldi" => Browser::Vivaldi,
            "opera" => Browser::Opera,
            "helium" => Browser::Helium,
            "firefox" | "mozilla" => Browser::Firefox,
            "zen" | "zen-browser" => Browser::Zen,
            _ => return Err(format!("unknown browser: {s}")),
        })
    }
}

/// Where registration reads/writes. `Default` = the real per-user locations.
#[derive(Debug, Clone, Default)]
pub struct Locations {
    /// Windows: directory for the manifest files (default `<data dir>/native-messaging`).
    pub manifest_dir: Option<PathBuf>,
    /// Testing: replaces the home directory used for Linux/macOS browser dirs.
    pub home: Option<PathBuf>,
    /// Testing (Windows): replaces `Software` as the HKCU root of all keys,
    /// e.g. `Software\PrismTest`, so tests never touch real browser keys.
    pub registry_root: Option<String>,
}

/// Options for [`register`].
#[derive(Debug, Clone)]
pub struct RegisterOptions {
    /// Browsers to register; empty = all supported (see [`Browser::ALL`]).
    pub browsers: Vec<Browser>,
    /// Absolute path of the `prism-native-host` executable the manifest points to.
    pub host_path: PathBuf,
    /// Chromium extension IDs allowed to connect (default: [`CHROME_EXTENSION_ID`]).
    pub chrome_extension_ids: Vec<String>,
    /// Firefox extension IDs allowed to connect (default: [`crate::firefox_extension_ids`]).
    pub firefox_extension_ids: Vec<String>,
    pub locations: Locations,
}

impl RegisterOptions {
    /// All browsers, default extension IDs, real locations.
    pub fn new(host_path: impl Into<PathBuf>) -> Self {
        RegisterOptions {
            browsers: Vec::new(),
            host_path: host_path.into(),
            chrome_extension_ids: vec![CHROME_EXTENSION_ID.to_string()],
            firefox_extension_ids: crate::firefox_extension_ids(),
            locations: Locations::default(),
        }
    }
}

/// Per-browser registration state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStatus {
    pub browser: Browser,
    pub name: String,
    /// The browser (its profile dir) was detected for this user.
    pub installed: bool,
    /// `false` for setups native messaging can't reach (e.g. Flatpak browsers).
    pub supported: bool,
    /// A valid `dev.prism.host` manifest is registered for this browser.
    pub registered: bool,
    pub manifest_path: Option<PathBuf>,
    /// The registered manifest's `path` is the host executable we compared against.
    pub points_to_this_host: bool,
    /// Origins (Chromium) or extension IDs (Firefox) the manifest allows.
    pub allowed: Vec<String>,
    /// Registry key (Windows) or directory (Linux/macOS) consulted.
    pub location: String,
    pub note: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistrationError {
    #[error("cannot determine the {0} directory")]
    NoDir(&'static str),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("registry key {key}: {source}")]
    Registry { key: String, source: std::io::Error },
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> RegistrationError + '_ {
    move |source| RegistrationError::Io {
        path: path.to_path_buf(),
        source,
    }
}

// ---------------------------------------------------------------------------
// Manifest generation (pure)

/// Chromium-family manifest JSON.
pub fn chromium_manifest(host_path: &Path, extension_ids: &[String]) -> Value {
    let origins: Vec<String> = extension_ids.iter().map(|id| chrome_origin(id)).collect();
    json!({
        "name": HOST_NAME,
        "description": HOST_DESCRIPTION,
        "path": host_path.to_string_lossy(),
        "type": "stdio",
        "allowed_origins": origins,
    })
}

/// Firefox manifest JSON.
pub fn firefox_manifest(host_path: &Path, extension_ids: &[String]) -> Value {
    json!({
        "name": HOST_NAME,
        "description": HOST_DESCRIPTION,
        "path": host_path.to_string_lossy(),
        "type": "stdio",
        "allowed_extensions": extension_ids,
    })
}

/// `abc` → `chrome-extension://abc/` (already-formed origins are kept).
pub fn chrome_origin(id: &str) -> String {
    if id.starts_with("chrome-extension://") {
        if id.ends_with('/') {
            id.to_string()
        } else {
            format!("{id}/")
        }
    } else {
        format!("chrome-extension://{id}/")
    }
}

fn manifest_for(browser: Browser, opts: &RegisterOptions) -> Value {
    if browser.is_firefox() {
        firefox_manifest(&opts.host_path, &opts.firefox_extension_ids)
    } else {
        chromium_manifest(&opts.host_path, &opts.chrome_extension_ids)
    }
}

fn manifest_bytes(v: &Value) -> Vec<u8> {
    let mut s = serde_json::to_string_pretty(v).expect("manifest serializes");
    s.push('\n');
    s.into_bytes()
}

/// Writes `bytes` to `path` unless it already has exactly that content.
fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<(), RegistrationError> {
    if fs::read(path).map(|cur| cur == bytes).unwrap_or(false) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io_err(dir))?;
    }
    fs::write(path, bytes).map_err(io_err(path))
}

/// Parsed view of an existing manifest.
#[derive(Debug, Default)]
struct ManifestInfo {
    valid: bool,
    host_path: Option<PathBuf>,
    allowed: Vec<String>,
}

fn read_manifest(path: &Path) -> ManifestInfo {
    let Ok(bytes) = fs::read(path) else {
        return ManifestInfo::default();
    };
    let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
        return ManifestInfo::default();
    };
    let name_ok = v.get("name").and_then(Value::as_str) == Some(HOST_NAME);
    let type_ok = v.get("type").and_then(Value::as_str) == Some("stdio");
    let mut host = v.get("path").and_then(Value::as_str).map(PathBuf::from);
    // Windows allows a path relative to the manifest.
    if let (Some(h), Some(dir)) = (&host, path.parent())
        && h.is_relative()
    {
        host = Some(dir.join(h));
    }
    let allowed = v
        .get("allowed_origins")
        .or_else(|| v.get("allowed_extensions"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    ManifestInfo {
        valid: name_ok && type_ok && host.is_some(),
        host_path: host,
        allowed,
    }
}

/// Compares two executable paths (canonicalized when possible; case-insensitive on Windows).
pub fn same_path(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let (a, b) = (canon(a), canon(b));
    if cfg!(windows) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

fn selected(browsers: &[Browser]) -> Vec<Browser> {
    if browsers.is_empty() {
        Browser::ALL.to_vec()
    } else {
        browsers.to_vec()
    }
}

// ---------------------------------------------------------------------------
// Windows

#[cfg(windows)]
mod platform {
    use super::*;
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};

    /// Vendor sub-paths (below the root) whose NativeMessagingHosts key the browser reads.
    pub fn vendor_keys(b: Browser) -> &'static [&'static str] {
        match b {
            // Brave, Vivaldi and Opera read Chrome's key on Windows.
            Browser::Chrome | Browser::Brave | Browser::Vivaldi | Browser::Opera => {
                &["Google\\Chrome"]
            }
            // Verified with Chrome for Testing 156 (Playwright's build): it reads the Chromium key.
            Browser::ChromeForTesting => &["Chromium"],
            Browser::Chromium => &["Chromium"],
            Browser::Edge => &["Microsoft\\Edge"],
            // Chromium's key under Helium's company and product name ("imput", "Helium"), from
            // imputnet/helium-windows patches/helium/windows/change-branding.patch.
            Browser::Helium => &["imput\\Helium"],
            // Gecko reads `Software\Mozilla\NativeMessagingHosts` (NativeManifests.sys.mjs); Zen
            // doesn't change it (zen-browser/desktop#10622: it uses Firefox's locations).
            Browser::Firefox | Browser::Zen => &["Mozilla"],
        }
    }

    fn key_path(loc: &Locations, vendor: &str) -> String {
        let root = loc.registry_root.as_deref().unwrap_or("Software");
        format!("{root}\\{vendor}\\NativeMessagingHosts\\{HOST_NAME}")
    }

    fn manifest_dir(loc: &Locations) -> Result<PathBuf, RegistrationError> {
        loc.manifest_dir
            .clone()
            .or_else(paths::manifests_dir)
            .ok_or(RegistrationError::NoDir("app data"))
    }

    pub fn manifest_file(loc: &Locations, b: Browser) -> Result<PathBuf, RegistrationError> {
        let name = if b.is_firefox() {
            format!("{HOST_NAME}.firefox.json")
        } else {
            format!("{HOST_NAME}.chromium.json")
        };
        Ok(manifest_dir(loc)?.join(name))
    }

    fn installed(b: Browser) -> bool {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        let roaming = std::env::var_os("APPDATA").map(PathBuf::from);
        let p = match b {
            Browser::Chrome => local.map(|d| d.join("Google\\Chrome\\User Data")),
            Browser::ChromeForTesting => {
                local.map(|d| d.join("Google\\Chrome for Testing\\User Data"))
            }
            Browser::Chromium => local.map(|d| d.join("Chromium\\User Data")),
            Browser::Edge => local.map(|d| d.join("Microsoft\\Edge\\User Data")),
            // Brave Origin (its own build and folder) reads the same key as Brave.
            Browser::Brave => {
                return ["Brave-Browser", "Brave-Origin"].iter().any(|p| {
                    local
                        .as_ref()
                        .is_some_and(|d| d.join("BraveSoftware").join(p).join("User Data").exists())
                });
            }
            Browser::Vivaldi => local.map(|d| d.join("Vivaldi\\User Data")),
            Browser::Opera => roaming.map(|d| d.join("Opera Software\\Opera Stable")),
            Browser::Helium => local.map(|d| d.join("imput\\Helium\\User Data")),
            Browser::Firefox => roaming.map(|d| d.join("Mozilla\\Firefox")),
            Browser::Zen => roaming.map(|d| d.join("zen")),
        };
        p.is_some_and(|p| p.exists())
    }

    fn reg_err(key: &str) -> impl FnOnce(std::io::Error) -> RegistrationError + '_ {
        move |source| RegistrationError::Registry {
            key: key.to_string(),
            source,
        }
    }

    fn read_default(key: &str) -> Option<String> {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(key, KEY_READ)
            .ok()?
            .get_value::<String, _>("")
            .ok()
    }

    pub fn register_one(b: Browser, opts: &RegisterOptions) -> Result<(), RegistrationError> {
        let file = manifest_file(&opts.locations, b)?;
        write_if_changed(&file, &manifest_bytes(&manifest_for(b, opts)))?;
        let value = file.to_string_lossy().to_string();
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        for vendor in vendor_keys(b) {
            let key = key_path(&opts.locations, vendor);
            if read_default(&key).as_deref() == Some(value.as_str()) {
                continue;
            }
            let (k, _) = hkcu.create_subkey(&key).map_err(reg_err(&key))?;
            k.set_value("", &value).map_err(reg_err(&key))?;
        }
        Ok(())
    }

    /// Removes the keys of `browsers`. A key shared with a browser that is not
    /// being unregistered (e.g. Chrome's key, also used by Brave) is kept.
    pub fn unregister(browsers: &[Browser], loc: &Locations) -> Result<(), RegistrationError> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let mut keys: Vec<&str> = Vec::new();
        for b in browsers {
            for v in vendor_keys(*b) {
                let shared_with_kept = Browser::ALL
                    .iter()
                    .any(|o| !browsers.contains(o) && vendor_keys(*o).contains(v));
                if !shared_with_kept && !keys.contains(v) {
                    keys.push(v);
                }
            }
        }
        for v in keys {
            let key = key_path(loc, v);
            match hkcu.delete_subkey_all(&key) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(reg_err(&key)(e)),
            }
        }
        // Remove manifest files nobody references any more.
        let chromium_left = Browser::ALL
            .iter()
            .any(|o| !o.is_firefox() && !browsers.contains(o));
        let firefox_left = Browser::ALL
            .iter()
            .any(|o| o.is_firefox() && !browsers.contains(o));
        for (b, keep) in [
            (Browser::Chrome, chromium_left),
            (Browser::Firefox, firefox_left),
        ] {
            if !keep && let Ok(f) = manifest_file(loc, b) {
                let _ = fs::remove_file(f);
            }
        }
        Ok(())
    }

    pub fn status_one(b: Browser, host: &Path, loc: &Locations) -> BrowserStatus {
        let mut st = BrowserStatus {
            browser: b,
            name: b.display_name().into(),
            installed: installed(b),
            supported: true,
            registered: false,
            manifest_path: None,
            points_to_this_host: false,
            allowed: Vec::new(),
            location: format!("HKCU\\{}", key_path(loc, vendor_keys(b)[0])),
            note: None,
        };
        for vendor in vendor_keys(b) {
            let key = key_path(loc, vendor);
            let Some(path) = read_default(&key) else {
                continue;
            };
            let path = PathBuf::from(path);
            let info = read_manifest(&path);
            st.location = format!("HKCU\\{key}");
            st.manifest_path = Some(path);
            if info.valid {
                st.registered = true;
                st.points_to_this_host = info
                    .host_path
                    .as_deref()
                    .is_some_and(|h| same_path(h, host));
                st.allowed = info.allowed;
                break;
            }
        }
        if matches!(b, Browser::Brave | Browser::Vivaldi | Browser::Opera) {
            st.note = Some("uses Google Chrome's registry key on Windows".into());
        }
        if b == Browser::Zen {
            st.note = Some("uses Firefox's registry key".into());
        }
        st
    }
}

// ---------------------------------------------------------------------------
// Linux / macOS

#[cfg(not(windows))]
mod platform {
    use super::*;

    fn home(loc: &Locations) -> Result<PathBuf, RegistrationError> {
        loc.home
            .clone()
            .or_else(paths::home_dir)
            .ok_or(RegistrationError::NoDir("home"))
    }

    /// Browser profile root (for detection) and the NativeMessagingHosts dirs it reads.
    fn dirs(b: Browser, loc: &Locations) -> Result<(PathBuf, Vec<PathBuf>), RegistrationError> {
        let home = home(loc)?;
        if cfg!(target_os = "macos") {
            let base = home.join("Library").join("Application Support");
            let nmh = |p: &str| base.join(p).join("NativeMessagingHosts");
            Ok(match b {
                // Every Chrome channel reads native messaging manifests from its own profile
                // dir, so Chrome Dev/Beta/Canary need their own copy.
                Browser::Chrome => {
                    let google = base.join("Google");
                    channel_dirs(vec![
                        google.join("Chrome"),
                        google.join("Chrome Dev"),
                        google.join("Chrome Beta"),
                        google.join("Chrome Canary"),
                    ])
                }
                Browser::ChromeForTesting => (
                    base.join("Google/Chrome for Testing"),
                    vec![nmh("Google/Chrome for Testing")],
                ),
                Browser::Chromium => (base.join("Chromium"), vec![nmh("Chromium")]),
                Browser::Edge => (base.join("Microsoft Edge"), vec![nmh("Microsoft Edge")]),
                Browser::Brave => brave(&base),
                Browser::Vivaldi => (base.join("Vivaldi"), vec![nmh("Vivaldi")]),
                // Opera reportedly only works with Chrome's directory on macOS.
                Browser::Opera => (
                    base.join("com.operasoftware.Opera"),
                    vec![nmh("Google/Chrome"), nmh("Opera")],
                ),
                // Helium's product dir (helium-macos change-product-dir-name.patch); it also
                // falls back to Chrome's directory (helium scan-chrome-native-messaging-hosts.patch).
                Browser::Helium => (base.join("net.imput.helium"), vec![nmh("net.imput.helium")]),
                Browser::Firefox => (base.join("Firefox"), vec![nmh("Mozilla")]),
                // Zen keeps its profiles in "zen" but reads Firefox's NativeMessagingHosts.
                Browser::Zen => (base.join("zen"), vec![nmh("Mozilla")]),
            })
        } else {
            let config = config_home(loc, &home);
            let nmh = |p: &str| config.join(p).join("NativeMessagingHosts");
            Ok(match b {
                // Chrome channels: stable, beta, dev (`google-chrome-unstable`).
                Browser::Chrome => channel_dirs(vec![
                    config.join("google-chrome"),
                    config.join("google-chrome-beta"),
                    config.join("google-chrome-unstable"),
                ]),
                Browser::ChromeForTesting => (
                    config.join("google-chrome-for-testing"),
                    vec![nmh("google-chrome-for-testing")],
                ),
                Browser::Chromium => (config.join("chromium"), vec![nmh("chromium")]),
                Browser::Edge => (config.join("microsoft-edge"), vec![nmh("microsoft-edge")]),
                Browser::Brave => brave(&config),
                Browser::Vivaldi => (config.join("vivaldi"), vec![nmh("vivaldi")]),
                // Some Opera builds read Chrome's directory: write both.
                Browser::Opera => (
                    config.join("opera"),
                    vec![nmh("opera"), nmh("google-chrome")],
                ),
                // Helium's config dir (helium-linux change-chromium-branding.patch).
                Browser::Helium => (
                    config.join("net.imput.helium"),
                    vec![nmh("net.imput.helium")],
                ),
                Browser::Firefox => (
                    home.join(".mozilla/firefox"),
                    vec![home.join(".mozilla/native-messaging-hosts")],
                ),
                // zen-browser/desktop#10622: Zen reads ~/.mozilla/native-messaging-hosts.
                Browser::Zen => (
                    home.join(".zen"),
                    vec![home.join(".mozilla/native-messaging-hosts")],
                ),
            })
        }
    }

    /// Brave and Brave Origin (a separate Brave build with its own folder, which reads native
    /// messaging hosts from there): whichever of them exists, plain Brave's folder when neither
    /// does, so nothing is created for a browser that isn't there.
    fn brave(base: &Path) -> (PathBuf, Vec<PathBuf>) {
        let roots = [
            base.join("BraveSoftware/Brave-Browser"),
            base.join("BraveSoftware/Brave-Origin"),
        ];
        let mut found: Vec<PathBuf> = roots.iter().filter(|r| r.exists()).cloned().collect();
        if found.is_empty() {
            found.push(roots[0].clone());
        }
        let nmh = found
            .iter()
            .map(|r| r.join("NativeMessagingHosts"))
            .collect();
        (found.swap_remove(0), nmh)
    }

    /// Chrome keeps one profile dir per channel and reads native messaging manifests only
    /// from the channel the user runs. Register into every channel that exists (stable, Dev,
    /// Beta, Canary); when none exists, return the stable dir so `status` still has a path.
    fn channel_dirs(roots: Vec<PathBuf>) -> (PathBuf, Vec<PathBuf>) {
        let mut found: Vec<PathBuf> = roots.iter().filter(|r| r.exists()).cloned().collect();
        if found.is_empty() {
            found.push(roots[0].clone());
        }
        let nmh = found
            .iter()
            .map(|r| r.join("NativeMessagingHosts"))
            .collect();
        (found.remove(0), nmh)
    }

    /// `$XDG_CONFIG_HOME` when set and absolute, else `~/.config` (always the latter in tests,
    /// which set `loc.home`).
    fn config_home(loc: &Locations, home: &Path) -> PathBuf {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| loc.home.is_none() && v.to_string_lossy().starts_with('/'))
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
    }

    /// Other profile roots that also mean "installed" (detection only, Linux). Firefox 147+ and
    /// Zen 1.18.6b+ put new profiles under XDG config (`mozilla/firefox`, `zen`) when the old
    /// dot-dir doesn't exist, yet both still read native messaging manifests from
    /// `~/.mozilla/native-messaging-hosts` (Firefox forces the legacy path for
    /// `XRE_USER_NATIVE_MANIFESTS` in nsXREDirProvider.cpp).
    fn other_profiles(b: Browser, loc: &Locations) -> Vec<PathBuf> {
        if cfg!(target_os = "macos") {
            return Vec::new();
        }
        let Ok(home) = home(loc) else {
            return Vec::new();
        };
        let config = config_home(loc, &home);
        match b {
            Browser::Firefox => vec![config.join("mozilla/firefox")],
            Browser::Zen => vec![config.join("zen")],
            _ => Vec::new(),
        }
    }

    /// Flatpak app id, used to detect sandboxed installs we can't serve.
    fn flatpak_id(b: Browser) -> Option<&'static str> {
        if cfg!(target_os = "macos") {
            return None;
        }
        Some(match b {
            Browser::Chrome => "com.google.Chrome",
            Browser::Chromium => "org.chromium.Chromium",
            Browser::Edge => "com.microsoft.Edge",
            Browser::Brave => "com.brave.Browser",
            Browser::Vivaldi => "com.vivaldi.Vivaldi",
            Browser::Opera => "com.opera.Opera",
            Browser::Firefox => "org.mozilla.firefox",
            Browser::Zen => "app.zen_browser.zen",
            Browser::ChromeForTesting | Browser::Helium => return None,
        })
    }

    fn manifest_name() -> String {
        format!("{HOST_NAME}.json")
    }

    pub fn register_one(b: Browser, opts: &RegisterOptions) -> Result<(), RegistrationError> {
        let (_, nmh_dirs) = dirs(b, &opts.locations)?;
        let bytes = manifest_bytes(&manifest_for(b, opts));
        for d in nmh_dirs {
            write_if_changed(&d.join(manifest_name()), &bytes)?;
        }
        Ok(())
    }

    pub fn unregister(browsers: &[Browser], loc: &Locations) -> Result<(), RegistrationError> {
        // Dirs still needed by browsers that stay registered (Opera shares Chrome's).
        let mut keep = Vec::new();
        for o in Browser::ALL.iter().filter(|o| !browsers.contains(o)) {
            keep.extend(dirs(*o, loc)?.1);
        }
        for b in browsers {
            for d in dirs(*b, loc)?.1 {
                if keep.contains(&d) {
                    continue;
                }
                let f = d.join(manifest_name());
                match fs::remove_file(&f) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(io_err(&f)(e)),
                }
            }
        }
        Ok(())
    }

    pub fn status_one(b: Browser, host: &Path, loc: &Locations) -> BrowserStatus {
        let mut st = BrowserStatus {
            browser: b,
            name: b.display_name().into(),
            installed: false,
            supported: true,
            registered: false,
            manifest_path: None,
            points_to_this_host: false,
            allowed: Vec::new(),
            location: String::new(),
            note: None,
        };
        let Ok((profile, nmh_dirs)) = dirs(b, loc) else {
            st.note = Some("home directory not found".into());
            return st;
        };
        st.installed = profile.exists() || other_profiles(b, loc).iter().any(|p| p.exists());
        st.location = nmh_dirs[0].to_string_lossy().into();
        for d in &nmh_dirs {
            let f = d.join(manifest_name());
            let info = read_manifest(&f);
            if f.exists() && st.manifest_path.is_none() {
                st.manifest_path = Some(f.clone());
            }
            if info.valid {
                st.registered = true;
                st.manifest_path = Some(f);
                st.location = d.to_string_lossy().into();
                st.points_to_this_host = info
                    .host_path
                    .as_deref()
                    .is_some_and(|h| same_path(h, host));
                st.allowed = info.allowed;
                break;
            }
        }
        if let (Some(id), Ok(h)) = (flatpak_id(b), home(loc))
            && !st.installed
            && h.join(".var/app").join(id).exists()
        {
            st.installed = true;
            st.supported = false;
            st.note = Some(format!(
                "Flatpak browser ({id}): native messaging is not supported inside the Flatpak sandbox"
            ));
        }
        if b == Browser::Firefox
            && let Ok(h) = home(loc)
            && !cfg!(target_os = "macos")
            && h.join("snap/firefox").exists()
        {
            st.installed = true;
            st.note = Some(
                "Snap Firefox: works through the WebExtensions portal (a consent prompt is shown)"
                    .into(),
            );
        }
        st
    }
}

// ---------------------------------------------------------------------------
// Public API

/// Registers the host for `opts.browsers` (empty = all) and returns the
/// resulting status. Idempotent; unchanged files/keys are not rewritten.
///
/// On Linux/macOS, browsers that are not installed are skipped unless they
/// were named explicitly (so we don't create config dirs for absent browsers).
/// On Windows every requested browser is registered (registry only).
pub fn register(opts: &RegisterOptions) -> Result<Vec<BrowserStatus>, RegistrationError> {
    let explicit = !opts.browsers.is_empty();
    let list = selected(&opts.browsers);
    for b in &list {
        if !cfg!(windows) && !explicit {
            let st = platform::status_one(*b, &opts.host_path, &opts.locations);
            if !st.installed || !st.supported {
                continue;
            }
        }
        platform::register_one(*b, opts)?;
    }
    Ok(status(&list, &opts.host_path, &opts.locations))
}

/// Removes the registration for `browsers` (empty = all).
pub fn unregister(
    browsers: &[Browser],
    loc: &Locations,
) -> Result<Vec<BrowserStatus>, RegistrationError> {
    let list = selected(browsers);
    platform::unregister(&list, loc)?;
    Ok(status(&list, Path::new(""), loc))
}

/// Inspects the registration of `browsers` (empty = all). `host_path` is the
/// executable `points_to_this_host` is compared against.
pub fn status(browsers: &[Browser], host_path: &Path, loc: &Locations) -> Vec<BrowserStatus> {
    selected(browsers)
        .into_iter()
        .map(|b| platform::status_one(b, host_path, loc))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifests() {
        let host = PathBuf::from(if cfg!(windows) {
            r"C:\Prism\prism-native-host.exe"
        } else {
            "/opt/prism/prism-native-host"
        });
        let m = chromium_manifest(&host, &["abc".into(), "chrome-extension://def".into()]);
        assert_eq!(m["name"], HOST_NAME);
        assert_eq!(m["type"], "stdio");
        assert_eq!(m["path"], host.to_string_lossy().as_ref());
        assert_eq!(
            m["allowed_origins"],
            json!(["chrome-extension://abc/", "chrome-extension://def/"])
        );
        assert!(m.get("allowed_extensions").is_none());
        let f = firefox_manifest(&host, &["prism@prism.dev".into()]);
        assert_eq!(f["allowed_extensions"], json!(["prism@prism.dev"]));
        assert!(f.get("allowed_origins").is_none());
        // round trip through the reader
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("m.json");
        fs::write(&p, manifest_bytes(&m)).unwrap();
        let info = read_manifest(&p);
        assert!(info.valid);
        assert_eq!(info.host_path.unwrap(), host);
        assert_eq!(info.allowed.len(), 2);
    }

    #[test]
    fn default_options_use_stable_ids() {
        let o = RegisterOptions::new("x");
        assert_eq!(
            o.chrome_extension_ids,
            vec![CHROME_EXTENSION_ID.to_string()]
        );
        assert_eq!(
            o.firefox_extension_ids,
            vec![
                crate::FIREFOX_EXTENSION_ID.to_string(),
                "prism@prism.dev".to_string()
            ],
            "the Firefox Add-ons ID, and the one copies loaded by hand used before"
        );
        assert!(o.browsers.is_empty());
    }

    #[test]
    fn browser_parse_roundtrip() {
        for b in Browser::ALL {
            assert_eq!(b.id().parse::<Browser>().unwrap(), b);
        }
        let s = serde_json::to_string(&Browser::ChromeForTesting).unwrap();
        assert_eq!(s, "\"chrome-for-testing\"");
    }

    /// Full register → status → unregister cycle against a sandboxed location
    /// (temp manifest dir + `HKCU\Software\PrismTest\...` on Windows, temp home elsewhere).
    #[test]
    fn register_status_unregister_sandboxed() {
        let dir = tempfile::tempdir().unwrap();
        let host = dir.path().join(crate::HOST_BINARY_NAME);
        fs::write(&host, b"").unwrap();
        let unique = format!("Software\\PrismTest{}", std::process::id());
        let loc = Locations {
            manifest_dir: Some(dir.path().join("nm")),
            home: Some(dir.path().join("home")),
            registry_root: Some(unique.clone()),
        };
        let mut opts = RegisterOptions::new(&host);
        opts.browsers = vec![
            Browser::Chrome,
            Browser::Edge,
            Browser::Firefox,
            Browser::Brave,
        ];
        opts.locations = loc.clone();
        let st = register(&opts).unwrap();
        assert_eq!(st.len(), 4);
        for s in &st {
            assert!(s.registered, "{s:?}");
            assert!(s.points_to_this_host, "{s:?}");
        }
        let ff = st.iter().find(|s| s.browser == Browser::Firefox).unwrap();
        assert_eq!(ff.allowed, crate::firefox_extension_ids());
        // idempotent
        assert_eq!(register(&opts).unwrap(), st);
        // status against a different host path
        let other = status(&[Browser::Chrome], Path::new("Z:/nope/other.exe"), &loc);
        assert!(other[0].registered && !other[0].points_to_this_host);

        // Brave shares Chrome's key on Windows: unregistering only Brave keeps it.
        unregister(&[Browser::Brave], &loc).unwrap();
        if cfg!(windows) {
            assert!(status(&[Browser::Chrome], &host, &loc)[0].registered);
        }
        let st = unregister(&[], &loc).unwrap();
        assert!(st.iter().all(|s| !s.registered), "{st:?}");
        #[cfg(windows)]
        {
            let _ =
                winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).delete_subkey_all(&unique);
        }
    }

    /// Helium gets the Chromium manifest in its own place; Zen gets Firefox's manifest where
    /// Firefox reads it, and unregistering Zen alone keeps Firefox's registration.
    #[test]
    fn helium_and_zen_register_where_they_read() {
        let dir = tempfile::tempdir().unwrap();
        let host = dir.path().join(crate::HOST_BINARY_NAME);
        fs::write(&host, b"").unwrap();
        let unique = format!("Software\\PrismTestHZ{}", std::process::id());
        let loc = Locations {
            manifest_dir: Some(dir.path().join("nm")),
            home: Some(dir.path().join("home")),
            registry_root: Some(unique.clone()),
        };
        let mut opts = RegisterOptions::new(&host);
        opts.browsers = vec![Browser::Helium, Browser::Zen, Browser::Firefox];
        opts.locations = loc.clone();
        let st = register(&opts).unwrap();
        let get = |b: Browser| st.iter().find(|s| s.browser == b).unwrap().clone();
        let (helium, zen, firefox) = (
            get(Browser::Helium),
            get(Browser::Zen),
            get(Browser::Firefox),
        );
        for s in [&helium, &zen, &firefox] {
            assert!(s.registered && s.points_to_this_host, "{s:?}");
        }
        assert_eq!(helium.allowed, vec![chrome_origin(CHROME_EXTENSION_ID)]);
        assert_eq!(zen.allowed, crate::firefox_extension_ids());
        assert_eq!(
            zen.location, firefox.location,
            "Zen reads Firefox's location"
        );
        let helium_place = if cfg!(windows) {
            "imput\\Helium\\NativeMessagingHosts"
        } else {
            "net.imput.helium"
        };
        assert!(
            helium.location.contains(helium_place),
            "{}",
            helium.location
        );
        assert_eq!("zen-browser".parse::<Browser>().unwrap(), Browser::Zen);

        unregister(&[Browser::Zen], &loc).unwrap();
        assert!(status(&[Browser::Firefox], &host, &loc)[0].registered);
        let st = unregister(&[], &loc).unwrap();
        assert!(st.iter().all(|s| !s.registered), "{st:?}");
        #[cfg(windows)]
        {
            let _ =
                winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).delete_subkey_all(&unique);
        }
    }

    /// Zen 1.18.6b+ and Firefox 147+ keep fresh profiles in `~/.config/zen` and
    /// `~/.config/mozilla/firefox` (no `~/.zen` / `~/.mozilla/firefox`). They still count as
    /// installed, so Repair (no explicit browsers) writes the manifest they read in
    /// `~/.mozilla/native-messaging-hosts`.
    #[cfg(target_os = "linux")]
    #[test]
    fn xdg_only_zen_and_firefox_are_detected_and_registered() {
        let dir = tempfile::tempdir().unwrap();
        let host = dir.path().join(crate::HOST_BINARY_NAME);
        fs::write(&host, b"").unwrap();
        let home = dir.path().join("home");
        let loc = Locations {
            manifest_dir: None,
            home: Some(home.clone()),
            registry_root: None,
        };
        let manifest = home
            .join(".mozilla/native-messaging-hosts")
            .join(format!("{HOST_NAME}.json"));
        let mut opts = RegisterOptions::new(&host);
        opts.locations = loc.clone();
        for profile in [".config/zen", ".config/mozilla/firefox"] {
            let _ = fs::remove_dir_all(&home);
            fs::create_dir_all(home.join(profile)).unwrap();
            register(&opts).unwrap();
            assert!(manifest.is_file(), "{profile}: manifest not written");
            let b = if profile.ends_with("zen") {
                Browser::Zen
            } else {
                Browser::Firefox
            };
            let st = &status(&[b], &host, &loc)[0];
            assert!(
                st.installed && st.registered && st.points_to_this_host,
                "{st:?}"
            );
        }
    }

    /// Brave Origin keeps its own folder (`~/.config/BraveSoftware/Brave-Origin`) and reads its
    /// NativeMessagingHosts from there; plain Brave's registration is unchanged.
    #[cfg(target_os = "linux")]
    #[test]
    fn brave_origin_is_detected_and_registered() {
        let dir = tempfile::tempdir().unwrap();
        let host = dir.path().join(crate::HOST_BINARY_NAME);
        fs::write(&host, b"").unwrap();
        let home = dir.path().join("home");
        let loc = Locations {
            manifest_dir: None,
            home: Some(home.clone()),
            registry_root: None,
        };
        let mut opts = RegisterOptions::new(&host);
        opts.locations = loc.clone();
        let brave = home.join(".config/BraveSoftware/Brave-Browser");
        let origin = home.join(".config/BraveSoftware/Brave-Origin");
        let manifest = |root: &Path| {
            root.join("NativeMessagingHosts")
                .join(format!("{HOST_NAME}.json"))
        };

        // Only Brave Origin: registered there, no Brave folder created.
        fs::create_dir_all(&origin).unwrap();
        register(&opts).unwrap();
        assert!(manifest(&origin).is_file());
        assert!(!brave.exists(), "nothing created for plain Brave");
        let st = &status(&[Browser::Brave], &host, &loc)[0];
        assert!(
            st.installed && st.registered && st.points_to_this_host,
            "{st:?}"
        );

        // Both: both folders get the manifest; unregistering removes both.
        fs::create_dir_all(&brave).unwrap();
        register(&opts).unwrap();
        assert!(manifest(&brave).is_file() && manifest(&origin).is_file());
        unregister(&[Browser::Brave], &loc).unwrap();
        assert!(!manifest(&brave).exists() && !manifest(&origin).exists());

        // Only plain Brave: as before, no Brave Origin folder.
        fs::remove_dir_all(&origin).unwrap();
        register(&opts).unwrap();
        assert!(manifest(&brave).is_file());
        assert!(!origin.exists());
    }

    /// Chrome Dev (and Beta/Canary) read native messaging manifests from their own profile
    /// dir, so a Dev-only install must be detected and get the manifest there.
    #[cfg(not(windows))]
    #[test]
    fn chrome_dev_channel_is_detected_and_registered() {
        let dir = tempfile::tempdir().unwrap();
        let host = dir.path().join(crate::HOST_BINARY_NAME);
        fs::write(&host, b"").unwrap();
        let home = dir.path().join("home");
        let loc = Locations {
            manifest_dir: None,
            home: Some(home.clone()),
            registry_root: None,
        };
        let (stable, dev) = if cfg!(target_os = "macos") {
            let google = home.join("Library/Application Support/Google");
            (google.join("Chrome"), google.join("Chrome Dev"))
        } else {
            (
                home.join(".config/google-chrome"),
                home.join(".config/google-chrome-unstable"),
            )
        };
        let manifest = |root: &Path| {
            root.join("NativeMessagingHosts")
                .join(format!("{HOST_NAME}.json"))
        };
        fs::create_dir_all(&dev).unwrap();
        let mut opts = RegisterOptions::new(&host);
        opts.locations = loc.clone();
        register(&opts).unwrap();
        assert!(manifest(&dev).is_file(), "Dev channel not registered");
        assert!(!stable.exists(), "nothing created for absent stable Chrome");
        let st = &status(&[Browser::Chrome], &host, &loc)[0];
        assert!(
            st.installed && st.registered && st.points_to_this_host,
            "{st:?}"
        );
        unregister(&[Browser::Chrome], &loc).unwrap();
        assert!(!manifest(&dev).exists());
    }
}
