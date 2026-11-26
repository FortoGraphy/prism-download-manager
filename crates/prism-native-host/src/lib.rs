//! Prism native messaging host.
//!
//! This crate is both:
//!
//! * the `prism-native-host` binary that browsers start (via native messaging)
//!   and that relays every extension message to the running Prism app over
//!   loopback HTTP (`POST http://127.0.0.1:<port>/v1/message`, bearer token), and
//! * a small library the app core reuses for its one-click "repair browser
//!   integration" page: [`registration`] (write/remove/inspect the native
//!   messaging host manifests + Windows registry keys) and [`paths`] (the
//!   app-data-dir / `ipc.json` location contract).
//!
//! The app core should depend on it with `default-features = false` (no HTTP
//! client is pulled in then).

pub mod args;
pub mod framing;
pub mod logger;
pub mod paths;
pub mod protocol;
pub mod registration;

#[cfg(feature = "host")]
pub mod host;
#[cfg(feature = "host")]
pub mod ipc;

/// Native messaging host name (the manifest `name` and registry key name).
pub const HOST_NAME: &str = "dev.prism.host";

/// Human readable description written into the host manifest.
pub const HOST_DESCRIPTION: &str = "Prism download manager browser integration";

/// Stable ID of the unpacked/dev Chromium extension. It is derived from the
/// public key embedded in `extension/manifest` (`key`), see `extension/README.md`.
pub const CHROME_EXTENSION_ID: &str = "mmcdeiglkbfbicdjboocmhehodogilfl";

/// Gecko ID of the Firefox build (`browser_specific_settings.gecko.id`), the one listed on
/// Firefox Add-ons. The ID used before 1.1.6 is no longer allowed (GHSA-v59w-jfj8-7pf6): it
/// belongs to someone else on Firefox Add-ons, who could sign an extension with it.
pub const FIREFOX_EXTENSION_ID: &str = "prism@prismdl.site";

/// Environment variable with more extension IDs the host serves, comma separated (for an
/// extension built with an ID of its own and registered with `--extension-id-*`).
pub const EXTRA_CALLERS_ENV: &str = "PRISM_EXTENSION_IDS";

/// Every Firefox extension ID the host manifest allows.
pub fn firefox_extension_ids() -> Vec<String> {
    vec![FIREFOX_EXTENSION_ID.to_string()]
}

/// Whether the host serves `caller`, the extension the browser started it for: Chromium passes
/// its origin (`chrome-extension://<id>/`), Firefox its ID. The browser checks its manifest
/// first; this second check turns away callers a stale or foreign manifest still lets in.
/// Without a caller (started by hand) the host runs as before. `extra` are IDs added through
/// [`EXTRA_CALLERS_ENV`].
pub fn caller_allowed(caller: Option<&str>, extra: &[String]) -> bool {
    let Some(caller) = caller.map(str::trim) else {
        return true;
    };
    let id = caller
        .strip_prefix("chrome-extension://")
        .map(|rest| rest.trim_end_matches('/'))
        .unwrap_or(caller);
    // A Firefox fork that names the caller by its per-profile moz-extension:// origin can't be
    // checked here; its manifest only allows Prism's ID.
    caller.starts_with("moz-extension://")
        || id == CHROME_EXTENSION_ID
        || id == FIREFOX_EXTENSION_ID
        || extra.iter().any(|e| e.trim() == id && !e.trim().is_empty())
}

/// Version of this host (same as the workspace version).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// File name of the host executable for the current platform.
#[cfg(windows)]
pub const HOST_BINARY_NAME: &str = "prism-native-host.exe";
/// File name of the host executable for the current platform.
#[cfg(not(windows))]
pub const HOST_BINARY_NAME: &str = "prism-native-host";

/// File name of the app executable the host launches (it lives next to the host).
#[cfg(windows)]
pub const APP_BINARY_NAME: &str = "prism.exe";
/// File name of the app executable the host launches (it lives next to the host).
#[cfg(not(windows))]
pub const APP_BINARY_NAME: &str = "prism";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_serves_only_prisms_extension() {
        let none: &[String] = &[];
        assert!(caller_allowed(
            Some("chrome-extension://mmcdeiglkbfbicdjboocmhehodogilfl/"),
            none
        ));
        assert!(caller_allowed(Some("prism@prismdl.site"), none));
        // GHSA-v59w-jfj8-7pf6: the old Firefox ID belongs to someone else on Firefox Add-ons.
        assert!(!caller_allowed(Some("prism@prism.dev"), none));
        assert!(!caller_allowed(
            Some("chrome-extension://abcdefghijklmnopabcdefghijklmnop/"),
            none
        ));
        assert!(!caller_allowed(Some(""), none));
        // Started by hand, or a fork that names a per-profile origin: its manifest decides.
        assert!(caller_allowed(None, none));
        assert!(caller_allowed(Some("moz-extension://1234-abcd/"), none));
        // IDs of a self-built extension, from PRISM_EXTENSION_IDS.
        let extra = vec![" me@example.org".to_string(), String::new()];
        assert!(caller_allowed(Some("me@example.org"), &extra));
        assert!(!caller_allowed(Some("other@example.org"), &extra));
        assert_eq!(
            firefox_extension_ids(),
            vec![FIREFOX_EXTENSION_ID.to_string()]
        );
    }
}
