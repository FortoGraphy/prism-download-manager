//! FTP / FTPS (explicit `AUTH TLS`) downloads with `suppaftp`: `SIZE` probe, multiple
//! connections via `REST` + `RETR` per segment, passive mode, resume with size/`MDTM` checks.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::{Bytes, BytesMut};
use percent_encoding::percent_decode_str;
use suppaftp::tokio::{
    AsyncRustlsConnector, AsyncRustlsFtpStream, AsyncRustlsStream, TransferStream,
};
use suppaftp::types::FileType;
use suppaftp::{FtpError, Mode};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc::UnboundedReceiver;
use url::Url;

use crate::backoff::Backoff;
use crate::engine::EngineInner;
use crate::error::EngineError;
use crate::filename;
use crate::job::{Command, JobShared, ProgressSource, StopReason};
use crate::limiter::RateLimiter;
use crate::progress::{Completed, DownloadEvent, DownloadState, ProbeInfo};
use crate::request::{Credentials, DownloadRequest};
use crate::resume::{ResumeKind, ResumeState};
use crate::segmented::supervisor::{self, Params};
use crate::segmented::table::{SegmentTable, WorkerId};
use crate::segmented::writer::Writer;
use crate::segmented::{ChunkStream, FailKind, FetchError, FetcherFactory, RangeFetcher, SegCtx};
use crate::storage;
use crate::transfer::{self, Paths, end_job};

type Ftp = AsyncRustlsFtpStream;

/// Read buffer per data-connection read.
const READ_CHUNK: usize = 64 * 1024;

/// Where and how to connect.
#[derive(Debug, Clone)]
pub(crate) struct FtpTarget {
    pub host: String,
    pub port: u16,
    pub path: String,
    pub user: String,
    pub pass: String,
    pub secure: bool,
}

impl FtpTarget {
    /// Parses an `ftp://` / `ftps://` URL; credentials come from the URL, then `creds`, then
    /// anonymous.
    pub(crate) fn parse(url: &Url, creds: Option<&Credentials>) -> Result<FtpTarget, EngineError> {
        let secure = match url.scheme() {
            "ftp" => false,
            "ftps" => true,
            s => {
                return Err(EngineError::InvalidUrl {
                    message: format!("unsupported scheme {s}"),
                });
            }
        };
        let host = url.host_str().ok_or_else(|| EngineError::InvalidUrl {
            message: "missing host".into(),
        })?;
        let decode = |s: &str| percent_decode_str(s).decode_utf8_lossy().into_owned();
        let path = decode(url.path());
        if path.is_empty() || path.ends_with('/') {
            return Err(EngineError::InvalidUrl {
                message: "the FTP URL does not name a file".into(),
            });
        }
        let (user, pass) = if !url.username().is_empty() {
            (
                decode(url.username()),
                url.password().map(decode).unwrap_or_default(),
            )
        } else if let Some(c) = creds {
            (c.username.clone(), c.password.clone())
        } else {
            ("anonymous".to_string(), "anonymous@".to_string())
        };
        if [&path, &user, &pass]
            .iter()
            .any(|s| s.bytes().any(|b| matches!(b, b'\r' | b'\n' | b'\0')))
        {
            return Err(EngineError::InvalidUrl {
                message: "control characters are not allowed in FTP URLs or credentials".into(),
            });
        }
        Ok(FtpTarget {
            host: host
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_string(),
            port: url.port().unwrap_or(21),
            path,
            user,
            pass,
            secure,
        })
    }
}

struct FtpCtx {
    target: FtpTarget,
    tls: Option<tokio_rustls::TlsConnector>,
    connect_timeout: Duration,
}

