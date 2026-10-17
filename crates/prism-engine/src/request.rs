//! Download requests and their options. All types are serde-serializable so the application
//! core can persist them.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use url::Url;

/// HTTP method of the download request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    /// `GET` (segmented).
    #[default]
    Get,
    /// `POST` with [`DownloadRequest::body`]; always downloaded over a single connection.
    Post,
}

/// Authentication scheme to use with [`Credentials`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthScheme {
    /// Whatever the server asks for in `WWW-Authenticate` (Basic, Digest or NTLM).
    #[default]
    Auto,
    /// Basic, sent pre-emptively.
    Basic,
    /// Digest (RFC 7616), after the server's challenge.
    Digest,
    /// NTLM (connection-bound handshake; best effort).
    Ntlm,
}

/// User name and password for HTTP/FTP authentication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credentials {
    /// User name (for NTLM, `DOMAIN\user` is accepted).
    pub username: String,
    /// Password.
    pub password: String,
    /// Scheme.
    #[serde(default)]
    pub scheme: AuthScheme,
}

/// Proxy selection.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProxyConfig {
    /// Direct connection.
    None,
    /// System proxy (environment variables; Windows registry; macOS system settings).
    #[default]
    System,
    /// HTTP(S) proxy, e.g. `http://proxy:3128`.
    Http {
        /// Proxy URL.
        url: String,
        /// Optional user name.
        #[serde(default)]
        username: Option<String>,
        /// Optional password.
        #[serde(default)]
        password: Option<String>,
    },
    /// SOCKS5 proxy, e.g. `socks5://host:1080` (DNS is resolved by the proxy).
    Socks5 {
        /// Proxy URL.
        url: String,
        /// Optional user name.
        #[serde(default)]
        username: Option<String>,
        /// Optional password.
        #[serde(default)]
        password: Option<String>,
    },
}

/// What to do when the destination file already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// Pick a unique name: `name (1).ext`, `name (2).ext`, …
    #[default]
    Rename,
    /// Replace the existing file on completion.
    Overwrite,
    /// Fail with [`crate::EngineError::FileExists`].
    Fail,
}

/// Where to save the download.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Destination {
    /// Target directory (created if missing).
    pub dir: PathBuf,
    /// File name; `None` = derive from `Content-Disposition`, the URL and the MIME type.
    #[serde(default)]
    pub file_name: Option<String>,
    /// Conflict handling.
    #[serde(default)]
    pub conflict: ConflictPolicy,
}

/// HTTP protocol preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpPreference {
    /// HTTP/1.1 only: one TCP connection per segment worker (fastest for bulk transfers).
    #[default]
    Http1Only,
    /// Allow HTTP/2 via ALPN (each worker still has its own client, hence its own connection).
    Auto,
    /// HTTP/3 (QUIC) with prior knowledge. Only available when the crate is built with the
    /// `http3` feature; otherwise behaves like [`HttpPreference::Auto`].
    Http3Experimental,
}

/// Checksum algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecksumAlgo {
    /// MD5.
    Md5,
    /// SHA-1.
    Sha1,
    /// SHA-256.
    Sha256,
}

/// Expected checksum verified after completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedChecksum {
    /// Algorithm.
    pub algo: ChecksumAlgo,
    /// Hex digest (case-insensitive).
    pub hex: String,
}

/// A download job description (HTTP(S), FTP(S)).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadRequest {
    /// Primary URL.
    pub url: Url,
    /// Alternative URLs of the same file; validated (same size) and used round-robin.
    #[serde(default)]
    pub mirrors: Vec<Url>,
    /// HTTP method.
    #[serde(default)]
    pub method: HttpMethod,
    /// Request body for `POST`.
    #[serde(default)]
    pub body: Option<String>,
    /// Extra request headers (e.g. captured from the browser).
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// `Cookie` header value.
    #[serde(default)]
    pub cookies: Option<String>,
    /// `Referer` header value.
    #[serde(default)]
    pub referer: Option<String>,
    /// `User-Agent` override.
    #[serde(default)]
    pub user_agent: Option<String>,
    /// Credentials (HTTP auth or FTP login). FTP URLs may also embed them.
    #[serde(default)]
    pub auth: Option<Credentials>,
    /// Proxy; `None` = the engine's [`crate::EngineConfig::default_proxy`].
    #[serde(default)]
    pub proxy: Option<ProxyConfig>,
    /// Where to save.
    pub destination: Destination,
    /// Connections to use (0 = engine default; capped by the engine).
    #[serde(default)]
    pub connections: u8,
    /// HTTP protocol preference.
    #[serde(default)]
    pub http: HttpPreference,
    /// Optional checksum verified after completion.
    #[serde(default)]
    pub checksum: Option<ExpectedChecksum>,
    /// Per-download speed limit in bytes/s (0 = unlimited).
    #[serde(default)]
    pub speed_limit: u64,
    /// Override of [`crate::EngineConfig::max_retries`].
    #[serde(default)]
    pub max_retries: Option<u32>,
    /// Upper bound of a single HTTP range request (e.g. 10 MiB for YouTube formats).
    #[serde(default)]
    pub max_request_size: Option<u64>,
}

