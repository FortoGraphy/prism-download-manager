//! The HTTP/1.1 server: accept loop, connection accounting and routing.

use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use bytes::Bytes;
use futures_util::stream;
use http_body_util::{BodyExt, Empty, Full, StreamBody, combinators::BoxBody};
use hyper::body::{Frame, Incoming};
use hyper::header::{self, HeaderValue};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use md5::{Digest, Md5};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio::task::JoinSet;

use crate::content;
use crate::options::{AuthMode, FileOptions, LimitMode, ServerConfig};

type Body = BoxBody<Bytes, io::Error>;

/// Statistics of one accepted (non-refused) TCP connection.
#[derive(Debug, Clone)]
pub struct ConnectionStats {
    /// 0-based accept index (refused connections also consume an index).
    pub index: u64,
    /// Client socket address.
    pub peer: SocketAddr,
    /// Number of HTTP requests served on this connection.
    pub requests: u64,
    /// Response body bytes sent on this connection.
    pub bytes_sent: u64,
    /// Whether the connection is still open.
    pub open: bool,
}

/// One logged request.
#[derive(Debug, Clone)]
pub struct RequestLog {
    /// Accept index of the connection that carried the request.
    pub connection: u64,
    /// HTTP method.
    pub method: String,
    /// Path and query.
    pub uri: String,
    /// `Range` header.
    pub range: Option<String>,
    /// `If-Range` header.
    pub if_range: Option<String>,
    /// `User-Agent` header.
    pub user_agent: Option<String>,
    /// `Cookie` header.
    pub cookie: Option<String>,
    /// `Referer` header.
    pub referer: Option<String>,
    /// `Accept-Encoding` header.
    pub accept_encoding: Option<String>,
    /// Whether an `Authorization` header was present.
    pub authorization: bool,
}

/// Snapshot of server statistics.
#[derive(Debug, Clone, Default)]
pub struct ServerStats {
    /// Accepted connections, in accept order.
    pub connections: Vec<ConnectionStats>,
    /// Connections refused because of `max_connections`.
    pub refused: u64,
    /// Highest number of simultaneously open accepted connections.
    pub max_concurrent: usize,
    /// All requests, in arrival order.
    pub requests: Vec<RequestLog>,
}

impl ServerStats {
    /// Number of accepted connections.
    pub fn total_connections(&self) -> usize {
        self.connections.len()
    }
    /// Number of requests served.
    pub fn total_requests(&self) -> usize {
        self.requests.len()
    }
    /// Highest number of requests carried by a single connection (keep-alive reuse indicator).
    pub fn max_requests_per_connection(&self) -> u64 {
        self.connections
            .iter()
            .map(|c| c.requests)
            .max()
            .unwrap_or(0)
    }
    /// Requests whose path starts with `prefix`.
    pub fn requests_to(&self, prefix: &str) -> usize {
        self.requests
            .iter()
            .filter(|r| r.uri.starts_with(prefix))
            .count()
    }
}

struct ConnCounters {
    index: u64,
    peer: SocketAddr,
    requests: AtomicU64,
    bytes: AtomicU64,
    open: std::sync::atomic::AtomicBool,
}

pub(crate) struct State {
    pub(crate) config: ServerConfig,
    max_connections: AtomicUsize,
    active: AtomicUsize,
    max_concurrent: AtomicUsize,
    next_index: AtomicU64,
    refused: AtomicU64,
    connections: Mutex<Vec<Arc<ConnCounters>>>,
    requests: Mutex<Vec<RequestLog>>,
    versions: Mutex<HashMap<String, u64>>,
    nonces: Mutex<HashSet<String>>,
    expiring_first_seen: Mutex<HashMap<String, Instant>>,
    redirect_counter: AtomicU64,
    /// `/blob/<name>` files: their bytes and content type.
    blobs: Mutex<HashMap<String, (Bytes, String)>>,
}