fn map_ftp(e: FtpError) -> FetchError {
    match e {
        FtpError::ConnectionError(io) => FetchError::maybe_refused(EngineError::Network {
            message: format!("FTP connection: {io}"),
        }),
        FtpError::UnexpectedResponse(r) => {
            let code = r.status.code();
            let text = format!("{code} {}", String::from_utf8_lossy(&r.body).trim());
            let lower = text.to_ascii_lowercase();
            let too_many =
                lower.contains("too many") || lower.contains("maximum") || lower.contains("limit");
            match code {
                421 => FetchError::refused(
                    EngineError::Network {
                        message: format!("FTP: {text}"),
                    },
                    None,
                ),
                530 if too_many => FetchError::refused(
                    EngineError::Network {
                        message: format!("FTP: {text}"),
                    },
                    None,
                ),
                530 | 331 | 332 => FetchError::fatal(EngineError::AuthRequired),
                400..=499 => FetchError::transient(EngineError::Network {
                    message: format!("FTP: {text}"),
                }),
                _ => FetchError::fatal(EngineError::Ftp { message: text }),
            }
        }
        FtpError::SecureError(s) => FetchError::fatal(EngineError::Ftp {
            message: format!("TLS: {s}"),
        }),
        other => FetchError::transient(EngineError::Network {
            message: format!("FTP: {other}"),
        }),
    }
}

async fn connect(ctx: &FtpCtx) -> Result<Ftp, FetchError> {
    let t = &ctx.target;
    let addr = tokio::time::timeout(
        ctx.connect_timeout,
        tokio::net::lookup_host((t.host.as_str(), t.port)),
    )
    .await
    .map_err(|_| FetchError::transient(EngineError::Timeout))?
    .map_err(|e| {
        FetchError::transient(EngineError::network(format!(
            "cannot resolve {}: {e}",
            t.host
        )))
    })?
    .next()
    .ok_or_else(|| FetchError::fatal(EngineError::network(format!("{} has no address", t.host))))?;
    let mut ftp = Ftp::connect_timeout(addr, ctx.connect_timeout)
        .await
        .map_err(map_ftp)?;
    if let Some(tls) = &ctx.tls {
        ftp = ftp
            .into_secure(AsyncRustlsConnector::from(tls.clone()), &t.host)
            .await
            .map_err(map_ftp)?;
    }
    ftp.set_mode(Mode::Passive);
    // Servers behind NAT often advertise private addresses in PASV replies.
    ftp.set_passive_nat_workaround(true);
    ftp.login(t.user.as_str(), t.pass.as_str())
        .await
        .map_err(map_ftp)?;
    ftp.transfer_type(FileType::Binary).await.map_err(map_ftp)?;
    Ok(ftp)
}

/// An FTP data connection (`RETR` from an offset).
pub(crate) struct FtpStream {
    inner: Option<TransferStream<AsyncRustlsStream>>,
    clean: Arc<AtomicBool>,
}

impl ChunkStream for FtpStream {
    async fn next_chunk(&mut self) -> Result<Option<Bytes>, FetchError> {
        let Some(s) = self.inner.as_mut() else {
            return Ok(None);
        };
        let mut buf = BytesMut::with_capacity(READ_CHUNK);
        let n = s.read_buf(&mut buf).await.map_err(|e| {
            FetchError::transient(EngineError::Network {
                message: format!("FTP data: {e}"),
            })
        })?;
        if n == 0 {
            let s = self.inner.take().expect("checked above");
            // Reading the transfer reply keeps the control connection usable.
            if s.finish().await.is_ok() {
                self.clean.store(true, Ordering::Release);
            }
            return Ok(None);
        }
        Ok(Some(buf.freeze()))
    }
}

/// One worker's FTP control connection.
pub(crate) struct FtpFetcher {
    ctx: Arc<FtpCtx>,
    conn: Option<Ftp>,
    /// Whether the previous transfer on `conn` was completed cleanly (otherwise reconnect).
    last_clean: Arc<AtomicBool>,
}

impl RangeFetcher for FtpFetcher {
    type Stream = FtpStream;

    async fn open(&mut self, from: u64, _to: Option<u64>) -> Result<FtpStream, FetchError> {
        if !self.last_clean.load(Ordering::Acquire) {
            // An aborted transfer leaves the control connection in an uncertain state.
            self.conn = None;
        }
        if self.conn.is_none() {
            self.conn = Some(connect(&self.ctx).await?);
        }
        let ftp = self.conn.as_mut().expect("connected above");
        let path = self.ctx.target.path.clone();
        let result = async {
            if from > 0 {
                ftp.resume_transfer(from as usize).await?;
            }
            ftp.retr_as_stream(path).await
        }
        .await;
        match result {
            Ok(ts) => {
                let clean = Arc::new(AtomicBool::new(false));
                self.last_clean = clean.clone();
                Ok(FtpStream {
                    inner: Some(ts),
                    clean,
                })
            }
            Err(e) => {
                self.conn = None;
                Err(map_ftp(e))
            }
        }
    }

