//! Adding downloads: the "Download File" capture flow, batches, browser IPC messages,
//! video captures and torrents.

use super::{Core, jobs};
use crate::categories;
use crate::events::*;
use crate::ipc::{BoxFuture, IpcHandler};
use crate::model::*;
use crate::settings::{ConflictAction, Settings};
use anyhow::{Result, anyhow};
use serde::Deserialize;
use std::path::PathBuf;

/// How a newly added download should begin.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "type", content = "queueId", rename_all = "camelCase")]
pub enum StartMode {
    /// Start immediately.
    #[default]
    Now,
    /// Add to the list without starting ("Download later").
    Later,
    /// Add to a queue (starts when the queue runs).
    Queue(String),
}

/// Network context of a download as captured from the browser or typed by the user.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewItem {
    pub url: String,
    pub kind: Option<ItemKind>,
    pub file_name: Option<String>,
    pub save_dir: Option<PathBuf>,
    pub category: Option<String>,
    pub description: String,
    pub referer: Option<String>,
    pub cookies: Option<String>,
    pub user_agent: Option<String>,
    pub headers: Vec<(String, String)>,
    pub method: Option<String>,
    pub post_data: Option<String>,
    pub size: Option<u64>,
    pub mirrors: Vec<String>,
    pub checksum: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub conflict: Option<ConflictAction>,
    pub media: Option<MediaInfo>,
    /// Pre-built job spec (video / torrent jobs).
    pub request: Option<serde_json::Value>,
    pub grabber_project: Option<String>,
    /// The HLS/DASH quality picked from the page menu's list (`None`: the best one).
    pub variant: Option<prism_engine::VariantSelection>,
}

pub(crate) struct PendingCapture {
    pub info: CaptureInfo,
    pub item: NewItem,
    /// "What's inside" a ZIP: its files once listed, and the files downloaded on their own,
    /// which stop when the capture goes away.
    pub peek: super::peek::PeekSlot,
    /// The name unless the server names the file (a stream named after its page, #59).
    pub fallback_name: Option<String>,
}

/// The user's answer in the "Download File" dialog.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct CaptureDecision {
    pub id: String,
    pub file_name: Option<String>,
    pub save_dir: Option<PathBuf>,
    pub category: Option<String>,
    pub description: Option<String>,
    pub mode: StartMode,
    pub username: Option<String>,
    pub password: Option<String>,
    /// Remember `save_dir` as the category's folder.
    pub remember_dir: bool,
    /// Resolution when the file already exists.
    pub conflict: Option<ConflictAction>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct BatchDecision {
    pub id: String,
    pub urls: Vec<String>,
    pub save_dir: Option<PathBuf>,
    pub mode: StartMode,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct VideoDecision {
    /// Video capture id (when it came from the browser), or empty for a pasted URL.
    pub capture_id: String,
    pub page_url: String,
    /// The `Choice` returned by `probe_video`, echoed back.
    pub choice: serde_json::Value,
    /// Output options (container, audio format, embeds, ...); missing fields use settings.
    pub options: serde_json::Value,
    pub save_dir: Option<PathBuf>,
    pub title: Option<String>,
    pub thumbnail: Option<String>,
    pub duration: Option<f64>,
    /// For playlists: the entries to download with the same choice.
    pub entries: Vec<VideoEntry>,
    pub mode: StartMode,
    /// Browser context (cookies, user agent, referer) for sites that need a session.
    pub context: Option<serde_json::Value>,
    /// Playlists: download the entries one after another (the "One at a time" queue).
    pub one_at_a_time: bool,
    /// A single video: only this part of it ("from 1:23 to 2:45").
    pub section: Option<prism_video::Section>,
    /// A single video: the name typed in the window (#94). The format decides the extension.
    pub file_name: Option<String>,
}

/// One playlist entry picked in the video dialog. A plain URL string is accepted too.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(from = "VideoEntryIn")]
pub struct VideoEntry {
    pub url: String,
    pub title: Option<String>,
    /// 1-based position in the playlist.
    pub index: Option<u32>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum VideoEntryIn {
    Url(String),
    Entry {
        url: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        index: Option<u32>,
    },
}

impl From<VideoEntryIn> for VideoEntry {
    fn from(e: VideoEntryIn) -> Self {
        match e {
            VideoEntryIn::Url(url) => VideoEntry {
                url,
                ..Default::default()
            },
            VideoEntryIn::Entry { url, title, index } => VideoEntry {
                url,
                title: title.filter(|t| !t.trim().is_empty()),
                index,
            },
        }
    }
}

/// The queue the video dialog's "One at a time" puts a playlist into.
pub const ONE_AT_A_TIME_QUEUE: &str = "one-at-a-time";

fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// Largest `.torrent` file Prism reads from a link (big torrents list a lot of pieces).
const MAX_TORRENT_FILE: usize = 16 << 20;

/// A magnet link, or an http(s) link to a `.torrent` file.
pub(crate) fn is_torrent_link(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    ItemKind::from_url(&lower) == ItemKind::Torrent
        && (lower.starts_with("magnet:")
            || lower.starts_with("http://")
            || lower.starts_with("https://"))
}

/// A plain GET (what a link click is), as opposed to a form the browser posted.
fn is_get(n: &NewItem) -> bool {
    n.method
        .as_deref()
        .is_none_or(|m| m.eq_ignore_ascii_case("GET"))
}

/// The bencoded dictionary of a `.torrent` file (not a web page or an error message).
fn looks_like_torrent(bytes: &[u8]) -> bool {
    bytes.first() == Some(&b'd') && bytes.windows(6).any(|w| w == b"4:info")
}

/// Keeps a fetched `.torrent` file where the "Add torrent" window reads it from.
fn save_torrent_file(name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join("prism-torrents");
    std::fs::create_dir_all(&dir)?;
    let mut name = super::sanitize_name(name);
    if !name.to_ascii_lowercase().ends_with(".torrent") {
        name.push_str(".torrent");
    }
    let path = dir.join(format!("{}-{name}", &new_id()[..8]));
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// File name guessed from a URL before the server tells us better.
pub(crate) fn name_from_url(url: &str) -> String {
    let parsed = url::Url::parse(url).ok();
    let last = parsed
        .as_ref()
        .and_then(|u| {
            u.path_segments()
                .and_then(|mut s| s.next_back())
                .map(String::from)
        })
        .unwrap_or_default();
    let decoded = percent_encoding::percent_decode_str(&last)
        .decode_utf8_lossy()
        .to_string();
    if decoded.trim().is_empty() {
        parsed
            .and_then(|u| u.host_str().map(String::from))
            .unwrap_or_else(|| "download".into())
    } else {
        super::sanitize_name(&decoded)
    }
}

/// Extensions of the media files pages play and the video window saves.
const MEDIA_EXTS: &[&str] = &[
    "mp4", "m4v", "webm", "mkv", "mov", "avi", "flv", "ts", "3gp", "ogv", "mp3", "m4a", "aac",
    "ogg", "oga", "opus", "flac", "wav", "weba",
];

/// `name` without a media extension: `Clip.mp4` → `Clip`.
fn without_media_ext(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, ext)) if MEDIA_EXTS.contains(&ext.to_ascii_lowercase().as_str()) => stem,
        _ => name,
    }
}

/// A file name stem from a title or typed text (`None` if empty). Slashes become `_` instead
/// of cutting the name to its last part.
fn name_stem(text: &str) -> Option<String> {
    let stem = without_media_ext(text.trim()).trim();
    (!stem.is_empty()).then(|| super::sanitize_name(&stem.replace(['/', '\\'], "_")))
}

/// The name typed in the video window, with the chosen format's extension (a typed media
/// extension is replaced by it).
pub(crate) fn typed_video_name(typed: &str, ext: &str) -> Option<String> {
    name_stem(typed).map(|stem| format!("{stem}.{ext}"))
}

/// The extension of a media file: from its address when that names a media file, else from
/// the MIME type, else mp4.
fn media_ext(url: &str, mime: Option<&str>) -> String {
    let from_url = std::path::Path::new(&name_from_url(url))
        .extension()
        .map(|e| e.to_string_lossy().into_owned());
    from_url
        .into_iter()
        .chain(mime.and_then(prism_engine::filename::extension_for_mime))
        .map(|e| e.to_ascii_lowercase())
        .find(|e| MEDIA_EXTS.contains(&e.as_str()))
        .unwrap_or_else(|| "mp4".into())
}

/// `Page title.webm`: a direct media file a page plays, named after the page and keeping its
/// own type (#59).
pub(crate) fn direct_stream_name(title: &str, url: &str, mime: Option<&str>) -> Option<String> {
    name_stem(title).map(|stem| format!("{stem}.{}", media_ext(url, mime)))
}

/// Whether the server named the file (Content-Disposition), not Prism from its address.
fn server_named(probe: &jobs::ProbeResult, url: &str) -> bool {
    let from_address = |u: &str| {
        prism_engine::Url::parse(u)
            .ok()
            .map(|u| prism_engine::filename::derive_file_name(None, &u, probe.mime.as_deref()))
    };
    !probe.file_name.is_empty()
        && from_address(&probe.final_url).as_deref() != Some(probe.file_name.as_str())
        && from_address(url).as_deref() != Some(probe.file_name.as_str())
}

impl Core {
    pub(crate) fn resolve_save_dir(&self, settings: &Settings, category: &str) -> PathBuf {
        let st = self.0.state.lock();
        let cat = st.categories.iter().find(|c| c.id == category);
        categories::save_dir_for(
            cat,
            &settings.downloads.default_dir,
            settings.downloads.use_category_folders,
        )
    }