impl DownloadRequest {
    /// A `GET` request for `url` saved into `dir` with default options.
    pub fn new(url: Url, dir: impl Into<PathBuf>) -> Self {
        Self {
            url,
            mirrors: Vec::new(),
            method: HttpMethod::Get,
            body: None,
            headers: Vec::new(),
            cookies: None,
            referer: None,
            user_agent: None,
            auth: None,
            proxy: None,
            destination: Destination {
                dir: dir.into(),
                file_name: None,
                conflict: ConflictPolicy::Rename,
            },
            connections: 0,
            http: HttpPreference::Http1Only,
            checksum: None,
            speed_limit: 0,
            max_retries: None,
            max_request_size: None,
        }
    }
}

/// Kind of adaptive stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    /// Detect from the URL / content (`.m3u8` → HLS, `.mpd` → DASH, otherwise sniff).
    #[default]
    Auto,
    /// HTTP Live Streaming.
    Hls,
    /// MPEG-DASH.
    Dash,
}

/// Which video variant/representation to download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum VariantSelection {
    /// Highest bandwidth.
    #[default]
    Best,
    /// Highest bandwidth whose height is at most this many pixels (falls back to the lowest).
    MaxHeight(u32),
    /// Index into the master playlist's variant list / the MPD's video representations
    /// (sorted as they appear in the manifest).
    Index(usize),
}

/// An HLS/DASH download job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaRequest {
    /// Master/media playlist or MPD URL.
    pub url: Url,
    /// Stream kind.
    #[serde(default)]
    pub kind: MediaKind,
    /// Variant selection.
    #[serde(default)]
    pub variant: VariantSelection,
    /// Extra request headers.
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// `Cookie` header value.
    #[serde(default)]
    pub cookies: Option<String>,
    /// `Referer` header value.
    #[serde(default)]
    pub referer: Option<String>,
    /// `User-Agent` override.
    #[serde(default)]
    pub user_agent: Option<String>,
    /// Proxy; `None` = engine default.
    #[serde(default)]
    pub proxy: Option<ProxyConfig>,
    /// Where to save. `file_name` should carry the desired final extension (`.mp4`); when
    /// absent the name is derived from the URL.
    pub destination: Destination,
    /// Parallel segment downloads (0 = engine default).
    #[serde(default)]
    pub connections: u8,
    /// Remux the result with ffmpeg into `.mp4` (default true). When false, or when ffmpeg is
    /// not available, the raw concatenated stream(s) are kept.
    #[serde(default = "default_true")]
    pub remux: bool,
    /// Speed limit in bytes/s (0 = unlimited).
    #[serde(default)]
    pub speed_limit: u64,
    /// Override of [`crate::EngineConfig::max_retries`].
    #[serde(default)]
    pub max_retries: Option<u32>,
}

fn default_true() -> bool {
    true
}

impl MediaRequest {
    /// A request for `url` saved into `dir` with default options.
    pub fn new(url: Url, dir: impl Into<PathBuf>) -> Self {
        Self {
            url,
            kind: MediaKind::Auto,
            variant: VariantSelection::Best,
            headers: Vec::new(),
            cookies: None,
            referer: None,
            user_agent: None,
            proxy: None,
            destination: Destination {
                dir: dir.into(),
                file_name: None,
                conflict: ConflictPolicy::Rename,
            },
            connections: 0,
            remux: true,
            speed_limit: 0,
            max_retries: None,
        }
    }
}

/// Parameters for listing a manifest's variants without downloading anything — a cheap
/// probe (one small text fetch) for showing a full quality picker before the user commits
/// to a download. Each returned variant's `index` is exactly what
/// [`VariantSelection::Index`] expects in a later [`MediaRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariantProbeRequest {
    /// Master/media playlist or MPD URL.
    pub url: Url,
    /// Stream kind.
    #[serde(default)]
    pub kind: MediaKind,
    /// Extra request headers.
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// `Cookie` header value.
    #[serde(default)]
    pub cookies: Option<String>,
    /// `Referer` header value.
    #[serde(default)]
    pub referer: Option<String>,
    /// `User-Agent` override.
    #[serde(default)]
    pub user_agent: Option<String>,
    /// Proxy; `None` = engine default.
    #[serde(default)]
    pub proxy: Option<ProxyConfig>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip() {
        let mut r = DownloadRequest::new(Url::parse("https://example.com/a.zip").unwrap(), "/tmp");
        r.auth = Some(Credentials {
            username: "u".into(),
            password: "p".into(),
            scheme: AuthScheme::Digest,
        });
        r.proxy = Some(ProxyConfig::Socks5 {
            url: "socks5://h:1080".into(),
            username: None,
            password: None,
        });
        r.checksum = Some(ExpectedChecksum {
            algo: ChecksumAlgo::Sha256,
            hex: "ab".into(),
        });
        r.headers.push(("X-A".into(), "1".into()));
        let json = serde_json::to_string(&r).unwrap();
        let back: DownloadRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
        // Minimal JSON uses defaults.
        let min: DownloadRequest =
            serde_json::from_str(r#"{"url":"http://x/y","destination":{"dir":"d"}}"#).unwrap();
        assert_eq!(min.method, HttpMethod::Get);
        assert_eq!(min.http, HttpPreference::Http1Only);
        let m: MediaRequest =
            serde_json::from_str(r#"{"url":"http://x/y.m3u8","destination":{"dir":"d"}}"#).unwrap();
        assert!(m.remux);
        assert_eq!(m.variant, VariantSelection::Best);
    }
}
