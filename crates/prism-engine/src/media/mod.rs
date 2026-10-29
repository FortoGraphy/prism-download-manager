//! Adaptive streaming downloads (HLS and DASH).
//!
//! A manifest is resolved into a [`MediaPlan`]: one or two tracks (video/muxed + optional
//! separate audio), each a list of media segments (with optional byte ranges, AES-128 keys
//! and initialization sections). Segments are fetched by N parallel workers into a work
//! directory (`<final>.prismpart.d/`), each written, fsynced and renamed on completion — so
//! resuming simply skips completed segments. At the end every track is concatenated and
//! (when ffmpeg is available and remuxing is enabled) muxed into the final `.mp4`.

pub(crate) mod dash;
pub(crate) mod hls;
mod remux;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reqwest::header::{self, HeaderMap};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::backoff::Backoff;
use crate::config::EngineConfig;
use crate::engine::EngineInner;
use crate::error::EngineError;
use crate::filename::{self, part_path_for};
use crate::http::client::{ClientSpec, base_tls_config, build_client};
use crate::http::fetcher::error_chain;
use crate::http::{base_headers, header_string, parse_content_range};
use crate::job::{Command, JobShared, ProgressSource, StopReason};
use crate::limiter::{RateLimiter, acquire_all};
use crate::progress::{
    Completed, DownloadEvent, DownloadState, ProbeInfo, SegmentStatus, SegmentView,
};
use crate::request::{
    ConflictPolicy, HttpPreference, MediaKind, MediaRequest, VariantProbeRequest,
};
use crate::resume::{MediaResumeState, RESUME_STATE_VERSION, ResumeKind, ResumeState};
use crate::storage;

/// Largest manifest accepted.
const MAX_MANIFEST: usize = 16 << 20;

/// AES-128 key reference of a segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeyRef {
    pub uri: Url,
    pub iv: [u8; 16],
}

/// One media segment (or initialization section).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MediaSegment {
    pub url: Url,
    /// Byte range `[start, end)` within `url`.
    pub range: Option<(u64, u64)>,
    pub key: Option<KeyRef>,
}

/// What a track carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrackKind {
    /// Video, possibly with muxed audio.
    Main,
    /// Separate audio rendition.
    Audio,
}

/// One output stream before muxing.
#[derive(Debug, Clone)]
pub(crate) struct Track {
    /// Stable identifier (media playlist URL or representation id) validated on resume.
    pub id: String,
    /// File extension of the concatenated raw track.
    pub ext: &'static str,
    pub segments: Vec<MediaSegment>,
}

/// A resolved manifest.
#[derive(Debug, Clone)]
pub(crate) struct MediaPlan {
    pub kind: MediaKind,
    pub tracks: Vec<Track>,
    /// Suggested base name (without extension).
    pub name: String,
}

impl MediaPlan {
    fn total_segments(&self) -> usize {
        self.tracks.iter().map(|t| t.segments.len()).sum()
    }

    /// `(track, segment)` of a global (track-major) index.
    fn locate(&self, mut global: usize) -> (usize, usize) {
        for (t, track) in self.tracks.iter().enumerate() {
            if global < track.segments.len() {
                return (t, global);
            }
            global -= track.segments.len();
        }
        unreachable!("index out of range")
    }
}

/// A manifest-declared variant/representation, shared shape between the HLS and DASH
/// parsers — not yet chosen for download.
#[derive(Debug, Clone)]
pub(crate) struct Variant {
    pub url: Url,
    pub bandwidth: Option<u64>,
    pub width: Option<u64>,
    pub height: Option<u64>,
}

/// A manifest-declared variant, as returned by [`list_variants`]. `index` is exactly what
/// [`crate::request::VariantSelection::Index`] expects in a later [`MediaRequest`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariantInfo {
    /// Position in the manifest's variant list (matches `VariantSelection::Index`).
    pub index: usize,
    /// Display/debug only — not required to start a download.
    pub url: Url,
    /// Bits per second, when the manifest declares one.
    pub bandwidth: Option<u64>,
    /// Pixel width, when the manifest declares one.
    pub width: Option<u64>,
    /// Pixel height, when the manifest declares one.
    pub height: Option<u64>,
}