    fn build_item(&self, n: NewItem, settings: &Settings) -> DownloadItem {
        let kind = n.kind.unwrap_or_else(|| ItemKind::from_url(&n.url));
        let file_name = n
            .file_name
            .clone()
            .map(|f| super::sanitize_name(&f))
            .unwrap_or_else(|| name_from_url(&n.url));
        let category = n.category.clone().unwrap_or_else(|| {
            let st = self.0.state.lock();
            categories::classify(&st.categories, &file_name)
        });
        let save_dir = n
            .save_dir
            .clone()
            .unwrap_or_else(|| self.resolve_save_dir(settings, &category));
        let mut request = n.request.clone().or_else(|| {
            let mut req = jobs::request_json(&n, settings);
            // Read back by `jobs::spawn_engine` for HLS/DASH items.
            if let (Some(v), Some(obj)) = (n.variant, req.as_object_mut()) {
                obj.insert("variant".into(), serde_json::json!(v));
            }
            Some(req)
        });
        // A torrent link added without its torrent (a batch, quiet copied links): the torrent
        // engine reads it from the link itself (#20).
        if kind == ItemKind::Torrent
            && is_torrent_link(&n.url)
            && let Some(obj) = request.as_mut().and_then(|r| r.as_object_mut())
            && obj.get("source").is_none_or(|s| s.is_null())
        {
            obj.insert(
                "source".into(),
                serde_json::json!(prism_torrent::TorrentSource::Url(n.url.trim().to_string())),
            );
        }
        DownloadItem {
            id: new_id(),
            kind,
            url: n.url.clone(),
            final_url: None,
            file_name,
            save_dir,
            size: n.size,
            downloaded: 0,
            status: Status::Idle,
            error: None,
            category,
            queue_id: None,
            description: n.description.clone(),
            referer: n.referer.clone(),
            added_at: now_ms(),
            last_try_at: None,
            finished_at: None,
            // yt-dlp's ffmpeg doesn't continue a cut: a paused part of a video starts over.
            resumable: n.media.as_ref().and_then(|m| m.section).map(|_| false),
            connections: settings.downloads.default_connections,
            speed_limit: 0,
            checksum: n.checksum.clone(),
            trashed: false,
            position: 0,
            mirrors: n.mirrors.clone(),
            grabber_project: n.grabber_project.clone(),
            request,
            resume: None,
            media: n.media.clone(),
        }
    }

    /// Add items directly (no dialog).
    pub fn add_items(&self, entries: Vec<NewItem>, mode: StartMode) -> Result<Vec<DownloadItem>> {
        let _rt = crate::rt::enter();
        let settings = self.settings();
        let mut items = Vec::with_capacity(entries.len());
        let queue = match &mode {
            StartMode::Queue(q) => Some(q.clone()),
            _ => None,
        };
        let first = self.next_position(queue.as_deref());
        for (position, n) in (first..).zip(entries) {
            let mut item = self.build_item(n, &settings);
            item.queue_id = queue.clone();
            item.position = position;
            items.push(item);
        }
        self.insert_items(items.clone())?;
        match mode {
            StartMode::Now => {
                self.start_items(&items.iter().map(|i| i.id.clone()).collect::<Vec<_>>())
            }
            StartMode::Queue(q) => {
                // Joining a running queue means waiting for a slot.
                if self.queue_running(&q) {
                    for i in &items {
                        self.with_item(&i.id, |it| it.status = Status::Queued);
                    }
                }
            }
            StartMode::Later => {}
        }
        Ok(items)
    }

    /// Open the "Download File" flow for a URL. The server is probed in the background
    /// and the dialog updates when the real name/size arrive.
    pub fn capture(&self, n: NewItem, source: CaptureSource) -> CaptureInfo {
        self.capture_probed(n, source, None, None)
    }

    /// [`Self::capture`] with the link already tried (`probed`), so the dialog doesn't ask the
    /// server again. `fallback_name` is the name unless the server names the file.
    fn capture_probed(
        &self,
        mut n: NewItem,
        source: CaptureSource,
        mut probed: Option<std::result::Result<jobs::ProbeResult, jobs::ProbeFailure>>,
        fallback_name: Option<String>,
    ) -> CaptureInfo {
        let _rt = crate::rt::enter();
        // A link to a model on Hugging Face opens the model window, where its files are picked
        // (#29); a link to a file's page there downloads the file instead of the page.
        if n.kind.is_none() && is_get(&n) {
            let server = crate::huggingface::endpoint(&self.settings().hugging_face);
            match crate::huggingface::parse(&n.url, &server) {
                Some(crate::huggingface::Link::Repo(_)) if n.url.contains("://") => {
                    let url = n.url.clone();
                    self.emit(CoreEvent::OpenModel {
                        source: url.clone(),
                    });
                    return CaptureInfo {
                        id: new_id(),
                        source,
                        url,
                        ..Default::default()
                    };
                }
                Some(crate::huggingface::Link::FilePage(file)) => {
                    n.url = file;
                    probed = None;
                }
                _ => {}
            }
        }
        // A typed or copied link to a video page (YouTube, Vimeo, ...) gets the quality
        // picker instead of the plain file dialog, like IDM's video panel.
        if source != CaptureSource::Browser && is_video_page(&n.url) {
            let v = self.open_video(n.url.clone(), None, Vec::new(), None);
            return CaptureInfo {
                id: v.id,
                source,
                kind: ItemKind::Video,
                url: n.url,
                ..Default::default()
            };
        }
        // A torrent link (a magnet, or a link to a .torrent file) opens "Add torrent", where
        // its files are picked, instead of downloading the link as a file (#20).
        if n.kind.is_none() && is_get(&n) && is_torrent_link(&n.url) {
            let url = n.url.clone();
            self.open_torrent_link(n, source, None);
            return CaptureInfo {
                id: new_id(),
                source,
                kind: ItemKind::Torrent,
                url,
                ..Default::default()
            };
        }
        let settings = self.settings();
        let kind = n.kind.unwrap_or_else(|| ItemKind::from_url(&n.url));
        let file_name = n
            .file_name
            .clone()
            .map(|f| super::sanitize_name(&f))
            .or_else(|| fallback_name.clone())
            .unwrap_or_else(|| name_from_url(&n.url));
        let category = {
            let st = self.0.state.lock();
            categories::classify(&st.categories, &file_name)
        };
        let save_dir = self.resolve_save_dir(&settings, &category);
        let info = CaptureInfo {
            id: new_id(),
            source,
            kind,
            url: n.url.clone(),
            final_url: None,
            exists: save_dir.join(&file_name).exists(),
            file_name,
            size: n.size,
            mime: None,
            resumable: None,
            category,
            save_dir,
            referer: n.referer.clone(),
            page_url: None,
            description: n.description.clone(),
            probing: matches!(kind, ItemKind::Http | ItemKind::Ftp),
            probe_error: None,
            needs_auth: false,
        };
        self.0.state.lock().captures.insert(
            info.id.clone(),
            PendingCapture {
                info: info.clone(),
                item: n,
                peek: Default::default(),
                fallback_name,
            },
        );
        self.emit(CoreEvent::CaptureRequested(info.clone()));
        if info.probing {
            match probed {
                Some(result) => {
                    let settings = self.settings();
                    self.apply_probe(&info.id, result, &settings);
                }
                None => {
                    let core = self.clone();
                    let id = info.id.clone();
                    tokio::spawn(async move { core.probe_capture(&id).await });
                }
            }
        }
        info
    }

    async fn probe_capture(&self, id: &str) {
        let (n, settings) = {
            let st = self.0.state.lock();
            let Some(p) = st.captures.get(id) else { return };
            (p.item.clone(), st.settings.clone())
        };
        let result = jobs::probe(self, &n, &settings).await;
        self.apply_probe(id, result, &settings);
    }

    /// Fills the Download File dialog with what the server said.
    fn apply_probe(
        &self,
        id: &str,
        result: std::result::Result<jobs::ProbeResult, jobs::ProbeFailure>,
        settings: &Settings,
    ) {
        let info = {
            let mut st = self.0.state.lock();
            let categories_list = st.categories.clone();
            let Some(p) = st.captures.get_mut(id) else {
                return;
            };
            p.info.probing = false;
            match result {
                Ok(probe) => {
                    let user_named = p.item.file_name.is_some();
                    let renamed = if !user_named && !probe.file_name.is_empty() {
                        p.info.file_name = match &p.fallback_name {
                            Some(name) if !server_named(&probe, &p.item.url) => name.clone(),
                            _ => super::sanitize_name(&probe.file_name),
                        };
                        true
                    } else if let Some(name) = prism_engine::filename::with_mime_extension(
                        &p.info.file_name,
                        probe.mime.as_deref(),
                    ) {
                        // A name without an extension (the browser's, from the address) gets
                        // the type the server sent (#121).
                        p.info.file_name = name;
                        true
                    } else {
                        false
                    };
                    if renamed {
                        let cat = categories::classify(&categories_list, &p.info.file_name);
                        if cat != p.info.category {
                            p.info.category = cat.clone();
                            let c = categories_list.iter().find(|c| c.id == cat);
                            p.info.save_dir = categories::save_dir_for(
                                c,
                                &settings.downloads.default_dir,
                                settings.downloads.use_category_folders,
                            );
                        }
                    }
                    p.info.size = probe.size.or(p.info.size);
                    p.info.mime = probe.mime;
                    p.info.resumable = Some(probe.resumable);
                    p.info.final_url = Some(probe.final_url);
                }
                Err(e) => {
                    p.info.needs_auth = e.needs_auth;
                    p.info.probe_error = Some(e.message);
                }
            }
            p.info.exists = p.info.save_dir.join(&p.info.file_name).exists();
            p.info.clone()
        };
        self.emit(CoreEvent::CaptureUpdated(info));
    }

    /// Retry the probe (e.g. after the user entered credentials in the dialog).
    pub fn reprobe_capture(&self, id: &str, username: Option<String>, password: Option<String>) {
        let _rt = crate::rt::enter();
        {
            let mut st = self.0.state.lock();
            let Some(p) = st.captures.get_mut(id) else {
                return;
            };
            if username.is_some() {
                p.item.username = username;
                p.item.password = password;
                // Listed again with the new credentials.
                p.peek.forget_listing();
            }
            p.info.probing = true;
            p.info.probe_error = None;
        }
        let core = self.clone();
        let id = id.to_string();
        tokio::spawn(async move { core.probe_capture(&id).await });
    }

    pub fn pending_capture(&self, id: &str) -> Option<CaptureInfo> {
        self.0.state.lock().captures.get(id).map(|p| p.info.clone())
    }

    pub fn cancel_capture(&self, id: &str) {
        self.0.state.lock().captures.remove(id);
    }

    /// Accepts a pending capture with its suggested name and folder and starts it (used when
    /// the dialog cannot be shown). Returns the file name.
    pub fn confirm_capture_default(&self, id: &str) -> Result<String> {
        let item = self.confirm_capture(CaptureDecision {
            id: id.to_string(),
            mode: StartMode::Now,
            conflict: Some(ConflictAction::Rename),
            ..Default::default()
        })?;
        Ok(item.file_name)
    }

