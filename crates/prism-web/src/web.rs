use crate::auth::Auth;
use crate::commands;
use crate::ipc;
use axum::extract::{Path, Request, State};
use axum::http::uri::Authority;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream::{Stream, unfold};
use prism_core::events::CoreEvent;
use prism_core::manager::Core;
use serde_json::{Value, json};
use std::convert::Infallible;
use std::path::{Component, PathBuf};
use std::sync::Arc;
use tokio::sync::watch;

include!(concat!(env!("OUT_DIR"), "/assets.rs"));

#[derive(Clone)]
struct AppState {
    core: Core,
    auth: Auth,
    shutdown: watch::Receiver<bool>,
}

pub fn router(core: Core, auth: Auth, ipc_auth: ipc::IpcAuth, shutdown: watch::Receiver<bool>) -> Router {
    let state = Arc::new(AppState {
        core: core.clone(),
        auth,
        shutdown,
    });
    let web = Router::new()
        .route("/", get(index))
        .route("/api/config", get(config))
        .route("/api/invoke", post(invoke))
        .route("/api/events", get(events))
        .route("/login.html", get(login))
        .route("/{*path}", get(asset))
        .layer(middleware::from_fn_with_state(state.clone(), origin_guard))
        .with_state(state);
    // The native-host IPC routes are merged separately and keep their token auth only.
    web.merge(ipc::router(core, ipc_auth).layer(middleware::from_fn(extension_cors)))
}

/// With no bearer token, a browser must not be able to reach the server through a
/// rebinding hostname. Token-authenticated deployments (Docker) stay untouched.
async fn origin_guard(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    if state.auth.enabled() || web_origin_allowed(&headers) {
        next.run(request).await
    } else {
        (StatusCode::FORBIDDEN, "forbidden origin").into_response()
    }
}

fn web_origin_allowed(headers: &HeaderMap) -> bool {
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<Authority>().ok())
    else {
        return false;
    };
    if !crate::auth::is_loopback_host(host.host()) {
        return false;
    }
    if let Some(site) = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok())
        && !matches!(site, "same-origin" | "none")
    {
        return false;
    }
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        let Ok(uri) = origin.parse::<Uri>() else {
            return false;
        };
        if !matches!(uri.scheme_str(), Some("http" | "https"))
            || uri.authority().is_none_or(|a| !a.as_str().eq_ignore_ascii_case(host.as_str()))
        {
            return false;
        }
    }
    true
}

const CHROME_EXTENSION_ORIGIN: &str = "chrome-extension://mmcdeiglkbfbicdjboocmhehodogilfl";

/// Only the extension's own origins may call `/v1/*` from a browser. Native hosts
/// send no Origin at all; Firefox uses a per-install UUID in its origin.
fn extension_origin_allowed(origin: Option<&str>) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    if origin == CHROME_EXTENSION_ORIGIN {
        return true;
    }
    let Ok(uri) = origin.parse::<Uri>() else {
        return false;
    };
    uri.scheme_str() == Some("moz-extension")
        && uri.authority().is_some()
        && matches!(uri.path(), "" | "/")
        && uri.query().is_none()
}

fn preflight(origin: Option<&str>) -> Response {
    with_cors(StatusCode::NO_CONTENT.into_response(), origin)
}

fn with_cors(mut response: Response, origin: Option<&str>) -> Response {
    let headers = response.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Authorization, Content-Type"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    headers.insert(header::VARY, HeaderValue::from_static("Origin"));
    if let Some(origin) = origin
        && let Ok(value) = HeaderValue::from_str(origin)
    {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
    }
    response
}

