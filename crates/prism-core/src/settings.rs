//! Typed application settings.
//!
//! Settings are stored as one JSON document in the `settings` table. Every field
//! has a default (`#[serde(default)]`), so older or partial documents always load
//! and newly added options pick up sensible values.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub general: General,
    pub appearance: Appearance,
    pub downloads: Downloads,
    pub connection: Connection,
    pub proxy: Proxy,
    pub capture: Capture,
    pub sounds: Sounds,
    pub clipboard: Clipboard,
    pub grabber: Grabber,
    pub antivirus: Antivirus,
    pub video: Video,
    pub hugging_face: HuggingFace,
    pub shortcuts: BTreeMap<String, String>,
    pub onboarding_done: bool,
    /// The app version whose "What's new" the user has seen ("" before 1.1.0).
    pub last_seen_version: String,
}

/// Downloading models from Hugging Face (see `crate::huggingface`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct HuggingFace {
    /// The server: "" = `HF_ENDPOINT` if it is set, else huggingface.co. Otherwise a URL like
    /// `https://hf-mirror.com`.
    pub endpoint: String,
    /// Access token for gated and private models ("" = the one `hf auth login` saved, if any).
    /// Sent only to the server above, never to the download servers it redirects to.
    pub token: String,
    /// Where models go, each in `<owner>/<name>` below it ("" = `Models` in the download folder).
    pub folder: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct General {
    pub launch_at_startup: bool,
    pub start_minimized: bool,
    pub close_to_tray: bool,
    /// Show the "Download File" dialog for downloads captured from the browser.
    pub show_capture_dialog: bool,
    /// Show the per-download progress window when a download starts.
    pub show_progress_window: bool,
    pub show_completion_dialog: bool,
    pub notifications: bool,
    pub confirm_delete: bool,
    /// Move deleted items to Trash (restorable) instead of forgetting them.
    pub use_trash: bool,
    /// Look for new Prism versions on start and every few hours.
    pub check_updates: bool,
    /// Finished downloads whose file was deleted from the computer leave the list (off by default).
    pub remove_deleted_files: bool,
    /// UI language code ("en", "cs", "de", "es", "fr", "it", "id", "tr", "ar"); empty = the
    /// system's language.
    pub language: String,
}

