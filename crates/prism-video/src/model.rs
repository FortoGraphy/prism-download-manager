//! Public data model (everything that crosses into the UI is serde camelCase)
//! plus the tolerant raw structs used to parse `yt-dlp -J` output.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::de::Deserializer;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// yt-dlp release channel used for managed installs and updates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateChannel {
    Stable,
    #[default]
    Nightly,
}

/// Service-wide settings. Empty strings coming from the UI are treated as "not set".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VideoOptions {
    /// Custom yt-dlp binary. `None` = managed copy, then system PATH.
    #[serde(deserialize_with = "empty_as_none")]
    pub ytdlp_path: Option<PathBuf>,
    /// Custom ffmpeg binary (ffprobe is looked up next to it). `None` = auto.
    #[serde(deserialize_with = "empty_as_none")]
    pub ffmpeg_path: Option<PathBuf>,
    /// Custom deno binary. `None` = managed copy, then system PATH.
    #[serde(deserialize_with = "empty_as_none")]
    pub deno_path: Option<PathBuf>,
    pub channel: UpdateChannel,
    /// Passed as `--cookies-from-browser` (e.g. `firefox`, `chrome:Profile 1`).
    #[serde(deserialize_with = "empty_as_none")]
    pub cookies_from_browser: Option<String>,
    /// Default User-Agent when the request context carries none.
    #[serde(deserialize_with = "empty_as_none")]
    pub user_agent: Option<String>,
    /// Proxy URL (`http://`, `https://`, `socks5://` ...) for yt-dlp and tool downloads.
    #[serde(deserialize_with = "empty_as_none")]
    pub proxy: Option<String>,
    /// Use an ffmpeg found on PATH instead of downloading a managed copy.
    pub prefer_system_ffmpeg: bool,
    /// Resolve YouTube formats natively via the Innertube API (logged-in session)
    /// before falling back to yt-dlp + deno.
    pub use_native_youtube: bool,
    /// `-N` for yt-dlp fragment downloads.
    pub concurrent_fragments: u8,
    /// Delete post-processing inputs (temp streams) after a successful ffmpeg run.
    pub delete_temp_inputs: bool,
    /// Extra raw yt-dlp arguments appended before the URL (power users).
    pub extra_ytdlp_args: Vec<String>,
}

impl Default for VideoOptions {
    fn default() -> Self {
        VideoOptions {
            ytdlp_path: None,
            ffmpeg_path: None,
            deno_path: None,
            channel: UpdateChannel::Nightly,
            cookies_from_browser: None,
            user_agent: None,
            proxy: None,
            prefer_system_ffmpeg: true,
            use_native_youtube: true,
            concurrent_fragments: 8,
            delete_temp_inputs: true,
            extra_ytdlp_args: Vec::new(),
        }
    }
}

/// Request context captured by the browser extension for the page.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RequestContext {
    /// `Cookie` header value (`a=b; c=d`) for the page's domain.
    #[serde(deserialize_with = "empty_as_none")]
    pub cookies: Option<String>,
    #[serde(deserialize_with = "empty_as_none")]
    pub referer: Option<String>,
    #[serde(deserialize_with = "empty_as_none")]
    pub user_agent: Option<String>,
    /// Extra headers to pass (`--add-headers`).
    pub headers: Vec<(String, String)>,
    /// Treat `watch?v=..&list=..` URLs as the playlist instead of the single video.
    pub want_playlist: bool,
}

