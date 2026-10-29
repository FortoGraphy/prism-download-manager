//! Running jobs: the glue between download items and the engines that execute them
//! (`prism-engine` for HTTP/FTP/HLS/DASH, `prism-video` for yt-dlp/ffmpeg, `prism-torrent`).

use super::Core;
use super::capture::NewItem;
use crate::events::CoreEvent;
use crate::model::*;
use crate::settings::{self, ConflictAction, HttpVersion, ProxyMode, Settings};
use anyhow::{Context, Result, anyhow};
use parking_lot::Mutex;
use prism_engine::{
    ConflictPolicy, Credentials, DownloadEvent, DownloadHandle, DownloadRequest, DownloadState,
    EngineConfig, EngineError, ExpectedChecksum, HttpPreference, MediaKind, MediaRequest,
    ProxyConfig, ResumeState, SegmentStatus, Url, VariantSelection,
};
use prism_video::{
    AudioFormat, Choice, ChoiceKind, Container, OutputOptions, Plan, Probe, RequestContext,
    SponsorBlockMode, Stage, StreamRole, VideoOptions, YtProgress,
};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// Details window payload.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemDetails {
    pub item: DownloadItem,
    pub segments: Vec<SegmentInfo>,
    pub progress: Option<Progress>,
    pub speed_history: Vec<u64>,
}

pub(crate) struct JobOutcome {
    pub id: String,
    pub result: JobResult,
}

pub(crate) enum JobResult {
    Completed {
        path: PathBuf,
        size: u64,
    },
    Paused {
        resume: Option<serde_json::Value>,
    },
    Cancelled,
    Failed {
        error: String,
        resume: Option<serde_json::Value>,
        resumable: Option<bool>,
    },
}

/// A running job (cheap to clone).
#[derive(Clone)]
pub(crate) enum Job {
    Engine(Arc<EngineJob>),
    Video(Arc<VideoJob>),
    Torrent(Arc<TorrentJob>),
}

pub(crate) struct EngineJob {
    handle: DownloadHandle,
    /// Status detail shown in the list ("Retrying in 4 s", "Server allows 4 connections").
    detail: Mutex<Option<String>>,
}

impl Job {
    pub fn progress(&self, item: &DownloadItem) -> Progress {
        match self {
            Job::Engine(j) => {
                let s = j.handle.snapshot();
                let status = match s.state {
                    DownloadState::Probing => Status::Connecting,
                    DownloadState::Downloading if s.active_connections == 0 && s.downloaded > 0 => {
                        Status::Retrying
                    }
                    DownloadState::Downloading => Status::Downloading,
                    DownloadState::Finalizing | DownloadState::Merging => Status::Processing,
                    _ => Status::Downloading,
                };
                let detail = match s.state {
                    DownloadState::Merging => Some("Remuxing with ffmpeg".to_string()),
                    DownloadState::Finalizing => Some("Finishing".to_string()),
                    _ => j.detail.lock().clone(),
                };
                Progress {
                    id: item.id.clone(),
                    status,
                    downloaded: s.downloaded,
                    size: s.total.or(item.size),
                    speed: s.speed,
                    eta_secs: s.eta.map(|d| d.as_secs()),
                    connections: s.active_connections,
                    detail,
                }
            }
            Job::Video(j) => j.progress(item),
            Job::Torrent(j) => j.progress(item),
        }
    }

    pub fn segments(&self) -> Vec<SegmentInfo> {
        match self {
            Job::Engine(j) => j
                .handle
                .snapshot()
                .segments
                .iter()
                .map(segment_info)
                .collect(),
            Job::Video(j) => j.segments(),
            Job::Torrent(_) => Vec::new(),
        }
    }

    pub fn pause(&self) {
        match self {
            Job::Engine(j) => j.handle.request_pause(),
            Job::Video(j) => j.stop(StopKind::Pause),
            Job::Torrent(j) => j.stop(StopKind::Pause),
        }
    }

    pub fn cancel(&self, delete_files: bool) {
        match self {
            Job::Engine(j) => j.handle.cancel(delete_files),
            Job::Video(j) => j.stop(if delete_files {
                StopKind::CancelDelete
            } else {
                StopKind::Cancel
            }),
            Job::Torrent(j) => j.stop(StopKind::Cancel),
        }
    }

    pub fn set_speed_limit(&self, bps: u64) {
        match self {
            Job::Engine(j) => j.handle.set_speed_limit(bps),
            Job::Video(j) => {
                for h in j.handles.lock().iter() {
                    h.set_speed_limit(bps);
                }
            }
            Job::Torrent(_) => {}
        }
    }

    pub fn set_connections(&self, n: u8) {
        match self {
            Job::Engine(j) => j.handle.set_connections(n),
            Job::Video(j) => {
                for h in j.handles.lock().iter() {
                    h.set_connections(n);
                }
            }
            Job::Torrent(_) => {}
        }
    }

    pub fn change_url(&self, url: Url) {
        if let Job::Engine(j) = self {
            let handle = j.handle.clone();
            tokio::spawn(async move {
                if let Err(e) = handle.change_url(url).await {
                    tracing::warn!("change url: {}", e.user_message());
                }
            });
        }
    }

    /// Durable resume state (flushes and fsyncs data first).
    pub async fn checkpoint(&self) -> Option<serde_json::Value> {
        match self {
            Job::Engine(j) => j
                .handle
                .checkpoint()
                .await
                .and_then(|s| serde_json::to_value(s).ok()),
            Job::Video(j) => j.checkpoint().await,
            Job::Torrent(_) => None,
        }
    }
}

fn segment_info(s: &prism_engine::SegmentView) -> SegmentInfo {
    SegmentInfo {
        start: s.start,
        pos: s.pos,
        end: s.end,
        speed: s.speed,
        state: match s.status {
            SegmentStatus::Active => "active",
            SegmentStatus::Idle => "idle",
            SegmentStatus::Done => "done",
            SegmentStatus::Retrying => "retrying",
            SegmentStatus::Failed => "failed",
        }
        .into(),
    }
}

/// Segment layout of a paused item, from its stored resume state.
pub(crate) fn stored_segments(item: &DownloadItem) -> Vec<SegmentInfo> {
    let Some(state) = item
        .resume
        .as_ref()
        .and_then(|v| serde_json::from_value::<ResumeState>(v.clone()).ok())
    else {
        return Vec::new();
    };
    state
        .segments
        .iter()
        .map(|s| SegmentInfo {
            start: s.start,
            pos: s.pos,
            end: s.end,
            speed: 0,
            state: if s.pos >= s.end { "done" } else { "idle" }.into(),
        })
        .collect()
}

// ----- configuration mapping ----------------------------------------------------------------

