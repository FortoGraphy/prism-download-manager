//! Loopback IPC server used by the native messaging host.
//!
//! Binds `127.0.0.1:<random port>`, authenticates with a 256-bit bearer token and
//! publishes `{port, token, pid, version}` in `ipc.json` (owner-only readable on
//! Unix). The host forwards every browser message as `POST /v1/message`.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Something that answers browser messages (implemented by the manager).
pub trait IpcHandler: Send + Sync + 'static {
    fn handle(&self, message: serde_json::Value) -> BoxFuture<serde_json::Value>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IpcInfo {
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub version: String,
}

#[derive(Clone)]
struct AppState {
    token: Arc<String>,
    handler: Arc<dyn IpcHandler>,
}

pub struct IpcServer {
    pub info: IpcInfo,
    file: PathBuf,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
}

impl IpcServer {
    /// Start serving and write `ipc_file`. Returns once the socket is bound.
    pub async fn start(
        handler: Arc<dyn IpcHandler>,
        ipc_file: &Path,
        version: &str,
    ) -> std::io::Result<Self> {
        let token = random_token();
        let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await?;
        let port = listener.local_addr()?.port();
        let state = AppState {
            token: Arc::new(token.clone()),
            handler,
        };
        let app = Router::new()
            .route("/v1/health", get(health))
            .route("/v1/message", post(message))
            .with_state(state);
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async {
                let _ = rx.await;
            });
            if let Err(e) = server.await {
                tracing::error!("ipc server stopped: {e}");
            }
        });
        let info = IpcInfo {
            port,
            token,
            pid: std::process::id(),
            version: version.to_string(),
        };
        write_info(ipc_file, &info)?;
        tracing::info!(port, "ipc server listening");
        Ok(Self {
            info,
            file: ipc_file.to_path_buf(),
            shutdown: Some(tx),
        })
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        // Only remove the file if it is still ours (another instance may have replaced it).
        if let Ok(text) = std::fs::read_to_string(&self.file)
            && serde_json::from_str::<IpcInfo>(&text).is_ok_and(|i| i.token == self.info.token)
        {
            let _ = std::fs::remove_file(&self.file);
        }
    }
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS random source");
    hex::encode(bytes)
}

fn write_info(path: &Path, info: &IpcInfo) -> std::io::Result<()> {
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

async fn health(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<serde_json::Value>) {
    // Health is unauthenticated on purpose (the host polls it while launching the app),
    // but only reveals that Prism runs; the version is shown to authenticated callers.
    let authed = authorized(&state, &headers);
    let body = if authed {
        serde_json::json!({"ok": true, "app": "prism", "version": env!("CARGO_PKG_VERSION")})
    } else {
        serde_json::json!({"ok": true, "app": "prism"})
    };
    (StatusCode::OK, Json(body))
}

fn authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(value) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let Some(token) = value.strip_prefix("Bearer ") else {
        return false;
    };
    constant_time_eq(token.trim().as_bytes(), state.token.as_bytes())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let hash_a = Sha256::digest(a);
    let hash_b = Sha256::digest(b);
    let mut diff = 0u8;
    for (&x, &y) in hash_a.iter().zip(hash_b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

async fn message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !authorized(&state, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"ok": false, "error": "unauthorized"})),
        );
    }
    let id = body.get("id").cloned();
    let mut reply = state.handler.handle(body).await;
    if let (Some(id), Some(obj)) = (id, reply.as_object_mut()) {
        obj.entry("id").or_insert(id);
    }
    (StatusCode::OK, Json(reply))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;
    impl IpcHandler for Echo {
        fn handle(&self, message: serde_json::Value) -> BoxFuture<serde_json::Value> {
            Box::pin(async move { serde_json::json!({"ok": true, "echo": message["type"]}) })
        }
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq(b"", b""));
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secr"));
        assert!(!constant_time_eq(b"secr", b"secret"));
        assert!(!constant_time_eq(b"secret1", b"secret2"));
        assert!(!constant_time_eq(b"", b"secret"));
        assert!(!constant_time_eq(b"secret", b""));
    }

    #[tokio::test]
    async fn authenticates_and_dispatches() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("ipc.json");
        let server = IpcServer::start(Arc::new(Echo), &file, "1.0.0")
            .await
            .unwrap();
        let info: IpcInfo = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(info, server.info);
        let url = format!("http://127.0.0.1:{}/v1/message", info.port);
        let client = reqwest::Client::new();
        let denied = client
            .post(&url)
            .json(&serde_json::json!({"type": "hello"}))
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), 401);
        let ok: serde_json::Value = client
            .post(&url)
            .bearer_auth(&info.token)
            .json(&serde_json::json!({"id": 7, "type": "hello"}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(ok["echo"], "hello");
        assert_eq!(ok["id"], 7);
        drop(server);
        assert!(!file.exists());
    }
}