// ---------------------------------------------------------------------------
// Probe result
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Probe {
    Video(Box<VideoInfo>),
    Playlist(PlaylistInfo),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistInfo {
    pub id: Option<String>,
    pub title: Option<String>,
    pub uploader: Option<String>,
    pub webpage_url: Option<String>,
    pub extractor: Option<String>,
    pub entries: Vec<PlaylistEntry>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistEntry {
    pub id: Option<String>,
    pub url: String,
    pub title: Option<String>,
    pub duration: Option<f64>,
    pub thumbnail: Option<String>,
    pub uploader: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoInfo {
    pub id: String,
    pub title: String,
    pub uploader: Option<String>,
    pub channel: Option<String>,
    /// Seconds.
    pub duration: Option<f64>,
    /// `YYYYMMDD` as reported by yt-dlp.
    pub upload_date: Option<String>,
    pub description: Option<String>,
    /// Best thumbnail URL.
    pub thumbnail: Option<String>,
    pub webpage_url: String,
    pub extractor: String,
    pub is_live: bool,
    /// `not_live`, `is_live`, `is_upcoming`, `was_live`, `post_live`.
    pub live_status: Option<String>,
    pub chapters: Vec<Chapter>,
    /// Manual subtitles first, then automatic captions (`auto: true`).
    pub subtitles: Vec<SubtitleTrack>,
    pub formats: Vec<Format>,
    /// Picker entries computed for the default container (mp4).
    /// Recompute with [`crate::choices::compute_choices`] when the user changes container.
    pub choices: Vec<Choice>,
    /// Every playable format is DRM protected.
    pub has_drm: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chapter {
    pub start: f64,
    pub end: f64,
    pub title: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleTrack {
    pub lang: String,
    pub name: Option<String>,
    /// Automatic captions (machine generated / translated).
    pub auto: bool,
    pub formats: Vec<SubtitleFormat>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleFormat {
    pub ext: String,
    pub url: String,
    pub protocol: Option<String>,
}

/// What a format carries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FormatKind {
    /// Video only.
    Video,
    /// Audio only.
    Audio,
    /// Video and audio in one file (or unknown codecs, e.g. a direct `.mp4`).
    #[default]
    Muxed,
    /// Storyboards, images and other non-media formats.
    Other,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Format {
    pub format_id: String,
    pub ext: String,
    pub kind: FormatKind,
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    /// kbit/s
    pub tbr: Option<f64>,
    pub vbr: Option<f64>,
    pub abr: Option<f64>,
    pub asr: Option<u32>,
    pub audio_channels: Option<u32>,
    pub filesize: Option<u64>,
    pub filesize_approx: Option<u64>,
    pub protocol: String,
    pub url: Option<String>,
    pub http_headers: BTreeMap<String, String>,
    /// `SDR`, `HDR10`, `HDR10+`, `HLG`, `DV` ...
    pub dynamic_range: Option<String>,
    pub language: Option<String>,
    pub language_preference: Option<i32>,
    pub format_note: Option<String>,
    pub container: Option<String>,
    pub has_drm: bool,
    /// The format is delivered as fragments (DASH/HLS manifest) rather than one file.
    pub fragmented: bool,
    /// `downloader_options.http_chunk_size` — YouTube throttles larger ranges.
    pub http_chunk_size: Option<u64>,
    /// Cookies yt-dlp would send for this format (`Set-Cookie`-like string).
    pub cookies: Option<String>,
    /// False for DRM-protected, storyboard and other unplayable formats.
    pub usable: bool,
}

impl Format {
    pub fn is_hdr(&self) -> bool {
        matches!(&self.dynamic_range, Some(d) if !d.eq_ignore_ascii_case("SDR"))
    }

    /// Plain progressive HTTP(S) file that the segmented engine can download.
    pub fn is_progressive_http(&self) -> bool {
        (self.protocol == "https" || self.protocol == "http")
            && !self.fragmented
            && self.url.is_some()
            && !self.has_drm
    }

    /// Size in bytes: exact, approximate, or estimated from bitrate × duration.
    pub fn size_estimate(&self, duration: Option<f64>) -> Option<u64> {
        self.filesize.or(self.filesize_approx).or_else(|| {
            let br = self.tbr.or(match (self.vbr, self.abr) {
                (Some(v), Some(a)) => Some(v + a),
                (Some(v), None) => Some(v),
                (None, Some(a)) => Some(a),
                _ => None,
            })?;
            let d = duration?;
            if br <= 0.0 || d <= 0.0 {
                return None;
            }
            Some((br * 1000.0 / 8.0 * d) as u64)
        })
    }

    pub fn has_video(&self) -> bool {
        matches!(self.kind, FormatKind::Video | FormatKind::Muxed)
    }

    pub fn has_audio(&self) -> bool {
        matches!(self.kind, FormatKind::Audio | FormatKind::Muxed)
    }

    /// Dynamic-range-compressed YouTube audio variant (avoid by default).
    pub fn is_drc(&self) -> bool {
        self.format_id.ends_with("-drc")
            || self
                .format_note
                .as_deref()
                .is_some_and(|n| n.contains("DRC"))
    }
}

// ---------------------------------------------------------------------------
// Quality picker
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Container {
    #[default]
    Mp4,
    Mkv,
    Webm,
}

impl Container {
    pub fn ext(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::Mkv => "mkv",
            Container::Webm => "webm",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AudioFormat {
    #[default]
    Mp3,
    M4a,
    Opus,
    Flac,
    Wav,
}

impl AudioFormat {
    pub fn ext(self) -> &'static str {
        match self {
            AudioFormat::Mp3 => "mp3",
            AudioFormat::M4a => "m4a",
            AudioFormat::Opus => "opus",
            AudioFormat::Flac => "flac",
            AudioFormat::Wav => "wav",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChoiceKind {
    #[default]
    Video,
    Audio,
}

/// One entry in the quality picker.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    /// Stable id within one probe (`v2160p60-hdr`, `a-mp3` ...).
    pub id: String,
    pub kind: ChoiceKind,
    /// `2160p60 · HDR · AV1`, `Audio · MP3 320 kbps`.
    pub label: String,
    /// `MP4 · ~712 MB`.
    pub sub_label: String,
    pub height: Option<u32>,
    pub width: Option<u32>,
    pub fps: Option<f64>,
    pub hdr: bool,
    pub dynamic_range: Option<String>,
    /// Codec family (`H.264`, `AV1`, `VP9`, `HEVC`, `AAC`, `Opus` ...).
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    pub est_size: Option<u64>,
    pub video_format_id: Option<String>,
    pub audio_format_id: Option<String>,
    /// Single muxed format (video+audio in one file).
    pub format_id: Option<String>,
    pub container: Container,
    /// Target audio format for audio-only choices.
    pub audio_format: Option<AudioFormat>,
    /// Audio bitrate for converted audio (kbps).
    pub audio_bitrate_kbps: Option<u32>,
    /// All referenced formats are plain HTTP(S) files (engine-accelerated path possible).
    pub accelerable: bool,
}

// ---------------------------------------------------------------------------
// Output options
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SponsorBlockMode {
    #[default]
    Off,
    /// Add chapters for the segments.
    Mark,
    /// Cut the segments out (yt-dlp only).
    Remove,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SponsorBlockOptions {
    pub mode: SponsorBlockMode,
    /// `sponsor`, `intro`, `outro`, `selfpromo`, `preview`, `filler`, `interaction`, `music_offtopic`, `hook` ...
    pub categories: Vec<String>,
}

impl Default for SponsorBlockOptions {
    fn default() -> Self {
        SponsorBlockOptions {
            mode: SponsorBlockMode::Off,
            categories: vec!["sponsor".into(), "selfpromo".into(), "interaction".into()],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OutputOptions {
    pub container: Container,
    /// Used for audio choices that carry no explicit format.
    pub audio_format: AudioFormat,
    pub mp3_bitrate_kbps: u32,
    pub embed_subtitles: bool,
    /// Language codes / patterns (`en`, `en.*`, `all`).
    pub sub_langs: Vec<String>,
    /// Also consider automatic captions when no manual subtitle matches.
    pub auto_subtitles: bool,
    /// Keep subtitle files next to the output.
    pub write_subtitles: bool,
    pub embed_thumbnail: bool,
    pub write_thumbnail: bool,
    pub embed_metadata: bool,
    pub embed_chapters: bool,
    pub sponsorblock: SponsorBlockOptions,
    /// yt-dlp style output template (`%(title)s [%(id)s].%(ext)s`).
    pub file_template: String,
    pub output_dir: PathBuf,
}

impl Default for OutputOptions {
    fn default() -> Self {
        OutputOptions {
            container: Container::Mp4,
            audio_format: AudioFormat::Mp3,
            mp3_bitrate_kbps: 320,
            embed_subtitles: false,
            sub_langs: vec!["en".into()],
            auto_subtitles: false,
            write_subtitles: false,
            embed_thumbnail: true,
            write_thumbnail: false,
            embed_metadata: true,
            embed_chapters: true,
            sponsorblock: SponsorBlockOptions::default(),
            file_template: DEFAULT_TEMPLATE.into(),
            output_dir: PathBuf::new(),
        }
    }
}

pub const DEFAULT_TEMPLATE: &str = "%(title)s [%(id)s].%(ext)s";

// ---------------------------------------------------------------------------
// serde helpers
// ---------------------------------------------------------------------------

/// Deserialize `""` / whitespace / `null` as `None`.
pub(crate) fn empty_as_none<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: From<String>,
{
    let v: Option<String> = Option::deserialize(d)?;
    Ok(v.and_then(|s| {
        if s.trim().is_empty() {
            None
        } else {
            Some(T::from(s))
        }
    }))
}

fn lenient_f64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    })
}

fn lenient_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => Some(s),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    })
}

fn lenient_bool<'de, D: Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Bool(b) => Some(b),
        Value::Number(n) => Some(n.as_f64().unwrap_or(0.0) != 0.0),
        Value::String(s) => match s.as_str() {
            "true" | "True" | "yes" => Some(true),
            "false" | "False" | "no" => Some(false),
            _ => None, // e.g. has_drm: "maybe"
        },
        _ => None,
    })
}

fn lenient_headers<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<String, String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Object(m) => m
            .into_iter()
            .filter_map(|(k, v)| match v {
                Value::String(s) => Some((k, s)),
                Value::Number(n) => Some((k, n.to_string())),
                _ => None,
            })
            .collect(),
        _ => BTreeMap::new(),
    })
}

fn lenient_vec<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    Ok(match Value::deserialize(d)? {
        Value::Array(items) => items
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect(),
        _ => Vec::new(),
    })
}