    pub fn confirm_capture(&self, d: CaptureDecision) -> Result<DownloadItem> {
        let pending = self
            .0
            .state
            .lock()
            .captures
            .remove(&d.id)
            .ok_or_else(|| anyhow!("capture expired"))?;
        let mut n = pending.item;
        let info = pending.info;
        let save_dir = d.save_dir.clone().unwrap_or(info.save_dir.clone());
        let category = d.category.clone().unwrap_or(info.category.clone());
        if d.remember_dir {
            let mut cat = self
                .0
                .state
                .lock()
                .categories
                .iter()
                .find(|c| c.id == category)
                .cloned();
            if let Some(c) = cat.as_mut() {
                c.save_dir = Some(save_dir.clone());
                self.upsert_category(c.clone())?;
            }
        }
        n.file_name = Some(d.file_name.clone().unwrap_or(info.file_name.clone()));
        n.save_dir = Some(save_dir);
        n.category = Some(category);
        n.size = info.size;
        if let Some(desc) = d.description {
            n.description = desc;
        }
        if d.username.is_some() {
            n.username = d.username;
            n.password = d.password;
        }
        n.conflict = d.conflict.or(Some(ConflictAction::Rename));
        let mut items = self.add_items(vec![n], d.mode)?;
        let item = items.pop().ok_or_else(|| anyhow!("nothing added"))?;
        if let Some(final_url) = info.final_url {
            self.with_item(&item.id, |i| {
                i.final_url = Some(final_url);
                i.resumable = info.resumable;
            });
        }
        Ok(self.item(&item.id).unwrap_or(item))
    }

    // ----- batch -------------------------------------------------------------------------------

    /// Expand a pattern like `https://x/file[001-100].zip` into a batch selection.
    pub fn batch_from_pattern(&self, pattern: &str) -> Result<BatchInfo> {
        let urls = prism_engine::util::expand_batch_pattern(pattern).map_err(|e| anyhow!("{e}"))?;
        Ok(self.open_batch(
            "Batch download".into(),
            None,
            urls.into_iter()
                .map(|url| BatchEntry {
                    url,
                    ..Default::default()
                })
                .collect(),
        ))
    }

    /// Open the batch selection dialog for a list of links.
    pub fn open_batch(
        &self,
        title: String,
        page_url: Option<String>,
        mut entries: Vec<BatchEntry>,
    ) -> BatchInfo {
        {
            let st = self.0.state.lock();
            for e in &mut entries {
                if e.file_name.is_empty() {
                    e.file_name = name_from_url(&e.url);
                }
                if e.category.is_empty() {
                    e.category = categories::classify(&st.categories, &e.file_name);
                }
            }
        }
        let mut seen = std::collections::HashSet::new();
        entries.retain(|e| seen.insert(e.url.clone()));
        let info = BatchInfo {
            id: new_id(),
            title,
            page_url,
            entries,
        };
        self.0
            .state
            .lock()
            .batches
            .insert(info.id.clone(), info.clone());
        self.emit(CoreEvent::BatchRequested(info.clone()));
        info
    }

    pub fn pending_batch(&self, id: &str) -> Option<BatchInfo> {
        self.0.state.lock().batches.get(id).cloned()
    }

    pub fn confirm_batch(&self, d: BatchDecision) -> Result<Vec<DownloadItem>> {
        let batch = self
            .0
            .state
            .lock()
            .batches
            .remove(&d.id)
            .ok_or_else(|| anyhow!("batch expired"))?;
        let wanted: std::collections::HashSet<&String> = d.urls.iter().collect();
        let entries: Vec<NewItem> = batch
            .entries
            .iter()
            .filter(|e| wanted.contains(&e.url))
            .map(|e| NewItem {
                url: e.url.clone(),
                file_name: Some(e.file_name.clone()),
                save_dir: d.save_dir.clone(),
                category: Some(e.category.clone()),
                referer: e.referer.clone().or(batch.page_url.clone()),
                size: e.size,
                conflict: Some(ConflictAction::Rename),
                ..Default::default()
            })
            .collect();
        self.add_items(entries, d.mode)
    }

    pub fn cancel_batch(&self, id: &str) {
        self.0.state.lock().batches.remove(id);
    }

    // ----- video -------------------------------------------------------------------------------

    pub fn open_video(
        &self,
        page_url: String,
        title: Option<String>,
        streams: Vec<SniffedStream>,
        preselected: Option<String>,
    ) -> VideoCapture {
        self.open_video_with(page_url, title, streams, preselected, None)
    }

    /// [`Self::open_video`] for a request from a browser tab, with its session.
    fn open_video_with(
        &self,
        page_url: String,
        title: Option<String>,
        streams: Vec<SniffedStream>,
        preselected: Option<String>,
        browser: Option<BrowserSession>,
    ) -> VideoCapture {
        let v = VideoCapture {
            id: new_id(),
            page_url,
            title,
            streams,
            preselected,
            browser,
        };
        self.0.state.lock().videos.insert(v.id.clone(), v.clone());
        self.emit(CoreEvent::VideoRequested(v.clone()));
        v
    }

    pub fn pending_video(&self, id: &str) -> Option<VideoCapture> {
        self.0.state.lock().videos.get(id).cloned()
    }

    pub fn cancel_video(&self, id: &str) {
        self.0.state.lock().videos.remove(id);
    }

    /// Formats of a video page for the Download video window: with the browser tab's session
    /// when the window was opened from it (age-restricted and members' videos), otherwise none
    /// (yt-dlp still reads the "Cookies from browser" setting itself).
    pub async fn probe_video_for_window(&self, page_url: &str) -> Result<serde_json::Value> {
        let browser = self
            .0
            .state
            .lock()
            .videos
            .values()
            .find(|v| v.page_url == page_url && v.browser.is_some())
            .and_then(|v| v.browser.clone());
        let ctx = match browser {
            Some(b) => prism_video::RequestContext {
                cookies: b.cookies,
                user_agent: b.user_agent,
                referer: Some(page_url.to_string()),
                ..Default::default()
            },
            None => Default::default(),
        };
        self.probe_video(page_url, ctx).await
    }

    /// Resolve formats of a video page (yt-dlp).
    pub async fn probe_video(
        &self,
        page_url: &str,
        ctx: prism_video::RequestContext,
    ) -> Result<serde_json::Value> {
        jobs::probe_video(self, page_url, ctx).await
    }

    /// Builds the `NewItem` for one video URL/format choice — shared by `confirm_video`'s
    /// immediate-start path and the "video" IPC handler's dialog-confirmed path, so a chosen
    /// quality is always named/categorized the same way regardless of which one starts it.
    #[allow(clippy::too_many_arguments)]
    fn video_item(
        &self,
        settings: &Settings,
        url: String,
        choice: &serde_json::Value,
        options: &serde_json::Value,
        context: Option<serde_json::Value>,
        title: Option<String>,
        thumbnail: Option<String>,
        duration: Option<f64>,
    ) -> NewItem {
        let media = MediaInfo {
            title: title.clone(),
            thumbnail,
            duration,
            quality: choice
                .get("label")
                .and_then(|v| v.as_str())
                .map(String::from),
            site: url::Url::parse(&url)
                .ok()
                .and_then(|u| u.host_str().map(String::from)),
            page_url: Some(url.clone()),
            playlist_index: None,
            section: None,
        };
        let title = media.title.clone().unwrap_or_else(|| "Video".into());
        let ext = jobs::video_ext(choice, options, settings);
        let stem = render_name_template(
            &settings.video.file_template,
            &title,
            video_id(&url).as_deref(),
        );
        let file_name = format!("{}.{ext}", super::sanitize_name(&stem));
        NewItem {
            url: url.clone(),
            kind: Some(ItemKind::Video),
            file_name: Some(file_name),
            category: Some(if jobs::choice_is_audio(choice) {
                "music".into()
            } else {
                "video".into()
            }),
            media: Some(media),
            request: Some(serde_json::json!({
                "pageUrl": url, "choice": choice, "options": options, "context": context,
            })),
            ..Default::default()
        }
    }

    /// Starts a video download right away, without a "Download File" confirmation dialog
    /// (used for playlist/batch entries, where showing one dialog per entry would be
    /// impractical). The single-video "video" IPC handler goes through `capture()` instead,
    /// so the user can still rename/relocate before it starts.
    pub fn confirm_video(&self, mut d: VideoDecision) -> Result<Vec<DownloadItem>> {
        let pending = if d.capture_id.is_empty() {
            None
        } else {
            self.0.state.lock().videos.remove(&d.capture_id)
        };
        // Opened from the in-page button: download with that tab's session.
        if d.context.is_none()
            && let Some(VideoCapture {
                page_url,
                browser: Some(b),
                ..
            }) = pending
        {
            d.context = Some(serde_json::json!({
                "cookies": b.cookies, "userAgent": b.user_agent, "referer": page_url,
            }));
        }
        let settings = self.settings();
        let playlist = !d.entries.is_empty();
        // Numbers as wide as the last position: 01..99, 001..120.
        let width = d
            .entries
            .iter()
            .filter_map(|e| e.index)
            .max()
            .map_or(2, |m| m.to_string().len().max(2));
        let entries: Vec<NewItem> = if playlist {
            d.entries
                .iter()
                .map(|e| {
                    // Each entry by its own title (the playlist's title is not the video's).
                    let mut n = self.video_item(
                        &settings,
                        e.url.clone(),
                        &d.choice,
                        &d.options,
                        d.context.clone(),
                        e.title.clone(),
                        None,
                        None,
                    );
                    if let Some(m) = n.media.as_mut() {
                        m.playlist_index = e.index;
                    }
                    if settings.video.number_playlist_videos
                        && let (Some(i), Some(name)) = (e.index, n.file_name.as_mut())
                    {
                        *name = numbered_name(name, i, width);
                    }
                    n.save_dir = d.save_dir.clone();
                    n
                })
                .collect()
        } else {
            let mut n = self.video_item(
                &settings,
                d.page_url.clone(),
                &d.choice,
                &d.options,
                d.context.clone(),
                d.title.clone(),
                d.thumbnail.clone(),
                d.duration,
            );
            if let Some(part) = d.section.and_then(|s| s.part_of(d.duration)) {
                only_part(&mut n, part);
            }
            // A name typed in the window, as typed (a part gets no range added to it).
            let ext = jobs::video_ext(&d.choice, &d.options, &settings);
            if let Some(name) = d
                .file_name
                .as_deref()
                .and_then(|f| typed_video_name(f, &ext))
            {
                n.file_name = Some(name);
            }
            n.save_dir = d.save_dir.clone();
            vec![n]
        };
        if playlist && d.one_at_a_time && matches!(d.mode, StartMode::Now | StartMode::Later) {
            let queue = self.one_at_a_time_queue()?;
            let items = self.add_items(entries, StartMode::Queue(queue.clone()))?;
            if d.mode == StartMode::Now {
                self.start_queue(&queue);
            }
            return Ok(items);
        }
        self.add_items(entries, d.mode)
    }

    /// The "One at a time" queue (one download at a time), created on first use.
    fn one_at_a_time_queue(&self) -> Result<String> {
        let existing = self
            .queues()
            .into_iter()
            .find(|q| q.id == ONE_AT_A_TIME_QUEUE);
        match existing {
            Some(q) if q.max_concurrent == 1 => {}
            Some(mut q) => {
                q.max_concurrent = 1;
                self.upsert_queue(q)?;
            }
            None => {
                let position = self.queues().len() as i64;
                self.upsert_queue(crate::model::Queue {
                    id: ONE_AT_A_TIME_QUEUE.into(),
                    name: "One at a time".into(),
                    max_concurrent: 1,
                    position,
                    ..Default::default()
                })?;
            }
        }
        Ok(ONE_AT_A_TIME_QUEUE.into())
    }

