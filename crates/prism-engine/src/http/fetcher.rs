//! The per-worker HTTP fetcher: one reqwest client (one connection) per worker, range
//! requests, response validation, auth handshakes, mirror/redirect fallback.

use std::sync::Arc;

use bytes::Bytes;
use reqwest::StatusCode;
use reqwest::header::{self, HeaderValue};
use url::Url;

use super::auth::ChallengeAction;
use super::client::build_client;
use super::{
    HttpCtx, SourceRef, Validators, header_string, parse_content_range, parse_unsatisfied_range,
    retry_after, strong_etag,
};
use crate::error::EngineError;
use crate::filename;
use crate::progress::ProbeInfo;
use crate::request::HttpMethod;
use crate::segmented::table::WorkerId;
use crate::segmented::{ChunkStream, FailKind, FetchError, FetcherFactory, RangeFetcher};

/// A response body being streamed.
pub(crate) struct HttpStream {
    resp: reqwest::Response,
    end: Option<u64>,
}

impl HttpStream {
    pub(crate) fn new(resp: reqwest::Response, end: Option<u64>) -> Self {
        Self { resp, end }
    }
}

impl ChunkStream for HttpStream {
    async fn next_chunk(&mut self) -> Result<Option<Bytes>, FetchError> {
        self.resp.chunk().await.map_err(|e| {
            if e.is_timeout() {
                FetchError::transient(EngineError::Timeout)
            } else {
                FetchError::transient(EngineError::network(error_chain(&e)))
            }
        })
    }

    fn range_end(&self) -> Option<u64> {
        self.end
    }
}

/// How the first response looked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProbeMode {
    /// `206` with a total size: multi-connection and resumable.
    Ranged { total: u64 },
    /// `200`: single connection, restart from zero on failure.
    Single { size: Option<u64> },
    /// Zero-length resource.
    Empty,
}

/// Result of the first request.
pub(crate) struct Probe {
    pub info: ProbeInfo,
    pub mode: ProbeMode,
    pub stream: Option<HttpStream>,
}

/// One worker's HTTP connection.
pub(crate) struct HttpFetcher {
    ctx: Arc<HttpCtx>,
    worker: WorkerId,
    client: Result<reqwest::Client, EngineError>,
    /// The NTLM handshake succeeded on the current connection.
    ntlm_ready: bool,
}

/// Creates fetchers for workers.
pub(crate) struct HttpFactory {
    pub ctx: Arc<HttpCtx>,
}

impl FetcherFactory for HttpFactory {
    type Fetcher = HttpFetcher;
    fn create(&self, worker: WorkerId) -> HttpFetcher {
        HttpFetcher::new(self.ctx.clone(), worker)
    }
}

/// Formats an error with its source chain (reqwest hides the interesting part in sources).
pub(crate) fn error_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        let cs = c.to_string();
        if !s.contains(&cs) {
            s.push_str(": ");
            s.push_str(&cs);
        }
        cur = c.source();
    }
    s
}

fn map_send_error(e: reqwest::Error) -> FetchError {
    if e.is_timeout() {
        FetchError::transient(EngineError::Timeout)
    } else if e.is_redirect() {
        FetchError::fatal(EngineError::network(format!(
            "too many redirects: {}",
            error_chain(&e)
        )))
    } else if e.is_builder() {
        FetchError::fatal(EngineError::InvalidUrl {
            message: error_chain(&e),
        })
    } else {
        // Refused/reset while connecting or before the response: possibly a connection limit,
        // possibly a dropped connection (the worker decides).
        FetchError::maybe_refused(EngineError::network(error_chain(&e)))
    }
}

fn status_error(status: u16) -> EngineError {
    match status {
        401 | 407 => EngineError::AuthRequired,
        s => EngineError::Http { status: s },
    }
}

impl HttpFetcher {
    pub(crate) fn new(ctx: Arc<HttpCtx>, worker: WorkerId) -> Self {
        let client = build_client(&ctx.spec);
        Self {
            ctx,
            worker,
            client,
            ntlm_ready: false,
        }
    }

