//! HTTP(S) downloads.

pub(crate) mod auth;
pub(crate) mod client;
pub(crate) mod download;
pub(crate) mod fetcher;
pub(crate) mod remote;

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use url::Url;

use crate::error::EngineError;
use crate::request::HttpMethod;
use crate::segmented::table::WorkerId;
use auth::AuthState;
use client::ClientSpec;

/// How long a failing mirror is not used.
const MIRROR_BENCH: Duration = Duration::from_secs(60);

/// Which source a URL came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceRef {
    Primary,
    Mirror(usize),
}

#[derive(Debug)]
struct Mirror {
    url: Url,
    final_url: Option<Url>,
    validated: bool,
    benched_until: Option<Instant>,
}

#[derive(Debug)]
struct SourcesInner {
    original: Url,
    final_url: Option<Url>,
    mirrors: Vec<Mirror>,
}

/// Primary URL (+ its redirect target) and validated mirrors; workers are distributed over
/// them round-robin.
#[derive(Debug)]
pub(crate) struct Sources {
    inner: Mutex<SourcesInner>,
}

impl Sources {
    pub(crate) fn new(original: Url, final_url: Option<Url>, mirrors: &[Url]) -> Self {
        Self {
            inner: Mutex::new(SourcesInner {
                original,
                final_url,
                mirrors: mirrors
                    .iter()
                    .map(|u| Mirror {
                        url: u.clone(),
                        final_url: None,
                        validated: false,
                        benched_until: None,
                    })
                    .collect(),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SourcesInner> {
        self.inner.lock().expect("sources lock")
    }

    /// Picks the source for a worker: `(source, url, url_is_redirect_target)`.
    pub(crate) fn pick(&self, worker: WorkerId) -> (SourceRef, Url, bool) {
        let g = self.lock();
        let now = Instant::now();
        let usable: Vec<usize> = g
            .mirrors
            .iter()
            .enumerate()
            .filter(|(_, m)| m.validated && m.benched_until.is_none_or(|t| now >= t))
            .map(|(i, _)| i)
            .collect();
        let slot = worker as usize % (usable.len() + 1);
        if slot == 0 {
            match &g.final_url {
                Some(f) => (SourceRef::Primary, f.clone(), true),
                None => (SourceRef::Primary, g.original.clone(), false),
            }
        } else {
            let i = usable[slot - 1];
            let m = &g.mirrors[i];
            match &m.final_url {
                Some(f) => (SourceRef::Mirror(i), f.clone(), true),
                None => (SourceRef::Mirror(i), m.url.clone(), false),
            }
        }
    }

    pub(crate) fn original(&self) -> Url {
        self.lock().original.clone()
    }

    pub(crate) fn final_url(&self) -> Option<Url> {
        self.lock().final_url.clone()
    }

    /// Records the redirect target of a source (only when it differs from the source URL).
    pub(crate) fn on_redirected(&self, src: SourceRef, final_url: &Url) {
        let mut g = self.lock();
        match src {
            SourceRef::Primary => {
                if *final_url != g.original {
                    g.final_url = Some(final_url.clone());
                }
            }
            SourceRef::Mirror(i) => {
                if let Some(m) = g.mirrors.get_mut(i)
                    && *final_url != m.url
                {
                    m.final_url = Some(final_url.clone());
                }
            }
        }
    }

    /// Forgets an expired redirect target so the next request goes to the original URL.
    pub(crate) fn drop_final(&self, src: SourceRef) {
        let mut g = self.lock();
        match src {
            SourceRef::Primary => g.final_url = None,
            SourceRef::Mirror(i) => {
                if let Some(m) = g.mirrors.get_mut(i) {
                    m.final_url = None;
                }
            }
        }
    }

    /// Takes a failing mirror out of rotation for a while.
    pub(crate) fn bench(&self, src: SourceRef) {
        if let SourceRef::Mirror(i) = src
            && let Some(m) = self.lock().mirrors.get_mut(i)
        {
            m.benched_until = Some(Instant::now() + MIRROR_BENCH);
        }
    }

    /// Marks a mirror as validated (same size as the primary).
    pub(crate) fn validate_mirror(&self, i: usize, final_url: Option<Url>) {
        if let Some(m) = self.lock().mirrors.get_mut(i) {
            m.validated = true;
            m.final_url = final_url.filter(|f| *f != m.url);
        }
    }

    pub(crate) fn mirror_urls(&self) -> Vec<Url> {
        self.lock().mirrors.iter().map(|m| m.url.clone()).collect()
    }

    /// Whether `url` is on the site of the download's own link or of one of its mirrors (not
    /// on a server one of them redirected to).
    pub(crate) fn is_own_site(&self, url: &Url) -> bool {
        let g = self.lock();
        let origin = url.origin();
        g.original.origin() == origin || g.mirrors.iter().any(|m| m.url.origin() == origin)
    }

    /// Replaces the primary URL (change_url).
    pub(crate) fn replace_primary(&self, url: Url, final_url: Option<Url>) {
        let mut g = self.lock();
        g.final_url = final_url.filter(|f| *f != url);
        g.original = url;
    }
}

/// Validators every ranged response must match.
#[derive(Debug, Clone, Default)]
pub(crate) struct Validators {
    pub total: Option<u64>,
    pub etag: Option<String>,
    /// Byte ranges work (multi-connection mode).
    pub ranged: bool,
}

/// Per-download HTTP context shared by its fetchers.
pub(crate) struct HttpCtx {
    pub spec: ClientSpec,
    pub method: HttpMethod,
    pub body: Option<String>,
    pub headers: HeaderMap,
    pub auth: AuthState,
    pub sources: Sources,
    pub validators: Mutex<Validators>,
    /// A successful response was received (403/410 afterwards mean an expired link).
    pub probed: AtomicBool,
    /// See [`crate::DownloadRequest::expect_file`].
    pub expect_file: bool,
    pub source_generation: Arc<AtomicU64>,
    /// See [`crate::DownloadRequest::window`].
    pub window: Option<(u64, u64)>,
}

impl HttpCtx {
    pub(crate) fn validators(&self) -> Validators {
        self.validators.lock().expect("validators lock").clone()
    }

    pub(crate) fn set_validators(&self, v: Validators) {
        *self.validators.lock().expect("validators lock") = v;
    }

    pub(crate) fn mark_probed(&self) {
        self.probed.store(true, Ordering::Release);
    }

    pub(crate) fn was_probed(&self) -> bool {
        self.probed.load(Ordering::Acquire)
    }
}

/// Builds the static request headers (custom headers, cookies, referer, identity encoding).
pub(crate) fn base_headers(
    custom: &[(String, String)],
    cookies: Option<&str>,
    referer: Option<&str>,
) -> Result<HeaderMap, EngineError> {
    let mut h = HeaderMap::new();
    let invalid = |m: String| EngineError::InvalidUrl {
        message: format!("header {m}"),
    };
    for (k, v) in custom {
        let name = HeaderName::from_bytes(k.trim().as_bytes()).map_err(|_| invalid(k.clone()))?;
        // Hop-by-hop / engine-managed headers are ignored.
        if matches!(
            name.as_str(),
            "range"
                | "if-range"
                | "accept-encoding"
                | "host"
                | "content-length"
                | "connection"
                | "transfer-encoding"
        ) {
            continue;
        }
        h.append(
            name,
            HeaderValue::from_str(v.trim()).map_err(|_| invalid(k.clone()))?,
        );
    }
    if let Some(c) = cookies.filter(|c| !c.is_empty()) {
        h.insert(
            reqwest::header::COOKIE,
            HeaderValue::from_str(c).map_err(|_| invalid("Cookie".into()))?,
        );
    }
    if let Some(r) = referer.filter(|r| !r.is_empty()) {
        h.insert(
            reqwest::header::REFERER,
            HeaderValue::from_str(r).map_err(|_| invalid("Referer".into()))?,
        );
    }
    // Ranges apply to the encoded representation; never let a proxy/server compress.
    h.insert(
        reqwest::header::ACCEPT_ENCODING,
        HeaderValue::from_static("identity"),
    );
    if !h.contains_key(reqwest::header::ACCEPT) {
        h.insert(reqwest::header::ACCEPT, HeaderValue::from_static("*/*"));
    }
    Ok(h)
}

/// `Content-Range: bytes a-b/total` → `(a, b, total)`; total `None` for `*`.
pub(crate) fn parse_content_range(v: &str) -> Option<(u64, u64, Option<u64>)> {
    let rest = v.trim().strip_prefix("bytes")?.trim_start();
    let (range, total) = rest.split_once('/')?;
    let (a, b) = range.trim().split_once('-')?;
    let (a, b): (u64, u64) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
    if b < a {
        return None;
    }
    let total = match total.trim() {
        "*" => None,
        t => Some(t.parse().ok()?),
    };
    Some((a, b, total))
}

/// `Content-Range: bytes */total` of a 416 response.
pub(crate) fn parse_unsatisfied_range(v: &str) -> Option<u64> {
    v.trim()
        .strip_prefix("bytes")?
        .trim_start()
        .strip_prefix("*/")?
        .trim()
        .parse()
        .ok()
}

/// A strong (non-weak) ETag usable for `If-Range`.
pub(crate) fn strong_etag(etag: Option<&str>) -> Option<&str> {
    etag.filter(|e| !e.starts_with("W/") && e.starts_with('"'))
}

/// `Retry-After` in seconds (HTTP-date values are ignored).
pub(crate) fn retry_after(h: &HeaderMap) -> Option<Duration> {
    h.get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

pub(crate) fn header_string(h: &HeaderMap, name: reqwest::header::HeaderName) -> Option<String> {
    h.get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// A header that may carry text beyond ASCII (a Content-Disposition with a raw UTF-8 file name):
/// read as UTF-8, else as Latin-1. [`header_string`] drops such a header altogether (#91).
pub(crate) fn header_text(h: &HeaderMap, name: reqwest::header::HeaderName) -> Option<String> {
    let bytes = h.get(name)?.as_bytes();
    let text = match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    };
    Some(text.trim().to_string()).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_raw_utf8_file_name_in_a_header_is_kept() {
        use reqwest::header::{CONTENT_DISPOSITION, HeaderValue};
        let mut h = HeaderMap::new();
        let cd = "attachment; filename=\"อัพเดท.zip\"";
        h.insert(
            CONTENT_DISPOSITION,
            HeaderValue::from_bytes(cd.as_bytes()).unwrap(),
        );
        assert_eq!(header_string(&h, CONTENT_DISPOSITION), None);
        assert_eq!(header_text(&h, CONTENT_DISPOSITION).as_deref(), Some(cd));
        let latin1 = HeaderValue::from_bytes(b"attachment; filename=\"caf\xe9.txt\"").unwrap();
        h.insert(CONTENT_DISPOSITION, latin1);
        assert_eq!(
            header_text(&h, CONTENT_DISPOSITION).as_deref(),
            Some("attachment; filename=\"café.txt\"")
        );
    }

    #[test]
    fn content_range_parsing() {
        assert_eq!(
            parse_content_range("bytes 0-99/1000"),
            Some((0, 99, Some(1000)))
        );
        assert_eq!(parse_content_range("bytes 5-5/*"), Some((5, 5, None)));
        assert_eq!(parse_content_range("bytes 9-5/10"), None);
        assert_eq!(parse_content_range("items 0-1/2"), None);
        assert_eq!(parse_unsatisfied_range("bytes */0"), Some(0));
        assert_eq!(parse_unsatisfied_range("bytes */77"), Some(77));
        assert_eq!(strong_etag(Some("\"a\"")), Some("\"a\""));
        assert_eq!(strong_etag(Some("W/\"a\"")), None);
    }

    #[test]
    fn headers_and_sources() {
        let h = base_headers(
            &[
                ("X-Test".into(), "1".into()),
                ("Range".into(), "bytes=1-".into()),
            ],
            Some("a=b"),
            Some("http://ref/"),
        )
        .unwrap();
        assert_eq!(h["x-test"], "1");
        assert!(!h.contains_key("range"));
        assert_eq!(h["cookie"], "a=b");
        assert_eq!(h["accept-encoding"], "identity");
        assert!(base_headers(&[("bad header".into(), "x".into())], None, None).is_err());

        let p = Url::parse("http://p/a").unwrap();
        let m = Url::parse("http://m/a").unwrap();
        let s = Sources::new(p.clone(), None, std::slice::from_ref(&m));
        // Unvalidated mirrors are not used.
        assert_eq!(s.pick(1).0, SourceRef::Primary);
        s.validate_mirror(0, None);
        assert_eq!(s.pick(0).0, SourceRef::Primary);
        assert_eq!(s.pick(1), (SourceRef::Mirror(0), m.clone(), false));
        s.bench(SourceRef::Mirror(0));
        assert_eq!(s.pick(1).0, SourceRef::Primary);
        let f = Url::parse("http://cdn/a?sig=1").unwrap();
        s.on_redirected(SourceRef::Primary, &f);
        assert_eq!(s.pick(0), (SourceRef::Primary, f, true));
        s.drop_final(SourceRef::Primary);
        assert_eq!(s.pick(0), (SourceRef::Primary, p, false));
    }
}