pub(crate) fn proxy_config(s: &Settings) -> ProxyConfig {
    let p = &s.proxy;
    let creds = |v: &str| (!v.is_empty()).then(|| v.to_string());
    let with_scheme = |addr: &str, scheme: &str| {
        if addr.contains("://") {
            addr.to_string()
        } else {
            format!("{scheme}://{addr}")
        }
    };
    match p.mode {
        ProxyMode::None => ProxyConfig::None,
        ProxyMode::System => ProxyConfig::System,
        ProxyMode::Http if !p.address.trim().is_empty() => ProxyConfig::Http {
            url: with_scheme(p.address.trim(), "http"),
            username: creds(&p.username),
            password: creds(&p.password),
        },
        ProxyMode::Socks5 if !p.address.trim().is_empty() => ProxyConfig::Socks5 {
            url: with_scheme(p.address.trim(), "socks5h"),
            username: creds(&p.username),
            password: creds(&p.password),
        },
        _ => ProxyConfig::System,
    }
}

fn proxy_url(s: &Settings) -> Option<String> {
    match proxy_config(s) {
        ProxyConfig::Http { url, .. } | ProxyConfig::Socks5 { url, .. } => Some(url),
        _ => None,
    }
}

fn parse_dns_overrides(raw: &[String]) -> Vec<(String, std::net::SocketAddr)> {
    let mut out = Vec::new();
    for entry in raw {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let parts = entry
            .split_once('=')
            .or_else(|| entry.split_once("->"))
            .or_else(|| entry.split_once(':'));
        if let Some((host, addr_str)) = parts {
            let host = host.trim().to_string();
            let addr_str = addr_str.trim();
            if let Ok(sa) = addr_str.parse::<std::net::SocketAddr>() {
                out.push((host, sa));
            } else {
                let clean_ip = addr_str.trim_matches(|c| c == '[' || c == ']');
                if let Ok(ip) = clean_ip.parse::<std::net::IpAddr>() {
                    out.push((host, std::net::SocketAddr::new(ip, 0)));
                }
            }
        }
    }
    out
}

pub(crate) fn engine_config(s: &Settings) -> EngineConfig {
    let user_agent = if s.connection.user_agent.trim().is_empty() {
        crate::DEFAULT_USER_AGENT.to_string()
    } else {
        s.connection.user_agent.clone()
    };
    EngineConfig {
        user_agent,
        max_connections_cap: s.downloads.max_connections.clamp(1, 32),
        default_connections: s.downloads.default_connections.clamp(1, 32),
        connect_timeout: Duration::from_secs(s.connection.connect_timeout_secs.max(3)),
        read_timeout: Duration::from_secs(s.connection.read_timeout_secs.max(5)),
        max_retries: s.downloads.max_retries.max(1),
        default_proxy: proxy_config(s),
        max_connections_per_host: (s.connection.max_connections_per_host > 0)
            .then(|| s.connection.max_connections_per_host.min(u16::MAX as u32) as u16),
        ffmpeg_path: (!s.video.ffmpeg_path.trim().is_empty())
            .then(|| PathBuf::from(s.video.ffmpeg_path.trim())),
        default_dns_overrides: parse_dns_overrides(&s.connection.dns_overrides),
        ..EngineConfig::default()
    }
}

pub(crate) fn video_options(s: &Settings) -> VideoOptions {
    let path = |p: &str| (!p.trim().is_empty()).then(|| PathBuf::from(p.trim()));
    VideoOptions {
        ytdlp_path: path(&s.video.ytdlp_path),
        ffmpeg_path: path(&s.video.ffmpeg_path),
        cookies_from_browser: (!s.video.cookies_from_browser.is_empty())
            .then(|| s.video.cookies_from_browser.clone()),
        proxy: proxy_url(s),
        ..VideoOptions::default()
    }
}

pub(crate) fn http_preference(s: &Settings) -> HttpPreference {
    match s.connection.http_version {
        HttpVersion::Http1 => HttpPreference::Http1Only,
        HttpVersion::Auto => HttpPreference::Auto,
        HttpVersion::Http3 => HttpPreference::Http3Experimental,
    }
}

fn parse_checksum(s: &str) -> Option<ExpectedChecksum> {
    let (algo, hex) = s.split_once(':').unwrap_or_else(|| {
        // Bare hex: infer the algorithm from its length.
        match s.trim().len() {
            32 => ("md5", s),
            40 => ("sha1", s),
            _ => ("sha256", s),
        }
    });
    let algo = match algo.trim().to_ascii_lowercase().as_str() {
        "md5" => prism_engine::ChecksumAlgo::Md5,
        "sha1" | "sha-1" => prism_engine::ChecksumAlgo::Sha1,
        "sha256" | "sha-256" => prism_engine::ChecksumAlgo::Sha256,
        _ => return None,
    };
    let hex = hex.trim().to_ascii_lowercase();
    (!hex.is_empty() && hex.chars().all(|c| c.is_ascii_hexdigit()))
        .then_some(ExpectedChecksum { algo, hex })
}

/// Serialized `DownloadRequest` for a new item (destination is filled in at start).
pub(crate) fn request_json(n: &NewItem, _settings: &Settings) -> serde_json::Value {
    let Ok(url) = Url::parse(n.url.trim()) else {
        return serde_json::json!({"url": n.url});
    };
    let mut req = DownloadRequest::new(url, n.save_dir.clone().unwrap_or_default());
    req.mirrors = n
        .mirrors
        .iter()
        .filter_map(|m| Url::parse(m).ok())
        .collect();
    if n.method
        .as_deref()
        .is_some_and(|m| m.eq_ignore_ascii_case("POST"))
    {
        req.method = prism_engine::HttpMethod::Post;
        req.body = n.post_data.clone();
    }
    req.headers = n
        .headers
        .iter()
        .filter(|(k, _)| {
            // The engine manages these itself.
            !matches!(
                k.to_ascii_lowercase().as_str(),
                "range"
                    | "if-range"
                    | "host"
                    | "content-length"
                    | "accept-encoding"
                    | "connection"
                    | "cookie"
                    | "referer"
                    | "user-agent"
            )
        })
        .cloned()
        .collect();
    req.cookies = n.cookies.clone().filter(|c| !c.is_empty());
    req.referer = n.referer.clone();
    req.user_agent = n.user_agent.clone();
    if let Some(user) = n.username.clone().filter(|u| !u.is_empty()) {
        req.auth = Some(Credentials {
            username: user,
            password: n.password.clone().unwrap_or_default(),
            scheme: Default::default(),
        });
    }
    req.destination.file_name = n.file_name.clone();
    req.destination.conflict = match n.conflict {
        Some(ConflictAction::Overwrite) => ConflictPolicy::Overwrite,
        _ => ConflictPolicy::Rename,
    };
    req.checksum = n.checksum.as_deref().and_then(parse_checksum);
    serde_json::to_value(&req).unwrap_or_default()
}

pub(crate) fn set_request_url(item: &mut DownloadItem, url: &str) {
    if let Some(req) = item.request.as_mut().and_then(|r| r.as_object_mut()) {
        req.insert("url".into(), serde_json::json!(url));
    }
    if let (Some(state), Ok(u)) = (item.resume.take(), Url::parse(url)) {
        item.resume = serde_json::from_value::<ResumeState>(state)
            .ok()
            .map(|s| s.with_url(u))
            .and_then(|s| serde_json::to_value(s).ok());
    }
}

