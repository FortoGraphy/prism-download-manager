//! reqwest client construction (one client per worker, shared rustls configuration).

use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use reqwest::redirect::Policy;

use crate::error::EngineError;
use crate::request::{HttpPreference, ProxyConfig};

/// Maximum redirects followed.
pub(crate) const MAX_REDIRECTS: usize = 10;

/// Builds the base rustls configuration: aws-lc-rs provider + the OS trust store
/// (rustls-platform-verifier). Cloning it shares the TLS session cache, so workers of the same
/// download resume TLS sessions instead of doing full handshakes.
pub(crate) fn base_tls_config() -> Result<rustls::ClientConfig, String> {
    use rustls_platform_verifier::BuilderVerifierExt;
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_platform_verifier()
        .map_err(|e| e.to_string())
        .map(|b| b.with_no_client_auth())
}

/// Which TLS implementation a client uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TlsBackend {
    /// rustls with the OS trust store (the default).
    Rustls,
    /// The operating system's TLS stack (Schannel on Windows). Only used as a fallback for
    /// servers rustls cannot talk to, see [`needs_native_tls`].
    Native,
}

/// Whether [`TlsBackend::Native`] exists on this platform.
pub(crate) const fn native_tls_available() -> bool {
    cfg!(windows)
}

/// Hosts (`host[:port]`) that rejected rustls and are reached through the native stack from the
/// start, so only the first request to a host pays for the failed handshake.
static NATIVE_TLS_HOSTS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// Whether `host` (a [`super::download::host_key`]) is known to need the native TLS stack.
pub(crate) fn prefers_native_tls(host: &str) -> bool {
    native_tls_available()
        && !host.is_empty()
        && NATIVE_TLS_HOSTS
            .lock()
            .is_ok_and(|hosts| hosts.contains(host))
}

/// Remembers that `host` must be reached through the native TLS stack.
pub(crate) fn remember_native_tls(host: &str) {
    if native_tls_available()
        && !host.is_empty()
        && let Ok(mut hosts) = NATIVE_TLS_HOSTS.lock()
    {
        hosts.insert(host.to_string());
    }
}

/// True when a connect error says the server and rustls share no usable TLS parameters.
///
/// rustls only implements TLS 1.3 and TLS 1.2 with ECDHE + AEAD suites. Some servers (older
/// banking and government portals) offer only TLS 1.2 with static-RSA key exchange, which rustls
/// refuses on purpose; browsers and the OS TLS stack still speak it. Certificate errors are not
/// matched: the native stack would not make an untrusted server acceptable.
pub(crate) fn needs_native_tls(error_chain: &str) -> bool {
    const MARKERS: [&str; 5] = [
        "HandshakeFailure",
        "ProtocolVersion",
        "InsufficientSecurity",
        "PeerIncompatible",
        "peer is incompatible",
    ];
    MARKERS.iter().any(|m| error_chain.contains(m))
}

/// Everything needed to build a client for a download.
#[derive(Clone)]
pub(crate) struct ClientSpec {
    pub proxy: ProxyConfig,
    pub http: HttpPreference,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub user_agent: String,
    pub tls: rustls::ClientConfig,
    /// Idle connections kept per host (1 for a segment worker's dedicated client).
    pub pool_max_idle: usize,
}

fn proxy_url(
    url: &str,
    username: &Option<String>,
    password: &Option<String>,
    socks: bool,
) -> Result<String, EngineError> {
    let invalid = |m: String| EngineError::InvalidUrl {
        message: format!("proxy: {m}"),
    };
    let with_scheme = if url.contains("://") {
        url.to_string()
    } else if socks {
        format!("socks5h://{url}")
    } else {
        format!("http://{url}")
    };
    let mut u = url::Url::parse(&with_scheme).map_err(|e| invalid(e.to_string()))?;
    // socks5:// would resolve DNS locally; socks5h:// lets the proxy resolve (no DNS leaks).
    if socks && u.scheme() == "socks5" {
        u = url::Url::parse(&with_scheme.replacen("socks5://", "socks5h://", 1))
            .map_err(|e| invalid(e.to_string()))?;
    }
    if let Some(user) = username {
        u.set_username(user)
            .map_err(|_| invalid("cannot set user name".into()))?;
        u.set_password(password.as_deref())
            .map_err(|_| invalid("cannot set password".into()))?;
    }
    Ok(u.to_string())
}

/// Builds a dedicated client. HTTP/1.1-only clients open exactly one connection per
/// concurrent request, which — with one client per worker — means one socket per worker,
/// reused across that worker's sequential range requests.
pub(crate) fn build_client(spec: &ClientSpec) -> Result<reqwest::Client, EngineError> {
    build_client_with(spec, TlsBackend::Rustls)
}

