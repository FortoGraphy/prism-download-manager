//! The HTTP download job: probe (or validated resume request), destination setup, the
//! segmented transfer, and the end of the job (finalize, pause, cancel, failure).

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use tokio::sync::mpsc::UnboundedReceiver;
use url::Url;

use super::auth::AuthState;
use super::client::ClientSpec;
use super::fetcher::{HttpFactory, HttpFetcher, ProbeMode};
use super::{HttpCtx, Sources, Validators, base_headers, strong_etag};
use crate::backoff::Backoff;
use crate::engine::EngineInner;
use crate::error::EngineError;
use crate::hostlimits::HostPermit;
use crate::job::{Command, JobShared, ProgressSource, StopReason};
use crate::limiter::RateLimiter;
use crate::progress::{Completed, DownloadEvent, DownloadState, ProbeInfo};
use crate::request::{DownloadRequest, HttpMethod};
use crate::resume::{ResumeKind, ResumeState};
use crate::segmented::supervisor::{self, Params, UrlChanger};
use crate::segmented::table::SegmentTable;
use crate::segmented::writer::Writer;
use crate::segmented::{ChunkStream, FailKind, InitialStream, SegCtx};
use crate::storage;
use crate::transfer::{self, Paths, Prepared, end_job};

/// Bytes compared on disk when resuming without validators.
const OVERLAP_CHECK: u64 = 64 * 1024;

/// `host[:port]` key for connection accounting.
pub(crate) fn host_key(url: &Url) -> String {
    match (url.host_str(), url.port()) {
        (Some(h), Some(p)) => format!("{h}:{p}"),
        (Some(h), None) => h.to_string(),
        _ => String::new(),
    }
}

/// Builds the per-download HTTP context.
pub(crate) fn build_ctx(
    engine: &EngineInner,
    req: &DownloadRequest,
    url: Url,
    final_url: Option<Url>,
) -> Result<Arc<HttpCtx>, EngineError> {
    let cfg = engine.config();
    let spec = ClientSpec {
        proxy: req
            .proxy
            .clone()
            .unwrap_or_else(|| cfg.default_proxy.clone()),
        http: req.http,
        connect_timeout: cfg.connect_timeout,
        read_timeout: cfg.read_timeout,
        user_agent: req
            .user_agent
            .clone()
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| cfg.user_agent.clone()),
        tls: engine.tls()?,
        pool_max_idle: 1,
        dns_overrides: if req.dns_overrides.is_empty() {
            cfg.default_dns_overrides.clone()
        } else {
            req.dns_overrides.clone()
        },
    };
    Ok(Arc::new(HttpCtx {
        spec,
        method: req.method,
        body: req.body.clone(),
        headers: base_headers(&req.headers, req.cookies.as_deref(), req.referer.as_deref())?,
        auth: AuthState::new(req.auth.clone()),
        sources: Sources::new(url, final_url, &req.mirrors),
        validators: Default::default(),
        probed: Default::default(),
        expect_file: req.expect_file,
        source_generation: Arc::new(AtomicU64::new(0)),
    }))
}

/// Runs `op` with retries for transient failures (used for the first request).
macro_rules! with_retries {
    ($job:expr, $max:expr, $op:expr) => {{
        let mut backoff = Backoff::default();
        let mut attempt = 0u32;
        loop {
            let r = tokio::select! {
                r = $op => r,
                _ = $job.stop.cancelled() => break Err(stopped_error($job)),
            };
            match r {
                Ok(v) => break Ok(v),
                Err(e) if e.kind != FailKind::Fatal && e.error.is_transient() && attempt < $max => {
                    attempt += 1;
                    let delay = e.retry_after.unwrap_or_else(|| backoff.next_delay());
                    $job.emit(DownloadEvent::Retrying {
                        reason: e.error.to_string(),
                        attempt,
                        delay,
                    });
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = $job.stop.cancelled() => break Err(stopped_error($job)),
                    }
                }
                Err(e) => break Err(e.error),
            }
        }
    }};
}

fn stopped_error(job: &JobShared) -> EngineError {
    match job.stop_reason() {
        Some(StopReason::Cancel { .. }) => EngineError::Cancelled,
        _ => EngineError::Paused,
    }
}

/// Validates new URLs for `change_url` and mirrors.
struct HttpUrlChanger {
    ctx: Arc<HttpCtx>,
    job: Arc<JobShared>,
}