    /// Download a sniffed HLS/DASH/direct stream (no yt-dlp involved).
    pub fn add_stream(
        &self,
        stream: SniffedStream,
        page_url: Option<String>,
        title: Option<String>,
        mode: StartMode,
    ) -> Result<DownloadItem> {
        let kind = match stream.kind.as_str() {
            "hls" => ItemKind::Hls,
            "dash" => ItemKind::Dash,
            _ => ItemKind::from_url(&stream.url),
        };
        let base = title.clone().unwrap_or_else(|| name_from_url(&stream.url));
        let file_name = match kind {
            ItemKind::Hls | ItemKind::Dash => format!(
                "{}.mp4",
                super::sanitize_name(base.trim_end_matches(".m3u8").trim_end_matches(".mpd"))
            ),
            // A direct file named after its page keeps its own type (#59).
            _ => title
                .as_deref()
                .and_then(|t| direct_stream_name(t, &stream.url, stream.mime.as_deref()))
                .unwrap_or_else(|| name_from_url(&stream.url)),
        };
        let n = NewItem {
            url: stream.url,
            kind: Some(kind),
            file_name: Some(file_name),
            referer: page_url.clone(),
            size: stream.size,
            media: Some(MediaInfo {
                title,
                page_url,
                quality: stream.label,
                ..Default::default()
            }),
            ..Default::default()
        };
        self.add_items(vec![n], mode)?
            .pop()
            .ok_or_else(|| anyhow!("nothing added"))
    }

    /// Copied links with "Ask before adding copied links" off: straight into the list as
    /// "Download later" items (like JDownloader's link list), no window. Video pages still get
    /// the quality picker, since they need a choice. Returns what went into the list.
    pub fn add_copied_links(&self, urls: &[String]) -> Result<Vec<DownloadItem>> {
        let (pages, files): (Vec<&String>, Vec<&String>) =
            urls.iter().partition(|u| is_video_page(u));
        for url in pages {
            self.capture(
                NewItem {
                    url: url.clone(),
                    ..Default::default()
                },
                CaptureSource::Clipboard,
            );
        }
        if files.is_empty() {
            return Ok(Vec::new());
        }
        let entries = files
            .into_iter()
            .map(|url| NewItem {
                url: url.clone(),
                ..Default::default()
            })
            .collect();
        self.add_items(entries, StartMode::Later)
    }

    // ----- torrent -----------------------------------------------------------------------------