pub(crate) fn set_request_auth(item: &mut DownloadItem, username: String, password: String) {
    if let Some(req) = item.request.as_mut().and_then(|r| r.as_object_mut()) {
        let auth = if username.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::json!({"username": username, "password": password})
        };
        req.insert("auth".into(), auth);
    }
}

pub(crate) fn torrent_hash(item: &DownloadItem) -> Option<String> {
    item.request
        .as_ref()?
        .get("infoHash")?
        .as_str()
        .map(String::from)
}

pub(crate) fn video_temp_dir(item: &DownloadItem) -> PathBuf {
    item.save_dir
        .join(format!(".prism-{}", &item.id[..item.id.len().min(12)]))
}

pub(crate) fn choice_is_audio(choice: &serde_json::Value) -> bool {
    choice.get("kind").and_then(|k| k.as_str()) == Some("audio")
}

/// Extension of the final file for a video choice.
pub(crate) fn video_ext(
    choice: &serde_json::Value,
    options: &serde_json::Value,
    settings: &Settings,
) -> String {
    if choice_is_audio(choice) {
        let fmt = choice
            .get("audioFormat")
            .and_then(|v| v.as_str())
            .or_else(|| options.get("audioFormat").and_then(|v| v.as_str()))
            .unwrap_or(&settings.video.audio_format);
        return fmt.to_string();
    }
    options
        .get("container")
        .and_then(|v| v.as_str())
        .or_else(|| choice.get("container").and_then(|v| v.as_str()))
        .unwrap_or(&settings.video.container)
        .to_string()
}

// ----- probing ------------------------------------------------------------------------------

pub(crate) struct ProbeResult {
    pub final_url: String,
    pub file_name: String,
    pub size: Option<u64>,
    pub mime: Option<String>,
    pub resumable: bool,
}

pub(crate) struct ProbeFailure {
    pub message: String,
    pub needs_auth: bool,
    /// The site lets only a browser download it (a bot check, or a web page instead of the file).
    pub browser_only: bool,
}

pub(crate) async fn probe(
    core: &Core,
    n: &NewItem,
    settings: &Settings,
) -> std::result::Result<ProbeResult, ProbeFailure> {
    let value = request_json(n, settings);
    let mut req: DownloadRequest = serde_json::from_value(value).map_err(|_| ProbeFailure {
        message: "Invalid address".into(),
        needs_auth: false,
        browser_only: false,
    })?;
    req.expect_file = n
        .file_name
        .as_deref()
        .is_some_and(crate::categories::is_binary_file_name);
    req.proxy = Some(proxy_config(settings));
    req.http = http_preference(settings);
    crate::huggingface::authorize(&mut req, settings);
    match tokio::time::timeout(Duration::from_secs(20), core.0.engine.probe(&req)).await {
        Ok(Ok(p)) => Ok(ProbeResult {
            final_url: p.final_url.to_string(),
            file_name: p.file_name,
            size: p.size,
            mime: p.mime,
            resumable: p.resumable,
        }),
        Ok(Err(e)) => Err(ProbeFailure {
            needs_auth: matches!(e, EngineError::AuthRequired),
            // A 403 for a link the browser just downloaded means the server lets only the browser
            // in (a bot check by browser fingerprint, like Cloudflare's on softpedia.com).
            browser_only: matches!(
                e,
                EngineError::BrowserCheck
                    | EngineError::WebPage
                    | EngineError::Http { status: 403 }
            ),
            message: e.user_message(),
        }),
        Err(_) => Err(ProbeFailure {
            message: "The server did not answer in time".into(),
            needs_auth: false,
            browser_only: false,
        }),
    }
}

// ----- spawning -----------------------------------------------------------------------------

pub(crate) fn spawn(core: &Core, item: &DownloadItem) -> Result<Job> {
    match item.kind {
        ItemKind::Http | ItemKind::Ftp => spawn_engine(core, item, false),
        ItemKind::Hls | ItemKind::Dash => spawn_engine(core, item, true),
        ItemKind::Video => Ok(VideoJob::spawn(core, item)),
        ItemKind::Torrent => Ok(TorrentJob::spawn(core, item)),
    }
}

fn limiters_for(core: &Core, item: &DownloadItem) -> Vec<Arc<prism_engine::RateLimiter>> {
    item.queue_id
        .as_deref()
        .map(|q| vec![core.queue_limiter(q)])
        .unwrap_or_default()
}

fn build_request(item: &DownloadItem, settings: &Settings) -> Result<DownloadRequest> {
    let value = item
        .request
        .clone()
        .ok_or_else(|| anyhow!("missing request"))?;
    let mut req: DownloadRequest =
        serde_json::from_value(value).context("invalid stored request")?;
    req.destination.dir = item.save_dir.clone();
    // The file name is fixed once known (dialog, probe, or a previous run).
    if !item.file_name.is_empty()
        && (req.destination.file_name.is_some() || item.downloaded > 0 || item.final_url.is_some())
    {
        req.destination.file_name = Some(item.file_name.clone());
    }
    req.expect_file = crate::categories::is_binary_file_name(&item.file_name);
    req.connections = item.connections.max(1);
    req.speed_limit = item.speed_limit;
    req.proxy = Some(proxy_config(settings));
    req.http = http_preference(settings);
    req.max_retries = Some(settings.downloads.max_retries);
    if let Some(c) = item.checksum.as_deref().and_then(parse_checksum) {
        req.checksum = Some(c);
    }
    // Gated and private Hugging Face models: the token is added now, not stored with the item.
    crate::huggingface::authorize(&mut req, settings);
    if req.user_agent.is_none() && !settings.connection.user_agent.is_empty() {
        req.user_agent = Some(settings.connection.user_agent.clone());
    }
    Ok(req)
}

