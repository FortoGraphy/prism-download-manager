//! The native messaging relay loop.
//!
//! Reads frames from stdin, handles each on its own thread (so a slow
//! `probeVideo` never blocks a `download` behind it on a long-lived
//! `connectNative` port) and writes replies to stdout under a lock. Works the
//! same for one-shot `sendNativeMessage` (one frame, reply, EOF). On EOF the
//! host finishes in-flight requests and exits 0.

use serde_json::{Value, json};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::framing::{self, FrameError, MAX_OUTGOING};
use crate::ipc::AppClient;
use crate::protocol::{self, codes};
use crate::{log_debug, log_error, log_info, paths};

/// Per-type timeout for the app's answer.
pub fn timeout_for(msg_type: &str) -> Duration {
    match msg_type {
        // yt-dlp format resolution can take a while on slow sites.
        "probeVideo" => Duration::from_secs(90),
        _ => Duration::from_secs(20),
    }
}

type Out = Arc<Mutex<Box<dyn Write + Send>>>;

fn write_reply(out: &Out, reply: &Value) {
    let mut bytes = serde_json::to_vec(reply).unwrap_or_default();
    if bytes.len() > MAX_OUTGOING {
        let id = reply.get("id").cloned().unwrap_or(Value::Null);
        let err = protocol::error_reply(
            &id,
            codes::REPLY_TOO_LARGE,
            format!("reply of {} bytes exceeds 1 MiB", bytes.len()),
        );
        bytes = serde_json::to_vec(&err).unwrap_or_default();
    }
    let mut w = out.lock().unwrap_or_else(|e| e.into_inner());
    if let Err(e) = framing::write_frame(&mut *w, &bytes) {
        log_error!("writing reply failed: {e}");
    }
}

/// Handles one raw message and returns the reply object.
pub fn handle_message(client: &AppClient, body: &[u8]) -> Value {
    let msg: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => {
            return protocol::error_reply(
                &Value::Null,
                codes::BAD_JSON,
                format!("invalid JSON: {e}"),
            );
        }
    };
    let (id, ty) = match protocol::validate(&msg) {
        Ok(x) => x,
        Err((id, why)) => return protocol::error_reply(&id, codes::BAD_REQUEST, why),
    };
    if ty == "hostPing" {
        return json!({
            "id": id,
            "ok": true,
            "host": {
                "version": crate::VERSION,
                "dataDir": paths::data_dir().map(|d| d.to_string_lossy().into_owned()),
            }
        });
    }
    let allow_launch = !msg
        .get("noLaunch")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let started = Instant::now();
    let reply = match client.send(body, timeout_for(&ty), allow_launch) {
        Ok(v) => protocol::attach_id(&id, v),
        Err((code, message)) => {
            log_info!("{ty} {id} failed: {code} {message}");
            protocol::error_reply(&id, code, message)
        }
    };
    log_debug!("{ty} {id} answered in {:?}", started.elapsed());
    reply
}

/// Runs the relay until stdin closes. Returns the process exit code.
pub fn run(caller: Option<String>) -> i32 {
    log_info!(
        "host {} started (caller: {})",
        crate::VERSION,
        caller.as_deref().unwrap_or("-")
    );
    let host_exe =
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from(crate::HOST_BINARY_NAME));
    let client = Arc::new(AppClient::new(
        paths::ipc_file(),
        paths::app_executable(&host_exe),
    ));
    let out: Out = Arc::new(Mutex::new(Box::new(io::stdout())));
    watch_for_update(host_exe, out.clone());
    run_with(client, io::stdin().lock(), out)
}

/// The browser keeps the host running for as long as the extension is connected. When an
/// update replaces the executable, exit so the browser starts the new version on the next
/// message (the extension reconnects by itself).
fn watch_for_update(exe: PathBuf, out: Out) {
    let stamp = |p: &PathBuf| {
        std::fs::metadata(p)
            .ok()
            .map(|m| (m.len(), m.modified().ok()))
    };
    let initial = stamp(&exe);
    if initial.is_none() {
        return;
    }
    let _ = thread::Builder::new()
        .name("update-watch".into())
        .spawn(move || {
            loop {
                thread::sleep(std::time::Duration::from_secs(15));
                if stamp(&exe) != initial {
                    log_info!(
                        "host executable was updated; exiting so the browser starts the new one"
                    );
                    // Never cut a reply in half: wait until no reply is being written.
                    let _guard = out.lock();
                    std::process::exit(0);
                }
            }
        });
}

const MAX_CONCURRENT_WORKERS: usize = 4;

/// The loop, generic over its input (used by tests).
pub fn run_with<R: io::Read>(client: Arc<AppClient>, mut input: R, out: Out) -> i32 {
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let rx = Arc::new(Mutex::new(rx));
    let mut workers = Vec::with_capacity(MAX_CONCURRENT_WORKERS);

    for _ in 0..MAX_CONCURRENT_WORKERS {
        let (c, o, r) = (client.clone(), out.clone(), rx.clone());
        workers.push(thread::spawn(move || loop {
            let msg = {
                let Ok(guard) = r.lock() else { break };
                guard.recv()
            };
            match msg {
                Ok(body) => {
                    let reply = handle_message(&c, &body);
                    write_reply(&o, &reply);
                }
                Err(_) => break,
            }
        }));
    }
    drop(rx);

    let code = loop {
        match framing::read_frame(&mut input) {
            Ok(Some(body)) => {
                if tx.send(body).is_err() {
                    break 1;
                }
            }
            Ok(None) => break 0,
            Err(FrameError::TooLarge(n)) => {
                // The stream is desynchronized after an oversized header: report and quit.
                log_error!("incoming frame of {n} bytes exceeds 64 MiB");
                write_reply(
                    &out,
                    &protocol::error_reply(
                        &Value::Null,
                        codes::TOO_LARGE,
                        format!("message of {n} bytes exceeds 64 MiB"),
                    ),
                );
                break 1;
            }
            Err(FrameError::Truncated) => {
                log_error!("stdin closed in the middle of a frame");
                break 0;
            }
            Err(FrameError::Io(e)) => {
                log_error!("stdin read error: {e}");
                break 0;
            }
        }
    };
    drop(tx);
    for w in workers {
        let _ = w.join();
    }
    log_debug!("host exiting with {code}");
    code
}