    /// Inspect a magnet / .torrent (resolving metadata) for the file-selection dialog.
    pub async fn inspect_torrent(
        &self,
        source: prism_torrent::TorrentSource,
    ) -> Result<prism_torrent::TorrentMeta> {
        let engine = self.torrent_engine().await?;
        engine.inspect(&source).await
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_torrent(
        &self,
        source: prism_torrent::TorrentSource,
        name: String,
        size: Option<u64>,
        only_files: Option<Vec<usize>>,
        save_dir: Option<PathBuf>,
        category: Option<String>,
        mode: StartMode,
    ) -> Result<DownloadItem> {
        let category = category
            .filter(|c| self.0.state.lock().categories.iter().any(|k| &k.id == c))
            .unwrap_or_else(|| "other".into());
        let url = match &source {
            prism_torrent::TorrentSource::Url(u) => u.clone(),
            prism_torrent::TorrentSource::File(_) => format!("torrent:{name}"),
        };
        let n = NewItem {
            url,
            kind: Some(ItemKind::Torrent),
            file_name: Some(name),
            save_dir,
            category: Some(category),
            size,
            request: Some(serde_json::json!({"source": source, "onlyFiles": only_files})),
            ..Default::default()
        };
        self.add_items(vec![n], mode)?
            .pop()
            .ok_or_else(|| anyhow!("nothing added"))
    }

    /// A torrent link captured like a download (#20): shown in "Add torrent" (`mode: None`), or
    /// with the Download File dialog off added right away. A `.torrent` file is fetched here,
    /// with the browser's cookies and referer, which the torrent engine wouldn't send; a link
    /// that doesn't give a torrent (a web page, an error) is downloaded as a plain file instead.
    fn open_torrent_link(&self, n: NewItem, source: CaptureSource, mode: Option<StartMode>) {
        let _rt = crate::rt::enter();
        let core = self.clone();
        tokio::spawn(async move {
            let link = n.url.trim().to_string();
            if link.to_ascii_lowercase().starts_with("magnet:") {
                match mode {
                    None => core.emit(CoreEvent::OpenTorrent { source: link }),
                    // Named "magnet…" until the metadata brings the torrent's own name.
                    Some(mode) => {
                        let src = prism_torrent::TorrentSource::Url(link);
                        let added =
                            core.add_torrent(src, "magnet".into(), None, None, None, None, mode);
                        if let Err(e) = added {
                            tracing::error!("could not add the magnet link: {e:#}");
                        }
                    }
                }
                return;
            }
            let name = n.file_name.clone().unwrap_or_else(|| name_from_url(&n.url));
            match core.fetch_torrent_file(&n).await {
                Ok(bytes) => match mode {
                    None => match save_torrent_file(&name, &bytes) {
                        Ok(path) => core.emit(CoreEvent::OpenTorrent {
                            source: path.to_string_lossy().into_owned(),
                        }),
                        Err(e) => tracing::error!("could not keep the .torrent file: {e}"),
                    },
                    Some(mode) => {
                        let name = super::sanitize_name(&name);
                        let name = name
                            .strip_suffix(".torrent")
                            .or_else(|| name.strip_suffix(".TORRENT"))
                            .unwrap_or(&name)
                            .to_string();
                        let src = prism_torrent::TorrentSource::File(bytes);
                        if let Err(e) = core.add_torrent(src, name, None, None, None, None, mode) {
                            tracing::error!("could not add the torrent: {e:#}");
                        }
                    }
                },
                Err(e) => {
                    tracing::info!("no torrent behind the link, downloading it as a file: {e:#}");
                    let n = NewItem {
                        kind: Some(ItemKind::Http),
                        ..n
                    };
                    match mode {
                        None => {
                            core.capture_probed(n, source, None, None);
                        }
                        Some(mode) => {
                            let n = NewItem {
                                conflict: Some(ConflictAction::Rename),
                                ..n
                            };
                            if let Err(e) = core.add_items(vec![n], mode) {
                                tracing::error!("could not add the download: {e:#}");
                            }
                        }
                    }
                }
            }
        });
    }

    /// Reads the `.torrent` file a link points to, with the download's headers and cookies.
    async fn fetch_torrent_file(&self, n: &NewItem) -> Result<Vec<u8>> {
        let settings = self.settings();
        let mut req: prism_engine::DownloadRequest =
            serde_json::from_value(jobs::request_json(n, &settings))?;
        req.proxy = Some(jobs::proxy_config(&settings));
        req.http = jobs::http_preference(&settings);
        let cfg = jobs::engine_config(&settings);
        let fetch = prism_engine::fetch_small_file(&cfg, &req, MAX_TORRENT_FILE);
        let file = tokio::time::timeout(std::time::Duration::from_secs(30), fetch)
            .await
            .map_err(|_| anyhow!("the server did not answer in time"))??;
        anyhow::ensure!(
            looks_like_torrent(&file.bytes),
            "the link gave {} instead of a torrent",
            file.mime.as_deref().unwrap_or("something else")
        );
        Ok(file.bytes)
    }

    // ----- browser messages --------------------------------------------------------------------

    /// Tries a browser download's link before Prism takes it over (`None`: not tried, or no
    /// answer within [`TRY_LINK_TIME`], so the extension's hand-off still answers in time).
    async fn try_link(
        &self,
        n: &NewItem,
        settings: &Settings,
    ) -> Option<std::result::Result<jobs::ProbeResult, jobs::ProbeFailure>> {
        let get = n
            .method
            .as_deref()
            .is_none_or(|m| m.eq_ignore_ascii_case("GET"));
        let http = n.url.starts_with("https://") || n.url.starts_with("http://");
        if !get || !http {
            return None;
        }
        let tried = tokio::time::timeout(TRY_LINK_TIME, jobs::probe(self, n, settings))
            .await
            .ok();
        match &tried {
            Some(Ok(p)) => tracing::info!(
                "link tried: {} bytes, {}",
                p.size.map_or("?".into(), |s| s.to_string()),
                p.mime.as_deref().unwrap_or("no type")
            ),
            Some(Err(f)) => tracing::info!("link tried: {}", f.message),
            None => tracing::info!("link tried: no answer in time"),
        }
        tried
    }

    /// Capture rules handed to the extension (protocol v1 `hello` reply `settings`).
    pub fn capture_rules(&self) -> serde_json::Value {
        let s = self.settings();
        serde_json::json!({
            "captureEnabled": s.capture.enabled,
            "extensions": s.capture.extensions.iter().map(|e| e.trim_start_matches('.').to_ascii_uppercase()).collect::<Vec<_>>(),
            "mimeTypes": s.capture.mime_types,
            "excludedSites": s.capture.excluded_sites,
            "minSize": s.capture.min_size_kb * 1024,
            "bypassKey": s.capture.bypass_key,
            "showVideoButton": s.capture.video_button,
            "videoSites": VIDEO_SITES,
        })
    }

    /// Answer one browser message (protocol v1, see `extension/README.md`). Replies are
    /// immediate: dialogs open asynchronously so a captured browser download never waits.
    async fn handle_message(&self, msg: serde_json::Value) -> serde_json::Value {
        let now = now_ms();
        let before = self
            .0
            .ext_seen
            .swap(now, std::sync::atomic::Ordering::Relaxed);
        // Remember it across restarts (at most once a minute), so the app doesn't claim the
        // browser is disconnected until the extension's next hello.
        if now - before > 60_000 {
            let _ = std::fs::write(self.0.data_dir.join("extension-seen"), now.to_string());
        }
        let ty = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let s = |key: &str| {
            msg.get(key)
                .and_then(|v| v.as_str())
                .filter(|v| !v.is_empty())
                .map(String::from)
        };
        let fail = |code: &str, message: String| serde_json::json!({"ok": false, "error": {"code": code, "message": message}});
        let app = serde_json::json!({"name": "Prism", "version": crate::APP_VERSION});
        match ty {
            "hello" => {
                if let Some(v) = s("version") {
                    let kind = s("installType");
                    let mut cur = self.0.ext_version.lock();
                    let mut cur_kind = self.0.ext_install_type.lock();
                    if cur.as_deref() != Some(v.as_str()) || *cur_kind != kind {
                        let text = format!("{v}\n{}", kind.as_deref().unwrap_or(""));
                        let _ = std::fs::write(self.0.data_dir.join("extension-version"), text);
                        *cur = Some(v);
                        *cur_kind = kind;
                    }
                }
                serde_json::json!({"ok": true, "app": app, "protocol": 1, "settings": self.capture_rules()})
            }
            "status" | "ping" => {
                serde_json::json!({"ok": true, "app": app, "captureEnabled": self.settings().capture.enabled})
            }
            "open" | "show" => {
                self.emit(CoreEvent::ShowMain);
                serde_json::json!({"ok": true})
            }
            "download" => {
                tracing::info!(
                    "browser download request from {}",
                    s("url")
                        .as_deref()
                        .and_then(|u| url::Url::parse(u).ok())
                        .and_then(|u| u.host_str().map(str::to_string))
                        .unwrap_or_default()
                );
                let Some(url) = s("url").or_else(|| s("finalUrl")) else {
                    return fail("bad_request", "missing url".into());
                };
                // When the browser followed a cross-origin redirect, use the final URL with its cookies.
                let (url, cookies) = match (s("finalUrl"), s("finalUrlCookies")) {
                    (Some(final_url), Some(c)) => (final_url, Some(c)),
                    _ => (url, s("cookies")),
                };
                // A name the browser gave without an extension gets the type's (#121).
                let mime = s("mime");
                let n = NewItem {
                    url,
                    file_name: s("filename")
                        .map(|f| f.rsplit(['/', '\\']).next().unwrap_or(&f).to_string())
                        .map(|f| {
                            prism_engine::filename::with_mime_extension(&f, mime.as_deref())
                                .unwrap_or(f)
                        }),
                    referer: s("referrer").or_else(|| s("pageUrl")),
                    cookies,
                    user_agent: s("userAgent"),
                    headers: header_list(msg.get("headers")),
                    method: s("method"),
                    post_data: s("postData"),
                    size: msg
                        .get("fileSize")
                        .and_then(|v| v.as_u64())
                        .filter(|v| *v > 0),
                    ..Default::default()
                };
                let settings = self.settings();
                // "Download with Prism" on a link to a model on Hugging Face: the model window.
                if is_get(&n)
                    && let Some(crate::huggingface::Link::Repo(_)) = crate::huggingface::parse(
                        &n.url,
                        &crate::huggingface::endpoint(&settings.hugging_face),
                    )
                {
                    self.emit(CoreEvent::OpenModel {
                        source: n.url.clone(),
                    });
                    return serde_json::json!({"ok": true, "accepted": true});
                }
                // Sites behind a bot check (Cloudflare's "Just a moment...", e.g. softpedia.com)
                // answer Prism with a page only the browser gets past. Try the link first, briefly
                // so the browser's download isn't held up, and leave such downloads to the browser.
                let probed = self.try_link(&n, &settings).await;
                if let Some(Err(f)) = &probed
                    && f.browser_only
                {
                    tracing::info!("left to the browser: {}", f.message);
                    let host = url::Url::parse(&n.url)
                        .ok()
                        .and_then(|u| u.host_str().map(str::to_string))
                        .unwrap_or_default();
                    self.emit(CoreEvent::LeftToBrowser { host });
                    return serde_json::json!({"ok": true, "accepted": false, "reason": "browserOnly"});
                }
                // A link to a .torrent file: "Add torrent" opens (with the dialog off, the torrent
                // goes straight into the list) instead of downloading the .torrent file (#20).
                if is_get(&n) && is_torrent_link(&n.url) {
                    let start = if settings.downloads.auto_start {
                        StartMode::Now
                    } else {
                        StartMode::Later
                    };
                    let mode = (!settings.general.show_capture_dialog).then_some(start);
                    self.open_torrent_link(n, CaptureSource::Browser, mode);
                    return serde_json::json!({"ok": true, "accepted": true});
                }
                if settings.general.show_capture_dialog {
                    let info = self.capture_probed(n, CaptureSource::Browser, probed, None);
                    serde_json::json!({"ok": true, "accepted": true, "downloadId": info.id})
                } else {
                    let mode = if settings.downloads.auto_start {
                        StartMode::Now
                    } else {
                        StartMode::Later
                    };
                    match self.add_items(
                        vec![NewItem {
                            conflict: Some(ConflictAction::Rename),
                            ..n
                        }],
                        mode,
                    ) {
                        Ok(items) => {
                            serde_json::json!({"ok": true, "accepted": true, "downloadId": items.first().map(|i| i.id.clone())})
                        }
                        Err(e) => fail("app_error", e.to_string()),
                    }
                }
            }
            "batch" => {
                let entries: Vec<BatchEntry> = msg
                    .get("items")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|e| {
                                let url = e.get("url")?.as_str()?.to_string();
                                let str_of = |k: &str| {
                                    e.get(k)
                                        .and_then(|v| v.as_str())
                                        .filter(|v| !v.is_empty())
                                        .map(String::from)
                                };
                                Some(BatchEntry {
                                    file_name: str_of("filename").unwrap_or_default(),
                                    size: e.get("fileSize").and_then(|v| v.as_u64()),
                                    mime: str_of("mime"),
                                    referer: str_of("referrer"),
                                    text: str_of("text"),
                                    category: String::new(),
                                    url,
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let count = entries.len();
                if count == 0 {
                    return serde_json::json!({"ok": true, "accepted": false, "count": 0});
                }
                let title = s("title").unwrap_or_else(|| "Links from page".into());
                self.open_batch(title, s("pageUrl"), entries);
                serde_json::json!({"ok": true, "accepted": true, "count": count})
            }
            "probeVideo" => {
                let Some(page_url) = s("pageUrl") else {
                    return fail("bad_request", "missing pageUrl".into());
                };
                if let Some(why) = not_a_single_video(&page_url) {
                    return fail("not_a_video_page", why.into());
                }
                // The extension sends the page's cookies + User-Agent: pass them on to the
                // probe (the native YouTube resolver needs the session; yt-dlp gets better
                // cookie coverage on the very first click).
                let ctx = prism_video::RequestContext {
                    cookies: s("cookies"),
                    user_agent: s("userAgent"),
                    referer: Some(page_url.clone()),
                    ..Default::default()
                };
                // `probe_video` keeps the result for the "video" request (the chosen format).
                match self.probe_video(&page_url, ctx).await {
                    Ok(probe) => video_formats_reply(&probe),
                    Err(e) => {
                        let raw = format!("{e:#}");
                        tracing::info!("probeVideo {page_url}: {raw}");
                        fail("app_error", readable_probe_error(&raw))
                    }
                }
            }
            "probeStream" => {
                // A sniffed HLS/DASH manifest: list its declared variants (no download) so the
                // page menu can show the real quality breakdown instead of one generic entry.
                let Some(url) = s("url") else {
                    return fail("bad_request", "missing url".into());
                };
                let Ok(parsed) = prism_engine::Url::parse(&url) else {
                    return fail("bad_request", "invalid url".into());
                };
                let kind = match s("kind").as_deref() {
                    Some("hls") => prism_engine::MediaKind::Hls,
                    Some("dash") => prism_engine::MediaKind::Dash,
                    _ => prism_engine::MediaKind::Auto,
                };
                let headers = header_list(msg.get("headers"));
                let referer = headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("referer"))
                    .map(|(_, v)| v.clone())
                    .or_else(|| s("pageUrl"));
                let probe = prism_engine::VariantProbeRequest {
                    url: parsed,
                    kind,
                    headers: headers
                        .into_iter()
                        .filter(|(k, _)| !k.eq_ignore_ascii_case("referer"))
                        .collect(),
                    cookies: s("cookies"),
                    referer,
                    user_agent: s("userAgent"),
                    proxy: None,
                };
                let cfg = jobs::engine_config(&self.settings());
                match prism_engine::list_variants(&cfg, &probe).await {
                    Ok(variants) => serde_json::json!({"ok": true, "variants": variants}),
                    Err(e) => fail("app_error", e.to_string()),
                }
            }
            "video" => {
                let Some(page_url) = s("pageUrl") else {
                    return fail("bad_request", "missing pageUrl".into());
                };
                let streams: Vec<serde_json::Value> = msg
                    .get("streams")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                // A format picked in the in-page menu: start right away using the cached probe.
                if let Some(format_id) = s("formatId") {
                    let cached = {
                        let mut st = self.0.state.lock();
                        st.video_probes
                            .retain(|_, (t, _)| t.elapsed() < std::time::Duration::from_secs(1800));
                        st.video_probes.get(&page_url).map(|(_, p)| p.clone())
                    };
                    let choice = cached
                        .as_ref()
                        .and_then(|p| p.get("choices"))
                        .and_then(|c| c.as_array())
                        .and_then(|c| {
                            c.iter()
                                .find(|c| {
                                    c.get("id").and_then(|v| v.as_str()) == Some(format_id.as_str())
                                })
                                .cloned()
                        })
                        .unwrap_or_else(|| serde_json::json!({"id": format_id}));
                    let info = cached.unwrap_or_default();
                    let title = s("title")
                        .or_else(|| info.get("title").and_then(|v| v.as_str()).map(String::from));
                    let thumbnail = info
                        .get("thumbnail")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let duration = info.get("duration").and_then(|v| v.as_f64());
                    let context = serde_json::json!({"cookies": s("cookies"), "userAgent": s("userAgent"), "referer": page_url});
                    let settings = self.settings();
                    // With the confirmation on: the Download video window with this quality
                    // picked — "Only a part", the folder and the other options are there, and
                    // the qualities show at once (the menu's probe is kept). It brings the tab's
                    // session along for its probe and the download.
                    if settings.general.show_capture_dialog {
                        let v = self.open_video_with(
                            page_url,
                            title,
                            Vec::new(),
                            Some(format_id),
                            Some(BrowserSession {
                                cookies: s("cookies"),
                                user_agent: s("userAgent"),
                            }),
                        );
                        return serde_json::json!({"ok": true, "accepted": true, "downloadId": v.id});
                    }
                    let n = self.video_item(
                        &settings,
                        page_url.clone(),
                        &choice,
                        &serde_json::Value::Null,
                        Some(context),
                        title,
                        thumbnail,
                        duration,
                    );
                    // The confirmation is off: it starts right away.
                    let mode = if settings.downloads.auto_start {
                        StartMode::Now
                    } else {
                        StartMode::Later
                    };
                    return match self.add_items(vec![n], mode) {
                        Ok(items) => {
                            serde_json::json!({"ok": true, "accepted": true, "downloadId": items.first().map(|i| i.id.clone())})
                        }
                        Err(e) => fail("app_error", e.to_string()),
                    };
                }
                // One sniffed stream (HLS/DASH/direct) chosen in the page menu or popup.
                if let Some(stream) = streams.first() {
                    let url = stream
                        .get("url")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let kind = stream
                        .get("kind")
                        .and_then(|v| v.as_str())
                        .unwrap_or("direct")
                        .to_string();
                    let headers = header_list(stream.get("headers"));
                    let referer = headers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("referer"))
                        .map(|(_, v)| v.clone())
                        .or(Some(page_url.clone()));
                    let variant_index = stream
                        .get("variantIndex")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as usize);
                    let settings = self.settings();
                    // Named after the page: a manifest's video as mp4, a direct file with its own
                    // type, unless the server names that file itself (#59).
                    let direct = kind == "direct";
                    let mime = stream.get("mime").and_then(|v| v.as_str());
                    let titled = s("title").and_then(|t| {
                        if direct {
                            direct_stream_name(&t, &url, mime)
                        } else {
                            Some(format!("{}.mp4", super::sanitize_name(&t)))
                        }
                    });
                    let mut n = NewItem {
                        url,
                        kind: Some(match kind.as_str() {
                            "hls" => ItemKind::Hls,
                            "dash" => ItemKind::Dash,
                            _ => ItemKind::Http,
                        }),
                        file_name: titled.clone().filter(|_| !direct),
                        referer,
                        headers: headers
                            .into_iter()
                            .filter(|(k, _)| !k.eq_ignore_ascii_case("referer"))
                            .collect(),
                        cookies: s("cookies"),
                        user_agent: s("userAgent"),
                        size: stream.get("size").and_then(|v| v.as_u64()),
                        media: Some(MediaInfo {
                            title: s("title"),
                            page_url: Some(page_url.clone()),
                            quality: stream
                                .get("quality")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                            ..Default::default()
                        }),
                        conflict: Some(ConflictAction::Rename),
                        // A quality from the probeStream list instead of the best one. The
                        // request is built when the item is added, after the dialog's name,
                        // folder and overwrite choice.
                        variant: variant_index.map(prism_engine::VariantSelection::Index),
                        ..Default::default()
                    };
                    // Same "Download File" dialog / `showCaptureDialog` setting as the formatId
                    // path above, instead of always starting right away.
                    let fallback = titled.filter(|_| direct);
                    return if settings.general.show_capture_dialog {
                        let info = self.capture_probed(n, CaptureSource::Browser, None, fallback);
                        serde_json::json!({"ok": true, "accepted": true, "downloadId": info.id})
                    } else {
                        // No dialog to show the server's name in: ask the server first.
                        if let Some(name) = fallback {
                            n.file_name = Some(match self.try_link(&n, &settings).await {
                                Some(Ok(p)) if server_named(&p, &n.url) => {
                                    super::sanitize_name(&p.file_name)
                                }
                                _ => name,
                            });
                        }
                        let mode = if settings.downloads.auto_start {
                            StartMode::Now
                        } else {
                            StartMode::Later
                        };
                        match self.add_items(vec![n], mode) {
                            Ok(items) => {
                                serde_json::json!({"ok": true, "accepted": true, "downloadId": items.first().map(|i| i.id.clone())})
                            }
                            Err(e) => fail("app_error", e.to_string()),
                        }
                    };
                }
                self.open_video_with(
                    page_url,
                    s("title"),
                    Vec::new(),
                    None,
                    Some(BrowserSession {
                        cookies: s("cookies"),
                        user_agent: s("userAgent"),
                    }),
                );
                serde_json::json!({"ok": true, "accepted": true})
            }
            _ => fail("bad_request", format!("unknown message type '{ty}'")),
        }
    }
}

/// Sites where the in-page button asks the app for formats (yt-dlp) instead of only
/// offering sniffed streams. The extension uses this list once connected (`capture_rules`), and
/// ships the same one as its default (`DEFAULT_VIDEO_SITES`, kept equal by a test there).
/// How long a browser download's link is tried before Prism takes it over: well inside the
/// extension's hand-off timeout (5 s), after which it would keep the download in the browser.
const TRY_LINK_TIME: std::time::Duration = std::time::Duration::from_millis(2500);

const VIDEO_SITES: &[&str] = &[
    "youtube.com",
    "youtu.be",
    "youtube-nocookie.com",
    "vimeo.com",
    "twitch.tv",
    "x.com",
    "twitter.com",
    "instagram.com",
    "tiktok.com",
    "facebook.com",
    "fb.watch",
    "reddit.com",
    "dailymotion.com",
    "soundcloud.com",
    "bilibili.com",
    "rumble.com",
    "streamable.com",
    "bandcamp.com",
    "ted.com",
    "nicovideo.jp",
    "odysee.com",
    "vk.com",
    "vk.ru",
    "vkvideo.ru",
    "ok.ru",
    "linkedin.com",
];

/// Pages of sites whose yt-dlp extractor only reads single posts (LinkedIn, Instagram): a
/// feed, profile or Explore page has nothing to read, and yt-dlp's answer (an "Unsupported URL"
/// or a redirect to the sign-in page) only confuses. `None` = probe it.
pub(crate) fn not_a_single_video(url: &str) -> Option<&'static str> {
    let u = url::Url::parse(url).ok()?;
    let host = u
        .host_str()?
        .trim_start_matches("www.")
        .trim_start_matches("m.");
    let path = u.path();
    let site = |s: &str| host == s || host.ends_with(&format!(".{s}"));
    // LinkedIn's listings (feed, company pages, profiles, search...). Other pages are read as
    // before: posts, learning, events, but also embeds and articles the generic reader may get.
    let first = path.trim_start_matches('/').split('/').next().unwrap_or("");
    if site("linkedin.com")
        && !path.starts_with("/feed/update/")
        && matches!(
            first,
            "" | "feed"
                | "company"
                | "in"
                | "school"
                | "showcase"
                | "groups"
                | "search"
                | "mynetwork"
                | "jobs"
                | "notifications"
                | "messaging"
                | "my-items"
        )
    {
        // Also what a post without its own id gets (LinkedIn's newer pages name a post only in
        // its text block, so a post without text has none).
        return Some(
            "Prism can't tell which LinkedIn post this video belongs to here. Open the post itself (··· › Copy link to post) and try there.",
        );
    }
    if site("instagram.com")
        && !path
            .split('/')
            .any(|seg| matches!(seg, "p" | "reel" | "reels" | "tv" | "stories"))
    {
        return Some(
            "This Instagram page lists posts. Point at the video inside a post, or open the post.",
        );
    }
    None
}