/// Lists every variant/representation a manifest declares, without downloading anything —
/// a cheap probe (one small text fetch) for showing a full quality picker before the user
/// commits to a download.
pub async fn list_variants(
    cfg: &EngineConfig,
    probe: &VariantProbeRequest,
) -> Result<Vec<VariantInfo>, EngineError> {
    let spec = ClientSpec {
        proxy: probe
            .proxy
            .clone()
            .unwrap_or_else(|| cfg.default_proxy.clone()),
        http: HttpPreference::Http1Only,
        connect_timeout: cfg.connect_timeout,
        read_timeout: cfg.read_timeout,
        user_agent: probe
            .user_agent
            .clone()
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| cfg.user_agent.clone()),
        tls: base_tls_config().map_err(|e| EngineError::Network {
            message: format!("TLS setup failed: {e}"),
        })?,
        pool_max_idle: 1,
        dns_overrides: cfg.default_dns_overrides.clone(),
    };
    let client = build_client(&spec)?;
    let headers = base_headers(
        &probe.headers,
        probe.cookies.as_deref(),
        probe.referer.as_deref(),
    )?;
    let (text, manifest_url) = fetch_text(&client, &headers, &probe.url).await?;
    let kind = detect_kind(probe.kind, &manifest_url, &text)?;
    let variants = match kind {
        MediaKind::Dash => dash::list_variants(&text, &manifest_url)?,
        _ => hls::list_variants(&text, &manifest_url)?,
    };
    Ok(variants
        .into_iter()
        .enumerate()
        .map(|(index, v)| VariantInfo {
            index,
            url: v.url,
            bandwidth: v.bandwidth,
            width: v.width,
            height: v.height,
        })
        .collect())
}

/// Fetches a small text resource (manifest).
pub(crate) async fn fetch_text(
    client: &reqwest::Client,
    headers: &HeaderMap,
    url: &Url,
) -> Result<(String, Url), EngineError> {
    let resp = client
        .get(url.clone())
        .headers(headers.clone())
        .send()
        .await
        .map_err(|e| EngineError::network(error_chain(&e)))?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(match status {
            401 | 407 => EngineError::AuthRequired,
            s => EngineError::Http { status: s },
        });
    }
    if resp
        .content_length()
        .is_some_and(|l| l as usize > MAX_MANIFEST)
    {
        return Err(EngineError::Unsupported {
            message: "manifest too large".into(),
        });
    }
    let final_url = resp.url().clone();
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| EngineError::network(error_chain(&e)))?;
    Ok((String::from_utf8_lossy(&bytes).into_owned(), final_url))
}

/// Progress of a media job (shared with handles for snapshots).
pub(crate) struct MediaProgress {
    inner: Mutex<MediaInner>,
    downloaded: AtomicU64,
    tracks: Vec<String>,
    counts: Vec<u32>,
    work_dir: PathBuf,
    kind: MediaKind,
}

struct MediaInner {
    done: Vec<bool>,
    active: HashSet<usize>,
    done_bytes: u64,
    done_count: usize,
}