    fn request(
        &self,
        client: &reqwest::Client,
        url: &Url,
        range: Option<(u64, Option<u64>)>,
        if_range: Option<&str>,
        authorization: Option<String>,
    ) -> reqwest::RequestBuilder {
        let method = match self.ctx.method {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
        };
        let own_site = self.ctx.sources.is_own_site(url);
        let mut headers = self.ctx.headers.clone();
        if !own_site {
            // A redirect target on another server (a CDN's signed link) doesn't get the site's
            // credentials, the same as the redirect itself (reqwest drops them across sites).
            headers.remove(header::AUTHORIZATION);
            headers.remove(header::COOKIE);
        }
        let mut rb = client.request(method, url.clone()).headers(headers);
        if let Some((from, to)) = range {
            let v = match to {
                Some(to) if to > from => format!("bytes={from}-{}", to - 1),
                _ => format!("bytes={from}-"),
            };
            rb = rb.header(header::RANGE, v);
        }
        if let Some(ir) = if_range
            && let Ok(v) = HeaderValue::from_str(ir)
        {
            rb = rb.header(header::IF_RANGE, v);
        }
        if own_site && let Some(a) = authorization {
            rb = rb.header(header::AUTHORIZATION, a);
        }
        if let Some(b) = &self.ctx.body {
            rb = rb.body(b.clone());
        }
        rb
    }

    /// Sends one request, transparently answering Basic/Digest challenges and running the
    /// NTLM handshake. A remaining `401` is returned to the caller.
    ///
    /// With a byte window ([`crate::DownloadRequest::window`]) the window is the file: the range
    /// asked for counts from its first byte, and the `Content-Range` of the answer is turned into
    /// the same terms (the window's length as the size), so the rest of the engine sees a file
    /// of that length.
    pub(crate) async fn send(
        &mut self,
        url: &Url,
        range: Option<(u64, Option<u64>)>,
        if_range: Option<&str>,
    ) -> Result<reqwest::Response, FetchError> {
        let Some((w0, w1)) = self.ctx.window else {
            return self.send_raw(url, range, if_range).await;
        };
        let (from, to) = range.unwrap_or((0, None));
        let a = w0.saturating_add(from);
        let b = to.map_or(w1, |t| w0.saturating_add(t).min(w1));
        if a >= b {
            return Err(FetchError::fatal(EngineError::network(
                "a range outside the part of the file",
            )));
        }
        let mut resp = self.send_raw(url, Some((a, Some(b))), if_range).await?;
        match resp.status().as_u16() {
            // The whole file instead of the bytes asked for.
            200 if if_range.is_some() => Err(FetchError::fatal(EngineError::ValidatorMismatch)),
            200 => Err(FetchError::fatal(EngineError::RangeNotSupported)),
            206 => {
                let cr = header_string(resp.headers(), header::CONTENT_RANGE)
                    .and_then(|v| parse_content_range(&v));
                match cr {
                    Some((x, y, _)) if x >= w0 && y < w1 => {
                        let v = format!("bytes {}-{}/{}", x - w0, y - w0, w1 - w0);
                        if let Ok(v) = HeaderValue::from_str(&v) {
                            resp.headers_mut().insert(header::CONTENT_RANGE, v);
                        }
                        Ok(resp)
                    }
                    _ => Err(FetchError::transient(EngineError::network(
                        "the server sent bytes outside the part of the file",
                    ))),
                }
            }
            _ => Ok(resp),
        }
    }

    async fn send_raw(
        &mut self,
        url: &Url,
        range: Option<(u64, Option<u64>)>,
        if_range: Option<&str>,
    ) -> Result<reqwest::Response, FetchError> {
        let client = self
            .client
            .as_ref()
            .map_err(|e| FetchError::fatal(e.clone()))?
            .clone();
        let method = match self.ctx.method {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
        };
        let own_site = self.ctx.sources.is_own_site(url);
        let mut rounds = 0;
        loop {
            let resp = if own_site && self.ctx.auth.uses_ntlm() && !self.ntlm_ready {
                self.ntlm_handshake(&client, url, range, if_range).await?
            } else {
                let auth = if own_site {
                    self.ctx.auth.authorization(method, url)
                } else {
                    None
                };
                self.request(&client, url, range, if_range, auth)
                    .send()
                    .await
                    .map_err(map_send_error)?
            };
            if own_site
                && resp.status() == StatusCode::UNAUTHORIZED
                && rounds < 2
                && self.ctx.auth.has_credentials()
            {
                let challenges: Vec<String> = resp
                    .headers()
                    .get_all(header::WWW_AUTHENTICATE)
                    .iter()
                    .filter_map(|v| v.to_str().ok().map(str::to_string))
                    .collect();
                match self.ctx.auth.on_challenge(&challenges) {
                    ChallengeAction::Retry | ChallengeAction::Ntlm => {
                        rounds += 1;
                        self.ntlm_ready = false;
                        continue;
                    }
                    ChallengeAction::GiveUp => return Ok(resp),
                }
            }
            if self.ctx.auth.uses_ntlm() {
                self.ntlm_ready = resp.status() != StatusCode::UNAUTHORIZED;
            }
            return Ok(resp);
        }
    }