impl State {
    pub(crate) fn new(config: ServerConfig) -> Self {
        Self {
            max_connections: AtomicUsize::new(config.max_connections.unwrap_or(usize::MAX)),
            config,
            active: AtomicUsize::new(0),
            max_concurrent: AtomicUsize::new(0),
            next_index: AtomicU64::new(0),
            refused: AtomicU64::new(0),
            connections: Mutex::new(Vec::new()),
            requests: Mutex::new(Vec::new()),
            versions: Mutex::new(HashMap::new()),
            nonces: Mutex::new(HashSet::new()),
            expiring_first_seen: Mutex::new(HashMap::new()),
            redirect_counter: AtomicU64::new(0),
            blobs: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn put_blob(&self, name: &str, data: Bytes, content_type: &str) {
        self.blobs
            .lock()
            .unwrap()
            .insert(name.to_string(), (data, content_type.to_string()));
    }

    pub(crate) fn stats(&self) -> ServerStats {
        ServerStats {
            connections: self
                .connections
                .lock()
                .unwrap()
                .iter()
                .map(|c| ConnectionStats {
                    index: c.index,
                    peer: c.peer,
                    requests: c.requests.load(Ordering::Relaxed),
                    bytes_sent: c.bytes.load(Ordering::Relaxed),
                    open: c.open.load(Ordering::Relaxed),
                })
                .collect(),
            refused: self.refused.load(Ordering::Relaxed),
            max_concurrent: self.max_concurrent.load(Ordering::Relaxed),
            requests: self.requests.lock().unwrap().clone(),
        }
    }

    pub(crate) fn reset_stats(&self) {
        self.connections
            .lock()
            .unwrap()
            .retain(|c| c.open.load(Ordering::Relaxed));
        for c in self.connections.lock().unwrap().iter() {
            c.requests.store(0, Ordering::Relaxed);
            c.bytes.store(0, Ordering::Relaxed);
        }
        self.requests.lock().unwrap().clear();
        self.refused.store(0, Ordering::Relaxed);
        self.max_concurrent
            .store(self.active.load(Ordering::Relaxed), Ordering::Relaxed);
    }

    pub(crate) fn set_version(&self, name: &str, version: u64) {
        self.versions
            .lock()
            .unwrap()
            .insert(name.to_string(), version);
    }

    pub(crate) fn set_max_connections(&self, max: Option<usize>) {
        self.max_connections
            .store(max.unwrap_or(usize::MAX), Ordering::Relaxed);
    }

    fn version(&self, name: &str) -> u64 {
        self.versions
            .lock()
            .unwrap()
            .get(name)
            .copied()
            .unwrap_or(0)
    }
}

/// Runs the accept loop until `shutdown` is notified.
pub(crate) async fn run(listener: TcpListener, state: Arc<State>, shutdown: Arc<Notify>) {
    let mut tasks = JoinSet::new();
    loop {
        let accepted = tokio::select! {
            _ = shutdown.notified() => break,
            r = listener.accept() => r,
            // Reap finished connection tasks so the set does not grow without bound.
            Some(_) = tasks.join_next(), if !tasks.is_empty() => continue,
        };
        let Ok((stream, peer)) = accepted else {
            continue;
        };
        let index = state.next_index.fetch_add(1, Ordering::Relaxed);
        let active = state.active.fetch_add(1, Ordering::SeqCst) + 1;
        if active > state.max_connections.load(Ordering::Relaxed) {
            state.active.fetch_sub(1, Ordering::SeqCst);
            state.refused.fetch_add(1, Ordering::Relaxed);
            tasks.spawn(refuse(stream, state.config.limit_mode));
            continue;
        }
        state.max_concurrent.fetch_max(active, Ordering::Relaxed);
        let _ = stream.set_nodelay(true);
        let counters = Arc::new(ConnCounters {
            index,
            peer,
            requests: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            open: std::sync::atomic::AtomicBool::new(true),
        });
        state.connections.lock().unwrap().push(counters.clone());
        let st = state.clone();
        tasks.spawn(async move {
            let svc_state = st.clone();
            let svc_counters = counters.clone();
            let svc = service_fn(move |req| {
                let st = svc_state.clone();
                let c = svc_counters.clone();
                async move { Ok::<_, Infallible>(handle(st, c, req).await) }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .keep_alive(true)
                .serve_connection(TokioIo::new(stream), svc)
                .await;
            counters.open.store(false, Ordering::Relaxed);
            st.active.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

async fn refuse(stream: TcpStream, mode: LimitMode) {
    let status = match mode {
        LimitMode::Reset => {
            // SO_LINGER 0 turns the close into a TCP RST.
            let _ = stream.set_zero_linger();
            drop(stream);
            return;
        }
        LimitMode::Status503 => StatusCode::SERVICE_UNAVAILABLE,
        LimitMode::Status429 => StatusCode::TOO_MANY_REQUESTS,
    };
    let svc = service_fn(move |_req: Request<Incoming>| async move {
        let mut r = Response::new(full(Bytes::from_static(b"too many connections")));
        *r.status_mut() = status;
        r.headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
        r.headers_mut()
            .insert(header::CONNECTION, HeaderValue::from_static("close"));
        Ok::<_, Infallible>(r)
    });
    let _ = hyper::server::conn::http1::Builder::new()
        .keep_alive(false)
        .serve_connection(TokioIo::new(stream), svc)
        .await;
}

fn full(b: Bytes) -> Body {
    Full::new(b).map_err(|never| match never {}).boxed()
}

fn empty() -> Body {
    Empty::new().map_err(|never| match never {}).boxed()
}

fn status(code: StatusCode, msg: &str) -> Response<Body> {
    let mut r = Response::new(full(Bytes::from(msg.to_string())));
    *r.status_mut() = code;
    r
}

/// What Cloudflare answers clients that haven't passed its bot check (a browser has): `403` with
/// `cf-mitigated: challenge` and a "Just a moment..." page.
fn challenge() -> Response<Body> {
    let page =
        "<!DOCTYPE html><html><head><title>Just a moment...</title></head><body></body></html>";
    let mut r = Response::new(full(Bytes::from_static(page.as_bytes())));
    *r.status_mut() = StatusCode::FORBIDDEN;
    let h = r.headers_mut();
    h.insert(
        "cf-mitigated",
        header::HeaderValue::from_static("challenge"),
    );
    h.insert(
        header::SERVER,
        header::HeaderValue::from_static("cloudflare"),
    );
    h.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/html; charset=UTF-8"),
    );
    r
}

fn header_str(req: &Request<Incoming>, name: header::HeaderName) -> Option<String> {
    req.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

async fn handle(
    state: Arc<State>,
    conn: Arc<ConnCounters>,
    req: Request<Incoming>,
) -> Response<Body> {
    conn.requests.fetch_add(1, Ordering::Relaxed);
    let uri = req
        .uri()
        .path_and_query()
        .map(|p| p.to_string())
        .unwrap_or_else(|| "/".into());
    state.requests.lock().unwrap().push(RequestLog {
        connection: conn.index,
        method: req.method().to_string(),
        uri: uri.clone(),
        range: header_str(&req, header::RANGE),
        if_range: header_str(&req, header::IF_RANGE),
        user_agent: header_str(&req, header::USER_AGENT),
        cookie: header_str(&req, header::COOKIE),
        referer: header_str(&req, header::REFERER),
        accept_encoding: header_str(&req, header::ACCEPT_ENCODING),
        authorization: req.headers().contains_key(header::AUTHORIZATION),
    });
    let path = req.uri().path().to_string();
    let query: HashMap<String, String> = req
        .uri()
        .query()
        .map(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .into_owned()
                .collect()
        })
        .unwrap_or_default();
    let parts: Vec<&str> = path.trim_start_matches('/').splitn(2, '/').collect();
    let (head, rest) = (parts[0], parts.get(1).copied().unwrap_or(""));
    match head {
        "file" | "" => file_route(&state, &conn, &req, rest, &query, &uri).await,
        "expiring" => {
            let mut opts = state.config.file_defaults.clone();
            if let Err(e) = opts.apply_query(&query) {
                return status(StatusCode::BAD_REQUEST, &e);
            }
            let ttl = opts.ttl.unwrap_or(Duration::from_secs(2));
            let first = *state
                .expiring_first_seen
                .lock()
                .unwrap()
                .entry(path.clone())
                .or_insert_with(Instant::now);
            if first.elapsed() > ttl {
                return status(StatusCode::GONE, "link expired");
            }
            file_route(&state, &conn, &req, rest, &query, &uri).await
        }
        "redirect" => redirect_route(&state, rest, &query),
        "blob" => blob_route(&state, &req, rest, &query, &uri),
        "status" => {
            let code = rest
                .parse::<u16>()
                .ok()
                .and_then(|c| StatusCode::from_u16(c).ok());
            match code {
                Some(c) => status(c, c.canonical_reason().unwrap_or("status")),
                None => status(StatusCode::BAD_REQUEST, "bad status code"),
            }
        }
        "challenge" => challenge(),
        "hls" => hls_route(&state, &req, rest),
        "dash" => dash_route(&state, &req, rest),
        _ => status(StatusCode::NOT_FOUND, "not found"),
    }
}

fn redirect_route(state: &State, rest: &str, query: &HashMap<String, String>) -> Response<Body> {
    let hops: u32 = rest.trim_end_matches('/').parse().unwrap_or(0);
    let code = query
        .get("code")
        .and_then(|c| c.parse::<u16>().ok())
        .and_then(|c| StatusCode::from_u16(c).ok())
        .filter(StatusCode::is_redirection)
        .unwrap_or(StatusCode::FOUND);
    let Some(to) = query.get("to") else {
        return status(StatusCode::BAD_REQUEST, "missing to=");
    };
    let location = if hops > 1 {
        let enc: String = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("code", &code.as_u16().to_string())
            .append_pair("to", to)
            .finish();
        format!("/redirect/{}?{}", hops - 1, enc)
    } else {
        let n = state.redirect_counter.fetch_add(1, Ordering::Relaxed);
        to.replace("{n}", &n.to_string())
    };
    let mut r = Response::new(empty());
    *r.status_mut() = code;
    r.headers_mut().insert(
        header::LOCATION,
        HeaderValue::from_str(&location).unwrap_or(HeaderValue::from_static("/")),
    );
    r
}

/// An in-memory file added with [`crate::TestServer::put_blob`]: single ranges (unless
/// `ranges=0`), a strong ETag of its content, `auth=basic|digest`.
fn blob_route(
    state: &State,
    req: &Request<Incoming>,
    name: &str,
    query: &HashMap<String, String>,
    uri: &str,
) -> Response<Body> {
    let Some((data, content_type)) = state.blobs.lock().unwrap().get(name).cloned() else {
        return status(StatusCode::NOT_FOUND, "no such blob");
    };
    let mut opts = state.config.file_defaults.clone();
    if let Err(e) = opts.apply_query(query) {
        return status(StatusCode::BAD_REQUEST, &e);
    }
    if let Some(r) = check_auth(state, req, opts.auth, uri) {
        return r;
    }
    let size = data.len() as u64;
    let range = req
        .headers()
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .filter(|_| opts.ranges)
        .map(|h| parse_range(h, size));
    let (code, start, end) = match range {
        Some(Err(())) => {
            let mut r = status(StatusCode::RANGE_NOT_SATISFIABLE, "range not satisfiable");
            r.headers_mut().insert(
                header::CONTENT_RANGE,
                HeaderValue::from_str(&format!("bytes */{size}")).expect("ascii"),
            );
            return r;
        }
        Some(Ok(Some((a, b)))) => (StatusCode::PARTIAL_CONTENT, a, b + 1),
        _ => (StatusCode::OK, 0, size),
    };
    let body = if req.method() == Method::HEAD {
        empty()
    } else {
        full(data.slice(start as usize..end as usize))
    };
    let mut r = Response::new(body);
    *r.status_mut() = code;
    let h = r.headers_mut();
    if code == StatusCode::PARTIAL_CONTENT {
        h.insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {start}-{}/{size}", end - 1)).expect("ascii"),
        );
    }
    if opts.ranges {
        h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    }
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(end - start));
    let etag = format!("\"{}\"", &content::sha256_hex(&data)[..16]);
    h.insert(header::ETAG, HeaderValue::from_str(&etag).expect("ascii"));
    if let Ok(v) = HeaderValue::from_str(&content_type) {
        h.insert(header::CONTENT_TYPE, v);
    }
    r
}

/// Result of evaluating the authorization header.
fn check_auth(
    state: &State,
    req: &Request<Incoming>,
    mode: AuthMode,
    uri: &str,
) -> Option<Response<Body>> {
    let cfg = &state.config;
    let given = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    let ok = match (mode, given) {
        (AuthMode::None, _) => true,
        (AuthMode::Basic, Some(h)) => h
            .strip_prefix("Basic ")
            .and_then(|b| {
                base64::engine::general_purpose::STANDARD
                    .decode(b.trim())
                    .ok()
            })
            .is_some_and(|d| d == format!("{}:{}", cfg.username, cfg.password).as_bytes()),
        (AuthMode::Digest, Some(h)) => h.strip_prefix("Digest ").is_some_and(|p| {
            verify_digest(state, &parse_auth_params(p), req.method().as_str(), uri)
        }),
        (_, None) => false,
    };
    if ok {
        return None;
    }
    let challenge = match mode {
        AuthMode::Basic => "Basic realm=\"prism\"".to_string(),
        _ => {
            let nonce = content::hex(&fastrand::u128(..).to_be_bytes());
            state.nonces.lock().unwrap().insert(nonce.clone());
            format!(
                "Digest realm=\"prism\", qop=\"auth\", algorithm=MD5, nonce=\"{nonce}\", opaque=\"0123abcd\""
            )
        }
    };
    let mut r = status(StatusCode::UNAUTHORIZED, "authentication required");
    r.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_str(&challenge).expect("ascii"),
    );
    Some(r)
}

/// Parses `k=v, k="v"` lists (quoted values may contain commas).
fn parse_auth_params(s: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut chars = s.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| *c == ',' || c.is_whitespace()) {
            chars.next();
        }
        let key: String = chars.by_ref().take_while(|c| *c != '=').collect();
        if key.is_empty() {
            break;
        }
        let mut val = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => val.extend(chars.next()),
                    '"' => break,
                    c => val.push(c),
                }
            }
        } else {
            while let Some(c) = chars.peek() {
                if *c == ',' {
                    break;
                }
                val.push(*c);
                chars.next();
            }
        }
        out.insert(key.trim().to_ascii_lowercase(), val.trim().to_string());
    }
    out
}