impl MediaProgress {
    fn new(plan: &MediaPlan, work_dir: PathBuf, done: Vec<bool>, done_bytes: u64) -> Self {
        let done_count = done.iter().filter(|d| **d).count();
        Self {
            downloaded: AtomicU64::new(done_bytes),
            inner: Mutex::new(MediaInner {
                done,
                active: HashSet::new(),
                done_bytes,
                done_count,
            }),
            tracks: plan.tracks.iter().map(|t| t.id.clone()).collect(),
            counts: plan
                .tracks
                .iter()
                .map(|t| t.segments.len() as u32)
                .collect(),
            work_dir,
            kind: plan.kind,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MediaInner> {
        self.inner.lock().expect("media progress lock")
    }

    pub(crate) fn downloaded(&self) -> u64 {
        self.downloaded.load(Ordering::Relaxed)
    }

    /// Estimated total size once a few segments are complete.
    pub(crate) fn total_estimate(&self) -> Option<u64> {
        let g = self.lock();
        if g.done_count == g.done.len() {
            return Some(g.done_bytes);
        }
        (g.done_count >= 3).then(|| g.done_bytes / g.done_count as u64 * g.done.len() as u64)
    }

    pub(crate) fn views(&self) -> Vec<SegmentView> {
        let g = self.lock();
        g.done
            .iter()
            .enumerate()
            .map(|(i, d)| SegmentView {
                start: i as u64,
                pos: if *d { i as u64 + 1 } else { i as u64 },
                end: i as u64 + 1,
                speed: 0,
                status: if *d {
                    SegmentStatus::Done
                } else if g.active.contains(&i) {
                    SegmentStatus::Active
                } else {
                    SegmentStatus::Idle
                },
            })
            .collect()
    }

    pub(crate) fn resume_state(&self, job: &JobShared) -> Option<ResumeState> {
        let completed: Vec<u32> = self
            .lock()
            .done
            .iter()
            .enumerate()
            .filter(|(_, d)| **d)
            .map(|(i, _)| i as u32)
            .collect();
        let m = job.meta();
        Some(ResumeState {
            version: RESUME_STATE_VERSION,
            kind: ResumeKind::Media,
            url: m.url.clone(),
            final_url: m.final_url.clone(),
            part_path: m.part_path.clone()?,
            final_path: m.final_path.clone()?,
            total_size: None,
            etag: None,
            last_modified: None,
            resumable: true,
            segments: Vec::new(),
            media: Some(MediaResumeState {
                kind: match self.kind {
                    MediaKind::Dash => "dash".into(),
                    _ => "hls".into(),
                },
                tracks: self.tracks.clone(),
                segment_counts: self.counts.clone(),
                completed,
                work_dir: self.work_dir.clone(),
            }),
        })
    }

    fn start(&self, i: usize) {
        self.lock().active.insert(i);
    }

    fn finish(&self, i: usize, bytes: Option<u64>) {
        let mut g = self.lock();
        g.active.remove(&i);
        if let Some(b) = bytes
            && !g.done[i]
        {
            g.done[i] = true;
            g.done_count += 1;
            g.done_bytes += b;
        }
    }

    fn all_done(&self) -> bool {
        let g = self.lock();
        g.done_count == g.done.len()
    }
}

fn seg_file(work_dir: &Path, global: usize) -> PathBuf {
    work_dir.join(format!("seg-{global:06}.bin"))
}

/// Shared state of the segment workers.
struct Fetch {
    client: reqwest::Client,
    headers: HeaderMap,
    plan: MediaPlan,
    progress: Arc<MediaProgress>,
    queue: Mutex<VecDeque<usize>>,
    keys: tokio::sync::Mutex<HashMap<Url, [u8; 16]>>,
    limiters: Vec<Arc<RateLimiter>>,
    work_dir: PathBuf,
    read_timeout: Duration,
    max_retries: u32,
    job: Arc<JobShared>,
}

impl Fetch {
    async fn key(&self, uri: &Url) -> Result<[u8; 16], EngineError> {
        let mut keys = self.keys.lock().await;
        if let Some(k) = keys.get(uri) {
            return Ok(*k);
        }
        let resp = self
            .client
            .get(uri.clone())
            .headers(self.headers.clone())
            .send()
            .await
            .map_err(|e| EngineError::network(error_chain(&e)))?;
        if !resp.status().is_success() {
            return Err(EngineError::Http {
                status: resp.status().as_u16(),
            });
        }
        let body = resp
            .bytes()
            .await
            .map_err(|e| EngineError::network(error_chain(&e)))?;
        let key: [u8; 16] = body.as_ref().try_into().map_err(|_| EngineError::Hls {
            message: format!("AES-128 key has {} bytes, expected 16", body.len()),
        })?;
        keys.insert(uri.clone(), key);
        Ok(key)
    }

    /// Downloads one segment into memory (rate limited), decrypting if needed.
    async fn download(&self, seg: &MediaSegment) -> Result<Vec<u8>, EngineError> {
        let mut rb = self
            .client
            .get(seg.url.clone())
            .headers(self.headers.clone());
        if let Some((a, b)) = seg.range {
            rb = rb.header(header::RANGE, format!("bytes={a}-{}", b - 1));
        }
        let mut resp = rb.send().await.map_err(|e| {
            if e.is_timeout() {
                EngineError::Timeout
            } else {
                EngineError::network(error_chain(&e))
            }
        })?;
        let status = resp.status().as_u16();
        let skip = match (status, seg.range) {
            (206, Some((a, _))) => {
                let start = header_string(resp.headers(), header::CONTENT_RANGE)
                    .and_then(|v| parse_content_range(&v))
                    .map(|(s, _, _)| s);
                if start != Some(a) {
                    return Err(EngineError::network(
                        "unexpected Content-Range for a media segment",
                    ));
                }
                0
            }
            // Server ignored the range: cut the slice out of the full body.
            (200, Some((a, _))) => a,
            (200, None) | (206, None) => 0,
            (s, _) => return Err(EngineError::Http { status: s }),
        };
        let want = seg.range.map(|(a, b)| b - a);
        let mut data = Vec::with_capacity(want.unwrap_or(1 << 20) as usize);
        let mut seen = 0u64;
        loop {
            let chunk = tokio::time::timeout(self.read_timeout, resp.chunk())
                .await
                .map_err(|_| EngineError::Timeout)?
                .map_err(|e| EngineError::network(error_chain(&e)))?;
            let Some(chunk) = chunk else { break };
            acquire_all(&self.limiters, chunk.len() as u64).await;
            let start = seen;
            seen += chunk.len() as u64;
            // Keep only [skip, skip + want).
            let lo = skip.saturating_sub(start).min(chunk.len() as u64) as usize;
            let mut hi = chunk.len();
            if let Some(w) = want {
                let limit = (skip + w).saturating_sub(start).min(chunk.len() as u64) as usize;
                hi = hi.min(limit);
            }
            if lo < hi {
                data.extend_from_slice(&chunk[lo..hi]);
                self.progress
                    .downloaded
                    .fetch_add((hi - lo) as u64, Ordering::Relaxed);
            }
            if want.is_some_and(|w| data.len() as u64 >= w) {
                break;
            }
        }
        if let Some(w) = want
            && (data.len() as u64) < w
        {
            // Undo the partial accounting: the segment will be fetched again.
            self.progress
                .downloaded
                .fetch_sub(data.len() as u64, Ordering::Relaxed);
            return Err(EngineError::network("media segment truncated"));
        }
        if let Some(k) = &seg.key {
            let key = self.key(&k.uri).await?;
            let before = data.len();
            data = hls::decrypt_aes128(&key, &k.iv, data)?;
            // Account the padding removed by decryption.
            self.progress
                .downloaded
                .fetch_sub((before - data.len()) as u64, Ordering::Relaxed);
        }
        Ok(data)
    }

    async fn worker(
        self: Arc<Self>,
        stop: tokio_util::sync::CancellationToken,
    ) -> Result<(), EngineError> {
        loop {
            if stop.is_cancelled() {
                return Ok(());
            }
            let Some(global) = self.queue.lock().expect("media queue").pop_front() else {
                return Ok(());
            };
            let (t, s) = self.plan.locate(global);
            let seg = self.plan.tracks[t].segments[s].clone();
            self.progress.start(global);
            let mut backoff = Backoff::default();
            let mut attempt = 0u32;
            let data = loop {
                let r = tokio::select! {
                    r = self.download(&seg) => r,
                    _ = stop.cancelled() => {
                        self.progress.finish(global, None);
                        return Ok(());
                    }
                };
                match r {
                    Ok(d) => break d,
                    Err(e) if e.is_transient() && attempt < self.max_retries => {
                        attempt += 1;
                        let delay = backoff.next_delay();
                        self.job.emit(DownloadEvent::Retrying {
                            reason: e.to_string(),
                            attempt,
                            delay,
                        });
                        tokio::select! {
                            _ = tokio::time::sleep(delay) => {}
                            _ = stop.cancelled() => {
                                self.progress.finish(global, None);
                                return Ok(());
                            }
                        }
                    }
                    Err(e) => {
                        self.progress.finish(global, None);
                        return Err(e);
                    }
                }
            };
            let path = seg_file(&self.work_dir, global);
            let len = data.len() as u64;
            tokio::task::spawn_blocking(move || write_segment(&path, &data))
                .await
                .map_err(EngineError::internal)?
                .map_err(|e| EngineError::from_io(&e, len))?;
            self.progress.finish(global, Some(len));
        }
    }
}

/// Writes a segment file durably: `.tmp` → fsync → rename.
fn write_segment(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(data)?;
    f.sync_data()?;
    drop(f);
    std::fs::rename(&tmp, path)
}

/// Concatenates the segments of each track into `track{t}.{ext}` files.
fn concat_tracks(plan: &MediaPlan, work_dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    use std::io::Write;
    let mut out = Vec::new();
    let mut global = 0usize;
    for (t, track) in plan.tracks.iter().enumerate() {
        let path = work_dir.join(format!("track{t}.{}", track.ext));
        let mut w = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&path)?);
        for _ in &track.segments {
            let mut r = std::fs::File::open(seg_file(work_dir, global))?;
            std::io::copy(&mut r, &mut w)?;
            global += 1;
        }
        w.flush()?;
        w.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        out.push(path);
    }
    Ok(out)
}