impl Default for General {
    fn default() -> Self {
        Self {
            launch_at_startup: true,
            start_minimized: false,
            close_to_tray: true,
            show_capture_dialog: true,
            show_progress_window: false,
            show_completion_dialog: true,
            notifications: true,
            confirm_delete: true,
            use_trash: true,
            check_updates: true,
            remove_deleted_files: false,
            language: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    #[default]
    Comfortable,
    Compact,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum MotionPref {
    #[default]
    System,
    Full,
    Reduced,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Appearance {
    pub theme: Theme,
    pub accent: String,
    pub density: Density,
    pub motion: MotionPref,
    /// Replace backdrop blur by solid tinted surfaces (slow GPUs / preference).
    pub reduce_transparency: bool,
    /// Use the OS window material (Mica / vibrancy) behind the window.
    pub window_material: bool,
    pub visible_columns: Vec<String>,
    /// Just the download list: no sidebar, details pane or tips (like a classic download manager).
    pub compact_view: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            accent: "prism".into(),
            density: Density::Comfortable,
            motion: MotionPref::System,
            reduce_transparency: false,
            window_material: true,
            visible_columns: [
                "name", "size", "status", "progress", "speed", "eta", "added",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            compact_view: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ConflictAction {
    #[default]
    Ask,
    Rename,
    Overwrite,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Downloads {
    /// Root folder; category sub-folders live below it unless overridden.
    pub default_dir: PathBuf,
    /// Put files into per-category sub-folders (Video, Music, ...).
    pub use_category_folders: bool,
    pub default_connections: u8,
    pub max_connections: u8,
    pub conflict: ConflictAction,
    pub max_retries: u32,
    /// Global speed limit in bytes/s used when the limiter is switched on.
    pub speed_limit: u64,
    pub speed_limiter_enabled: bool,
    pub verify_checksums: bool,
    /// Start downloads added from the browser immediately when no dialog is shown.
    pub auto_start: bool,
}

impl Default for Downloads {
    fn default() -> Self {
        Self {
            default_dir: crate::paths::default_download_dir(),
            use_category_folders: true,
            default_connections: 8,
            max_connections: 32,
            conflict: ConflictAction::Ask,
            max_retries: 20,
            speed_limit: 5 * 1024 * 1024,
            speed_limiter_enabled: false,
            verify_checksums: true,
            auto_start: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum HttpVersion {
    /// One TCP connection per segment (fastest for bulk transfers).
    #[default]
    Http1,
    /// Allow HTTP/2 when the server offers it.
    Auto,
    /// Experimental QUIC transport (requires a build with the `http3` feature).
    Http3,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Connection {
    /// Empty → engine default (a current desktop browser UA).
    pub user_agent: String,
    pub connect_timeout_secs: u64,
    pub read_timeout_secs: u64,
    pub http_version: HttpVersion,
    /// Upper bound of simultaneous connections to one host across all downloads (0 = unlimited).
    pub max_connections_per_host: u32,
    /// Custom DNS / host-to-IP overrides in "host:ip" or "host:ip:port" format.
    pub dns_overrides: Vec<String>,
}

impl Default for Connection {
    fn default() -> Self {
        Self {
            user_agent: String::new(),
            connect_timeout_secs: 15,
            read_timeout_secs: 30,
            http_version: HttpVersion::Http1,
            max_connections_per_host: 0,
            dns_overrides: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProxyMode {
    None,
    #[default]
    System,
    Http,
    Socks5,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Proxy {
    pub mode: ProxyMode,
    /// `host:port` or full URL.
    pub address: String,
    pub username: String,
    pub password: String,
    /// Hosts that bypass the proxy (wildcards allowed).
    pub bypass: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum BypassKey {
    #[default]
    Alt,
    Ctrl,
    Shift,
    None,
}

/// Browser capture rules (sent to the extension on `hello`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Capture {
    pub enabled: bool,
    pub extensions: Vec<String>,
    pub mime_types: Vec<String>,
    /// Sites (wildcards) whose downloads are never captured.
    pub excluded_sites: Vec<String>,
    /// Do not capture files smaller than this (KiB, 0 = no minimum).
    pub min_size_kb: u64,
    pub bypass_key: BypassKey,
    pub video_button: bool,
    /// Show the floating button only for videos at least this many seconds long (0 = always).
    pub video_min_duration_secs: u32,
}

pub const DEFAULT_CAPTURE_EXTENSIONS: &[&str] = &[
    "3gp", "7z", "aac", "ace", "aif", "aiff", "apk", "appimage", "arj", "asf", "avi", "bin", "bz2",
    "cab", "deb", "dmg", "exe", "flac", "flv", "gz", "gzip", "img", "iso", "lzh", "m4a", "m4v",
    "mkv", "mov", "mp3", "mp4", "mpa", "mpe", "mpeg", "mpg", "msi", "msix", "msu", "ogg", "ogv",
    "opus", "pdf", "pkg", "ppt", "pptx", "qt", "r00", "r01", "ra", "rar", "rm", "rmvb", "rpm",
    "sea", "sit", "sitx", "tar", "tgz", "tif", "tiff", "torrent", "wav", "webm", "wma", "wmv",
    "xz", "z", "zip", "zst",
];

impl Default for Capture {
    fn default() -> Self {
        Self {
            enabled: true,
            extensions: DEFAULT_CAPTURE_EXTENSIONS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            mime_types: [
                "application/octet-stream",
                "application/zip",
                "application/x-msdownload",
                "application/x-rar-compressed",
                "application/x-7z-compressed",
                "application/vnd.rar",
                "application/x-iso9660-image",
                "application/x-bittorrent",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            excluded_sites: vec![
                "*.update.microsoft.com".into(),
                "*.windowsupdate.com".into(),
                "accounts.google.com".into(),
            ],
            min_size_kb: 0,
            bypass_key: BypassKey::Alt,
            video_button: true,
            video_min_duration_secs: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Sounds {
    pub enabled: bool,
    pub on_complete: bool,
    pub on_failed: bool,
    pub on_queue_done: bool,
    pub volume: f32,
    /// The "download complete" sound: "classic" or one of the app's `complete-<id>.wav`.
    pub complete_sound: String,
}

impl Default for Sounds {
    fn default() -> Self {
        Self {
            enabled: true,
            on_complete: true,
            on_failed: true,
            on_queue_done: true,
            volume: 0.6,
            complete_sound: "classic".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Clipboard {
    pub monitor: bool,
    /// Ask before adding (`true`) or add silently into the capture dialog flow.
    pub ask: bool,
}

impl Default for Clipboard {
    fn default() -> Self {
        Self {
            monitor: true,
            ask: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Grabber {
    pub default_depth: u32,
    pub max_pages: u32,
    pub concurrent_requests: u32,
    pub respect_robots: bool,
}

impl Default for Grabber {
    fn default() -> Self {
        Self {
            default_depth: 2,
            max_pages: 2000,
            concurrent_requests: 6,
            respect_robots: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum AntivirusKind {
    #[default]
    Auto,
    Defender,
    ClamAv,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Antivirus {
    pub enabled: bool,
    pub kind: AntivirusKind,
    /// Executable for `Custom`.
    pub command: String,
    /// Arguments for `Custom`; `{file}` is replaced by the downloaded file path.
    pub args: String,
    /// Only scan these extensions (empty = all files).
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SponsorBlock {
    #[default]
    Off,
    Mark,
    Remove,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Video {
    pub ytdlp_path: String,
    pub ffmpeg_path: String,
    pub auto_update: bool,
    /// Preferred max height ("best" = 0).
    pub preferred_height: u32,
    pub container: String,
    pub audio_format: String,
    pub embed_subtitles: bool,
    pub subtitle_langs: String,
    pub embed_thumbnail: bool,
    pub embed_metadata: bool,
    pub embed_chapters: bool,
    pub sponsorblock: SponsorBlock,
    /// Use our multi-connection engine for plain HTTP formats.
    pub accelerate: bool,
    /// Browser to read cookies from for sites that need login ("" = none).
    pub cookies_from_browser: String,
    pub file_template: String,
    /// Start the file names of playlist videos with their position ("01 - Title").
    pub number_playlist_videos: bool,
}

impl Default for Video {
    fn default() -> Self {
        Self {
            ytdlp_path: String::new(),
            ffmpeg_path: String::new(),
            auto_update: true,
            preferred_height: 1080,
            container: "mp4".into(),
            audio_format: "m4a".into(),
            embed_subtitles: false,
            subtitle_langs: "en.*".into(),
            embed_thumbnail: true,
            embed_metadata: true,
            embed_chapters: true,
            sponsorblock: SponsorBlock::Off,
            accelerate: true,
            cookies_from_browser: String::new(),
            file_template: "%(title)s [%(id)s].%(ext)s".into(),
            number_playlist_videos: false,
        }
    }
}

impl Settings {
    /// Merge a partial JSON patch (as sent by the settings UI) into these settings.
    pub fn apply_patch(&mut self, patch: &serde_json::Value) -> Result<(), serde_json::Error> {
        let mut current = serde_json::to_value(&*self)?;
        merge_json(&mut current, patch);
        *self = serde_json::from_value(current)?;
        Ok(())
    }
}

fn merge_json(target: &mut serde_json::Value, patch: &serde_json::Value) {
    match (target, patch) {
        (serde_json::Value::Object(t), serde_json::Value::Object(p)) => {
            for (k, v) in p {
                merge_json(t.entry(k.clone()).or_insert(serde_json::Value::Null), v);
            }
        }
        (t, p) => *t = p.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_documents_fill_defaults() {
        let s: Settings =
            serde_json::from_str(r#"{"downloads":{"defaultConnections":16}}"#).unwrap();
        assert_eq!(s.downloads.default_connections, 16);
        assert_eq!(s.downloads.max_connections, 32);
        assert!(s.capture.extensions.iter().any(|e| e == "zip"));
    }

    #[test]
    fn patch_merges_nested_values() {
        let mut s = Settings::default();
        s.apply_patch(&serde_json::json!({"appearance": {"theme": "light"}, "downloads": {"speedLimiterEnabled": true}}))
            .unwrap();
        assert_eq!(s.appearance.theme, Theme::Light);
        assert!(s.downloads.speed_limiter_enabled);
        assert_eq!(s.appearance.accent, "prism");
    }
}