    fn bounded(&self) -> bool {
        false
    }
}

struct FtpFactory {
    ctx: Arc<FtpCtx>,
    /// The probe's control connection, handed to the first worker.
    first: Mutex<Option<Ftp>>,
}

impl FetcherFactory for FtpFactory {
    type Fetcher = FtpFetcher;
    fn create(&self, _worker: WorkerId) -> FtpFetcher {
        FtpFetcher {
            ctx: self.ctx.clone(),
            conn: self.first.lock().expect("ftp first lock").take(),
            last_clean: Arc::new(AtomicBool::new(true)),
        }
    }
}

/// Result of the FTP probe.
struct FtpProbe {
    info: ProbeInfo,
    size: Option<u64>,
    resumable: bool,
    conn: Ftp,
}

async fn probe_ftp(ctx: &FtpCtx, url: &Url) -> Result<FtpProbe, FetchError> {
    let mut ftp = connect(ctx).await?;
    let path = ctx.target.path.clone();
    let size = match ftp.size(&path).await {
        Ok(s) => Some(s as u64),
        Err(FtpError::UnexpectedResponse(r)) if r.status.code() == 550 => {
            // SIZE is answered with 550 both for missing files and for unsupported cases;
            // MDTM tells them apart (a missing file fails there too).
            if ftp.mdtm(&path).await.is_err() {
                return Err(FetchError::fatal(EngineError::Ftp {
                    message: format!("550 {} not found", ctx.target.path),
                }));
            }
            None
        }
        Err(e) => return Err(map_ftp(e)),
    };
    let last_modified = ftp.mdtm(&path).await.ok().map(|t| t.to_string());
    // REST support: REST 0 must be accepted with 350.
    let resumable = size.is_some() && ftp.resume_transfer(0).await.is_ok();
    let name = filename::derive_file_name(None, url, None);
    let mime = mime_guess::from_path(&name)
        .first()
        .map(|m| m.essence_str().to_string());
    let info = ProbeInfo {
        final_url: url.clone(),
        file_name: name,
        size,
        resumable,
        mime,
        etag: None,
        last_modified,
        accept_ranges: resumable,
    };
    Ok(FtpProbe {
        info,
        size,
        resumable,
        conn: ftp,
    })
}

fn build_ctx(
    engine: &EngineInner,
    url: &Url,
    creds: Option<&Credentials>,
) -> Result<FtpCtx, EngineError> {
    let target = FtpTarget::parse(url, creds)?;
    let tls = if target.secure {
        let mut cfg = engine.tls()?;
        cfg.alpn_protocols.clear();
        Some(tokio_rustls::TlsConnector::from(Arc::new(cfg)))
    } else {
        None
    };
    Ok(FtpCtx {
        target,
        tls,
        connect_timeout: engine.config().connect_timeout,
    })
}

/// Probes an FTP URL (for [`crate::Engine::probe`]).
pub(crate) async fn probe(
    engine: &EngineInner,
    req: &DownloadRequest,
) -> Result<ProbeInfo, EngineError> {
    let ctx = build_ctx(engine, &req.url, req.auth.as_ref())?;
    let mut p = probe_ftp(&ctx, &req.url).await.map_err(|e| e.error)?;
    let _ = p.conn.quit().await;
    Ok(p.info)
}

fn stopped_error(job: &JobShared) -> EngineError {
    match job.stop_reason() {
        Some(StopReason::Cancel { .. }) => EngineError::Cancelled,
        _ => EngineError::Paused,
    }
}