    /// NTLM: negotiate → challenge → authenticate, all on this worker's single connection.
    async fn ntlm_handshake(
        &self,
        client: &reqwest::Client,
        url: &Url,
        range: Option<(u64, Option<u64>)>,
        if_range: Option<&str>,
    ) -> Result<reqwest::Response, FetchError> {
        let negotiate = self
            .ctx
            .auth
            .ntlm_negotiate()
            .ok_or_else(|| FetchError::fatal(EngineError::AuthRequired))?;
        let r1 = self
            .request(client, url, range, if_range, Some(negotiate))
            .send()
            .await
            .map_err(map_send_error)?;
        if r1.status() != StatusCode::UNAUTHORIZED {
            return Ok(r1);
        }
        let challenge = r1
            .headers()
            .get_all(header::WWW_AUTHENTICATE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|v| v.trim_start().len() > 5 && v.trim_start()[..5].eq_ignore_ascii_case("ntlm "))
            .map(str::to_string)
            .ok_or_else(|| FetchError::fatal(EngineError::AuthRequired))?;
        // Drain the body so the same connection carries the third leg.
        let _ = r1.bytes().await;
        let authenticate = self
            .ctx
            .auth
            .ntlm_authenticate(&challenge)
            .ok_or_else(|| FetchError::fatal(EngineError::AuthRequired))?;
        self.request(client, url, range, if_range, Some(authenticate))
            .send()
            .await
            .map_err(map_send_error)
    }

    /// First request of a fresh download: `GET Range: bytes=0-<first_chunk-1>` (or no range
    /// for POST). The body becomes the first worker's stream.
    pub(crate) async fn probe(
        &mut self,
        first_chunk: u64,
        use_ranges: bool,
    ) -> Result<Probe, FetchError> {
        let url = self.ctx.sources.original();
        let mut use_ranges = use_ranges;
        loop {
            let range = use_ranges.then_some((0, Some(first_chunk)));
            let resp = self.send(&url, range, None).await?;
            self.ctx
                .sources
                .on_redirected(SourceRef::Primary, resp.url());
            let status = resp.status().as_u16();
            let headers = resp.headers().clone();
            if is_browser_check(&headers) {
                return Err(FetchError::fatal(EngineError::BrowserCheck));
            }
            match status {
                206 => {
                    let cr = header_string(&headers, header::CONTENT_RANGE)
                        .and_then(|v| parse_content_range(&v));
                    match cr {
                        Some((0, b, Some(total))) => {
                            if self.ctx.expect_file && is_web_page(&headers, Some(total)) {
                                return Err(FetchError::fatal(EngineError::WebPage));
                            }
                            self.ctx.mark_probed();
                            let info = probe_info(&resp, Some(total), true);
                            let mode = if total == 0 {
                                ProbeMode::Empty
                            } else {
                                ProbeMode::Ranged { total }
                            };
                            return Ok(Probe {
                                info,
                                mode,
                                stream: Some(HttpStream {
                                    resp,
                                    end: Some(b + 1),
                                }),
                            });
                        }
                        // Unknown total or odd range: fall back to a plain request.
                        _ => use_ranges = false,
                    }
                }
                200 => {
                    self.ctx.mark_probed();
                    let encoded = headers
                        .get(header::CONTENT_ENCODING)
                        .and_then(|v| v.to_str().ok())
                        .is_some_and(|v| !v.eq_ignore_ascii_case("identity"));
                    let size = if encoded { None } else { resp.content_length() };
                    if self.ctx.expect_file && is_web_page(&headers, size) {
                        return Err(FetchError::fatal(EngineError::WebPage));
                    }
                    let info = probe_info(&resp, size, false);
                    let mode = if size == Some(0) {
                        ProbeMode::Empty
                    } else {
                        ProbeMode::Single { size }
                    };
                    return Ok(Probe {
                        info,
                        mode,
                        stream: Some(HttpStream { resp, end: size }),
                    });
                }
                416 if use_ranges => {
                    if header_string(&headers, header::CONTENT_RANGE)
                        .and_then(|v| parse_unsatisfied_range(&v))
                        == Some(0)
                    {
                        self.ctx.mark_probed();
                        let info = probe_info(&resp, Some(0), true);
                        return Ok(Probe {
                            info,
                            mode: ProbeMode::Empty,
                            stream: None,
                        });
                    }
                    use_ranges = false;
                }
                429 | 503 | 408 | 500..=599 => {
                    return Err(FetchError {
                        kind: FailKind::Transient,
                        error: EngineError::Http { status },
                        retry_after: retry_after(&headers),
                    });
                }
                s => return Err(FetchError::fatal(status_error(s))),
            }
        }
    }