/// [`build_client`] with an explicit TLS backend.
pub(crate) fn build_client_with(
    spec: &ClientSpec,
    backend: TlsBackend,
) -> Result<reqwest::Client, EngineError> {
    let mut b = reqwest::Client::builder();
    b = match backend {
        #[cfg(windows)]
        TlsBackend::Native => b.tls_backend_native(),
        _ => {
            let mut tls = spec.tls.clone();
            tls.alpn_protocols = match spec.http {
                HttpPreference::Http1Only => vec![b"http/1.1".to_vec()],
                _ => vec![b"h2".to_vec(), b"http/1.1".to_vec()],
            };
            b.tls_backend_preconfigured(tls)
        }
    };
    let mut b = b
        .connect_timeout(spec.connect_timeout)
        .read_timeout(spec.read_timeout)
        .pool_max_idle_per_host(spec.pool_max_idle.max(1))
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_nodelay(true)
        .tcp_keepalive(Duration::from_secs(30))
        .redirect(Policy::limited(MAX_REDIRECTS))
        .referer(false)
        .user_agent(spec.user_agent.clone());
    b = match spec.http {
        HttpPreference::Http1Only => b.http1_only(),
        HttpPreference::Auto => b,
        HttpPreference::Http3Experimental => {
            #[cfg(feature = "http3")]
            {
                b.http3_prior_knowledge()
            }
            #[cfg(not(feature = "http3"))]
            {
                b
            }
        }
    };
    b = match &spec.proxy {
        ProxyConfig::None => b.no_proxy(),
        ProxyConfig::System => b,
        ProxyConfig::Http {
            url,
            username,
            password,
        } => {
            let p = reqwest::Proxy::all(proxy_url(url, &None, &None, false)?).map_err(|e| {
                EngineError::InvalidUrl {
                    message: format!("proxy: {e}"),
                }
            })?;
            let p = match username {
                Some(u) => p.basic_auth(u, password.as_deref().unwrap_or("")),
                None => p,
            };
            b.proxy(p)
        }
        ProxyConfig::Socks5 {
            url,
            username,
            password,
        } => b.proxy(
            reqwest::Proxy::all(proxy_url(url, username, password, true)?).map_err(|e| {
                EngineError::InvalidUrl {
                    message: format!("proxy: {e}"),
                }
            })?,
        ),
    };
    b.build()
        .map_err(|e| EngineError::network(format!("cannot create HTTP client: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_urls() {
        assert_eq!(
            proxy_url("proxy:3128", &None, &None, false).unwrap(),
            "http://proxy:3128/"
        );
        assert_eq!(
            proxy_url("socks5://h:1080", &None, &None, true).unwrap(),
            "socks5h://h:1080"
        );
        assert_eq!(
            proxy_url("h:1080", &Some("u".into()), &Some("p w".into()), true).unwrap(),
            "socks5h://u:p%20w@h:1080"
        );
        assert!(proxy_url("http://[bad", &None, &None, false).is_err());
    }

    #[test]
    fn native_tls_is_only_for_incompatible_servers() {
        // What rustls reports against a TLS 1.2 static-RSA-only server (the reported bug).
        assert!(needs_native_tls(
            "client error (Connect): received fatal alert: HandshakeFailure"
        ));
        assert!(needs_native_tls("received fatal alert: ProtocolVersion"));
        assert!(needs_native_tls(
            "peer is incompatible: no common cipher suite"
        ));
        // An untrusted certificate or a plain network failure must not trigger the fallback.
        assert!(!needs_native_tls("invalid peer certificate: UnknownIssuer"));
        assert!(!needs_native_tls("tcp connect error: connection refused"));
    }

    #[test]
    fn native_tls_hosts_are_remembered() {
        let host = "legacy-tls.example.test:443";
        assert!(!prefers_native_tls(host));
        remember_native_tls(host);
        assert_eq!(prefers_native_tls(host), native_tls_available());
        assert!(!prefers_native_tls(""));
    }

    #[cfg(windows)]
    #[test]
    fn native_client_builds() {
        let spec = ClientSpec {
            proxy: ProxyConfig::None,
            http: HttpPreference::Auto,
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(5),
            user_agent: "t".into(),
            tls: base_tls_config().unwrap(),
            pool_max_idle: 1,
        };
        build_client_with(&spec, TlsBackend::Native).unwrap();
    }

    #[test]
    fn clients_build_for_all_preferences() {
        let tls = base_tls_config().unwrap();
        for http in [
            HttpPreference::Http1Only,
            HttpPreference::Auto,
            HttpPreference::Http3Experimental,
        ] {
            for proxy in [
                ProxyConfig::None,
                ProxyConfig::System,
                ProxyConfig::Http {
                    url: "http://127.0.0.1:3128".into(),
                    username: Some("u".into()),
                    password: None,
                },
                ProxyConfig::Socks5 {
                    url: "127.0.0.1:1080".into(),
                    username: None,
                    password: None,
                },
            ] {
                let spec = ClientSpec {
                    proxy,
                    http,
                    connect_timeout: Duration::from_secs(5),
                    read_timeout: Duration::from_secs(5),
                    user_agent: "t".into(),
                    tls: tls.clone(),
                    pool_max_idle: 1,
                };
                build_client(&spec).unwrap();
            }
        }
    }
}
