//! Events emitted by the core for the UI (and anything else that listens).

use crate::manager::Bootstrap;
use crate::model::*;
use crate::settings::Settings;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Where a capture came from (affects defaults, e.g. clipboard captures always ask).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum CaptureSource {
    #[default]
    Manual,
    Browser,
    Clipboard,
    Batch,
    Grabber,
}

/// A download waiting for the user's decision in the "Download File" dialog.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct CaptureInfo {
    pub id: String,
    pub source: CaptureSource,
    pub kind: ItemKind,
    pub url: String,
    pub final_url: Option<String>,
    pub file_name: String,
    pub size: Option<u64>,
    pub mime: Option<String>,
    pub resumable: Option<bool>,
    pub category: String,
    pub save_dir: PathBuf,
    pub referer: Option<String>,
    pub page_url: Option<String>,
    pub description: String,
    /// The server probe is still running (size/name may change).
    pub probing: bool,
    pub probe_error: Option<String>,
    /// A file with this name already exists in `save_dir`.
    pub exists: bool,
    /// The server asked for credentials.
    pub needs_auth: bool,
}

/// One candidate link in a batch ("Download all links") selection dialog.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct BatchEntry {
    pub url: String,
    pub file_name: String,
    pub size: Option<u64>,
    pub mime: Option<String>,
    pub category: String,
    pub referer: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct BatchInfo {
    pub id: String,
    pub title: String,
    pub page_url: Option<String>,
    pub entries: Vec<BatchEntry>,
}

/// A video page or sniffed stream waiting for format selection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct VideoCapture {
    pub id: String,
    pub page_url: String,
    pub title: Option<String>,
    /// Direct streams sniffed by the extension (HLS/DASH/progressive).
    pub streams: Vec<SniffedStream>,
    /// Pre-selected choice from the in-page button menu.
    pub preselected: Option<String>,
    /// The browser's session for the page (cookies, user agent) when the request came from the
    /// in-page button: the window's probe and the download use it (age-restricted and members'
    /// videos). Stays in the core.
    #[serde(skip)]
    pub browser: Option<BrowserSession>,
}

/// Cookies and user agent of the browser tab a video request came from.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BrowserSession {
    pub cookies: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SniffedStream {
    pub url: String,
    /// `hls`, `dash` or `direct`.
    pub kind: String,
    pub mime: Option<String>,
    pub size: Option<u64>,
    pub label: Option<String>,
}

/// A file of a ZIP downloaded on its own from the Download File window ("What's inside").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ZipProgress {
    pub capture_id: String,
    /// The file's number in the archive.
    pub index: u32,
    /// Unpacked bytes written so far.
    pub done: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum CoreEvent {
    /// Full state after a reconnect or lagged stream: clients replace, not merge.
    Bootstrap(Box<Bootstrap>),
    ItemsAdded(Vec<DownloadItem>),
    ItemUpdated(DownloadItem),
    ItemsRemoved(Vec<String>),
    /// Batched live progress (~4 Hz) of running items.
    Progress(Vec<Progress>),
    QueuesChanged(Vec<Queue>),
    CategoriesChanged(Vec<Category>),
    SettingsChanged(Box<Settings>),
    CaptureRequested(CaptureInfo),
    CaptureUpdated(CaptureInfo),
    ZipProgress(ZipProgress),
    BatchRequested(BatchInfo),
    VideoRequested(VideoCapture),
    /// A download finished (UI shows the completion dialog / notification).
    Completed(DownloadItem),
    Failed(DownloadItem),
    QueueFinished {
        queue_id: String,
        name: String,
    },
    /// A post-queue power action will run after `seconds` unless cancelled.
    PostActionPending {
        action: PostAction,
        seconds: u32,
    },
    PlaySound {
        kind: String,
    },
    Notify {
        title: String,
        body: String,
    },
    /// A browser download Prism didn't take over: the site lets only a browser download it (a
    /// bot check), so the browser keeps downloading it.
    LeftToBrowser {
        host: String,
    },
    /// Show a torrent in the "Add torrent" window: a magnet link, or the path of a `.torrent`
    /// file Prism fetched from a link (#20).
    OpenTorrent {
        source: String,
    },
    /// Open the Hugging Face model window for a model's name or link (`""`: search).
    OpenModel {
        source: String,
    },
    ClipboardUrls(Vec<String>),
    Grabber {
        project_id: String,
        event: crate::grabber::GrabberEvent,
    },
    ToolsProgress {
        tool: String,
        downloaded: u64,
        total: Option<u64>,
    },
    /// A Tools job (convert, make smaller, ZIP) was added or changed.
    MediaJob(crate::media::MediaJob),
    /// Ask the shell to bring the main window forward (second instance, tray, ...).
    ShowMain,
    /// The app should exit (post action "exit").
    ExitRequested,
}
