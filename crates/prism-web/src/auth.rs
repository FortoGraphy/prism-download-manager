use axum::http::{HeaderMap, header};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WebInfo {
    port: u16,
    token: String,
    pid: u32,
    version: String,
}

#[derive(Clone)]
pub struct Auth {
    token: Option<Arc<String>>,
    file: PathBuf,
}

impl Auth {
    /// Loads the saved token, or generates and stores a new one. `--no-auth-token` stores nothing.
    pub fn new(data_dir: &Path, port: u16, no_token: bool) -> io::Result<Self> {
        let file = data_dir.join("web-ui.json");
        if no_token {
            return Ok(Self { token: None, file });
        }
        let token = std::fs::read_to_string(&file)
            .ok()
            .and_then(|s| serde_json::from_str::<WebInfo>(&s).ok())
            .map(|i| i.token)
            .filter(|t| t.len() == 64)
            .unwrap_or_else(random_token);
        let info = WebInfo {
            port,
            token: token.clone(),
            pid: std::process::id(),
            version: prism_core::APP_VERSION.to_string(),
        };
        write_info(&file, &info)?;
        Ok(Self {
            token: Some(Arc::new(token)),
            file,
        })
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref().map(String::as_str)
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    pub fn enabled(&self) -> bool {
        self.token.is_some()
    }

    pub fn authorized(&self, headers: &HeaderMap) -> bool {
        let Some(expected) = &self.token else {
            return true;
        };
        let Some(value) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) else {
            return false;
        };
        let Some(given) = value.strip_prefix("Bearer ") else {
            return false;
        };
        constant_time_eq(given.trim().as_bytes(), expected.as_bytes())
    }
}

/// Accepts only loopback names/addresses (IPv4 and IPv6, with or without brackets).
pub(crate) fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']);
    host.eq_ignore_ascii_case("localhost")
        || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

pub(crate) fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS random source");
    hex::encode(bytes)
}

pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn write_info(path: &Path, info: &WebInfo) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(info)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_and_reuses_token() {
        let dir = tempfile::tempdir().unwrap();
        let a = Auth::new(dir.path(), 8080, false).unwrap();
        let token = a.token().unwrap().to_string();
        assert_eq!(token.len(), 64);
        assert!(token.bytes().all(|b| b.is_ascii_hexdigit()));
        let b = Auth::new(dir.path(), 8080, false).unwrap();
        assert_eq!(b.token().unwrap(), token);
    }

    #[test]
    fn no_token_mode() {
        let dir = tempfile::tempdir().unwrap();
        let a = Auth::new(dir.path(), 8080, true).unwrap();
        assert!(a.token().is_none());
        assert!(a.authorized(&HeaderMap::new()));
        assert!(!dir.path().join("web-ui.json").exists());
    }

    #[test]
    fn loopback_host_check() {
        for host in ["localhost", "LOCALHOST", "127.0.0.1", "127.0.0.2", "::1", "[::1]"] {
            assert!(is_loopback_host(host), "{host}");
        }
        for host in ["0.0.0.0", "example.com", "126.0.0.1", "::2", ""] {
            assert!(!is_loopback_host(host), "{host}");
        }
    }

    #[test]
    fn checks_bearer_header() {
        let dir = tempfile::tempdir().unwrap();
        let a = Auth::new(dir.path(), 8080, false).unwrap();
        let mut headers = HeaderMap::new();
        assert!(!a.authorized(&headers));
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", a.token().unwrap()).parse().unwrap(),
        );
        assert!(a.authorized(&headers));
        headers.insert(
            header::AUTHORIZATION,
            "Bearer 0000000000000000000000000000000000000000000000000000000000000000"
                .parse()
                .unwrap(),
        );
        assert!(!a.authorized(&headers));
    }
}