/// Fetches `[0, 1)` of `url` and returns `(total, strong etag, final url)`.
async fn check_url(
    ctx: &Arc<HttpCtx>,
    url: &Url,
) -> Result<(Option<u64>, Option<String>, Url), EngineError> {
    let mut f = HttpFetcher::new(ctx.clone(), u32::MAX);
    let resp = f
        .send(url, Some((0, Some(1))), None)
        .await
        .map_err(|e| e.error)?;
    let status = resp.status().as_u16();
    if status != 206 {
        return Err(match status {
            200 => EngineError::RangeNotSupported,
            401 | 407 => EngineError::AuthRequired,
            s => EngineError::Http { status: s },
        });
    }
    let total = super::header_string(resp.headers(), reqwest::header::CONTENT_RANGE)
        .and_then(|v| super::parse_content_range(&v))
        .and_then(|(_, _, t)| t);
    let etag = super::header_string(resp.headers(), reqwest::header::ETAG);
    Ok((total, etag, resp.url().clone()))
}

impl UrlChanger for HttpUrlChanger {
    fn change_url(
        &self,
        url: Url,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<(), EngineError>> + Send + '_>> {
        Box::pin(async move {
            let v = self.ctx.validators();
            if !v.ranged {
                return Err(EngineError::RangeNotSupported);
            }
            let (total, etag, final_url) = check_url(&self.ctx, &url).await?;
            if total != v.total {
                return Err(EngineError::ValidatorMismatch);
            }
            if let (Some(a), Some(b)) =
                (strong_etag(v.etag.as_deref()), strong_etag(etag.as_deref()))
                && a != b
            {
                return Err(EngineError::ValidatorMismatch);
            }
            self.ctx
                .sources
                .replace_primary(url.clone(), Some(final_url.clone()));
            self.ctx.source_generation.fetch_add(1, Ordering::AcqRel);
            let mut m = self.job.meta();
            m.url = url;
            m.final_url = Some(final_url);
            Ok(())
        })
    }
}

/// Validates mirrors in the background; valid ones join the rotation.
fn spawn_mirror_validation(ctx: Arc<HttpCtx>, job: Arc<JobShared>) {
    let mirrors = ctx.sources.mirror_urls();
    if mirrors.is_empty() {
        return;
    }
    tokio::spawn(async move {
        for (i, url) in mirrors.into_iter().enumerate() {
            let check = tokio::select! {
                r = check_url(&ctx, &url) => r,
                _ = job.stop.cancelled() => return,
            };
            match check {
                Ok((total, _, final_url)) if total.is_some() && total == ctx.validators().total => {
                    ctx.sources.validate_mirror(i, Some(final_url));
                }
                Ok(_) => tracing::info!(%url, "mirror rejected: different size"),
                Err(e) => tracing::info!(%url, error = %e, "mirror rejected"),
            }
        }
    });
}

/// Entry point of an HTTP job. Returns the result and the final resume state.
pub(crate) async fn run(
    engine: Arc<EngineInner>,
    job: Arc<JobShared>,
    req: DownloadRequest,
    resume: Option<ResumeState>,
    extra_limiters: Vec<Arc<RateLimiter>>,
    mut cmd_rx: UnboundedReceiver<Command>,
) -> (Result<Completed, EngineError>, Option<ResumeState>) {
    let run_started = Instant::now();
    let mut setup = match setup(&engine, &job, &req, resume).await {
        Ok(s) => s,
        Err(e) => return (Err(e), None),
    };
    let cfg = engine.config();
    let mut limiters = vec![job.limiter.clone()];
    limiters.extend(extra_limiters);
    limiters.push(engine.global.clone());
    let ranged = setup.ctx.validators().ranged;
    let requested = if ranged {
        engine.connections(req.connections)
    } else {
        1
    };
    let (min_request, max_request) = engine.request_sizes(req.max_request_size);
    let seg = Arc::new(SegCtx {
        job: job.clone(),
        table: setup.table.clone(),
        writer: setup.writer.clone(),
        limiters,
        stop: job.stop.child_token(),
        host: setup.host.clone(),
        host_limits: engine.host_limits.clone(),
        connected: AtomicUsize::new(0),
        connected_notify: Default::default(),
        active_workers: AtomicUsize::new(0),
        target: AtomicUsize::new(requested),
        source_generation: setup.ctx.source_generation.clone(),
        min_request,
        max_request,
        read_timeout: cfg.read_timeout,
        max_retries: req.max_retries.unwrap_or(cfg.max_retries),
        race: ranged,
        few_requests: Default::default(),
    });
    if ranged {
        spawn_mirror_validation(setup.ctx.clone(), job.clone());
    }
    job.set_state(DownloadState::Downloading);
    let changer = HttpUrlChanger {
        ctx: setup.ctx.clone(),
        job: job.clone(),
    };
    let factory = Arc::new(HttpFactory {
        ctx: setup.ctx.clone(),
    });
    let downloaded_before = setup.table.downloaded();
    let outcome = supervisor::supervise(
        seg,
        factory,
        setup.first.take(),
        &mut cmd_rx,
        Some(&changer),
        Params {
            requested,
            initial: usize::from(cfg.initial_connections),
            cap: usize::from(cfg.max_connections_cap),
        },
    )
    .await;
    let transferred = setup.table.downloaded().saturating_sub(downloaded_before);
    end_job(
        &job,
        &setup.writer,
        &setup.prepared_paths,
        outcome,
        &req,
        transferred,
        run_started,
    )
    .await
}