/// Detects the stream kind from the requested kind, URL and content.
fn detect_kind(kind: MediaKind, url: &Url, text: &str) -> Result<MediaKind, EngineError> {
    match kind {
        MediaKind::Hls | MediaKind::Dash => Ok(kind),
        MediaKind::Auto => {
            let path = url.path().to_ascii_lowercase();
            if path.ends_with(".m3u8")
                || path.ends_with(".m3u")
                || text.trim_start().starts_with("#EXTM3U")
            {
                Ok(MediaKind::Hls)
            } else if path.ends_with(".mpd") || text.contains("<MPD") {
                Ok(MediaKind::Dash)
            } else {
                Err(EngineError::Unsupported {
                    message: "not an HLS playlist or DASH manifest".into(),
                })
            }
        }
    }
}

fn stopped(job: &JobShared) -> EngineError {
    match job.stop_reason() {
        Some(StopReason::Cancel { .. }) => EngineError::Cancelled,
        _ => EngineError::Paused,
    }
}

/// Entry point of a media job.
pub(crate) async fn run(
    engine: Arc<EngineInner>,
    job: Arc<JobShared>,
    req: MediaRequest,
    resume: Option<ResumeState>,
    extra_limiters: Vec<Arc<RateLimiter>>,
    mut cmd_rx: tokio::sync::mpsc::UnboundedReceiver<Command>,
) -> (Result<Completed, EngineError>, Option<ResumeState>) {
    let run_started = Instant::now();
    let r = tokio::select! {
        r = run_inner(&engine, &job, &req, resume, extra_limiters, run_started) => r,
        // Media jobs accept no live commands except the speed limit (handled by the limiter);
        // reject URL changes explicitly.
        _ = async {
            while let Some(cmd) = cmd_rx.recv().await {
                if let Command::ChangeUrl(_, reply) = cmd {
                    let _ = reply.send(Err(EngineError::Unsupported { message: "changing the address of a stream".into() }));
                }
            }
            std::future::pending::<()>().await
        } => unreachable!("the command loop never ends"),
    };
    match r {
        Ok(c) => (Ok(c), None),
        Err(failure) => {
            let (e, state) = *failure;
            if let (EngineError::Cancelled, Some(StopReason::Cancel { delete_file: true })) =
                (&e, job.stop_reason())
                && let Some(s) = &state
                && let Some(m) = &s.media
            {
                let _ = std::fs::remove_dir_all(&m.work_dir);
                return (Err(e), None);
            }
            (Err(e), state)
        }
    }
}