/// Entry point of an FTP job.
pub(crate) async fn run(
    engine: Arc<EngineInner>,
    job: Arc<JobShared>,
    req: DownloadRequest,
    resume: Option<ResumeState>,
    extra_limiters: Vec<Arc<RateLimiter>>,
    mut cmd_rx: UnboundedReceiver<Command>,
) -> (Result<Completed, EngineError>, Option<ResumeState>) {
    let run_started = Instant::now();
    let cfg = engine.config();
    let max_retries = req.max_retries.unwrap_or(cfg.max_retries);
    let url = resume
        .as_ref()
        .map_or_else(|| req.url.clone(), |r| r.url.clone());
    let ctx = match build_ctx(&engine, &url, req.auth.as_ref()) {
        Ok(c) => Arc::new(c),
        Err(e) => return (Err(e), None),
    };
    // Probe with retries.
    let mut backoff = Backoff::default();
    let mut attempt = 0;
    let probe = loop {
        let r = tokio::select! {
            r = probe_ftp(&ctx, &url) => r,
            _ = job.stop.cancelled() => return (Err(stopped_error(&job)), None),
        };
        match r {
            Ok(p) => break p,
            Err(e)
                if e.kind != FailKind::Fatal && e.error.is_transient() && attempt < max_retries =>
            {
                attempt += 1;
                let delay = backoff.next_delay();
                job.emit(DownloadEvent::Retrying {
                    reason: e.error.to_string(),
                    attempt,
                    delay,
                });
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = job.stop.cancelled() => return (Err(stopped_error(&job)), None),
                }
            }
            Err(e) => return (Err(e.error), None),
        }
    };

    // Resume or fresh start.
    let mut fixed = None;
    let resume = match resume {
        Some(r) if r.kind == ResumeKind::Ftp && r.resumable && probe.resumable => {
            let same = r.total_size == probe.size
                && (r.last_modified.is_none()
                    || probe.info.last_modified.is_none()
                    || r.last_modified == probe.info.last_modified);
            if !same {
                return (Err(EngineError::ValidatorMismatch), Some(r));
            }
            match r.verify_against_disk() {
                Ok(()) => Some(r),
                Err(e) => {
                    job.emit(DownloadEvent::Restarted {
                        reason: e.to_string(),
                    });
                    let _ = std::fs::remove_file(&r.part_path);
                    fixed = Some(r.final_path.clone());
                    None
                }
            }
        }
        Some(r) => {
            job.emit(DownloadEvent::Restarted {
                reason: "the download cannot be resumed; starting over".into(),
            });
            let _ = std::fs::remove_file(&r.part_path);
            fixed = Some(r.final_path.clone());
            None
        }
        None => None,
    };
    job.emit(DownloadEvent::Probed(probe.info.clone()));
    let ranged = probe.resumable;
    let (table, file, paths) = match &resume {
        Some(r) => {
            let total = r.total_size.unwrap_or(0);
            let file = match storage::open_part_file(&r.part_path) {
                Ok(f) => f,
                Err(e) => return (Err(EngineError::from_io(&e, 0)), None),
            };
            let table = Arc::new(SegmentTable::from_states(
                &r.segments,
                total,
                cfg.min_split_size,
            ));
            (
                table,
                file,
                Paths {
                    part: r.part_path.clone(),
                    final_path: r.final_path.clone(),
                },
            )
        }
        None => {
            let prepared = match transfer::prepare_destination(
                &req.destination,
                &probe.info.file_name,
                &cfg.temp_suffix,
                probe.size.filter(|_| ranged),
                fixed.as_deref(),
            ) {
                Ok(p) => p,
                Err(e) => return (Err(e), None),
            };
            let table = Arc::new(SegmentTable::new(probe.size, cfg.min_split_size, ranged));
            (
                table,
                prepared.file,
                Paths {
                    part: prepared.part_path,
                    final_path: prepared.final_path,
                },
            )
        }
    };
    {
        let mut m = job.meta();
        m.url = url.clone();
        m.final_url = Some(url.clone());
        m.total = probe.size;
        m.last_modified = probe.info.last_modified.clone();
        m.resumable = ranged;
        m.part_path = Some(paths.part.clone());
        m.final_path = Some(paths.final_path.clone());
    }
    let writer = match Writer::spawn(
        file,
        table.clone(),
        engine.budget.clone(),
        engine.budget_total(),
        cfg.write_buffer,
    ) {
        Ok(w) => w,
        Err(e) => {
            return (
                Err(EngineError::Io {
                    message: format!("cannot start writer thread: {e}"),
                }),
                None,
            );
        }
    };
    job.set_source(ProgressSource::Segmented {
        table: table.clone(),
        writer: writer.clone(),
    });
    job.set_state(DownloadState::Downloading);

    let mut limiters = vec![job.limiter.clone()];
    limiters.extend(extra_limiters);
    limiters.push(engine.global.clone());
    let requested = if ranged {
        engine.connections(req.connections)
    } else {
        1
    };
    let host = format!("{}:{}", ctx.target.host, ctx.target.port);
    let seg = Arc::new(SegCtx {
        job: job.clone(),
        table: table.clone(),
        writer: writer.clone(),
        limiters,
        stop: job.stop.child_token(),
        host,
        host_limits: engine.host_limits.clone(),
        connected: AtomicUsize::new(0),
        connected_notify: Default::default(),
        active_workers: AtomicUsize::new(0),
        target: AtomicUsize::new(requested),
        source_generation: Arc::new(AtomicU64::new(0)),
        min_request: cfg.min_request_size,
        max_request: cfg.max_request_size,
        read_timeout: cfg.read_timeout,
        max_retries,
        race: ranged,
        few_requests: Default::default(),
    });
    let factory = Arc::new(FtpFactory {
        ctx,
        first: Mutex::new(Some(probe.conn)),
    });
    let before = table.downloaded();
    let outcome = supervisor::supervise(
        seg,
        factory,
        None,
        &mut cmd_rx,
        None,
        Params {
            requested,
            initial: usize::from(cfg.initial_connections),
            cap: usize::from(cfg.max_connections_cap),
        },
    )
    .await;
    let transferred = table.downloaded().saturating_sub(before);
    end_job(
        &job,
        &writer,
        &paths,
        outcome,
        &req,
        transferred,
        run_started,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_parsing() {
        let t = FtpTarget::parse(
            &Url::parse("ftp://ftp.example.com/pub/a%20b.iso").unwrap(),
            None,
        )
        .unwrap();
        assert_eq!(
            (t.host.as_str(), t.port, t.path.as_str()),
            ("ftp.example.com", 21, "/pub/a b.iso")
        );
        assert_eq!(
            (t.user.as_str(), t.pass.as_str()),
            ("anonymous", "anonymous@")
        );
        assert!(!t.secure);
        let t =
            FtpTarget::parse(&Url::parse("ftps://u%40x:p%3Aw@h:2121/f").unwrap(), None).unwrap();
        assert_eq!(
            (t.user.as_str(), t.pass.as_str(), t.port),
            ("u@x", "p:w", 2121)
        );
        assert!(t.secure);
        let creds = Credentials {
            username: "c".into(),
            password: "d".into(),
            scheme: Default::default(),
        };
        let t = FtpTarget::parse(&Url::parse("ftp://h/f").unwrap(), Some(&creds)).unwrap();
        assert_eq!((t.user.as_str(), t.pass.as_str()), ("c", "d"));
        assert!(FtpTarget::parse(&Url::parse("ftp://h/dir/").unwrap(), None).is_err());
        assert!(FtpTarget::parse(&Url::parse("http://h/f").unwrap(), None).is_err());
        assert!(FtpTarget::parse(&Url::parse("ftp://h/a%0d%0aNOOP").unwrap(), None).is_err());
        assert!(FtpTarget::parse(&Url::parse("ftp://u%0d%0a:p@h/f").unwrap(), None).is_err());
        assert!(FtpTarget::parse(&Url::parse("ftp://u:p%0a@h/f").unwrap(), None).is_err());
        let bad_creds = Credentials {
            username: "u\r\n".into(),
            password: "p".into(),
            scheme: Default::default(),
        };
        assert!(FtpTarget::parse(&Url::parse("ftp://h/f").unwrap(), Some(&bad_creds)).is_err());
    }

    #[test]
    fn error_mapping() {
        let resp = |code: u32, body: &str| {
            FtpError::UnexpectedResponse(suppaftp::types::Response {
                status: suppaftp::Status::from(code),
                body: body.as_bytes().to_vec(),
            })
        };
        assert_eq!(
            map_ftp(resp(421, "Too many connections")).kind,
            FailKind::Refused
        );
        assert_eq!(
            map_ftp(resp(530, "Sorry, maximum number of clients")).kind,
            FailKind::Refused
        );
        let e = map_ftp(resp(530, "Login incorrect"));
        assert_eq!(
            (e.kind, e.error),
            (FailKind::Fatal, EngineError::AuthRequired)
        );
        assert_eq!(map_ftp(resp(550, "No such file")).kind, FailKind::Fatal);
        assert_eq!(
            map_ftp(resp(425, "Can't open data connection")).kind,
            FailKind::Transient
        );
    }
}