struct Setup {
    ctx: Arc<HttpCtx>,
    host: String,
    table: Arc<SegmentTable>,
    writer: Arc<Writer>,
    first: Option<(HttpFetcher, InitialStream<super::fetcher::HttpStream>)>,
    prepared_paths: Paths,
}

async fn setup(
    engine: &Arc<EngineInner>,
    job: &Arc<JobShared>,
    req: &DownloadRequest,
    resume: Option<ResumeState>,
) -> Result<Setup, EngineError> {
    let cfg = engine.config();
    let max_retries = req.max_retries.unwrap_or(cfg.max_retries);
    // Decide whether the resume data is usable.
    let mut fixed_path = None;
    let resume = match resume {
        Some(r) if r.kind == ResumeKind::Http && r.resumable => match r.verify_against_disk() {
            Ok(()) => Some(r),
            Err(e) => {
                job.emit(DownloadEvent::Restarted {
                    reason: e.to_string(),
                });
                let _ = std::fs::remove_file(&r.part_path);
                fixed_path = Some(r.final_path.clone());
                None
            }
        },
        Some(r) => {
            job.emit(DownloadEvent::Restarted {
                reason: "the download cannot be resumed; starting over".into(),
            });
            let _ = std::fs::remove_file(&r.part_path);
            fixed_path = Some(r.final_path.clone());
            None
        }
        None => None,
    };
    let (url, final_url) = match &resume {
        Some(r) => (r.url.clone(), r.final_url.clone()),
        None => (req.url.clone(), None),
    };
    let ctx = build_ctx(engine, req, url.clone(), final_url)?;
    let host = host_key(&url);
    let permit = tokio::select! {
        p = engine.host_limits.acquire(&host) => p,
        _ = job.stop.cancelled() => return Err(stopped_error(job)),
    };
    let mut fetcher = HttpFetcher::new(ctx.clone(), 0);
    let (min_request, _) = engine.request_sizes(req.max_request_size);
    match resume {
        Some(state) => {
            setup_resume(
                engine,
                job,
                ctx,
                host,
                permit,
                fetcher,
                state,
                min_request,
                max_retries,
            )
            .await
        }
        None => {
            let use_ranges = req.method == HttpMethod::Get;
            let probe = with_retries!(job, max_retries, fetcher.probe(min_request, use_ranges))?;
            let (total, ranged) = match probe.mode {
                ProbeMode::Ranged { total } => (Some(total), true),
                ProbeMode::Single { size } => (size, false),
                ProbeMode::Empty => (Some(0), true),
            };
            ctx.set_validators(Validators {
                total,
                etag: probe.info.etag.clone(),
                ranged,
            });
            job.emit(DownloadEvent::Probed(probe.info.clone()));
            let prepared = transfer::prepare_destination(
                &req.destination,
                &probe.info.file_name,
                &cfg.temp_suffix,
                total.filter(|_| ranged),
                fixed_path.as_deref(),
            )?;
            record_meta(job, &ctx, &probe.info, total, ranged, &prepared);
            let Prepared {
                final_path,
                part_path,
                file,
            } = prepared;
            let table = Arc::new(SegmentTable::new(total, cfg.min_split_size, ranged));
            let writer = spawn_writer(engine, file, &table)?;
            job.set_source(ProgressSource::Segmented {
                table: table.clone(),
                writer: writer.clone(),
            });
            let first = match probe.stream {
                Some(stream) if total != Some(0) => {
                    let (assignment, _) = table
                        .acquire(0, false)
                        .ok_or_else(|| EngineError::internal("empty table"))?;
                    let req_end = stream.range_end();
                    Some((
                        fetcher,
                        InitialStream {
                            assignment,
                            stream,
                            req_end,
                            permit,
                        },
                    ))
                }
                _ => None,
            };
            Ok(Setup {
                ctx,
                host,
                table,
                writer,
                first,
                prepared_paths: Paths {
                    part: part_path,
                    final_path,
                },
            })
        }
    }
}

fn spawn_writer(
    engine: &EngineInner,
    file: std::fs::File,
    table: &Arc<SegmentTable>,
) -> Result<Arc<Writer>, EngineError> {
    Writer::spawn(
        file,
        table.clone(),
        engine.budget.clone(),
        engine.budget_total(),
        engine.config().write_buffer,
    )
    .map_err(|e| EngineError::Io {
        message: format!("cannot start writer thread: {e}"),
    })
}