/// Failure with the resume state to keep (boxed: the pair is large).
type MediaResult = Result<Completed, Box<(EngineError, Option<ResumeState>)>>;

async fn run_inner(
    engine: &Arc<EngineInner>,
    job: &Arc<JobShared>,
    req: &MediaRequest,
    resume: Option<ResumeState>,
    extra_limiters: Vec<Arc<RateLimiter>>,
    run_started: Instant,
) -> MediaResult {
    let cfg = engine.config();
    let fail = |e: EngineError| Box::new((e, None));
    let connections = engine.connections(req.connections);
    let spec = ClientSpec {
        proxy: req
            .proxy
            .clone()
            .unwrap_or_else(|| cfg.default_proxy.clone()),
        http: HttpPreference::Http1Only,
        connect_timeout: cfg.connect_timeout,
        read_timeout: cfg.read_timeout,
        user_agent: req
            .user_agent
            .clone()
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| cfg.user_agent.clone()),
        tls: engine.tls().map_err(fail)?,
        pool_max_idle: connections,
        dns_overrides: cfg.default_dns_overrides.clone(),
    };
    let client = build_client(&spec).map_err(fail)?;
    let headers =
        base_headers(&req.headers, req.cookies.as_deref(), req.referer.as_deref()).map_err(fail)?;

    // Resolve the manifest.
    let (text, manifest_url) = tokio::select! {
        r = fetch_text(&client, &headers, &req.url) => r.map_err(fail)?,
        _ = job.stop.cancelled() => return Err(Box::new((stopped(job), None))),
    };
    let kind = detect_kind(req.kind, &manifest_url, &text).map_err(fail)?;
    let plan = tokio::select! {
        r = async {
            match kind {
                MediaKind::Dash => dash::plan(&client, &headers, &manifest_url, &text, req.variant).await,
                _ => hls::plan(&client, &headers, &manifest_url, &text, req.variant).await,
            }
        } => r.map_err(fail)?,
        _ = job.stop.cancelled() => return Err(Box::new((stopped(job), None))),
    };
    if plan.total_segments() == 0 {
        return Err(fail(EngineError::Unsupported {
            message: "the stream has no segments".into(),
        }));
    }
    let ffmpeg = remux::ffmpeg_path(cfg.ffmpeg_path.as_deref());
    let can_remux = req.remux && remux::available(&ffmpeg).await;

    // Destination and work directory.
    let (final_path, work_dir, done) = match resume.filter(|r| r.kind == ResumeKind::Media) {
        Some(r) => {
            let m = r.media.clone().ok_or_else(|| {
                fail(EngineError::InvalidResumeState {
                    message: "media state missing".into(),
                })
            })?;
            let ids: Vec<String> = plan.tracks.iter().map(|t| t.id.clone()).collect();
            let counts: Vec<u32> = plan
                .tracks
                .iter()
                .map(|t| t.segments.len() as u32)
                .collect();
            if ids != m.tracks || counts != m.segment_counts {
                return Err(Box::new((EngineError::ValidatorMismatch, Some(r))));
            }
            std::fs::create_dir_all(&m.work_dir).map_err(|e| fail(EngineError::from_io(&e, 0)))?;
            let mut done = vec![false; plan.total_segments()];
            for &i in &m.completed {
                if let Some(d) = done.get_mut(i as usize)
                    && seg_file(&m.work_dir, i as usize).is_file()
                {
                    *d = true;
                }
            }
            (r.final_path, m.work_dir, done)
        }
        None => {
            let ext = match (can_remux, kind) {
                (true, _) | (false, MediaKind::Dash) => "mp4",
                _ => "ts",
            };
            let name = match &req.destination.file_name {
                Some(n) if !n.trim().is_empty() => filename::sanitize(n),
                _ => filename::sanitize(&format!("{}.{ext}", plan.name)),
            };
            std::fs::create_dir_all(&req.destination.dir)
                .map_err(|e| fail(EngineError::from_io(&e, 0)))?;
            let (final_path, work_dir) = claim_work_dir(
                &req.destination.dir,
                &name,
                &cfg.temp_suffix,
                req.destination.conflict,
            )
            .map_err(fail)?;
            let total = plan.total_segments();
            (final_path, work_dir, vec![false; total])
        }
    };
    let done_bytes: u64 = done
        .iter()
        .enumerate()
        .filter(|(_, d)| **d)
        .filter_map(|(i, _)| {
            std::fs::metadata(seg_file(&work_dir, i))
                .ok()
                .map(|m| m.len())
        })
        .sum();
    let progress = Arc::new(MediaProgress::new(
        &plan,
        work_dir.clone(),
        done.clone(),
        done_bytes,
    ));
    {
        let mut m = job.meta();
        m.final_url = Some(manifest_url.clone());
        m.part_path = Some(work_dir.clone());
        m.final_path = Some(final_path.clone());
        m.resumable = true;
    }
    job.set_source(ProgressSource::Media(progress.clone()));
    job.emit(DownloadEvent::Probed(ProbeInfo {
        final_url: manifest_url.clone(),
        file_name: final_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        size: None,
        resumable: true,
        mime: Some(if can_remux || kind == MediaKind::Dash {
            "video/mp4".into()
        } else {
            "video/mp2t".into()
        }),
        etag: None,
        last_modified: None,
        accept_ranges: false,
    }));
    job.set_state(DownloadState::Downloading);

    // Parallel segment download.
    let mut limiters = vec![job.limiter.clone()];
    limiters.extend(extra_limiters);
    limiters.push(engine.global.clone());
    let before = progress.downloaded();
    let fetch = Arc::new(Fetch {
        client,
        headers,
        plan: plan.clone(),
        progress: progress.clone(),
        queue: Mutex::new(
            done.iter()
                .enumerate()
                .filter(|(_, d)| !**d)
                .map(|(i, _)| i)
                .collect(),
        ),
        keys: tokio::sync::Mutex::new(HashMap::new()),
        limiters,
        work_dir: work_dir.clone(),
        read_timeout: cfg.read_timeout,
        max_retries: req.max_retries.unwrap_or(cfg.max_retries),
        job: job.clone(),
    });
    let stop = job.stop.child_token();
    let mut workers = tokio::task::JoinSet::new();
    for _ in 0..connections {
        workers.spawn(fetch.clone().worker(stop.clone()));
    }
    let mut meter_tick = tokio::time::interval(Duration::from_millis(250));
    let mut error: Option<EngineError> = None;
    while !workers.is_empty() {
        tokio::select! {
            Some(r) = workers.join_next() => {
                let r = r.unwrap_or_else(|e| Err(EngineError::internal(format!("segment worker panicked: {e}"))));
                if let Err(e) = r {
                    error.get_or_insert(e);
                    stop.cancel();
                }
            }
            _ = meter_tick.tick() => {
                let active = progress.lock().active.len() as u32;
                job.update_rates(progress.downloaded(), progress.total_estimate(), active, connections as u32);
            }
        }
    }
    let state = progress.resume_state(job);
    if let Some(e) = error {
        return Err(Box::new((e, state)));
    }
    if job.stop.is_cancelled() && !progress.all_done() {
        return Err(Box::new((stopped(job), state)));
    }

    // Merge.
    job.set_state(DownloadState::Merging);
    let plan2 = plan.clone();
    let wd = work_dir.clone();
    let tracks = tokio::task::spawn_blocking(move || concat_tracks(&plan2, &wd))
        .await
        .map_err(|e| Box::new((EngineError::internal(e), state.clone())))?
        .map_err(|e| Box::new((EngineError::from_io(&e, 0), state.clone())))?;
    let mut warning = None;
    let mut extra_files = Vec::new();
    let output = if can_remux {
        let tmp = part_path_for(&final_path, ".remux.mp4");
        tokio::select! {
            r = remux::mux(&ffmpeg, &tracks, &tmp) => r.map_err(|e| Box::new((e, state.clone())))?,
            _ = job.stop.cancelled() => return Err(Box::new((stopped(job), state))),
        }
        place(&tmp, &final_path, req.destination.conflict)
            .map_err(|e| Box::new((e, state.clone())))?
    } else {
        warning = Some(if req.remux {
            "ffmpeg was not found; the stream was saved without remuxing".to_string()
        } else {
            "remuxing disabled; the stream was saved as downloaded".to_string()
        });
        let main = place(&tracks[0], &final_path, req.destination.conflict)
            .map_err(|e| Box::new((e, state.clone())))?;
        for (t, path) in tracks.iter().enumerate().skip(1) {
            let stem = main
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let target = main.with_file_name(format!(
                "{stem}.{}.{}",
                if t == 1 { "audio" } else { "extra" },
                plan.tracks[t].ext
            ));
            extra_files.push(
                place(path, &target, ConflictPolicy::Rename)
                    .map_err(|e| Box::new((e, state.clone())))?,
            );
        }
        main
    };
    let _ = std::fs::remove_dir_all(&work_dir);
    let size = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
    {
        let mut m = job.meta();
        m.final_path = Some(output.clone());
        m.total = Some(size);
    }
    Ok(Completed {
        path: output,
        size,
        checksum_ok: None,
        elapsed: run_started.elapsed(),
        transferred: progress.downloaded().saturating_sub(before),
        extra_files,
        warning,
    })
}