fn lenient_subs<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, Vec<RawSubtitle>>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Object(m) => m
            .into_iter()
            .map(|(k, v)| {
                let list = match v {
                    Value::Array(items) => items
                        .into_iter()
                        .filter_map(|v| serde_json::from_value(v).ok())
                        .collect(),
                    _ => Vec::new(),
                };
                (k, list)
            })
            .collect(),
        _ => BTreeMap::new(),
    })
}

fn to_u32(v: Option<f64>) -> Option<u32> {
    v.filter(|x| x.is_finite() && *x >= 0.0)
        .map(|x| x.round() as u32)
}

fn to_u64(v: Option<f64>) -> Option<u64> {
    v.filter(|x| x.is_finite() && *x > 0.0)
        .map(|x| x.round() as u64)
}

// ---------------------------------------------------------------------------
// Raw yt-dlp JSON
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct RawInfo {
    #[serde(rename = "_type", deserialize_with = "lenient_string")]
    pub kind: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub id: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub title: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub fulltitle: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub uploader: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub channel: Option<String>,
    #[serde(deserialize_with = "lenient_f64")]
    pub duration: Option<f64>,
    #[serde(deserialize_with = "lenient_string")]
    pub upload_date: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub description: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub thumbnail: Option<String>,
    #[serde(deserialize_with = "lenient_vec")]
    pub thumbnails: Vec<RawThumbnail>,
    #[serde(deserialize_with = "lenient_string")]
    pub webpage_url: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub original_url: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub url: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub extractor: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub extractor_key: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub ie_key: Option<String>,
    #[serde(deserialize_with = "lenient_bool")]
    pub is_live: Option<bool>,
    #[serde(deserialize_with = "lenient_string")]
    pub live_status: Option<String>,
    #[serde(deserialize_with = "lenient_vec")]
    pub chapters: Vec<RawChapter>,
    #[serde(deserialize_with = "lenient_subs")]
    pub subtitles: BTreeMap<String, Vec<RawSubtitle>>,
    #[serde(deserialize_with = "lenient_subs")]
    pub automatic_captions: BTreeMap<String, Vec<RawSubtitle>>,
    #[serde(deserialize_with = "lenient_vec")]
    pub formats: Vec<RawFormat>,
    #[serde(deserialize_with = "lenient_headers")]
    pub http_headers: BTreeMap<String, String>,
    /// Playlist entries. `null` entries (unavailable videos) are dropped.
    #[serde(deserialize_with = "lenient_entries")]
    pub entries: Vec<RawInfo>,
    // Single-format results (no `formats` list) carry the format fields at top level.
    #[serde(deserialize_with = "lenient_string")]
    pub format_id: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub ext: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub protocol: Option<String>,
}

