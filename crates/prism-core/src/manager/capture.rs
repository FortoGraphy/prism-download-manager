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
}

pub(crate) struct PendingCapture {
    pub info: CaptureInfo,
    pub item: NewItem,
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
    /// For playlists: the entry URLs to download with the same choice.
    pub entries: Vec<String>,
    pub mode: StartMode,
    /// Browser context (cookies, user agent, referer) for sites that need a session.
    pub context: Option<serde_json::Value>,
}

fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
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
        let request = n
            .request
            .clone()
            .or_else(|| Some(jobs::request_json(&n, settings)));
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
            resumable: None,
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
        let _rt = crate::rt::enter();
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
        let settings = self.settings();
        let kind = n.kind.unwrap_or_else(|| ItemKind::from_url(&n.url));
        let file_name = n
            .file_name
            .clone()
            .map(|f| super::sanitize_name(&f))
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
            },
        );
        self.emit(CoreEvent::CaptureRequested(info.clone()));
        if info.probing {
            let core = self.clone();
            let id = info.id.clone();
            tokio::spawn(async move { core.probe_capture(&id).await });
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
                    if !user_named && !probe.file_name.is_empty() {
                        p.info.file_name = super::sanitize_name(&probe.file_name);
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
        let v = VideoCapture {
            id: new_id(),
            page_url,
            title,
            streams,
            preselected,
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

    /// Resolve formats of a video page (yt-dlp).
    pub async fn probe_video(
        &self,
        page_url: &str,
        ctx: prism_video::RequestContext,
    ) -> Result<serde_json::Value> {
        jobs::probe_video(self, page_url, ctx).await
    }

    pub fn confirm_video(&self, d: VideoDecision) -> Result<Vec<DownloadItem>> {
        if !d.capture_id.is_empty() {
            self.0.state.lock().videos.remove(&d.capture_id);
        }
        let settings = self.settings();
        let urls = if d.entries.is_empty() {
            vec![d.page_url.clone()]
        } else {
            d.entries.clone()
        };
        let single = urls.len() == 1;
        let entries: Vec<NewItem> = urls
            .into_iter()
            .map(|url| {
                let media = MediaInfo {
                    title: if single { d.title.clone() } else { None },
                    thumbnail: if single { d.thumbnail.clone() } else { None },
                    duration: if single { d.duration } else { None },
                    quality: d.choice.get("label").and_then(|v| v.as_str()).map(String::from),
                    site: url::Url::parse(&url).ok().and_then(|u| u.host_str().map(String::from)),
                    page_url: Some(url.clone()),
                };
                let title = media.title.clone().unwrap_or_else(|| "Video".into());
                let ext = jobs::video_ext(&d.choice, &d.options, &settings);
                let stem = render_name_template(&settings.video.file_template, &title, video_id(&url).as_deref());
                let file_name = format!("{}.{ext}", super::sanitize_name(&stem));
                NewItem {
                    url: url.clone(),
                    kind: Some(ItemKind::Video),
                    file_name: Some(file_name),
                    save_dir: d.save_dir.clone(),
                    category: Some(if jobs::choice_is_audio(&d.choice) { "music".into() } else { "video".into() }),
                    media: Some(media),
                    request: Some(serde_json::json!({
                        "pageUrl": url, "choice": d.choice, "options": d.options, "context": d.context,
                    })),
                    ..Default::default()
                }
            })
            .collect();
        self.add_items(entries, d.mode)
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
            _ => base,
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

    // ----- torrent -----------------------------------------------------------------------------

    /// Inspect a magnet / .torrent (resolving metadata) for the file-selection dialog.
    pub async fn inspect_torrent(
        &self,
        source: prism_torrent::TorrentSource,
    ) -> Result<prism_torrent::TorrentMeta> {
        let engine = self.torrent_engine().await?;
        engine.inspect(&source).await
    }

    pub fn add_torrent(
        &self,
        source: prism_torrent::TorrentSource,
        name: String,
        size: Option<u64>,
        only_files: Option<Vec<usize>>,
        save_dir: Option<PathBuf>,
        mode: StartMode,
    ) -> Result<DownloadItem> {
        let url = match &source {
            prism_torrent::TorrentSource::Url(u) => u.clone(),
            prism_torrent::TorrentSource::File(_) => format!("torrent:{name}"),
        };
        let n = NewItem {
            url,
            kind: Some(ItemKind::Torrent),
            file_name: Some(name),
            save_dir,
            category: Some("other".into()),
            size,
            request: Some(serde_json::json!({"source": source, "onlyFiles": only_files})),
            ..Default::default()
        };
        self.add_items(vec![n], mode)?
            .pop()
            .ok_or_else(|| anyhow!("nothing added"))
    }

    // ----- browser messages --------------------------------------------------------------------

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
                let n = NewItem {
                    url,
                    file_name: s("filename")
                        .map(|f| f.rsplit(['/', '\\']).next().unwrap_or(&f).to_string()),
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
                if settings.general.show_capture_dialog {
                    let info = self.capture(n, CaptureSource::Browser);
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
                    let d = VideoDecision {
                        page_url: page_url.clone(),
                        choice,
                        title: s("title").or_else(|| {
                            info.get("title").and_then(|v| v.as_str()).map(String::from)
                        }),
                        thumbnail: info
                            .get("thumbnail")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                        duration: info.get("duration").and_then(|v| v.as_f64()),
                        context: Some(
                            serde_json::json!({"cookies": s("cookies"), "userAgent": s("userAgent"), "referer": page_url}),
                        ),
                        ..Default::default()
                    };
                    return match self.confirm_video(d) {
                        Ok(_) => serde_json::json!({"ok": true, "accepted": true}),
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
                    let n = NewItem {
                        url,
                        kind: Some(match kind.as_str() {
                            "hls" => ItemKind::Hls,
                            "dash" => ItemKind::Dash,
                            _ => ItemKind::Http,
                        }),
                        file_name: s("title")
                            .filter(|_| kind != "direct")
                            .map(|t| format!("{}.mp4", super::sanitize_name(&t))),
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
                        ..Default::default()
                    };
                    return match self.add_items(vec![n], StartMode::Now) {
                        Ok(_) => serde_json::json!({"ok": true, "accepted": true}),
                        Err(e) => fail("app_error", e.to_string()),
                    };
                }
                self.open_video(page_url, s("title"), Vec::new(), None);
                serde_json::json!({"ok": true, "accepted": true})
            }
            _ => fail("bad_request", format!("unknown message type '{ty}'")),
        }
    }
}

/// Sites where the in-page button asks the app for formats (yt-dlp) instead of only
/// offering sniffed streams. The extension uses this list once connected (`capture_rules`), and
/// ships the same one as its default (`DEFAULT_VIDEO_SITES`, kept equal by a test there).
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
fn render_name_template(template: &str, title: &str, id: Option<&str>) -> String {
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
fn video_id(page_url: &str) -> Option<String> {
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