/// yt-dlp's errors for the in-page menu: what to do, not its raw output ("please report this
/// issue on ..."). The raw text goes to the log.
pub(crate) fn readable_probe_error(raw: &str) -> String {
    let m = raw.to_ascii_lowercase();
    if m.contains("couldn't read the cookies") {
        // prism-video's own explanation (see its error.rs), already readable.
        return raw.trim().to_string();
    }
    let login = [
        "/login",
        "authwall",
        "signin",
        "sign in",
        "log in",
        "logged-in",
        "login required",
    ];
    if login.iter().any(|w| m.contains(w)) {
        return "The site wants you to sign in to show this video. Sign in in your browser and try again.".into();
    }
    if m.contains("unsupported url") {
        return "This page doesn't show a single video Prism can read. Point at the video in a post, or open the video's own page.".into();
    }
    let gone = [
        "private",
        "video unavailable",
        "video is unavailable",
        "video is not available",
        "not available in your",
        "no longer available",
        "has been removed",
    ];
    if gone.iter().any(|w| m.contains(w)) {
        return "This video is private or not available.".into();
    }
    if m.contains("unable to extract") || m.contains("unable to download") {
        return "Prism couldn't find the video on this page (the site may have changed; Prism updates its video tool by itself).".into();
    }
    let first = raw
        .split(['\n', ';'])
        .next()
        .unwrap_or(raw)
        .split("please report")
        .next()
        .unwrap_or(raw)
        .trim()
        .trim_end_matches([',', '.', ' ']);
    let short: String = first.chars().take(160).collect();
    format!("Prism couldn't read this page: {short}.")
}

/// `a` is an older version than `b` (dotted numbers; anything unparsable compares as 0).
pub(crate) fn version_older(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.trim()
            .trim_start_matches('v')
            .split(['.', '-', '+'])
            .take(3)
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    let (mut a, mut b) = (parse(a), parse(b));
    a.resize(3, 0);
    b.resize(3, 0);
    a < b
}

/// Is this a page on a known video site (as opposed to a direct media file)?
pub(crate) fn is_video_page(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else {
        return false;
    };
    let Some(host) = u.host_str() else {
        return false;
    };
    let host = host.trim_start_matches("www.").trim_start_matches("m.");
    let direct = matches!(
        ItemKind::from_url(url),
        ItemKind::Hls | ItemKind::Dash | ItemKind::Torrent
    ) || crate::categories::extension_of(u.path())
        .is_some_and(|e| e.len() <= 4 && e != "html" && e != "php");
    !direct
        && VIDEO_SITES
            .iter()
            .any(|s| host == *s || host.ends_with(&format!(".{s}")))
}

