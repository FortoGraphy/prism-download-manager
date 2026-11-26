use crate::auth::Auth;
use crate::commands;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::{Event, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream::{Stream, unfold};
use prism_core::events::CoreEvent;
use prism_core::manager::Core;
use serde_json::{Value, json};
use std::convert::Infallible;
use std::path::{Component, PathBuf};
use std::sync::Arc;

include!(concat!(env!("OUT_DIR"), "/assets.rs"));

#[derive(Clone)]
struct AppState {
    core: Core,
    auth: Auth,
}

pub fn router(core: Core, auth: Auth) -> Router {
    let state = Arc::new(AppState { core, auth });
    Router::new()
        .route("/", get(index))
        .route("/api/config", get(config))
        .route("/api/invoke", post(invoke))
        .route("/api/events", get(events))
        .route("/login.html", get(login))
        .route("/{*path}", get(asset))
        .with_state(state)
}

async fn index(State(state): State<Arc<AppState>>) -> ([(header::HeaderName, &'static str); 2], Vec<u8>) {
    let body = asset_bytes("/").await.unwrap_or_else(|| {
        b"<!doctype html><title>Prism Web</title><p>Run npm run build --prefix app to build the UI.</p>".to_vec()
    });
    html_response(state.auth.enabled(), body)
}

async fn config(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // The login page validates the entered token here; a plain config read stays public.
    if headers.contains_key(header::AUTHORIZATION) && !state.auth.authorized(&headers) {
        return Err((StatusCode::UNAUTHORIZED, Json(json!({ "error": "unauthorized" }))));
    }
    Ok(Json(json!({ "auth": state.auth.enabled() })))
}

async fn invoke(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if !state.auth.authorized(&headers) {
        return Err((StatusCode::UNAUTHORIZED, Json(json!({ "error": "unauthorized" }))));
    }
    let cmd = body
        .get("cmd")
        .and_then(Value::as_str)
        .ok_or((StatusCode::BAD_REQUEST, Json(json!({ "error": "missing cmd" }))))?;
    let args = body.get("args").cloned().unwrap_or_else(|| json!({}));
    match commands::invoke(&state.core, cmd, args).await {
        Ok(value) => Ok(Json(value)),
        Err(error) => {
            tracing::warn!(cmd, %error, "invoke failed");
            Err((StatusCode::BAD_REQUEST, Json(json!({ "error": error }))))
        }
    }
}

async fn events(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    if !state.auth.authorized(&headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let core = state.core.clone();
    let rx = core.subscribe();
    let stream = unfold((rx, core), |(mut rx, core)| async move {
        match rx.recv().await {
            Ok(event) => {
                let data = serde_json::to_string(&event).unwrap_or_else(|_| "{}".into());
                Some((Ok(Event::default().data(data)), (rx, core)))
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                // A slow browser missed events: resend the full list so rows recover.
                let resync = CoreEvent::ItemsAdded(core.bootstrap().items);
                let data = serde_json::to_string(&resync).unwrap_or_else(|_| "{}".into());
                Some((Ok(Event::default().data(data)), (rx, core)))
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => None,
        }
    });
    Ok(Sse::new(stream))
}

async fn login() -> ([(header::HeaderName, &'static str); 1], Vec<u8>) {
    let body = br#"<!doctype html>
<meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Prism Web</title>
<style>body{font:14px system-ui;background:#101319;color:#e8ecf4;display:grid;place-items:center;height:100vh;margin:0}
form{background:#1a1f28;padding:28px;border-radius:16px;display:grid;gap:12px;width:min(340px,90vw)}
input,button{font:inherit;padding:10px;border-radius:8px;border:1px solid #39414f;background:#12161d;color:inherit}
button{background:#5b8cff;border-color:#5b8cff;cursor:pointer}.err{color:#ff9090}</style>
<form id="f"><strong>Prism Web</strong><span>Enter the access token printed at startup.</span>
<input id="token" type="password" autocomplete="off" autofocus placeholder="Access token">
<button type="submit">Connect</button><span class="err" id="err"></span></form>
<script>
const next = new URLSearchParams(location.search).get('next') || '/';
document.getElementById('f').addEventListener('submit', async e => {
  e.preventDefault();
  const token = document.getElementById('token').value.trim();
  const r = await fetch('/api/config', {headers:{Authorization:'Bearer '+token}}).catch(() => null);
  if (r && r.ok) { sessionStorage.setItem('prism-web-token', token); location.href = next; }
  else document.getElementById('err').textContent = 'Wrong token. Try again.';
});
</script>"#.to_vec();
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], body)
}

async fn asset(Path(path): Path<String>) -> ([(header::HeaderName, &'static str); 1], Vec<u8>) {
    let url = format!("/{path}");
    let (mime, body) = if let Some((_, mime, bytes)) = ASSETS.iter().find(|(u, _, _)| *u == url) {
        (*mime, (*bytes).to_vec())
    } else {
        let mime = mime_for(&url);
        (mime, asset_bytes(&url).await.unwrap_or_default())
    };
    ([(header::CONTENT_TYPE, mime)], body)
}

fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "woff2" => "font/woff2",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    }
}

async fn asset_bytes(path: &str) -> Option<Vec<u8>> {
    if let Some((_, _, bytes)) = ASSETS.iter().find(|(url, _, _)| *url == path) {
        return Some((*bytes).to_vec());
    }
    let root = std::env::var_os("PRISM_WEB_DIST")?;
    let relative = if path == "/" { "index.html" } else { path.trim_start_matches('/') };
    let clean: PathBuf = PathBuf::from(relative)
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .collect();
    tokio::fs::read(PathBuf::from(root).join(clean)).await.ok()
}

fn html_response(auth: bool, body: Vec<u8>) -> ([(header::HeaderName, &'static str); 2], Vec<u8>) {
    let script = format!("<script>window.__PRISM_WEB__={{auth:{}}};</script>", auth);
    let mut html = String::from_utf8_lossy(&body).into_owned();
    if html.contains("</head>") {
        html = html.replacen("</head>", &format!("{script}</head>"), 1);
    } else {
        html.insert_str(0, &script);
    }
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        html.into_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injection_adds_web_config() {
        let (_, body) = html_response(true, b"<html><head></head><body></body></html>".to_vec());
        let html = String::from_utf8(body).unwrap();
        assert!(html.contains("window.__PRISM_WEB__={auth:true}"));
        assert!(html.find("window.__PRISM_WEB__").unwrap() < html.find("</head>").unwrap());
    }
}