    /// The first request of a resumed download: a range request carrying `If-Range`. A `200`
    /// means the resource changed.
    pub(crate) async fn open_validated(
        &mut self,
        from: u64,
        to: Option<u64>,
        if_range: Option<&str>,
    ) -> Result<(HttpStream, ProbeInfo), FetchError> {
        let stream = self.open_inner(from, to, if_range).await?;
        let v = self.ctx.validators();
        let info = probe_info(&stream.resp, v.total, true);
        Ok((stream, info))
    }

    async fn open_inner(
        &mut self,
        from: u64,
        to: Option<u64>,
        if_range: Option<&str>,
    ) -> Result<HttpStream, FetchError> {
        let mut switches = 0usize;
        let max_switches = 2 + self.ctx.sources.mirror_urls().len();
        loop {
            let (src, url, via_final) = self.ctx.sources.pick(self.worker);
            let v = self.ctx.validators();
            let range = if v.ranged { Some((from, to)) } else { None };
            let resp = match self.send(&url, range, if_range).await {
                Ok(r) => r,
                Err(e)
                    if matches!(src, SourceRef::Mirror(_))
                        && e.kind != FailKind::Fatal
                        && switches < max_switches =>
                {
                    self.ctx.sources.bench(src);
                    switches += 1;
                    continue;
                }
                Err(e) => return Err(e),
            };
            self.ctx.sources.on_redirected(src, resp.url());
            let status = resp.status().as_u16();
            if is_browser_check(resp.headers()) {
                return Err(FetchError::fatal(EngineError::BrowserCheck));
            }
            let switch = |sw: &mut usize| -> bool {
                if *sw < max_switches {
                    *sw += 1;
                    true
                } else {
                    false
                }
            };
            match status {
                206 => match self.accept_partial(resp, from, src, &v) {
                    Ok(s) => return Ok(s),
                    Err(e) if matches!(src, SourceRef::Mirror(_)) && switch(&mut switches) => {
                        tracing::debug!(error = %e.error, "mirror rejected");
                        self.ctx.sources.bench(src);
                    }
                    Err(e) => return Err(e),
                },
                200 if !v.ranged => {
                    if from != 0 {
                        return Err(FetchError::fatal(EngineError::RangeNotSupported));
                    }
                    let size = resp.content_length();
                    if let (Some(a), Some(b)) = (v.total, size)
                        && a != b
                    {
                        return Err(FetchError::fatal(EngineError::ValidatorMismatch));
                    }
                    self.ctx.mark_probed();
                    return Ok(HttpStream {
                        resp,
                        end: v.total.or(size),
                    });
                }
                200 => {
                    if if_range.is_some() {
                        return Err(FetchError::fatal(EngineError::ValidatorMismatch));
                    }
                    if matches!(src, SourceRef::Mirror(_)) && switch(&mut switches) {
                        self.ctx.sources.bench(src);
                        continue;
                    }
                    return Err(FetchError::transient(EngineError::network(
                        "the server ignored the range request (HTTP 200)",
                    )));
                }
                401 | 407 => return Err(FetchError::fatal(EngineError::AuthRequired)),
                403 | 404 | 410 => {
                    if via_final && switch(&mut switches) {
                        // Signed redirect target expired: go through the original URL again.
                        self.ctx.sources.drop_final(src);
                        continue;
                    }
                    if matches!(src, SourceRef::Mirror(_)) && switch(&mut switches) {
                        self.ctx.sources.bench(src);
                        continue;
                    }
                    let probed = self.ctx.was_probed();
                    return Err(match status {
                        403 => FetchError::refused(
                            if probed {
                                EngineError::LinkExpired
                            } else {
                                EngineError::Http { status }
                            },
                            retry_after(resp.headers()),
                        ),
                        410 if probed => FetchError::fatal(EngineError::LinkExpired),
                        s => FetchError::fatal(EngineError::Http { status: s }),
                    });
                }
                416 => return Err(FetchError::fatal(EngineError::ValidatorMismatch)),
                429 | 503 => {
                    if matches!(src, SourceRef::Mirror(_)) && switch(&mut switches) {
                        self.ctx.sources.bench(src);
                        continue;
                    }
                    return Err(FetchError::refused(
                        EngineError::Http { status },
                        retry_after(resp.headers()),
                    ));
                }
                408 | 500..=599 => {
                    if matches!(src, SourceRef::Mirror(_)) && switch(&mut switches) {
                        self.ctx.sources.bench(src);
                        continue;
                    }
                    return Err(FetchError {
                        kind: FailKind::Transient,
                        error: EngineError::Http { status },
                        retry_after: retry_after(resp.headers()),
                    });
                }
                s => return Err(FetchError::fatal(EngineError::Http { status: s })),
            }
        }
    }