fn spawn_engine(core: &Core, item: &DownloadItem, media: bool) -> Result<Job> {
    let settings = core.settings();
    let req = build_request(item, &settings)?;
    std::fs::create_dir_all(&item.save_dir)
        .with_context(|| format!("create {}", item.save_dir.display()))?;
    let resume = item
        .resume
        .clone()
        .and_then(|v| serde_json::from_value::<ResumeState>(v).ok());
    let limiters = limiters_for(core, item);
    let handle = if media {
        let mut m = MediaRequest::new(req.url.clone(), req.destination.dir.clone());
        m.kind = if item.kind == ItemKind::Dash {
            MediaKind::Dash
        } else {
            MediaKind::Hls
        };
        m.headers = req.headers;
        m.cookies = req.cookies;
        m.referer = req.referer;
        m.user_agent = req.user_agent;
        m.proxy = req.proxy;
        m.destination = req.destination;
        m.connections = req.connections;
        m.speed_limit = req.speed_limit;
        m.max_retries = req.max_retries;
        // The quality picked from the page menu (`NewItem::variant`), stored next to the
        // request: `DownloadRequest` has no such field.
        if let Some(sel) = item
            .request
            .as_ref()
            .and_then(|r| r.get("variant"))
            .and_then(|v| serde_json::from_value::<VariantSelection>(v.clone()).ok())
        {
            m.variant = sel;
        }
        core.0.engine.start_media(m, resume, limiters)
    } else {
        core.0.engine.start(req, resume, limiters)
    };
    let job = Arc::new(EngineJob {
        handle: handle.clone(),
        detail: Mutex::new(None),
    });

    // Event listener: keep the item in sync with what the engine learns.
    let mut events = handle.events();
    let core2 = core.clone();
    let id = item.id.clone();
    let job2 = job.clone();
    tokio::spawn(async move {
        while let Ok(ev) = events.recv().await {
            match ev {
                DownloadEvent::Probed(p) => {
                    core2.with_item(&id, |i| {
                        i.final_url = Some(p.final_url.to_string());
                        i.size = p.size.or(i.size);
                        i.resumable = Some(p.resumable);
                    });
                }
                DownloadEvent::Retrying { reason, delay, .. } => {
                    *job2.detail.lock() = Some(format!(
                        "{reason} — retrying in {} s",
                        delay.as_secs().max(1)
                    ));
                }
                DownloadEvent::ServerLimitDetected { connections } => {
                    *job2.detail.lock() = Some(format!("Server allows {connections} connections"));
                }
                DownloadEvent::LinkExpired => {
                    *job2.detail.lock() = Some("Link expired — use Refresh address".into());
                }
                DownloadEvent::StateChanged {
                    state: DownloadState::Downloading,
                } => {
                    *job2.detail.lock() = None;
                }
                _ => {}
            }
        }
    });

    // Outcome reporter.
    let core3 = core.clone();
    let id = item.id.clone();
    tokio::spawn(async move {
        let result = handle.wait().await;
        // Learn the real file name (Content-Disposition) before reporting.
        if let Some(name) = handle.snapshot().file_name {
            core3.with_item(&id, |i| {
                if i.file_name != name {
                    i.file_name = name.clone();
                }
            });
        }
        let resume = || {
            handle
                .resume_state()
                .and_then(|s| serde_json::to_value(s).ok())
        };
        let result = match result {
            Ok(done) => JobResult::Completed {
                path: done.path,
                size: done.size,
            },
            Err(EngineError::Paused) => JobResult::Paused { resume: resume() },
            Err(EngineError::Cancelled) => JobResult::Cancelled,
            Err(e) => JobResult::Failed {
                error: e.user_message(),
                resume: resume(),
                resumable: matches!(e, EngineError::RangeNotSupported).then_some(false),
            },
        };
        let _ = core3.0.outcomes.send(JobOutcome { id, result });
    });
    Ok(Job::Engine(job))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StopKind {
    Pause,
    Cancel,
    CancelDelete,
}

// ----- video --------------------------------------------------------------------------------

#[derive(Default)]
struct VideoState {
    status: Status,
    detail: Option<String>,
    downloaded: u64,
    total: Option<u64>,
    speed: u64,
    eta: Option<u64>,
}

pub(crate) struct VideoJob {
    state: Mutex<VideoState>,
    /// Engine handles of the accelerated streams currently downloading.
    handles: Mutex<Vec<DownloadHandle>>,
    cancel: CancellationToken,
    stop: Mutex<Option<StopKind>>,
}

impl VideoJob {
    fn spawn(core: &Core, item: &DownloadItem) -> Job {
        let job = Arc::new(VideoJob {
            state: Mutex::new(VideoState {
                status: Status::Connecting,
                detail: Some("Resolving formats".into()),
                ..Default::default()
            }),
            handles: Mutex::new(Vec::new()),
            cancel: CancellationToken::new(),
            stop: Mutex::new(None),
        });
        let core = core.clone();
        let item = item.clone();
        let j = job.clone();
        tokio::spawn(async move {
            let result = j.run(&core, &item).await;
            let stop = *j.stop.lock();
            let resume = j.resume_json();
            let result = match (result, stop) {
                (_, Some(StopKind::Pause)) => JobResult::Paused { resume },
                (_, Some(StopKind::Cancel | StopKind::CancelDelete)) => {
                    if stop == Some(StopKind::CancelDelete) {
                        let _ = tokio::fs::remove_dir_all(video_temp_dir(&item)).await;
                    }
                    JobResult::Cancelled
                }
                (Ok((path, size)), None) => JobResult::Completed { path, size },
                (Err(e), None) => JobResult::Failed {
                    error: format!("{e:#}"),
                    resume,
                    resumable: None,
                },
            };
            let _ = core.0.outcomes.send(JobOutcome {
                id: item.id.clone(),
                result,
            });
        });
        Job::Video(job)
    }

    fn set(&self, f: impl FnOnce(&mut VideoState)) {
        f(&mut self.state.lock());
    }

    fn stop(&self, kind: StopKind) {
        *self.stop.lock() = Some(kind);
        for h in self.handles.lock().iter() {
            if kind == StopKind::Pause {
                h.request_pause()
            } else {
                h.cancel(kind == StopKind::CancelDelete)
            }
        }
        self.cancel.cancel();
    }

    fn progress(&self, item: &DownloadItem) -> Progress {
        let handles = self.handles.lock();
        let st = self.state.lock();
        // Accelerated: aggregate the engine handles; yt-dlp: use parsed progress.
        let (downloaded, total, speed, conns) = if handles.is_empty() {
            (st.downloaded, st.total, st.speed, 0)
        } else {
            let snaps: Vec<_> = handles.iter().map(|h| h.snapshot()).collect();
            let total = snaps.iter().map(|s| s.total).sum::<Option<u64>>();
            (
                snaps.iter().map(|s| s.downloaded).sum(),
                total,
                snaps.iter().map(|s| s.speed).sum(),
                snaps.iter().map(|s| s.active_connections).sum(),
            )
        };
        let eta = if handles.is_empty() {
            st.eta
        } else {
            total
                .filter(|_| speed > 0)
                .map(|t| t.saturating_sub(downloaded) / speed.max(1))
        };
        Progress {
            id: item.id.clone(),
            status: st.status,
            downloaded,
            size: total.or(item.size),
            speed,
            eta_secs: eta,
            connections: conns,
            detail: st.detail.clone(),
        }
    }

    fn segments(&self) -> Vec<SegmentInfo> {
        self.handles
            .lock()
            .iter()
            .flat_map(|h| {
                h.snapshot()
                    .segments
                    .iter()
                    .map(segment_info)
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn resume_json(&self) -> Option<serde_json::Value> {
        let handles = self.handles.lock();
        let states: HashMap<String, ResumeState> = handles
            .iter()
            .filter_map(|h| h.resume_state())
            .filter_map(|s| {
                s.final_path
                    .file_name()
                    .map(|n| (n.to_string_lossy().into_owned(), s.clone()))
            })
            .collect();
        (!states.is_empty()).then(|| serde_json::json!({ "streams": states }))
    }

    async fn checkpoint(&self) -> Option<serde_json::Value> {
        let handles: Vec<DownloadHandle> = self.handles.lock().clone();
        let mut states = serde_json::Map::new();
        for h in handles {
            if let Some(s) = h.checkpoint().await
                && let Some(name) = s.final_path.file_name()
            {
                states.insert(
                    name.to_string_lossy().into_owned(),
                    serde_json::to_value(&s).ok()?,
                );
            }
        }
        (!states.is_empty()).then(|| serde_json::json!({ "streams": states }))
    }

    async fn run(&self, core: &Core, item: &DownloadItem) -> Result<(PathBuf, u64)> {
        let settings = core.settings();
        let spec = item.request.clone().unwrap_or_default();
        let page_url = spec
            .get("pageUrl")
            .and_then(|v| v.as_str())
            .unwrap_or(&item.url)
            .to_string();
        let ctx: RequestContext = spec
            .get("context")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let video = &core.0.video;

        // Tools first (downloads yt-dlp/deno on first use).
        self.set(|s| s.detail = Some("Preparing yt-dlp".into()));
        let status = video.tool_status().await;
        if status.yt_dlp.path.is_none() || status.deno.path.is_none() {
            let c = core.clone();
            video
                .ensure_tools(move |p| {
                    c.emit(CoreEvent::ToolsProgress {
                        tool: format!("{:?}", p.tool).to_lowercase(),
                        downloaded: p.downloaded,
                        total: p.total,
                    })
                })
                .await
                .context("installing yt-dlp")?;
        }
        if self.cancel.is_cancelled() {
            return Err(anyhow!("stopped"));
        }

        self.set(|s| s.detail = Some("Resolving formats".into()));
        let probe = tokio::select! {
            r = video.probe(&page_url, &ctx) => r.map_err(|e| anyhow!("{e}"))?,
            _ = self.cancel.cancelled() => return Err(anyhow!("stopped")),
        };
        let mut info = match probe {
            Probe::Video(info) => info,
            Probe::Playlist(_) => {
                return Err(anyhow!(
                    "This address is a playlist; pick its videos in the video dialog"
                ));
            }
        };
        // The formats for the container asked for (the dialog's, else Settings › Video): the
        // choice's container is what the file is merged into.
        info.choices = video.choices(&info, wanted_container(spec.get("options"), &settings));
        let renamed = adopt_title(core, item, &info, &settings, &page_url);
        let item = &renamed;

        let opts = output_options(&spec, item, &settings);
        let stored_choice: Option<Choice> = spec
            .get("choice")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok());
        let choice = pick_choice(&info.choices, stored_choice.as_ref(), &settings)
            .ok_or_else(|| anyhow!("No downloadable format (DRM-protected or unsupported)"))?;

        let plan = if settings.video.accelerate {
            video.plan(&info, &choice, &opts, &ctx)
        } else {
            Plan::YtDlp(video.plan_ytdlp_forced(&info, &choice, &opts, &ctx))
        };
        match plan {
            Plan::Accelerated(plan) => match self.run_accelerated(core, item, plan).await {
                Ok(done) => Ok(done),
                Err(e) if self.stop.lock().is_none() => {
                    tracing::warn!(
                        "accelerated video download failed ({e:#}); falling back to yt-dlp"
                    );
                    self.handles.lock().clear();
                    let job = video.plan_ytdlp_forced(&info, &choice, &opts, &ctx);
                    self.run_ytdlp(core, job).await
                }
                Err(e) => Err(e),
            },
            Plan::YtDlp(job) => self.run_ytdlp(core, job).await,
        }
    }

    async fn run_accelerated(
        &self,
        core: &Core,
        item: &DownloadItem,
        mut plan: prism_video::AcceleratedPlan,
    ) -> Result<(PathBuf, u64)> {
        let settings = core.settings();
        let temp = video_temp_dir(item);
        tokio::fs::create_dir_all(&temp).await?;
        hide_dir(&temp);
        let stored: HashMap<String, ResumeState> = item
            .resume
            .as_ref()
            .and_then(|v| v.get("streams").cloned())
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        // googlevideo throttles many parallel ranges per URL; stay polite there.
        let youtube = plan
            .streams
            .iter()
            .any(|s| s.url.contains("googlevideo.com"));
        let connections = if youtube {
            item.connections.clamp(1, 4)
        } else {
            item.connections.max(1)
        };

        let mut handles = Vec::new();
        let mut optional = Vec::new();
        for s in &plan.streams {
            let Ok(url) = Url::parse(&s.url) else {
                continue;
            };
            let mut req = DownloadRequest::new(url.clone(), temp.clone());
            req.headers = s.headers.clone();
            req.destination.file_name = Some(s.file_name.clone());
            req.destination.conflict = ConflictPolicy::Overwrite;
            req.connections = if matches!(s.role, StreamRole::Subtitle | StreamRole::Thumbnail) {
                1
            } else {
                connections
            };
            req.max_request_size = s.chunk_size;
            req.speed_limit = item.speed_limit;
            req.proxy = Some(proxy_config(&settings));
            let resume = stored.get(&s.file_name).cloned().map(|st| st.with_url(url));
            let h = core.0.engine.start(req, resume, limiters_for(core, item));
            if s.optional {
                optional.push(h.clone());
            }
            handles.push((h, s.optional));
        }
        *self.handles.lock() = handles.iter().map(|(h, _)| h.clone()).collect();
        self.set(|s| {
            s.status = Status::Downloading;
            s.detail = None;
        });
        for (h, is_optional) in &handles {
            match h.wait().await {
                Ok(_) => {}
                Err(e) if *is_optional => {
                    tracing::info!("optional stream skipped: {}", e.user_message())
                }
                Err(EngineError::Paused) | Err(EngineError::Cancelled) => {
                    return Err(anyhow!("stopped"));
                }
                Err(e) => {
                    for (other, _) in &handles {
                        other.cancel(false);
                    }
                    return Err(anyhow!("{}", e.user_message()));
                }
            }
        }
        self.handles.lock().clear();

        self.set(|s| {
            s.status = Status::Processing;
            s.detail = Some("Muxing with ffmpeg".into());
            s.speed = 0;
        });
        plan.post.set_input_dir(&temp);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<YtProgress>();
        let post = core
            .0
            .video
            .postprocess(&plan.post, Some(tx), self.cancel.clone());
        tokio::pin!(post);
        let out = loop {
            tokio::select! {
                r = &mut post => break r.map_err(|e| anyhow!("{e}"))?,
                Some(p) = rx.recv() => {
                    let pct = p.percent.map(|v| format!(" {v:.0}%")).unwrap_or_default();
                    self.set(|s| s.detail = Some(format!("{}{pct}", stage_label(p.stage))));
                }
            }
        };
        let _ = tokio::fs::remove_dir_all(&temp).await;
        let size = tokio::fs::metadata(&out)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        Ok((out, size))
    }

    async fn run_ytdlp(&self, core: &Core, job: prism_video::YtDlpJob) -> Result<(PathBuf, u64)> {
        self.set(|s| {
            s.status = Status::Downloading;
            s.detail = Some(format!("yt-dlp: {}", job.reason));
        });
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<YtProgress>();
        let video = core.0.video.clone();
        let cancel = self.cancel.clone();
        let run = tokio::spawn(async move { video.run_ytdlp(&job, tx, cancel).await });
        while let Some(p) = rx.recv().await {
            self.apply_yt_progress(&p);
        }
        let out = run
            .await
            .map_err(|e| anyhow!("{e}"))?
            .map_err(|e| anyhow!("{e}"))?;
        let size = tokio::fs::metadata(&out)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        Ok((out, size))
    }

    fn apply_yt_progress(&self, p: &YtProgress) {
        self.set(|s| match p.stage {
            Stage::Downloading => {
                s.status = Status::Downloading;
                s.detail = p
                    .fragment_count
                    .map(|n| format!("Fragment {} of {n}", p.fragment_index.unwrap_or(0)));
                if let Some(d) = p.downloaded {
                    s.downloaded = d;
                }
                if p.total.is_some() {
                    s.total = p.total;
                }
                s.speed = p.speed.unwrap_or(0.0).max(0.0) as u64;
                s.eta = p.eta;
            }
            Stage::Starting => s.detail = Some("Starting".into()),
            Stage::Finished => {}
            other => {
                s.status = Status::Processing;
                s.speed = 0;
                s.detail = Some(stage_label(other).into());
            }
        });
    }
}

fn stage_label(stage: Stage) -> &'static str {
    match stage {
        Stage::Merging => "Merging video and audio",
        Stage::ExtractingAudio => "Extracting audio",
        Stage::Converting => "Converting",
        Stage::EmbeddingSubtitles => "Embedding subtitles",
        Stage::EmbeddingThumbnail => "Embedding thumbnail",
        Stage::EmbeddingMetadata => "Writing metadata",
        Stage::SponsorBlock => "Applying SponsorBlock",
        Stage::Moving => "Moving file",
        _ => "Post-processing",
    }
}

/// Mark a temp folder hidden on Windows (dot-folders are hidden elsewhere already).
fn hide_dir(_path: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("attrib")
            .arg("+h")
            .arg(_path)
            .creation_flags(0x0800_0000)
            .status();
    }
}

fn output_options(
    spec: &serde_json::Value,
    item: &DownloadItem,
    settings: &Settings,
) -> OutputOptions {
    let v = &settings.video;
    let mut o = OutputOptions {
        container: match v.container.as_str() {
            "mkv" => Container::Mkv,
            "webm" => Container::Webm,
            _ => Container::Mp4,
        },
        audio_format: match v.audio_format.as_str() {
            "mp3" => AudioFormat::Mp3,
            "opus" => AudioFormat::Opus,
            "flac" => AudioFormat::Flac,
            "wav" => AudioFormat::Wav,
            _ => AudioFormat::M4a,
        },
        embed_subtitles: v.embed_subtitles,
        sub_langs: v
            .subtitle_langs
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        embed_thumbnail: v.embed_thumbnail,
        embed_metadata: v.embed_metadata,
        embed_chapters: v.embed_chapters,
        output_dir: item.save_dir.clone(),
        ..OutputOptions::default()
    };
    o.sponsorblock.mode = match v.sponsorblock {
        settings::SponsorBlock::Off => SponsorBlockMode::Off,
        settings::SponsorBlock::Mark => SponsorBlockMode::Mark,
        settings::SponsorBlock::Remove => SponsorBlockMode::Remove,
    };
    // Per-download overrides chosen in the video dialog.
    if let Some(overrides) = spec.get("options").filter(|o| o.is_object()) {
        let mut merged = serde_json::to_value(&o).unwrap_or_default();
        if let (Some(m), Some(over)) = (merged.as_object_mut(), overrides.as_object()) {
            for (k, val) in over {
                m.insert(k.clone(), val.clone());
            }
        }
        if let Ok(parsed) = serde_json::from_value::<OutputOptions>(merged) {
            o = parsed;
        }
        o.output_dir = item.save_dir.clone();
    }
    // Name the output after the list entry so both stay in sync.
    let stem = Path::new(&item.file_name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !stem.is_empty() {
        o.file_template = format!("{}.%(ext)s", stem.replace('%', "%%"));
    }
    o
}

/// The stored choice if its formats still exist, else the closest match by height/kind.
fn pick_choice(choices: &[Choice], stored: Option<&Choice>, settings: &Settings) -> Option<Choice> {
    if let Some(s) = stored {
        if let Some(c) = choices.iter().find(|c| c.id == s.id) {
            return Some(c.clone());
        }
        if s.kind == ChoiceKind::Audio {
            return choices
                .iter()
                .find(|c| c.kind == ChoiceKind::Audio && c.audio_format == s.audio_format)
                .or_else(|| choices.iter().find(|c| c.kind == ChoiceKind::Audio))
                .cloned()
                .or_else(|| Some(s.clone()));
        }
        let h = s.height.unwrap_or(u32::MAX);
        return choices
            .iter()
            .filter(|c| c.kind == ChoiceKind::Video && c.height.unwrap_or(0) <= h)
            .find(|c| c.hdr == s.hdr)
            .or_else(|| choices.iter().find(|c| c.kind == ChoiceKind::Video))
            .cloned()
            .or_else(|| Some(s.clone()));
    }
    let max = if settings.video.preferred_height == 0 {
        u32::MAX
    } else {
        settings.video.preferred_height
    };
    choices
        .iter()
        .filter(|c| c.kind == ChoiceKind::Video && !c.hdr && c.height.unwrap_or(0) <= max)
        .chain(choices.iter())
        .next()
        .cloned()
}

// ----- torrent ------------------------------------------------------------------------------

pub(crate) struct TorrentJob {
    hash: Mutex<Option<String>>,
    stop: CancellationToken,
    last: Mutex<Option<prism_torrent::TorrentProgress>>,
}

impl TorrentJob {
    fn spawn(core: &Core, item: &DownloadItem) -> Job {
        let job = Arc::new(TorrentJob {
            hash: Mutex::new(torrent_hash(item)),
            stop: CancellationToken::new(),
            last: Mutex::new(None),
        });
        let j = job.clone();
        let core = core.clone();
        let item = item.clone();
        tokio::spawn(async move {
            let result = j.run(&core, &item).await;
            let result = match result {
                Ok(Some((path, size))) => JobResult::Completed { path, size },
                Ok(None) => JobResult::Paused { resume: None },
                Err(e) => JobResult::Failed {
                    error: format!("{e:#}"),
                    resume: None,
                    resumable: Some(true),
                },
            };
            let _ = core.0.outcomes.send(JobOutcome {
                id: item.id.clone(),
                result,
            });
        });
        Job::Torrent(job)
    }

    fn stop(&self, _kind: StopKind) {
        self.stop.cancel();
    }

    fn progress(&self, item: &DownloadItem) -> Progress {
        let last = self.last.lock().clone();
        match last {
            Some(p) => Progress {
                id: item.id.clone(),
                status: match p.state {
                    prism_torrent::TorrentState::Initializing => Status::Connecting,
                    _ => Status::Downloading,
                },
                downloaded: p.downloaded,
                size: Some(p.total).filter(|t| *t > 0).or(item.size),
                speed: p.download_speed,
                eta_secs: p.eta_secs,
                connections: p.peers,
                detail: Some(match p.state {
                    prism_torrent::TorrentState::Initializing => {
                        "Checking existing data".to_string()
                    }
                    _ => format!("{} peers", p.peers),
                }),
            },
            None => Progress {
                id: item.id.clone(),
                status: Status::Connecting,
                downloaded: item.downloaded,
                size: item.size,
                detail: Some("Fetching metadata".into()),
                ..Default::default()
            },
        }
    }

    /// Returns `Ok(Some(..))` when finished, `Ok(None)` when paused by the user.
    async fn run(&self, core: &Core, item: &DownloadItem) -> Result<Option<(PathBuf, u64)>> {
        let engine = core.torrent_engine().await?;
        let spec = item.request.clone().unwrap_or_default();
        let existing = self.hash.lock().clone();
        let hash = match existing.filter(|h| engine.is_managed(h)) {
            Some(h) => {
                engine.resume(&h).await?;
                h
            }
            None => {
                let source: prism_torrent::TorrentSource =
                    match spec.get("source").filter(|s| !s.is_null()) {
                        Some(s) => serde_json::from_value(s.clone())
                            .context("unreadable torrent source")?,
                        // A link to a .torrent file added as a download had no torrent of its
                        // own (#20): the torrent engine reads it from the link.
                        None if super::capture::is_torrent_link(&item.url) => {
                            prism_torrent::TorrentSource::Url(item.url.trim().to_string())
                        }
                        None => anyhow::bail!("missing torrent source"),
                    };
                let only: Option<Vec<usize>> = spec
                    .get("onlyFiles")
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok());
                tokio::fs::create_dir_all(&item.save_dir).await?;
                let h = tokio::select! {
                    r = engine.add(&source, &item.save_dir, only, false) => r?,
                    _ = self.stop.cancelled() => return Ok(None),
                };
                core.with_item(&item.id, |i| {
                    if let Some(obj) = i.request.as_mut().and_then(|r| r.as_object_mut()) {
                        obj.insert("infoHash".into(), serde_json::json!(h));
                    }
                });
                h
            }
        };
        *self.hash.lock() = Some(hash.clone());
        let started = Instant::now();
        loop {
            if let Some(p) = engine.progress(&hash) {
                if let Some(name) = p.name.clone()
                    && (item.file_name.starts_with("magnet") || item.file_name.is_empty())
                {
                    core.with_item(&item.id, |i| i.file_name = name.clone());
                }
                if p.finished {
                    // Personal use: stop seeding once complete.
                    let _ = engine.pause(&hash).await;
                    let path = match engine.meta(&hash) {
                        Some(meta) => prism_torrent::content_path(&item.save_dir, &meta),
                        None => item
                            .save_dir
                            .join(p.name.clone().unwrap_or_else(|| item.file_name.clone())),
                    };
                    return Ok(Some((path, p.total)));
                }
                if p.state == prism_torrent::TorrentState::Error {
                    return Err(anyhow!(p.error.unwrap_or_else(|| "torrent error".into())));
                }
                *self.last.lock() = Some(p);
            } else if started.elapsed() > Duration::from_secs(5) && !engine.is_managed(&hash) {
                return Err(anyhow!("the torrent was removed"));
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                _ = self.stop.cancelled() => {
                    let _ = engine.pause(&hash).await;
                    return Ok(None);
                }
            }
        }
    }
}

// ----- video probing for the UI & tools -----------------------------------------------------

/// How long a page's probe answers new requests (the in-page menu opening again, the video
/// window); the download itself still probes fresh.
pub(crate) const PROBE_REUSE: Duration = Duration::from_secs(600);

fn recent_probe(core: &Core, page_url: &str) -> Option<serde_json::Value> {
    let st = core.0.state.lock();
    st.video_probes
        .get(page_url)
        .filter(|(at, _)| at.elapsed() < PROBE_REUSE)
        .map(|(_, p)| p.clone())
}

/// Probe a video page for the quality picker: a recent result is reused and a probe of the
/// same page that is already running is waited for.
pub(crate) async fn probe_video(
    core: &Core,
    page_url: &str,
    ctx: RequestContext,
) -> Result<serde_json::Value> {
    if let Some(p) = recent_probe(core, page_url) {
        return Ok(p);
    }
    let lock = core
        .0
        .probe_locks
        .lock()
        .entry(page_url.to_string())
        .or_default()
        .clone();
    let res = {
        let _same_page = lock.lock().await;
        match recent_probe(core, page_url) {
            Some(p) => Ok(p),
            None => {
                let res = run_probe_video(core, page_url, ctx).await;
                if let Ok(p) = &res {
                    core.0
                        .state
                        .lock()
                        .video_probes
                        .insert(page_url.to_string(), (std::time::Instant::now(), p.clone()));
                }
                res
            }
        }
    };
    let mut locks = core.0.probe_locks.lock();
    if locks
        .get(page_url)
        .is_some_and(|l| Arc::strong_count(l) <= 2)
    {
        locks.remove(page_url);
    }
    res
}

/// Resolve a page with yt-dlp for the quality picker (raw `prism_video::Probe` JSON). The
/// extension's cookies and User-Agent for the page come along in `ctx`.
async fn run_probe_video(
    core: &Core,
    page_url: &str,
    ctx: RequestContext,
) -> Result<serde_json::Value> {
    let video = &core.0.video;
    let status = video.tool_status().await;
    if status.yt_dlp.path.is_none() || status.deno.path.is_none() {
        let c = core.clone();
        video
            .ensure_tools(move |p| {
                c.emit(CoreEvent::ToolsProgress {
                    tool: format!("{:?}", p.tool).to_lowercase(),
                    downloaded: p.downloaded,
                    total: p.total,
                })
            })
            .await
            .map_err(|e| anyhow!("{e}"))?;
    }
    let probe = match video
        .probe(page_url, &ctx)
        .await
        .map_err(|e| anyhow!("{e}"))?
    {
        // yt-dlp's result names its choices for MP4: list them for the container in Settings.
        Probe::Video(mut info) => {
            info.choices = video.choices(&info, wanted_container(None, &core.settings()));
            Probe::Video(info)
        }
        playlist => playlist,
    };
    Ok(serde_json::to_value(probe)?)
}

/// The container the user asked for: the video dialog's choice, else Settings › Video.
fn wanted_container(options: Option<&serde_json::Value>, settings: &Settings) -> Container {
    let name = options
        .and_then(|o| o.get("container"))
        .and_then(|v| v.as_str())
        .unwrap_or(&settings.video.container);
    match name {
        "mkv" => Container::Mkv,
        "webm" => Container::Webm,
        _ => Container::Mp4,
    }
}

/// A video added without its title (a playlist that didn't list one) is still named "Video …":
/// name it after the video now that its page is read, before anything is written. A file
/// renamed in the meantime keeps its name; a playlist number in front stays.
fn adopt_title(
    core: &Core,
    item: &DownloadItem,
    info: &prism_video::VideoInfo,
    settings: &Settings,
    page_url: &str,
) -> DownloadItem {
    let untitled = item.media.as_ref().is_some_and(|m| m.title.is_none());
    if !untitled || info.title.trim().is_empty() || item.downloaded > 0 {
        return item.clone();
    }
    let template = &settings.video.file_template;
    let id = super::capture::video_id(page_url);
    let file = Path::new(&item.file_name);
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let placeholder = super::sanitize_name(&super::capture::render_name_template(
        template,
        "Video",
        id.as_deref(),
    ));
    let Some(prefix) = stem.strip_suffix(placeholder.as_str()) else {
        return item.clone();
    };
    let numbered = prefix
        .strip_suffix(" - ")
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    if !prefix.is_empty() && !numbered {
        return item.clone();
    }
    let titled = super::sanitize_name(&super::capture::render_name_template(
        template,
        &info.title,
        id.as_deref(),
    ));
    let name = match file.extension() {
        Some(ext) => format!("{prefix}{titled}.{}", ext.to_string_lossy()),
        None => format!("{prefix}{titled}"),
    };
    core.with_item(&item.id, |i| {
        i.file_name = name.clone();
        if let Some(m) = i.media.as_mut() {
            m.title = Some(info.title.clone());
            m.thumbnail = m.thumbnail.take().or_else(|| info.thumbnail.clone());
            m.duration = m.duration.or(info.duration);
        }
    });
    core.item(&item.id).unwrap_or_else(|| item.clone())
}

impl Core {
    pub async fn tool_status(&self) -> serde_json::Value {
        serde_json::to_value(self.0.video.tool_status().await).unwrap_or_default()
    }

    pub async fn ensure_tools(&self) -> Result<serde_json::Value> {
        let c = self.clone();
        let status = self
            .0
            .video
            .ensure_tools(move |p| {
                c.emit(CoreEvent::ToolsProgress {
                    tool: format!("{:?}", p.tool).to_lowercase(),
                    downloaded: p.downloaded,
                    total: p.total,
                })
            })
            .await
            .map_err(|e| anyhow!("{e}"))?;
        self.sync_ffmpeg_path().await;
        Ok(serde_json::to_value(status)?)
    }

    pub async fn update_ytdlp(&self) -> Result<serde_json::Value> {
        let outcome = self
            .0
            .video
            .update_ytdlp()
            .await
            .map_err(|e| anyhow!("{e}"))?;
        Ok(serde_json::to_value(outcome)?)
    }

    /// Point the engine's HLS/DASH remuxer at the ffmpeg the video service resolved.
    pub async fn sync_ffmpeg_path(&self) {
        let status = self.0.video.tool_status().await;
        *self.0.ffmpeg_path.lock() = status.ffmpeg.path.clone();
        self.apply_engine_config();
    }

    /// Engine settings from the user's settings plus the resolved ffmpeg (kept across changes).
    pub(crate) fn apply_engine_config(&self) {
        let mut cfg = engine_config(&self.settings());
        if cfg.ffmpeg_path.is_none() {
            cfg.ffmpeg_path = self.0.ffmpeg_path.lock().clone();
        }
        self.0.engine.update_config(cfg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_parsing() {
        assert!(matches!(
            parse_checksum("sha256:ABCDEF01").map(|c| c.algo),
            Some(prism_engine::ChecksumAlgo::Sha256)
        ));
        assert!(matches!(
            parse_checksum(&"a".repeat(32)).map(|c| c.algo),
            Some(prism_engine::ChecksumAlgo::Md5)
        ));
        assert!(parse_checksum("md5:not-hex").is_none());
    }

    #[test]
    fn request_json_roundtrips_into_engine_request() {
        let n = NewItem {
            url: "https://example.com/a.zip".into(),
            cookies: Some("a=b".into()),
            headers: vec![
                ("Range".into(), "bytes=0-".into()),
                ("X-Test".into(), "1".into()),
            ],
            username: Some("u".into()),
            password: Some("p".into()),
            checksum: Some("sha1:".to_string() + &"0".repeat(40)),
            ..Default::default()
        };
        let v = request_json(&n, &Settings::default());
        let req: DownloadRequest = serde_json::from_value(v).unwrap();
        assert_eq!(req.headers, vec![("X-Test".to_string(), "1".to_string())]);
        assert_eq!(req.cookies.as_deref(), Some("a=b"));
        assert!(req.auth.is_some());
        assert!(req.checksum.is_some());
    }

    #[test]
    fn proxy_mapping() {
        let mut s = Settings::default();
        s.proxy.mode = ProxyMode::Socks5;
        s.proxy.address = "127.0.0.1:1080".into();
        assert!(
            matches!(proxy_config(&s), ProxyConfig::Socks5 { url, .. } if url == "socks5h://127.0.0.1:1080")
        );
        s.proxy.mode = ProxyMode::None;
        assert_eq!(proxy_config(&s), ProxyConfig::None);
    }

    #[test]
    fn test_parse_dns_overrides() {
        let raw = vec![
            "example.com=1.1.1.1".to_string(),
            "custom.org:2.2.2.2:8080".to_string(),
            "arrow.net->3.3.3.3".to_string(),
            "ipv6.org=2001:db8::1".to_string(),
            "bracketed.org=[2001:db8::2]".to_string(),
            "bracketed-port.org=[2001:db8::3]:8443".to_string(),
            "".to_string(),
            "   ".to_string(),
        ];
        let parsed = parse_dns_overrides(&raw);
        assert_eq!(parsed.len(), 6);
        assert_eq!(parsed[0], ("example.com".into(), "1.1.1.1:0".parse().unwrap()));
        assert_eq!(parsed[1], ("custom.org".into(), "2.2.2.2:8080".parse().unwrap()));
        assert_eq!(parsed[2], ("arrow.net".into(), "3.3.3.3:0".parse().unwrap()));
        assert_eq!(parsed[3], ("ipv6.org".into(), "[2001:db8::1]:0".parse().unwrap()));
        assert_eq!(parsed[4], ("bracketed.org".into(), "[2001:db8::2]:0".parse().unwrap()));
        assert_eq!(parsed[5], ("bracketed-port.org".into(), "[2001:db8::3]:8443".parse().unwrap()));
    }
}