/// Chooses the final path and creates its work directory atomically.
fn claim_work_dir(
    dir: &Path,
    name: &str,
    suffix: &str,
    conflict: ConflictPolicy,
) -> Result<(PathBuf, PathBuf), EngineError> {
    let dsuffix = format!("{suffix}.d");
    for n in 0..10_000u32 {
        let candidate = if n == 0 {
            name.to_string()
        } else {
            filename::numbered_name(name, n)
        };
        let final_path = dir.join(&candidate);
        if final_path.exists() {
            match conflict {
                ConflictPolicy::Rename => continue,
                ConflictPolicy::Fail => return Err(EngineError::FileExists { path: final_path }),
                ConflictPolicy::Overwrite => {}
            }
        }
        let work = part_path_for(&final_path, &dsuffix);
        match std::fs::create_dir(&work) {
            Ok(()) => return Ok((final_path, work)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if conflict == ConflictPolicy::Rename {
                    continue;
                }
                return Ok((final_path, work));
            }
            Err(e) => return Err(EngineError::from_io(&e, 0)),
        }
    }
    Err(EngineError::FileExists {
        path: dir.join(name),
    })
}

/// Moves a finished output into place according to the conflict policy.
fn place(from: &Path, to: &Path, conflict: ConflictPolicy) -> Result<PathBuf, EngineError> {
    let io = |e: std::io::Error| EngineError::Io {
        message: format!("cannot move {}: {e}", from.display()),
    };
    match conflict {
        ConflictPolicy::Overwrite => storage::rename_replace(from, to)
            .map(|_| to.to_path_buf())
            .map_err(io),
        ConflictPolicy::Fail => {
            if storage::rename_no_replace(from, to).map_err(io)? {
                Ok(to.to_path_buf())
            } else {
                Err(EngineError::FileExists {
                    path: to.to_path_buf(),
                })
            }
        }
        ConflictPolicy::Rename => {
            let name = to
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            for n in 0..10_000u32 {
                let candidate = if n == 0 {
                    to.to_path_buf()
                } else {
                    to.with_file_name(filename::numbered_name(&name, n))
                };
                if storage::rename_no_replace(from, &candidate).map_err(io)? {
                    return Ok(candidate);
                }
            }
            Err(EngineError::FileExists {
                path: to.to_path_buf(),
            })
        }
    }
}