fn md5_hex(s: &str) -> String {
    content::hex(&Md5::digest(s.as_bytes()))
}

fn verify_digest(state: &State, p: &HashMap<String, String>, method: &str, uri: &str) -> bool {
    let cfg = &state.config;
    let get = |k: &str| p.get(k).map(String::as_str).unwrap_or("");
    if get("username") != cfg.username || get("uri") != uri || get("qop") != "auth" {
        return false;
    }
    if !state.nonces.lock().unwrap().contains(get("nonce")) {
        return false;
    }
    let ha1 = md5_hex(&format!("{}:prism:{}", cfg.username, cfg.password));
    let ha2 = md5_hex(&format!("{method}:{uri}"));
    let expected = md5_hex(&format!(
        "{ha1}:{}:{}:{}:auth:{ha2}",
        get("nonce"),
        get("nc"),
        get("cnonce")
    ));
    expected == get("response")
}

/// Formats a Unix timestamp as an IMF-fixdate (`Sun, 06 Nov 1994 08:49:37 GMT`).
pub(crate) fn http_date(unix: u64) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = unix / 86_400;
    let secs = unix % 86_400;
    // Civil-from-days (Howard Hinnant).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        DAYS[(days % 7) as usize],
        d,
        MONTHS[(m - 1) as usize],
        y,
        secs / 3600,
        (secs / 60) % 60,
        secs % 60
    )
}