async fn extension_cors(headers: HeaderMap, request: Request, next: Next) -> Response {
    let origin = match headers.get(header::ORIGIN) {
        None => None,
        Some(value) => match value.to_str().ok().filter(|o| extension_origin_allowed(Some(o))) {
            Some(origin) => Some(origin),
            None => return (StatusCode::FORBIDDEN, "forbidden origin").into_response(),
        },
    };
    if request.method() == Method::OPTIONS {
        return preflight(origin);
    }
    with_cors(next.run(request).await, origin)
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
    let shutdown = state.shutdown.clone();
    let stream = unfold((rx, core, shutdown), |(mut rx, core, mut shutdown)| async move {
        let event = tokio::select! {
            biased;
            _ = shutdown.wait_for(|shutting_down| *shutting_down) => return None,
            event = rx.recv() => event,
        };
        match event {
            Ok(event) => {
                let data = serde_json::to_string(&event).unwrap_or_else(|_| "{}".into());
                Some((Ok(Event::default().data(data)), (rx, core, shutdown)))
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                // A slow browser missed events: resend the full list so rows recover.
                let resync = CoreEvent::ItemsAdded(core.bootstrap().items);
                let data = serde_json::to_string(&resync).unwrap_or_else(|_| "{}".into());
                Some((Ok(Event::default().data(data)), (rx, core, shutdown)))
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => None,
        }
    });
    Ok(Sse::new(stream))
}

async fn login(uri: Uri) -> ([(header::HeaderName, &'static str); 1], Vec<u8>) {
    let next = serde_json::to_string(&next_path(&uri)).unwrap_or_else(|_| "\"/\"".into());
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
const next = __NEXT__;
document.getElementById('f').addEventListener('submit', async e => {
  e.preventDefault();
  const token = document.getElementById('token').value.trim();
  const r = await fetch('/api/config', {headers:{Authorization:'Bearer '+token}}).catch(() => null);
  if (r && r.ok) { sessionStorage.setItem('prism-web-token', token); location.href = next; }
  else document.getElementById('err').textContent = 'Wrong token. Try again.';
});
</script>"#;
    let body = String::from_utf8_lossy(body)
        .replace("__NEXT__", &next)
        .into_bytes();
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], body)
}

/// Resolve the post-login target server-side so a crafted query cannot turn it into
/// a script URL or a protocol-relative off-origin redirect.
fn next_path(uri: &Uri) -> String {
    uri.query()
        .and_then(|query| {
            query.split('&').find_map(|part| {
                let (key, value) = part.split_once('=')?;
                (key == "next").then_some(value)
            })
        })
        .and_then(decode_component)
        .filter(|path| {
            path.starts_with('/')
                && !path.starts_with("//")
                && !path.contains(['\\', '<', '>', '"', '\''])
                && !path.chars().any(char::is_control)
        })
        .unwrap_or_else(|| "/".into())
}

fn decode_component(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            let hi = (hex[0] as char).to_digit(16)?;
            let lo = (hex[1] as char).to_digit(16)?;
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
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
    use axum::body::HttpBody as _;

    #[test]
    fn injection_adds_web_config() {
        let (_, body) = html_response(true, b"<html><head></head><body></body></html>".to_vec());
        let html = String::from_utf8(body).unwrap();
        assert!(html.contains("window.__PRISM_WEB__={auth:true}"));
        assert!(html.find("window.__PRISM_WEB__").unwrap() < html.find("</head>").unwrap());
    }

    #[tokio::test]
    async fn router_merges_ipc_routes_without_overlap() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::start(prism_core::manager::CoreOptions {
            data_dir: dir.path().to_path_buf(),
            ipc: false,
            clipboard: false,
        })
        .await
        .unwrap();
        let auth = crate::auth::Auth::new(dir.path(), 8080, true, None).unwrap();
        let ipc_auth = crate::ipc::IpcAuth::new(dir.path().join("ipc.json"), None);
        let (_, shutdown_rx) = watch::channel(false);
        let _ = router(core.clone(), auth, ipc_auth, shutdown_rx);
        core.shutdown().await;
    }

    #[tokio::test]
    async fn events_stream_ends_on_shutdown_signal() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::start(prism_core::manager::CoreOptions {
            data_dir: dir.path().to_path_buf(),
            ipc: false,
            clipboard: false,
        })
        .await
        .unwrap();
        let auth = crate::auth::Auth::new(dir.path(), 8080, true, None).unwrap();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let state = Arc::new(AppState {
            core: core.clone(),
            auth,
            shutdown: shutdown_rx,
        });
        let response = events(State(state), HeaderMap::new()).await.unwrap().into_response();
        let mut body = Box::pin(response.into_body());
        let before = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            std::future::poll_fn(|cx| body.as_mut().poll_frame(cx)),
        )
        .await;
        assert!(!matches!(before, Ok(None)), "stream ended before shutdown");

        shutdown_tx.send(true).unwrap();
        let next = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            std::future::poll_fn(|cx| body.as_mut().poll_frame(cx)),
        )
        .await
        .expect("stream did not end after shutdown");
        assert!(next.is_none());
        core.shutdown().await;
    }

    #[test]
    fn extension_cors_allows_only_extension_origins() {
        assert!(extension_origin_allowed(None));
        assert!(extension_origin_allowed(Some(CHROME_EXTENSION_ORIGIN)));
        assert!(extension_origin_allowed(Some("moz-extension://12345678-1234")));
        assert!(!extension_origin_allowed(Some("chrome-extension://other")));
        assert!(!extension_origin_allowed(Some("http://evil.example")));
        assert!(!extension_origin_allowed(Some("moz-extension://uuid/path")));
        assert!(!extension_origin_allowed(Some("null")));
    }

    #[test]
    fn extension_cors_preflight_and_response_headers() {
        let res = preflight(Some(CHROME_EXTENSION_ORIGIN));
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            res.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
            CHROME_EXTENSION_ORIGIN
        );
        assert_eq!(
            res.headers().get(header::ACCESS_CONTROL_ALLOW_HEADERS).unwrap(),
            "Authorization, Content-Type"
        );
        assert_eq!(
            res.headers().get(header::ACCESS_CONTROL_ALLOW_METHODS).unwrap(),
            "GET, POST, OPTIONS"
        );

        let native = with_cors((StatusCode::OK, "ok").into_response(), None);
        assert!(!native.headers().contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));
        assert!(native.headers().contains_key(header::ACCESS_CONTROL_ALLOW_METHODS));
    }

    #[test]
    fn no_auth_requires_loopback_origin() {
        let headers = |host: &str, origin: Option<&str>, site: Option<&str>| {
            let mut headers = HeaderMap::new();
            headers.insert(header::HOST, host.parse().unwrap());
            if let Some(origin) = origin {
                headers.insert(header::ORIGIN, origin.parse().unwrap());
            }
            if let Some(site) = site {
                headers.insert("sec-fetch-site", site.parse().unwrap());
            }
            headers
        };
        assert!(web_origin_allowed(&headers("127.0.0.1:8080", None, None)));
        assert!(web_origin_allowed(&headers(
            "localhost:8080",
            Some("http://localhost:8080"),
            Some("same-origin")
        )));
        assert!(!web_origin_allowed(&headers("evil.example:8080", None, None)));
        assert!(!web_origin_allowed(&headers(
            "127.0.0.1:8080",
            Some("http://evil.example"),
            None
        )));
        assert!(!web_origin_allowed(&headers(
            "127.0.0.1:8080",
            None,
            Some("cross-site")
        )));
        assert!(!web_origin_allowed(&headers(
            "127.0.0.1:8080",
            Some("null"),
            None
        )));
        assert!(!web_origin_allowed(&HeaderMap::new()));
    }

    #[tokio::test]
    async fn login_accepts_only_same_origin_next_paths() {
        for (query, expected) in [
            ("next=%2Fsettings", "/settings"),
            ("next=%2F%23%2Ftools%2Fshrink", "/#/tools/shrink"),
            ("next=%2F%2Fevil.example", "/"),
            ("next=%5C%5Cevil.example", "/"),
            ("next=javascript%3Aalert(1)", "/"),
            ("next=https%3A%2F%2Fevil.example", "/"),
            ("next=%2F%3Cscript%3E", "/"),
            ("next=%2Fx%0Aevil", "/"),
        ] {
            let uri: Uri = format!("/login.html?{query}").parse().unwrap();
            assert_eq!(next_path(&uri), expected, "{query}");
        }
        let uri: Uri = "/login.html?next=javascript%3Aalert(1)".parse().unwrap();
        let (_, body) = login(uri).await;
        let html = String::from_utf8(body).unwrap();
        assert!(html.contains("const next = \"/\";"));
        assert!(!html.contains("URLSearchParams"));
    }
}
