//! Protocol v1 endpoints for the browser extension's native host.

use crate::auth::{constant_time_eq, random_token};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::routing::{get, post};
use axum::{Json, Router};
use prism_core::ipc::{IpcHandler, IpcInfo};
use prism_core::manager::Core;
use serde_json::{Value, json};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone)]
pub struct IpcAuth {
    token: Arc<String>,
    file: PathBuf,
}

impl IpcAuth {
    pub fn new(file: PathBuf) -> Self {
        Self {
            token: Arc::new(random_token()),
            file,
        }
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Publish the host-visible address once the listener is bound.
    pub fn publish(&self, port: u16) -> io::Result<()> {
        let info = IpcInfo {
            port,
            token: self.token.to_string(),
            pid: std::process::id(),
            version: env!("CARGO_PKG_VERSION").into(),
        };
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.file.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&info)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, &self.file)
    }

    pub fn authorized(&self, headers: &HeaderMap) -> bool {
        let Some(value) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) else {
            return false;
        };
        let Some(given) = value.strip_prefix("Bearer ") else {
            return false;
        };
        constant_time_eq(given.trim().as_bytes(), self.token.as_bytes())
    }
}

impl Drop for IpcAuth {
    fn drop(&mut self) {
        // Another instance may have replaced the file; only remove our own.
        if let Ok(bytes) = std::fs::read(&self.file)
            && let Ok(info) = serde_json::from_slice::<IpcInfo>(&bytes)
            && info.token == *self.token
        {
            let _ = std::fs::remove_file(&self.file);
        }
    }
}

struct IpcState {
    handler: Core,
    auth: IpcAuth,
}

pub fn router(core: Core, auth: IpcAuth) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/message", post(message))
        .with_state(Arc::new(IpcState { handler: core, auth }))
}

async fn health() -> Json<Value> {
    Json(json!({"ok": true, "app": "prism", "version": env!("CARGO_PKG_VERSION")}))
}

async fn message(
    State(state): State<Arc<IpcState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if !state.auth.authorized(&headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok": false, "error": "unauthorized"})),
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

    #[test]
    fn publishes_owner_only_info_and_cleans_up_its_own_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("nested").join("ipc.json");
        let auth = IpcAuth::new(file.clone());
        assert_eq!(auth.token.len(), 64);
        auth.publish(4321).unwrap();
        let info: IpcInfo = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(info.port, 4321);
        assert_eq!(info.token, *auth.token);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        drop(auth);
        assert!(!file.exists());
    }

    #[test]
    fn keeps_a_file_written_by_another_instance() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("ipc.json");
        let auth = IpcAuth::new(file.clone());
        auth.publish(1234).unwrap();
        std::fs::write(&file, b"{\"token\":\"somebody-else\"}").unwrap();
        drop(auth);
        assert!(file.exists());
    }
}