/// Base name for a manifest URL (`.../master.m3u8` → `master`).
pub(crate) fn name_from_manifest(url: &Url) -> String {
    let name = filename::name_from_url(url).unwrap_or_else(|| "video".into());
    let stem = match name.rsplit_once('.') {
        Some((s, ext)) if ["m3u8", "m3u", "mpd"].contains(&ext.to_ascii_lowercase().as_str()) => {
            s.to_string()
        }
        _ => name,
    };
    if stem.is_empty() {
        "video".into()
    } else {
        stem
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_detection_and_names() {
        let mut req = MediaRequest::new(Url::parse("http://h/a.m3u8").unwrap(), ".");
        assert_eq!(
            detect_kind(req.kind, &req.url.clone(), "").unwrap(),
            MediaKind::Hls
        );
        let u = Url::parse("http://h/x?y").unwrap();
        assert_eq!(
            detect_kind(req.kind, &u, "<?xml?><MPD>").unwrap(),
            MediaKind::Dash
        );
        assert!(detect_kind(req.kind, &u, "hello").is_err());
        req.kind = MediaKind::Dash;
        assert_eq!(
            detect_kind(req.kind, &u, "#EXTM3U").unwrap(),
            MediaKind::Dash
        );
        assert_eq!(
            name_from_manifest(&Url::parse("http://h/v/master.m3u8").unwrap()),
            "master"
        );
        assert_eq!(
            name_from_manifest(&Url::parse("http://h/").unwrap()),
            "video"
        );
    }

    #[test]
    fn work_dir_claiming() {
        let dir = tempfile::tempdir().unwrap();
        let (f1, w1) =
            claim_work_dir(dir.path(), "v.mp4", ".prismpart", ConflictPolicy::Rename).unwrap();
        assert_eq!(f1, dir.path().join("v.mp4"));
        assert!(w1.is_dir());
        let (f2, _) =
            claim_work_dir(dir.path(), "v.mp4", ".prismpart", ConflictPolicy::Rename).unwrap();
        assert_eq!(f2, dir.path().join("v (1).mp4"));
        std::fs::write(dir.path().join("x.mp4"), b"1").unwrap();
        assert!(claim_work_dir(dir.path(), "x.mp4", ".prismpart", ConflictPolicy::Fail).is_err());
    }
}