/// Map a `prism_video::Probe` to the `probeVideo` reply the extension renders.
fn video_formats_reply(probe: &serde_json::Value) -> serde_json::Value {
    let formats: Vec<serde_json::Value> = probe
        .get("choices")
        .and_then(|c| c.as_array())
        .map(|choices| {
            // Same simple model as the app dialog: MP4 qualities (one per height/fps/HDR) and MP3.
            let mut seen = std::collections::HashSet::new();
            choices
                .iter()
                .filter(|c| match c.get("kind").and_then(|v| v.as_str()) {
                    Some("audio") => c.get("audioFormat").and_then(|v| v.as_str()) == Some("mp3"),
                    _ => seen.insert((
                        c.get("height").and_then(|v| v.as_u64()),
                        c.get("fps").and_then(|v| v.as_f64()).map(|f| f.round() as u64),
                        c.get("hdr").and_then(|v| v.as_bool()),
                    )),
                })
                .map(|c| {
                    let kind = c.get("kind").and_then(|v| v.as_str()).unwrap_or("video");
                    let (label, ext) = if kind == "audio" {
                        (serde_json::json!("MP3 · audio only"), serde_json::json!("mp3"))
                    } else {
                        let quality = c.get("label").and_then(|v| v.as_str()).and_then(|l| l.split(" · ").next()).unwrap_or("Video");
                        (serde_json::json!(format!("{quality} · MP4")), serde_json::json!("mp4"))
                    };
                    serde_json::json!({
                        "id": c.get("id"), "label": label, "kind": kind, "ext": ext,
                        "height": c.get("height"), "fps": c.get("fps"), "size": c.get("estSize"), "note": c.get("subLabel"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    serde_json::json!({
        "ok": true,
        "title": probe.get("title"),
        "thumbnail": probe.get("thumbnail"),
        "duration": probe.get("duration"),
        "formats": formats,
    })
}

fn header_list(v: Option<&serde_json::Value>) -> Vec<(String, String)> {
    match v {
        Some(serde_json::Value::Object(map)) => map
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
            .collect(),
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|h| {
                let name = h.get("name")?.as_str()?;
                let value = h.get("value")?.as_str()?;
                Some((name.to_string(), value.to_string()))
            })
            .collect(),
        _ => Vec::new(),
    }
}

impl IpcHandler for Core {
    fn handle(&self, message: serde_json::Value) -> BoxFuture<serde_json::Value> {
        let core = self.clone();
        Box::pin(async move { core.handle_message(message).await })
    }
}

#[cfg(test)]
mod tests {
    use super::is_video_page;

    #[test]
    fn video_pages_are_detected() {
        assert!(is_video_page("https://www.youtube.com/watch?v=aqz-KE-bpKQ"));
        assert!(is_video_page("https://youtu.be/aqz-KE-bpKQ"));
        assert!(is_video_page("https://vimeo.com/76979871"));
        assert!(is_video_page("https://m.youtube.com/shorts/abc"));
        // #25: VK serves videos on vk.ru and vkvideo.ru too.
        assert!(is_video_page("https://vk.ru/video-12345_67890"));
        assert!(is_video_page("https://vkvideo.ru/video-12345_67890"));
        assert!(is_video_page("https://ok.ru/video/1234567890"));
        // Privacy-enhanced embeds (the extension's video button inside the embed's frame).
        assert!(is_video_page(
            "https://www.youtube-nocookie.com/embed/aqz-KE-bpKQ"
        ));
        assert!(is_video_page(
            "https://www.linkedin.com/feed/update/urn:li:activity:7151241570371948544/"
        ));
        assert!(!is_video_page("https://example.com/video.mp4"));
        assert!(!is_video_page("https://cdn.vimeo.com/file.mp4"));
        assert!(!is_video_page("https://notyoutube.com/watch"));
    }
}

/// Applies the user's yt-dlp style name template (`%(title)s [%(id)s].%(ext)s`) to the fields
/// Prism knows; unknown fields and the brackets left empty by them are dropped.
/// `01 - Name.mkv`: a playlist video's file name with its position, `width` digits at least.
pub(crate) fn numbered_name(name: &str, index: u32, width: usize) -> String {
    format!("{index:0width$} - {name}")
}

/// Makes a video item download only `part`: the job keeps the range (retries and resumes cut the
/// same part), the list shows it, and the file is named apart from the whole video:
/// `Title [id] (1m23s–2m45s).mp4`.
pub(crate) fn only_part(n: &mut NewItem, part: prism_video::Section) {
    if let Some(name) = n.file_name.as_mut() {
        let path = std::path::Path::new(name.as_str());
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        *name = match path.extension() {
            Some(ext) => format!("{stem}{}.{}", part.file_suffix(), ext.to_string_lossy()),
            None => format!("{stem}{}", part.file_suffix()),
        };
    }
    if let Some(req) = n.request.as_mut().and_then(|r| r.as_object_mut()) {
        req.insert("section".into(), serde_json::json!(part));
    }
    if let Some(m) = n.media.as_mut() {
        m.section = Some(part);
    }
}

pub(crate) fn render_name_template(template: &str, title: &str, id: Option<&str>) -> String {
    let tpl = template.trim();
    let tpl = tpl.strip_suffix(".%(ext)s").unwrap_or(tpl);
    if tpl.is_empty() {
        return title.to_string();
    }
    let mut out = tpl
        .replace("%(title)s", title)
        .replace("%(id)s", id.unwrap_or(""));
    // Fields Prism doesn't know (uploader, upload_date, ...).
    while let Some(start) = out.find("%(") {
        let Some(len) = out[start..].find(")s") else {
            break;
        };
        out.replace_range(start..start + len + 2, "");
    }
    for empty in ["[]", "()", "{}"] {
        out = out.replace(empty, "");
    }
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let out = out.trim_matches(|c: char| c == '-' || c == '_' || c.is_whitespace());
    if out.is_empty() {
        title.to_string()
    } else {
        out.to_string()
    }
}

/// The site's video id where it is in the URL (YouTube watch/shorts/youtu.be links).
pub(crate) fn video_id(page_url: &str) -> Option<String> {
    let u = url::Url::parse(page_url).ok()?;
    let host = u
        .host_str()?
        .trim_start_matches("www.")
        .trim_start_matches("m.");
    match host {
        "youtube.com" | "music.youtube.com" => u
            .query_pairs()
            .find(|(k, _)| k == "v")
            .map(|(_, v)| v.into_owned())
            .or_else(|| {
                let mut seg = u.path_segments()?;
                matches!(seg.next(), Some("shorts" | "live" | "embed"))
                    .then(|| seg.next().map(String::from))?
            }),
        "youtu.be" => u.path_segments()?.next().map(String::from),
        _ => None,
    }
    .filter(|id| !id.is_empty())
}

#[cfg(test)]
mod name_template_tests {
    use super::*;

    #[test]
    fn renders_title_and_id() {
        let t = "%(title)s [%(id)s].%(ext)s";
        assert_eq!(
            render_name_template(t, "Gearbox", Some("OcP-mCFhMZg")),
            "Gearbox [OcP-mCFhMZg]"
        );
        assert_eq!(render_name_template(t, "Gearbox", None), "Gearbox");
        assert_eq!(
            render_name_template("%(uploader)s - %(title)s.%(ext)s", "Clip", None),
            "Clip"
        );
        assert_eq!(render_name_template("", "Clip", None), "Clip");
        assert_eq!(
            video_id("https://www.youtube.com/watch?v=OcP-mCFhMZg&t=3").as_deref(),
            Some("OcP-mCFhMZg")
        );
        assert_eq!(
            video_id("https://youtu.be/abc123").as_deref(),
            Some("abc123")
        );
        assert_eq!(
            video_id("https://www.youtube.com/shorts/xyz").as_deref(),
            Some("xyz")
        );
        assert_eq!(video_id("https://vimeo.com/123"), None);
    }
}

#[cfg(test)]
mod probe_message_tests {
    use super::{not_a_single_video, readable_probe_error, version_older};

    #[test]
    fn feed_and_profile_pages_of_post_only_sites_are_not_probed() {
        for url in [
            "https://www.linkedin.com/feed/",
            "https://www.linkedin.com/company/nerostech/posts/",
            "https://www.linkedin.com/in/someone/recent-activity/all/",
            "https://www.linkedin.com/",
            "https://www.linkedin.com/search/results/content/?keywords=prism",
            "https://www.instagram.com/explore/",
            "https://www.instagram.com/someone/",
        ] {
            assert!(not_a_single_video(url).is_some(), "{url}");
        }
        for url in [
            "https://www.linkedin.com/feed/update/urn:li:activity:7151241570371948544/",
            "https://www.linkedin.com/posts/the-mathworks_2_what-is-mathworks-cloud-center-activity-7151241570371948544-4Gu7",
            "https://www.linkedin.com/learning/some-course/intro",
            "https://www.linkedin.com/embed/feed/update/urn:li:share:7151241569537282048",
            "https://www.linkedin.com/pulse/some-article-someone/",
            "https://www.instagram.com/p/DBx1yZ2abc/",
            "https://www.instagram.com/reel/DBx1yZ2abc/",
            "https://www.youtube.com/watch?v=aqz-KE-bpKQ",
            "https://x.com/home",
        ] {
            assert!(not_a_single_video(url).is_none(), "{url}");
        }
    }

    #[test]
    fn yt_dlp_errors_become_readable() {
        // The company-page report: LinkedIn sent yt-dlp to its sign-in page.
        let login = "yt-dlp failed (exit code 1): Unsupported URL: https://www.linkedin.com/uas/login?session_redirect=https%3A%2F%2Fwww.linkedin.com%2Fcompany%2Fnerostech%2Fposts%2F\n\nTip: sign in to the site in your browser, then choose that browser under Settings";
        assert!(readable_probe_error(login).starts_with("The site wants you to sign in"));
        let unsupported =
            "yt-dlp failed (exit code 1): Unsupported URL: https://www.reddit.com/r/videos/";
        assert!(
            readable_probe_error(unsupported).starts_with("This page doesn't show a single video")
        );
        let extract = "yt-dlp failed (exit code 1): [instagram:user] explore: Unable to extract data; please report this issue on https://github.com/yt-dlp/yt-dlp/issues?q= , filling out the appropriate issue template. Confirm you are on the latest version using yt-dlp -U";
        let r = readable_probe_error(extract);
        assert!(
            r.starts_with("Prism couldn't find the video") && !r.contains("report"),
            "{r}"
        );
        let other = "yt-dlp failed (exit code 1): HTTP Error 500: Internal Server Error; please report this issue on https://github.com/yt-dlp/yt-dlp";
        assert_eq!(
            readable_probe_error(other),
            "Prism couldn't read this page: yt-dlp failed (exit code 1): HTTP Error 500: Internal Server Error."
        );
        let gone =
            "yt-dlp failed (exit code 1): [youtube] abc: Video unavailable. This video is private";
        assert_eq!(
            readable_probe_error(gone),
            "This video is private or not available."
        );
        let busy = "yt-dlp failed (exit code 1): HTTP Error 503: Service Unavailable";
        assert!(readable_probe_error(busy).starts_with("Prism couldn't read this page: "));
        let cookies = "yt-dlp failed (exit code 1): Could not copy Chrome cookie database\n\nTip: Prism couldn't read the cookies of the browser chosen under Settings";
        assert!(readable_probe_error(cookies).contains("couldn't read the cookies"));
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(version_older("1.0.0", "1.0.6"));
        assert!(version_older("1.0.5", "1.0.10"));
        assert!(version_older("0.9", "1.0.0"));
        assert!(!version_older("1.0.6", "1.0.6"));
        assert!(!version_older("1.0.7", "1.0.6"));
        assert!(!version_older("v1.0.6", "1.0.6"));
    }
}

#[cfg(test)]
mod file_name_tests {
    use super::*;
    use crate::manager::CoreOptions;
    use serde_json::json;
    use std::path::Path;
    use std::time::Duration;

    #[test]
    fn a_name_typed_for_a_video_gets_the_formats_extension() {
        let name = |typed: &str, ext: &str| typed_video_name(typed, ext);
        assert_eq!(name("My clip", "mp4").as_deref(), Some("My clip.mp4"));
        // A typed media extension is replaced by the format's.
        assert_eq!(name(" My clip.MKV ", "mp4").as_deref(), Some("My clip.mp4"));
        assert_eq!(name("My clip.mp3", "mp3").as_deref(), Some("My clip.mp3"));
        // Other dots stay.
        assert_eq!(
            name("Episode 1.5", "m4a").as_deref(),
            Some("Episode 1.5.m4a")
        );
        assert_eq!(
            name("AC/DC: Live?", "mp3").as_deref(),
            Some("AC_DC_ Live_.mp3")
        );
        for empty in ["", "   ", ".mp4"] {
            assert_eq!(name(empty, "mp4"), None, "{empty:?}");
        }
    }

    #[test]
    fn a_direct_stream_is_named_after_its_page_with_its_own_type() {
        let name =
            |title: &str, url: &str, mime: Option<&str>| direct_stream_name(title, url, mime);
        let cdn = "https://cdn.example.com/v";
        assert_eq!(
            name("Big Buck Bunny", &format!("{cdn}/1080p.mp4"), None).as_deref(),
            Some("Big Buck Bunny.mp4")
        );
        assert_eq!(
            name("Clip", &format!("{cdn}/720p.WEBM?sig=1"), Some("video/mp4")).as_deref(),
            Some("Clip.webm")
        );
        assert_eq!(
            name("Song", &format!("{cdn}/track.mp3"), None).as_deref(),
            Some("Song.mp3")
        );
        // No extension in the address: the MIME type's.
        assert_eq!(
            name(
                "Clip",
                &format!("{cdn}/videoplayback?id=3"),
                Some("video/webm")
            )
            .as_deref(),
            Some("Clip.webm")
        );
        assert_eq!(
            name(
                "Song",
                &format!("{cdn}/stream"),
                Some("audio/mp4; codecs=mp4a")
            )
            .as_deref(),
            Some("Song.m4a")
        );
        // Neither tells: mp4.
        assert_eq!(
            name(
                "Clip",
                &format!("{cdn}/get.php?id=3"),
                Some("application/octet-stream")
            )
            .as_deref(),
            Some("Clip.mp4")
        );
        // A tab showing the file itself is titled with the file's name.
        assert_eq!(
            name("1080p.mp4", &format!("{cdn}/1080p.mp4"), None).as_deref(),
            Some("1080p.mp4")
        );
        assert_eq!(
            name("Live 24/7", &format!("{cdn}/a.mp4"), None).as_deref(),
            Some("Live 24_7.mp4")
        );
        assert_eq!(name("  ", &format!("{cdn}/1080p.mp4"), None), None);
    }

    #[test]
    fn a_name_from_the_server_is_told_from_one_read_from_the_address() {
        let probe = |name: &str, final_url: &str, mime: Option<&str>| jobs::ProbeResult {
            final_url: final_url.into(),
            file_name: name.into(),
            size: None,
            mime: mime.map(String::from),
            resumable: true,
        };
        let url = "https://cdn.example.com/v/1080p.mp4";
        assert!(!server_named(
            &probe("1080p.mp4", url, Some("video/mp4")),
            url
        ));
        assert!(server_named(
            &probe("Trailer (2026).mp4", url, Some("video/mp4")),
            url
        ));
        // Redirected: the name of the address it ended at isn't the server's either.
        let edge = "https://edge.example.com/abc.mp4";
        assert!(!server_named(&probe("abc.mp4", edge, None), url));
        // An extension added for the MIME type.
        let bare = "https://cdn.example.com/videoplayback";
        assert!(!server_named(
            &probe("videoplayback.webm", bare, Some("video/webm")),
            bare
        ));
    }

    async fn core_in(dir: &Path) -> Core {
        let core = Core::start(CoreOptions {
            data_dir: dir.join("data"),
            ipc: false,
            clipboard: false,
        })
        .await
        .unwrap();
        core.update_settings(&json!({
            "downloads": {"defaultDir": dir.join("dl"), "useCategoryFolders": false, "autoStart": false},
            "general": {"showCaptureDialog": false},
            "antivirus": {"enabled": false},
        }))
        .unwrap();
        core
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_video_is_saved_under_the_name_typed_in_the_window() {
        let tmp = tempfile::tempdir().unwrap();
        let core = core_in(tmp.path()).await;
        let decide = |extra: serde_json::Value| -> VideoDecision {
            let mut d = json!({
                "pageUrl": "https://www.youtube.com/watch?v=aqz-KE-bpKQ",
                "title": "Big Buck Bunny",
                "duration": 635,
                "choice": {"id": "v1080", "kind": "video", "label": "1080p", "subLabel": "", "container": "mp4", "height": 1080},
                "options": {"container": "mp4"},
                "mode": {"type": "later"},
            });
            d.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            serde_json::from_value(d).unwrap()
        };
        let add = |extra| core.confirm_video(decide(extra)).unwrap();
        let name = |extra| add(extra)[0].file_name.clone();
        // Nothing typed (or an older window): named after the title, as before.
        assert_eq!(name(json!({})), "Big Buck Bunny [aqz-KE-bpKQ].mp4");
        assert_eq!(
            name(json!({"fileName": "  "})),
            "Big Buck Bunny [aqz-KE-bpKQ].mp4"
        );
        assert_eq!(name(json!({"fileName": "Bunny"})), "Bunny.mp4");
        assert_eq!(
            name(json!({"fileName": "Bunny.mp4", "options": {"container": "mkv"}})),
            "Bunny.mkv"
        );
        let mp3 = json!({
            "fileName": "Bunny theme",
            "choice": {"id": "a-mp3", "kind": "audio", "label": "MP3", "subLabel": "", "container": "mp4", "audioFormat": "mp3"},
            "options": {"container": "mp4", "audioFormat": "mp3"},
        });
        assert_eq!(name(mp3), "Bunny theme.mp3");
        // Only a part: the typed name, and the job keeps the range.
        let part = add(json!({"fileName": "Intro", "section": {"start": 83, "end": 165}}));
        assert_eq!(part[0].file_name, "Intro.mp4");
        assert_eq!(
            part[0].media.as_ref().unwrap().section,
            Some(prism_video::Section::new(83.0, 165.0))
        );
        assert_eq!(
            part[0].request.as_ref().unwrap()["section"],
            json!({"start": 83.0, "end": 165.0})
        );
        // Playlist entries keep their own titles.
        let list = add(json!({
            "fileName": "Bunny",
            "entries": [{"url": "https://www.youtube.com/watch?v=aaaaaaaaaaa", "title": "First", "index": 1}],
        }));
        assert_eq!(list[0].file_name, "First [aaaaaaaaaaa].mp4");
        core.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_name_without_an_extension_gets_the_type_of_the_file() {
        // #121: a .webp the browser named after its address came without an extension.
        let server = prism_testserver::TestServer::start(Default::default())
            .await
            .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let core = core_in(tmp.path()).await;
        let url = server.url("/file/photo?size=1000&type=image/webp");
        let send = |mime: Option<&str>| {
            let mut msg = json!({"type": "download", "url": url, "filename": "photo"});
            if let Some(m) = mime {
                msg["mime"] = json!(m);
            }
            core.handle_message(msg)
        };
        // Without the dialog: the type the browser saw.
        let reply = send(Some("image/webp")).await;
        let id = reply["downloadId"].as_str().expect("added").to_string();
        assert_eq!(core.item(&id).unwrap().file_name, "photo.webp");
        // In the Download File dialog: the type the server sends, once it answered.
        core.update_settings(&json!({"general": {"showCaptureDialog": true}}))
            .unwrap();
        let reply = send(None).await;
        let id = reply["downloadId"].as_str().expect("shown").to_string();
        let info = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let info = core.pending_capture(&id).unwrap();
                if !info.probing {
                    return info;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the server was not asked");
        assert_eq!(info.probe_error, None);
        assert_eq!(info.file_name, "photo.webp");
        // A name with an extension stays as the browser gave it.
        let reply = core
            .handle_message(json!({"type": "download", "url": url, "filename": "photo.png", "mime": "image/webp"}))
            .await;
        let id = reply["downloadId"].as_str().expect("shown").to_string();
        assert_eq!(core.pending_capture(&id).unwrap().file_name, "photo.png");
        core.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_direct_stream_from_the_page_menu_is_named_after_the_page() {
        let server = prism_testserver::TestServer::start(Default::default())
            .await
            .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let core = core_in(tmp.path()).await;
        let send = |url: String| {
            core.handle_message(json!({
                "type": "video",
                "pageUrl": "https://example.com/watch/42",
                "title": "Big Buck Bunny",
                "streams": [{"url": url, "kind": "direct", "mime": "video/mp4"}],
            }))
        };
        let cd = "&cd=attachment%3B%20filename%3D%22Bunny%20(2008).mp4%22";
        let mp4 = server.url("/file/1080p.mp4?size=1000&type=video/mp4");
        let webm = server.url("/file/1080p.webm?size=1000&type=video/webm");
        let named = format!("{mp4}{cd}");
        let cases = [
            (&mp4, "Big Buck Bunny.mp4"),
            (&webm, "Big Buck Bunny.webm"),
            (&named, "Bunny (2008).mp4"),
        ];

        // Started without the dialog: the name is settled before it is added.
        for (url, expected) in cases {
            let reply = send(url.clone()).await;
            let id = reply["downloadId"].as_str().expect("added").to_string();
            assert_eq!(core.item(&id).unwrap().file_name, expected, "{url}");
        }

        // The Download File dialog shows the same names once the server answered.
        core.update_settings(&json!({"general": {"showCaptureDialog": true}}))
            .unwrap();
        for (url, expected) in cases {
            let reply = send(url.clone()).await;
            let id = reply["downloadId"].as_str().expect("shown").to_string();
            let info = tokio::time::timeout(Duration::from_secs(20), async {
                loop {
                    let info = core.pending_capture(&id).unwrap();
                    if !info.probing {
                        return info;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await
            .expect("the server was not asked");
            assert_eq!(info.probe_error, None, "{url}");
            assert_eq!(info.file_name, expected, "{url}");
        }

        // A stream added in the Download video window.
        let stream = |url: &str| SniffedStream {
            url: url.into(),
            kind: "direct".into(),
            mime: None,
            size: None,
            label: None,
        };
        let page = Some("https://example.com/watch/42".to_string());
        let title = Some("Big Buck Bunny".to_string());
        let added = core
            .add_stream(stream(&webm), page.clone(), title, StartMode::Later)
            .unwrap();
        assert_eq!(added.file_name, "Big Buck Bunny.webm");
        let untitled = core
            .add_stream(stream(&webm), page, None, StartMode::Later)
            .unwrap();
        assert_eq!(untitled.file_name, "1080p.webm");
        core.shutdown().await;
    }
}