fn lenient_entries<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<RawInfo>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Array(items) => items
            .into_iter()
            .filter(|v| v.is_object())
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect(),
        _ => Vec::new(),
    })
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct RawThumbnail {
    #[serde(deserialize_with = "lenient_string")]
    pub url: Option<String>,
    #[serde(deserialize_with = "lenient_f64")]
    pub width: Option<f64>,
    #[serde(deserialize_with = "lenient_f64")]
    pub height: Option<f64>,
    #[serde(deserialize_with = "lenient_f64")]
    pub preference: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct RawChapter {
    #[serde(deserialize_with = "lenient_f64")]
    pub start_time: Option<f64>,
    #[serde(deserialize_with = "lenient_f64")]
    pub end_time: Option<f64>,
    #[serde(deserialize_with = "lenient_string")]
    pub title: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct RawSubtitle {
    #[serde(deserialize_with = "lenient_string")]
    pub ext: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub url: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub name: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub protocol: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct RawDownloaderOptions {
    #[serde(deserialize_with = "lenient_f64")]
    pub http_chunk_size: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct RawFormat {
    #[serde(deserialize_with = "lenient_string")]
    pub format_id: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub ext: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub vcodec: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub acodec: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub video_ext: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub audio_ext: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub resolution: Option<String>,
    #[serde(deserialize_with = "lenient_f64")]
    pub width: Option<f64>,
    #[serde(deserialize_with = "lenient_f64")]
    pub height: Option<f64>,
    #[serde(deserialize_with = "lenient_f64")]
    pub fps: Option<f64>,
    #[serde(deserialize_with = "lenient_f64")]
    pub tbr: Option<f64>,
    #[serde(deserialize_with = "lenient_f64")]
    pub vbr: Option<f64>,
    #[serde(deserialize_with = "lenient_f64")]
    pub abr: Option<f64>,
    #[serde(deserialize_with = "lenient_f64")]
    pub asr: Option<f64>,
    #[serde(deserialize_with = "lenient_f64")]
    pub audio_channels: Option<f64>,
    #[serde(deserialize_with = "lenient_f64")]
    pub filesize: Option<f64>,
    #[serde(deserialize_with = "lenient_f64")]
    pub filesize_approx: Option<f64>,
    #[serde(deserialize_with = "lenient_string")]
    pub protocol: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub url: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub manifest_url: Option<String>,
    #[serde(deserialize_with = "lenient_headers")]
    pub http_headers: BTreeMap<String, String>,
    #[serde(deserialize_with = "lenient_string")]
    pub dynamic_range: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub language: Option<String>,
    #[serde(deserialize_with = "lenient_f64")]
    pub language_preference: Option<f64>,
    #[serde(deserialize_with = "lenient_string")]
    pub format_note: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub container: Option<String>,
    #[serde(deserialize_with = "lenient_bool")]
    pub has_drm: Option<bool>,
    pub fragments: Option<Value>,
    #[serde(deserialize_with = "lenient_string")]
    pub fragment_base_url: Option<String>,
    pub downloader_options: Option<RawDownloaderOptions>,
    #[serde(deserialize_with = "lenient_string")]
    pub cookies: Option<String>,
}

fn none_codec(c: &Option<String>) -> bool {
    matches!(c.as_deref(), Some("none"))
}

impl RawFormat {
    pub(crate) fn into_format(self) -> Format {
        let protocol = self
            .protocol
            .clone()
            .unwrap_or_else(|| guess_protocol(self.url.as_deref()));
        let ext = self.ext.clone().unwrap_or_default();
        let kind = classify(&self, &protocol, &ext);
        let fragmented = self.fragments.as_ref().is_some_and(|v| !v.is_null())
            || self.fragment_base_url.is_some()
            || !(protocol == "https" || protocol == "http");
        let has_drm = self.has_drm.unwrap_or(false);
        let usable = !has_drm
            && kind != FormatKind::Other
            && (self.url.is_some() || self.manifest_url.is_some());
        Format {
            format_id: self.format_id.unwrap_or_default(),
            ext,
            kind,
            vcodec: self.vcodec.filter(|c| c != "none"),
            acodec: self.acodec.filter(|c| c != "none"),
            width: to_u32(self.width),
            height: to_u32(self.height),
            fps: self.fps.filter(|f| *f > 0.0),
            tbr: self.tbr.filter(|v| *v > 0.0),
            vbr: self.vbr.filter(|v| *v > 0.0),
            abr: self.abr.filter(|v| *v > 0.0),
            asr: to_u32(self.asr),
            audio_channels: to_u32(self.audio_channels),
            filesize: to_u64(self.filesize),
            filesize_approx: to_u64(self.filesize_approx),
            protocol,
            url: self.url,
            http_headers: self.http_headers,
            dynamic_range: self.dynamic_range,
            language: self.language,
            language_preference: self.language_preference.map(|v| v as i32),
            format_note: self.format_note,
            container: self.container,
            has_drm,
            fragmented,
            http_chunk_size: self
                .downloader_options
                .and_then(|o| to_u64(o.http_chunk_size)),
            cookies: self.cookies,
            usable,
        }
    }
}

fn guess_protocol(url: Option<&str>) -> String {
    match url {
        Some(u) if u.contains(".m3u8") => "m3u8_native".into(),
        Some(u) if u.starts_with("https:") => "https".into(),
        Some(u) if u.starts_with("http:") => "http".into(),
        _ => "https".into(),
    }
}

fn classify(f: &RawFormat, protocol: &str, ext: &str) -> FormatKind {
    if protocol == "mhtml" || matches!(ext, "mhtml" | "jpg" | "png" | "webp") {
        return FormatKind::Other;
    }
    let v_none = none_codec(&f.vcodec);
    let a_none = none_codec(&f.acodec);
    if v_none && a_none {
        return FormatKind::Other;
    }
    if v_none {
        return FormatKind::Audio;
    }
    if a_none {
        return FormatKind::Video;
    }
    // At least one codec is unknown (null). Use the auxiliary hints yt-dlp emits.
    let audio_only_hint =
        f.resolution.as_deref() == Some("audio only") || f.video_ext.as_deref() == Some("none");
    let video_only_hint = f.audio_ext.as_deref() == Some("none") && f.vcodec.is_some();
    match (f.vcodec.is_some(), f.acodec.is_some()) {
        (true, true) => FormatKind::Muxed,
        _ if audio_only_hint => FormatKind::Audio,
        _ if video_only_hint => FormatKind::Video,
        (false, true)
            if matches!(
                ext,
                "m4a" | "mp3" | "opus" | "ogg" | "aac" | "flac" | "wav" | "weba"
            ) =>
        {
            FormatKind::Audio
        }
        (false, false)
            if matches!(
                ext,
                "m4a" | "mp3" | "opus" | "ogg" | "aac" | "flac" | "wav" | "weba"
            ) =>
        {
            FormatKind::Audio
        }
        _ => FormatKind::Muxed,
    }
}

impl RawInfo {
    pub(crate) fn is_playlist(&self) -> bool {
        matches!(self.kind.as_deref(), Some("playlist" | "multi_video"))
    }

    pub(crate) fn best_thumbnail(&self) -> Option<String> {
        if let Some(t) = &self.thumbnail {
            return Some(t.clone());
        }
        self.thumbnails
            .iter()
            .filter(|t| t.url.is_some())
            .max_by(|a, b| {
                let ka = (a.preference.unwrap_or(0.0), a.width.unwrap_or(0.0));
                let kb = (b.preference.unwrap_or(0.0), b.width.unwrap_or(0.0));
                ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .and_then(|t| t.url.clone())
    }

    pub(crate) fn into_playlist(self) -> PlaylistInfo {
        let entries = self
            .entries
            .iter()
            .filter_map(|e| {
                let url = e
                    .url
                    .clone()
                    .or_else(|| e.webpage_url.clone())
                    .or_else(|| e.original_url.clone())?;
                Some(PlaylistEntry {
                    id: e.id.clone(),
                    url,
                    title: e.title.clone(),
                    duration: e.duration,
                    thumbnail: e
                        .thumbnails
                        .last()
                        .and_then(|t| t.url.clone())
                        .or_else(|| e.thumbnail.clone()),
                    uploader: e.uploader.clone().or_else(|| e.channel.clone()),
                })
            })
            .collect();
        PlaylistInfo {
            id: self.id,
            title: self.title,
            uploader: self.uploader.or(self.channel),
            webpage_url: self.webpage_url.or(self.original_url),
            extractor: self.extractor,
            entries,
        }
    }

    pub(crate) fn into_video(self) -> VideoInfo {
        let thumbnail = self.best_thumbnail();
        let mut formats: Vec<Format> = self
            .formats
            .into_iter()
            .map(RawFormat::into_format)
            .collect();
        if formats.is_empty() {
            // Single-format result (e.g. some generic pages): synthesize one muxed format.
            if let Some(url) = self.url.clone() {
                let raw = RawFormat {
                    format_id: self.format_id.clone().or(Some("0".into())),
                    ext: self.ext.clone(),
                    protocol: self.protocol.clone(),
                    url: Some(url),
                    http_headers: self.http_headers.clone(),
                    ..Default::default()
                };
                formats.push(raw.into_format());
            }
        }
        let playable: Vec<&Format> = formats
            .iter()
            .filter(|f| f.kind != FormatKind::Other)
            .collect();
        let has_drm = !playable.is_empty() && playable.iter().all(|f| f.has_drm);

        let mut subtitles = Vec::new();
        for (auto, map) in [(false, &self.subtitles), (true, &self.automatic_captions)] {
            for (lang, list) in map {
                if lang == "live_chat" || lang == "rechat" {
                    continue;
                }
                let formats: Vec<SubtitleFormat> = list
                    .iter()
                    .filter_map(|s| {
                        Some(SubtitleFormat {
                            ext: s.ext.clone().unwrap_or_else(|| "vtt".into()),
                            url: s.url.clone()?,
                            protocol: s.protocol.clone(),
                        })
                    })
                    .collect();
                if formats.is_empty() {
                    continue;
                }
                subtitles.push(SubtitleTrack {
                    lang: lang.clone(),
                    name: list.iter().find_map(|s| s.name.clone()),
                    auto,
                    formats,
                });
            }
        }

        let duration = self.duration;
        let chapters = self
            .chapters
            .iter()
            .filter_map(|c| {
                let start = c.start_time?;
                let end = c.end_time.or(duration)?;
                (end > start).then(|| Chapter {
                    start,
                    end,
                    title: c.title.clone().unwrap_or_default(),
                })
            })
            .collect();

        let live_status = self.live_status.clone();
        let is_live = self.is_live.unwrap_or(false)
            || matches!(live_status.as_deref(), Some("is_live" | "is_upcoming"));
        let id = self.id.clone().unwrap_or_default();
        let mut info = VideoInfo {
            title: self.title.or(self.fulltitle).unwrap_or_else(|| id.clone()),
            id,
            uploader: self.uploader.clone().or_else(|| self.channel.clone()),
            channel: self.channel,
            duration,
            upload_date: self.upload_date,
            description: self.description,
            thumbnail,
            webpage_url: self.webpage_url.or(self.original_url).unwrap_or_default(),
            extractor: self.extractor.or(self.extractor_key).unwrap_or_default(),
            is_live,
            live_status,
            chapters,
            subtitles,
            formats,
            choices: Vec::new(),
            has_drm,
        };
        info.choices = crate::choices::compute_choices(&info, Container::Mp4);
        info
    }
}

/// Parse `yt-dlp -J` output into a [`Probe`].
pub fn parse_probe_json(json: &str) -> crate::Result<Probe> {
    let raw: RawInfo = serde_json::from_str(json)?;
    Ok(if raw.is_playlist() {
        Probe::Playlist(raw.into_playlist())
    } else {
        Probe::Video(Box::new(raw.into_video()))
    })
}