/// Parses a single `bytes=` range against `size`. `Ok(None)` = no/ignored range,
/// `Err(())` = unsatisfiable.
fn parse_range(h: &str, size: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some(spec) = h.trim().strip_prefix("bytes=") else {
        return Ok(None);
    };
    if spec.contains(',') {
        return Ok(None);
    }
    let (a, b) = spec.split_once('-').ok_or(())?;
    let (a, b) = (a.trim(), b.trim());
    if a.is_empty() {
        let n: u64 = b.parse().map_err(|_| ())?;
        if n == 0 || size == 0 {
            return Err(());
        }
        return Ok(Some((size.saturating_sub(n), size - 1)));
    }
    let start: u64 = a.parse().map_err(|_| ())?;
    if start >= size {
        return Err(());
    }
    let end = if b.is_empty() {
        size - 1
    } else {
        b.parse::<u64>().map_err(|_| ())?.min(size - 1)
    };
    if end < start {
        return Err(());
    }
    Ok(Some((start, end)))
}

async fn file_route(
    state: &State,
    conn: &Arc<ConnCounters>,
    req: &Request<Incoming>,
    name: &str,
    query: &HashMap<String, String>,
    uri: &str,
) -> Response<Body> {
    if req.method() != Method::GET && req.method() != Method::HEAD && req.method() != Method::POST {
        return status(StatusCode::METHOD_NOT_ALLOWED, "method not allowed");
    }
    let mut opts: FileOptions = state.config.file_defaults.clone();
    if let Err(e) = opts.apply_query(query) {
        return status(StatusCode::BAD_REQUEST, &e);
    }
    if let Some(r) = check_auth(state, req, opts.auth, uri) {
        return r;
    }
    if !opts.ttfb.is_zero() {
        tokio::time::sleep(opts.ttfb).await;
    }
    let version = state.version(name);
    let seed = content::versioned_seed(opts.seed, version);
    let size = opts.size;
    let etag = format!(
        "{}\"{:x}-{}-{}\"",
        if opts.weak_etag { "W/" } else { "" },
        opts.seed,
        version,
        size
    );
    let last_modified = http_date(1_700_000_000 + version * 3600);

    let mut range = None;
    if opts.ranges
        && let Some(h) = req
            .headers()
            .get(header::RANGE)
            .and_then(|v| v.to_str().ok())
    {
        let if_range_ok = match req
            .headers()
            .get(header::IF_RANGE)
            .and_then(|v| v.to_str().ok())
        {
            None => true,
            Some(v) if v.starts_with('"') => opts.etag && !opts.weak_etag && v == etag,
            Some(v) if v.starts_with("W/") => false,
            Some(v) => opts.last_modified && v == last_modified,
        };
        if if_range_ok {
            match parse_range(h, size) {
                Ok(r) => range = r,
                Err(()) => {
                    let mut r = status(StatusCode::RANGE_NOT_SATISFIABLE, "range not satisfiable");
                    r.headers_mut().insert(
                        header::CONTENT_RANGE,
                        HeaderValue::from_str(&format!("bytes */{size}")).expect("ascii"),
                    );
                    return r;
                }
            }
        }
    }

    let (start, end_incl) = range.unwrap_or((0, size.saturating_sub(1)));
    let len = if size == 0 { 0 } else { end_incl - start + 1 };
    let rate = match opts.slow_conn {
        Some(i) if i == conn.index => opts.slow_rate,
        _ => opts.throttle,
    };
    let body = if req.method() == Method::HEAD || len == 0 {
        empty()
    } else {
        content_body(
            seed,
            start,
            start + len,
            rate,
            opts.drop_after,
            opts.drop_prob,
            conn.clone(),
        )
    };
    let mut r = Response::new(body);
    if range.is_some() {
        *r.status_mut() = StatusCode::PARTIAL_CONTENT;
        r.headers_mut().insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {start}-{end_incl}/{size}")).expect("ascii"),
        );
    }
    let h2 = r.headers_mut();
    if opts.ranges {
        h2.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    }
    if !opts.chunked {
        h2.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    }
    if opts.etag {
        h2.insert(header::ETAG, HeaderValue::from_str(&etag).expect("ascii"));
    }
    if opts.last_modified {
        h2.insert(
            header::LAST_MODIFIED,
            HeaderValue::from_str(&last_modified).expect("ascii"),
        );
    }
    if let Ok(v) = HeaderValue::from_str(&opts.content_type) {
        h2.insert(header::CONTENT_TYPE, v);
    }
    // Raw bytes: a file name in UTF-8 goes out unencoded, as some servers send it (#91).
    if let Some(cd) = &opts.content_disposition
        && let Ok(v) = HeaderValue::from_bytes(cd.as_bytes())
    {
        h2.insert(header::CONTENT_DISPOSITION, v);
    }
    r
}

