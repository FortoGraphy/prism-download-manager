//! Command-line handling: tell browser invocations apart from installer/app
//! subcommands.
//!
//! Browsers start the host with:
//! * Chromium: `prism-native-host chrome-extension://<id>/` (on Windows also `--parent-window=<HWND>`)
//! * Firefox: `prism-native-host <path to manifest.json> <extension id>`
//!
//! Anything whose first argument is not one of our subcommand keywords is
//! treated as a browser invocation, so a browser can never trigger a subcommand.

use crate::registration::Browser;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterArgs {
    /// Empty = all supported browsers.
    pub browsers: Vec<Browser>,
    pub chrome_extension_ids: Vec<String>,
    pub firefox_extension_ids: Vec<String>,
    pub manifest_dir: Option<PathBuf>,
    pub host_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation {
    /// Started by a browser: run the native messaging relay.
    Host {
        caller: Option<String>,
    },
    Register(RegisterArgs),
    Unregister {
        browsers: Vec<Browser>,
        manifest_dir: Option<PathBuf>,
    },
    Status {
        browsers: Vec<Browser>,
        manifest_dir: Option<PathBuf>,
        host_path: Option<PathBuf>,
    },
    Help,
    Version,
}

pub const USAGE: &str = "\
prism-native-host — Prism browser integration bridge

USAGE (normally started by the browser through native messaging):
  prism-native-host register   [--browsers all|chrome,edge,...] [--extension-id-chrome <id>]...
                               [--extension-id-firefox <id>]... [--manifest-dir <dir>] [--host-path <exe>]
  prism-native-host unregister [--browsers all|...] [--manifest-dir <dir>]
  prism-native-host status     [--browsers all|...] [--manifest-dir <dir>] [--host-path <exe>]
  prism-native-host --version | --help

Browsers: chrome, chromium, edge, brave, vivaldi, opera, chrome-for-testing, firefox, zen, waterfox, all.
`status` prints a JSON array of per-browser registration states.";

fn is_subcommand(s: &str) -> bool {
    matches!(
        s,
        "register"
            | "unregister"
            | "status"
            | "help"
            | "--help"
            | "-h"
            | "--version"
            | "-V"
            | "version"
    )
}

/// Classifies `args` (without argv[0]).
pub fn classify(args: &[String]) -> Result<Invocation, String> {
    let Some(first) = args.first() else {
        return Ok(Invocation::Host { caller: None });
    };
    if !is_subcommand(first) {
        return Ok(Invocation::Host {
            caller: browser_caller(args),
        });
    }
    let rest = &args[1..];
    match first.as_str() {
        "help" | "--help" | "-h" => Ok(Invocation::Help),
        "version" | "--version" | "-V" => Ok(Invocation::Version),
        cmd => {
            let mut browsers = Vec::new();
            let mut chrome = Vec::new();
            let mut firefox = Vec::new();
            let mut manifest_dir = None;
            let mut host_path = None;
            let mut it = rest.iter();
            while let Some(a) = it.next() {
                let (key, inline) = match a.split_once('=') {
                    Some((k, v)) if k.starts_with("--") => (k.to_string(), Some(v.to_string())),
                    _ => (a.clone(), None),
                };
                let mut value = || -> Result<String, String> {
                    inline
                        .clone()
                        .or_else(|| it.next().cloned())
                        .ok_or_else(|| format!("missing value for {key}"))
                };
                match key.as_str() {
                    "--browsers" | "--browser" => browsers = parse_browsers(&value()?)?,
                    "--extension-id-chrome" => chrome.push(value()?),
                    "--extension-id-firefox" => firefox.push(value()?),
                    "--manifest-dir" => manifest_dir = Some(PathBuf::from(value()?)),
                    "--host-path" => host_path = Some(PathBuf::from(value()?)),
                    other => return Err(format!("unknown argument: {other}")),
                }
            }
            match cmd {
                "register" => Ok(Invocation::Register(RegisterArgs {
                    browsers,
                    chrome_extension_ids: chrome,
                    firefox_extension_ids: firefox,
                    manifest_dir,
                    host_path,
                })),
                "unregister" => Ok(Invocation::Unregister {
                    browsers,
                    manifest_dir,
                }),
                _ => Ok(Invocation::Status {
                    browsers,
                    manifest_dir,
                    host_path,
                }),
            }
        }
    }
}

/// Extracts the calling extension (origin for Chromium, id for Firefox).
fn browser_caller(args: &[String]) -> Option<String> {
    let first = &args[0];
    if first.starts_with("chrome-extension://") || first.starts_with("moz-extension://") {
        return Some(first.clone());
    }
    // Firefox: <manifest path> <extension id>
    if first.to_ascii_lowercase().ends_with(".json") {
        return args.get(1).cloned();
    }
    None
}

/// Parses `all` or a comma separated browser list.
pub fn parse_browsers(s: &str) -> Result<Vec<Browser>, String> {
    let mut out = Vec::new();
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if part.eq_ignore_ascii_case("all") {
            return Ok(Vec::new());
        }
        let b: Browser = part.parse()?;
        if !out.contains(&b) {
            out.push(b);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn chrome_invocation_is_host() {
        let inv = classify(&v(&["chrome-extension://abc/", "--parent-window=0"])).unwrap();
        assert_eq!(
            inv,
            Invocation::Host {
                caller: Some("chrome-extension://abc/".into())
            }
        );
        let inv = classify(&v(&["chrome-extension://abc/"])).unwrap();
        assert!(matches!(inv, Invocation::Host { .. }));
    }

    #[test]
    fn firefox_invocation_is_host() {
        let inv = classify(&v(&[
            r"C:\Users\u\AppData\Roaming\Prism\native-messaging\dev.prism.host.firefox.json",
            "prism@prism.dev",
        ]))
        .unwrap();
        assert_eq!(
            inv,
            Invocation::Host {
                caller: Some("prism@prism.dev".into())
            }
        );
        let inv = classify(&v(&[
            "/home/u/.mozilla/native-messaging-hosts/dev.prism.host.json",
            "prism@prism.dev",
        ]))
        .unwrap();
        assert!(matches!(inv, Invocation::Host { .. }));
    }

    #[test]
    fn no_args_or_unknown_is_host() {
        assert_eq!(classify(&[]).unwrap(), Invocation::Host { caller: None });
        assert_eq!(
            classify(&v(&["--parent-window=5"])).unwrap(),
            Invocation::Host { caller: None }
        );
        assert_eq!(
            classify(&v(&["registerx"])).unwrap(),
            Invocation::Host { caller: None }
        );
    }

    #[test]
    fn register_args() {
        let inv = classify(&v(&[
            "register",
            "--browsers",
            "chrome,edge",
            "--extension-id-chrome=aaaa",
            "--extension-id-chrome",
            "bbbb",
            "--extension-id-firefox",
            "x@y",
            "--manifest-dir",
            "/tmp/m",
        ]))
        .unwrap();
        let Invocation::Register(r) = inv else {
            panic!()
        };
        assert_eq!(r.browsers, vec![Browser::Chrome, Browser::Edge]);
        assert_eq!(r.chrome_extension_ids, vec!["aaaa", "bbbb"]);
        assert_eq!(r.firefox_extension_ids, vec!["x@y"]);
        assert_eq!(r.manifest_dir, Some(PathBuf::from("/tmp/m")));
    }

    #[test]
    fn browsers_all_and_errors() {
        assert_eq!(parse_browsers("all").unwrap(), vec![]);
        assert!(parse_browsers("netscape").is_err());
        assert!(classify(&v(&["status", "--bogus"])).is_err());
        assert!(classify(&v(&["register", "--browsers"])).is_err());
        assert_eq!(classify(&v(&["--version"])).unwrap(), Invocation::Version);
        assert!(matches!(
            classify(&v(&["status"])).unwrap(),
            Invocation::Status { .. }
        ));
        assert!(matches!(
            classify(&v(&["unregister", "--browsers", "firefox"])).unwrap(),
            Invocation::Unregister { .. }
        ));
    }
}