    /// Validates a `206` against the expected range and validators.
    fn accept_partial(
        &self,
        resp: reqwest::Response,
        from: u64,
        src: SourceRef,
        v: &Validators,
    ) -> Result<HttpStream, FetchError> {
        let cr = header_string(resp.headers(), header::CONTENT_RANGE)
            .and_then(|s| parse_content_range(&s));
        let Some((a, b, total)) = cr else {
            return Err(FetchError::transient(EngineError::network(
                "206 response without a valid Content-Range",
            )));
        };
        if a != from {
            return Err(FetchError::transient(EngineError::network(format!(
                "server returned range starting at {a} instead of {from}"
            ))));
        }
        if let (Some(expected), Some(t)) = (v.total, total)
            && expected != t
        {
            return Err(FetchError::fatal(EngineError::ValidatorMismatch));
        }
        if src == SourceRef::Primary {
            let got = header_string(resp.headers(), header::ETAG);
            if let (Some(e1), Some(e2)) =
                (strong_etag(v.etag.as_deref()), strong_etag(got.as_deref()))
                && e1 != e2
            {
                return Err(FetchError::fatal(EngineError::ValidatorMismatch));
            }
        }
        self.ctx.mark_probed();
        Ok(HttpStream {
            resp,
            end: Some(b + 1),
        })
    }
}

impl RangeFetcher for HttpFetcher {
    type Stream = HttpStream;

    async fn open(&mut self, from: u64, to: Option<u64>) -> Result<HttpStream, FetchError> {
        self.open_inner(from, to, None).await
    }

    fn bounded(&self) -> bool {
        self.ctx.validators().ranged
    }
}

/// Cloudflare's bot check ("Just a moment..."): a challenge only a real browser passes, so
/// retrying (or another connection) can't help.
pub(crate) fn is_browser_check(headers: &header::HeaderMap) -> bool {
    headers
        .get("cf-mitigated")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("challenge"))
}

/// A small HTML answer (an error or check page). Big ones are left alone: a large file sent with
/// a wrong `text/html` type is still the file.
fn is_web_page(headers: &header::HeaderMap, size: Option<u64>) -> bool {
    let html = header_string(headers, header::CONTENT_TYPE).is_some_and(|m| {
        let m = m.trim().to_ascii_lowercase();
        m.starts_with("text/html") || m.starts_with("application/xhtml")
    });
    html && size.is_none_or(|s| s < 2 * 1024 * 1024)
}

/// Builds [`ProbeInfo`] from a response.
pub(crate) fn probe_info(
    resp: &reqwest::Response,
    size: Option<u64>,
    resumable: bool,
) -> ProbeInfo {
    let h = resp.headers();
    let mime = header_string(h, header::CONTENT_TYPE)
        .map(|m| m.split(';').next().unwrap_or("").trim().to_string());
    let cd = super::header_text(h, header::CONTENT_DISPOSITION);
    ProbeInfo {
        final_url: resp.url().clone(),
        file_name: filename::derive_file_name(cd.as_deref(), resp.url(), mime.as_deref()),
        size,
        resumable,
        mime,
        etag: header_string(h, header::ETAG),
        last_modified: header_string(h, header::LAST_MODIFIED),
        accept_ranges: h
            .get(header::ACCEPT_RANGES)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.to_ascii_lowercase().contains("bytes")),
    }
}