fn record_meta(
    job: &JobShared,
    ctx: &HttpCtx,
    info: &ProbeInfo,
    total: Option<u64>,
    ranged: bool,
    p: &Prepared,
) {
    let mut m = job.meta();
    m.final_url = ctx.sources.final_url();
    m.total = total;
    m.etag = info.etag.clone();
    m.last_modified = info.last_modified.clone();
    m.resumable = ranged;
    m.part_path = Some(p.part_path.clone());
    m.final_path = Some(p.final_path.clone());
}

#[allow(clippy::too_many_arguments)]
async fn setup_resume(
    engine: &Arc<EngineInner>,
    job: &Arc<JobShared>,
    ctx: Arc<HttpCtx>,
    host: String,
    permit: HostPermit,
    mut fetcher: HttpFetcher,
    state: ResumeState,
    min_request: u64,
    max_retries: u32,
) -> Result<Setup, EngineError> {
    let total = state
        .total_size
        .ok_or_else(|| EngineError::InvalidResumeState {
            message: "no size".into(),
        })?;
    ctx.set_validators(Validators {
        total: Some(total),
        etag: state.etag.clone(),
        ranged: true,
    });
    {
        let mut m = job.meta();
        m.url = state.url.clone();
        m.final_url = state.final_url.clone();
        m.total = Some(total);
        m.etag = state.etag.clone();
        m.last_modified = state.last_modified.clone();
        m.resumable = true;
        m.part_path = Some(state.part_path.clone());
        m.final_path = Some(state.final_path.clone());
    }
    let file =
        storage::open_part_file(&state.part_path).map_err(|e| EngineError::from_io(&e, 0))?;
    let table = Arc::new(SegmentTable::from_states(
        &state.segments,
        total,
        engine.config().min_split_size,
    ));
    // Free space for what is still missing (the file is sparse on Windows).
    if let Some(dir) = state.part_path.parent() {
        storage::check_free_space(dir, table.remaining().unwrap_or(0))?;
    }
    let if_range = strong_etag(state.etag.as_deref())
        .map(str::to_string)
        .or_else(|| state.last_modified.clone());
    if if_range.is_none() {
        verify_overlap(job, &mut fetcher, &file, &table, max_retries).await?;
    }
    let first = match table.first_pending() {
        None => None,
        Some(seg) => {
            let assignment = table
                .claim(seg, 0)
                .ok_or_else(|| EngineError::internal("cannot claim segment"))?;
            let (_, pos, end) = table.bounds(seg);
            let req_end = end.min(pos + min_request);
            table.set_request_end(seg, 0, req_end);
            let (stream, mut info) = with_retries!(
                job,
                max_retries,
                fetcher.open_validated(pos, Some(req_end), if_range.as_deref())
            )?;
            if let Some(name) = state.final_path.file_name() {
                info.file_name = name.to_string_lossy().into_owned();
            }
            job.meta().final_url = ctx.sources.final_url();
            job.emit(DownloadEvent::Probed(info));
            let req_end = stream.range_end().or(Some(req_end));
            Some((
                fetcher,
                InitialStream {
                    assignment,
                    stream,
                    req_end,
                    permit,
                },
            ))
        }
    };
    let writer = spawn_writer(engine, file, &table)?;
    job.set_source(ProgressSource::Segmented {
        table: table.clone(),
        writer: writer.clone(),
    });
    Ok(Setup {
        ctx,
        host,
        table,
        writer,
        first,
        prepared_paths: Paths {
            part: state.part_path,
            final_path: state.final_path,
        },
    })
}

/// Without validators, re-downloads up to 64 KiB just before a resume point and compares it
/// with the bytes on disk.
async fn verify_overlap(
    job: &Arc<JobShared>,
    fetcher: &mut HttpFetcher,
    file: &std::fs::File,
    table: &SegmentTable,
    max_retries: u32,
) -> Result<(), EngineError> {
    // The segment with the most data on disk gives the longest comparable stretch.
    let Some((start, pos)) = table
        .durable_states()
        .into_iter()
        .filter(|s| s.pos > s.start)
        .max_by_key(|s| s.pos - s.start)
        .map(|s| (s.start, s.pos))
    else {
        return Ok(());
    };
    let len = (pos - start).min(OVERLAP_CHECK);
    let from = pos - len;
    let (mut stream, _) = with_retries!(
        job,
        max_retries,
        fetcher.open_validated(from, Some(pos), None)
    )?;
    let mut remote = Vec::with_capacity(len as usize);
    while (remote.len() as u64) < len {
        match stream.next_chunk().await.map_err(|e| e.error)? {
            Some(c) => remote.extend_from_slice(&c),
            None => break,
        }
    }
    remote.truncate(len as usize);
    let mut local = vec![0u8; len as usize];
    storage::read_exact_at(file, &mut local, from).map_err(|e| EngineError::from_io(&e, 0))?;
    if remote != local {
        return Err(EngineError::ValidatorMismatch);
    }
    Ok(())
}