struct BodyGen {
    seed: u64,
    pos: u64,
    end: u64,
    rate: u64,
    started: Instant,
    sent: u64,
    drop_after: Option<u64>,
    drop_prob: f64,
    conn: Arc<ConnCounters>,
}

/// Streams generated content `[start, end)` with optional throttling and fault injection.
fn content_body(
    seed: u64,
    start: u64,
    end: u64,
    rate: u64,
    drop_after: Option<u64>,
    drop_prob: f64,
    conn: Arc<ConnCounters>,
) -> Body {
    let gen_state = BodyGen {
        seed,
        pos: start,
        end,
        rate,
        started: Instant::now(),
        sent: 0,
        drop_after,
        drop_prob,
        conn,
    };
    let s = stream::unfold(gen_state, |mut g| async move {
        if g.pos >= g.end {
            return None;
        }
        let dropped = || io::Error::new(io::ErrorKind::ConnectionReset, "injected connection drop");
        if g.drop_after.is_some_and(|d| g.sent >= d)
            || (g.drop_prob > 0.0 && fastrand::f64() < g.drop_prob)
        {
            g.pos = g.end;
            return Some((Err(dropped()), g));
        }
        let mut block: u64 = if g.rate > 0 {
            (g.rate / 16).clamp(1024, 16 * 1024)
        } else {
            64 * 1024
        };
        block = block.min(g.end - g.pos);
        if let Some(d) = g.drop_after {
            block = block.min(d - g.sent);
        }
        if g.rate > 0 {
            let due = g.started + Duration::from_secs_f64(g.sent as f64 / g.rate as f64);
            tokio::time::sleep_until(due.into()).await;
        }
        let mut buf = vec![0u8; block as usize];
        content::fill(g.seed, g.pos, &mut buf);
        g.pos += block;
        g.sent += block;
        g.conn.bytes.fetch_add(block, Ordering::Relaxed);
        Some((Ok(Frame::data(Bytes::from(buf))), g))
    });
    BodyExt::boxed(StreamBody::new(s))
}

/// Serves an in-memory resource with single-range support.
fn serve_bytes(req: &Request<Incoming>, data: Bytes, content_type: &'static str) -> Response<Body> {
    let size = data.len() as u64;
    let range = req
        .headers()
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(|h| parse_range(h, size));
    let (code, start, end) = match range {
        Some(Err(())) => return status(StatusCode::RANGE_NOT_SATISFIABLE, "range not satisfiable"),
        Some(Ok(Some((a, b)))) => (StatusCode::PARTIAL_CONTENT, a, b + 1),
        _ => (StatusCode::OK, 0, size),
    };
    let mut r = Response::new(full(data.slice(start as usize..end as usize)));
    *r.status_mut() = code;
    if code == StatusCode::PARTIAL_CONTENT {
        r.headers_mut().insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {start}-{}/{size}", end - 1)).expect("ascii"),
        );
    }
    r.headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    r.headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    r
}

fn hls_route(state: &State, req: &Request<Incoming>, rest: &str) -> Response<Body> {
    let Some(hls) = &state.config.hls else {
        return status(StatusCode::NOT_FOUND, "no HLS fixture");
    };
    const PLAYLIST: &str = "application/vnd.apple.mpegurl";
    if rest == "master.m3u8" {
        return serve_bytes(req, Bytes::from(hls.master_playlist()), PLAYLIST);
    }
    if rest == "key.bin" {
        return match hls.key {
            Some(k) => serve_bytes(req, Bytes::copy_from_slice(&k), "application/octet-stream"),
            None => status(StatusCode::NOT_FOUND, "no key"),
        };
    }
    let Some((dir, file)) = rest.split_once('/') else {
        return status(StatusCode::NOT_FOUND, "not found");
    };
    let (segments, init, ext): (&[Bytes], Option<&Bytes>, &str) = if dir == "audio" {
        match &hls.audio {
            Some(a) => (a, None, "aac"),
            None => return status(StatusCode::NOT_FOUND, "no audio"),
        }
    } else {
        match dir
            .strip_prefix('v')
            .and_then(|i| i.parse::<usize>().ok())
            .and_then(|i| hls.variants.get(i))
        {
            Some(v) => (&v.segments, v.init.as_ref(), "ts"),
            None => return status(StatusCode::NOT_FOUND, "no such variant"),
        }
    };
    if file == "media.m3u8" {
        return serve_bytes(
            req,
            Bytes::from(hls.media_playlist(segments, init.is_some(), ext)),
            PLAYLIST,
        );
    }
    if file == "init.mp4" {
        return match init {
            Some(b) => serve_bytes(req, hls.served_init(b), "video/mp4"),
            None => status(StatusCode::NOT_FOUND, "no init"),
        };
    }
    if file == format!("all.{ext}") {
        return serve_bytes(req, hls.served_all(segments), "video/mp2t");
    }
    let n = file
        .strip_prefix("seg")
        .and_then(|f| f.strip_suffix(&format!(".{ext}")))
        .and_then(|n| n.parse::<usize>().ok());
    match n.and_then(|n| segments.get(n).map(|s| (n, s))) {
        Some((n, s)) => serve_bytes(req, hls.served_segment(s, n), "video/mp2t"),
        None => status(StatusCode::NOT_FOUND, "no such segment"),
    }
}

fn dash_route(state: &State, req: &Request<Incoming>, rest: &str) -> Response<Body> {
    let Some(dash) = &state.config.dash else {
        return status(StatusCode::NOT_FOUND, "no DASH fixture");
    };
    if rest == "manifest.mpd" {
        return serve_bytes(req, Bytes::from(dash.manifest()), "application/dash+xml");
    }
    let Some((id, file)) = rest.split_once('/') else {
        return status(StatusCode::NOT_FOUND, "not found");
    };
    let Some(rep) = dash.find(id) else {
        return status(StatusCode::NOT_FOUND, "no such representation");
    };
    if file == "init.mp4" {
        return serve_bytes(req, rep.init.clone(), "video/mp4");
    }
    let n = file
        .strip_prefix("seg-")
        .and_then(|f| f.strip_suffix(".m4s"))
        .and_then(|n| n.parse::<usize>().ok());
    match n.filter(|n| *n >= 1).and_then(|n| rep.segments.get(n - 1)) {
        Some(s) => serve_bytes(req, s.clone(), "video/iso.segment"),
        None => status(StatusCode::NOT_FOUND, "no such segment"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_format() {
        assert_eq!(http_date(784_111_777), "Sun, 06 Nov 1994 08:49:37 GMT");
        assert_eq!(http_date(0), "Thu, 01 Jan 1970 00:00:00 GMT");
    }

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-", 10), Ok(Some((0, 9))));
        assert_eq!(parse_range("bytes=2-4", 10), Ok(Some((2, 4))));
        assert_eq!(parse_range("bytes=5-100", 10), Ok(Some((5, 9))));
        assert_eq!(parse_range("bytes=-3", 10), Ok(Some((7, 9))));
        assert_eq!(parse_range("bytes=10-", 10), Err(()));
        assert_eq!(parse_range("items=1-2", 10), Ok(None));
    }

    #[test]
    fn auth_params() {
        let p = parse_auth_params(r#"username="u", uri="/a?b=1,2", qop=auth, nc=00000001"#);
        assert_eq!(p["username"], "u");
        assert_eq!(p["uri"], "/a?b=1,2");
        assert_eq!(p["qop"], "auth");
        assert_eq!(p["nc"], "00000001");
    }
}
