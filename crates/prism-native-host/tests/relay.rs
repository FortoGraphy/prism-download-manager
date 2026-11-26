//! End-to-end tests of the built `prism-native-host` binary against a mock app
//! (a tiny std-only HTTP server) using `PRISM_DATA_DIR` for ipc.json.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

type Received = Arc<Mutex<Vec<(String, Option<String>, Value)>>>;

struct MockApp {
    port: u16,
    received: Received,
}

fn handle_conn(mut s: TcpStream, received: Received) {
    let mut reader = BufReader::new(s.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
    let mut len = 0usize;
    let mut auth = None;
    loop {
        let mut h = String::new();
        reader.read_line(&mut h).unwrap();
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        let (k, v) = h.split_once(':').unwrap();
        match k.to_ascii_lowercase().as_str() {
            "content-length" => len = v.trim().parse().unwrap(),
            "authorization" => auth = Some(v.trim().to_string()),
            _ => {}
        }
    }
    let mut body = vec![0; len];
    reader.read_exact(&mut body).unwrap();
    let (status, reply) = if auth.as_deref() != Some(&format!("Bearer {TOKEN}")) {
        (
            401,
            json!({"ok": false, "error": {"code": "unauthorized", "message": "bad token"}}),
        )
    } else if path == "/v1/health" {
        (200, json!({"ok": true, "version": "9.9.9"}))
    } else {
        let msg: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        received
            .lock()
            .unwrap()
            .push((path.clone(), auth.clone(), msg.clone()));
        match msg["type"].as_str() {
            Some("slow") => {
                thread::sleep(Duration::from_millis(600));
                (200, json!({"ok": true, "slow": true}))
            }
            Some("big") => (200, json!({"ok": true, "blob": "x".repeat(1_100_000)})),
            Some("batch") => (
                200,
                json!({"ok": true, "padLen": msg["pad"].as_str().map(str::len)}),
            ),
            Some("boom") => (
                500,
                json!({"ok": false, "error": {"code": "internal", "message": "kaput"}}),
            ),
            _ => (200, json!({"ok": true, "echo": msg})),
        }
    };
    let body = serde_json::to_vec(&reply).unwrap();
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(&body);
}

fn start_mock() -> MockApp {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let received = Arc::new(Mutex::new(Vec::new()));
    let r = received.clone();
    thread::spawn(move || {
        for s in l.incoming().flatten() {
            let r = r.clone();
            thread::spawn(move || handle_conn(s, r));
        }
    });
    MockApp { port, received }
}

fn ipc_json(port: u16) -> String {
    json!({"port": port, "token": TOKEN, "pid": std::process::id(), "version": "9.9.9"}).to_string()
}

fn spawn_host(data_dir: &Path, app_path: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_prism-native-host"))
        .arg("chrome-extension://mmcdeiglkbfbicdjboocmhehodogilfl/")
        .arg("--parent-window=0")
        .env("PRISM_DATA_DIR", data_dir)
        .env("PRISM_APP_PATH", app_path)
        .env("PRISM_HOST_DEBUG", "1")
        .env_remove("HTTP_PROXY")
        .env_remove("http_proxy")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn host")
}

fn send(w: &mut impl Write, v: &Value) {
    let b = serde_json::to_vec(v).unwrap();
    w.write_all(&(b.len() as u32).to_ne_bytes()).unwrap();
    w.write_all(&b).unwrap();
    w.flush().unwrap();
}

fn send_raw(w: &mut impl Write, b: &[u8]) {
    w.write_all(&(b.len() as u32).to_ne_bytes()).unwrap();
    w.write_all(b).unwrap();
    w.flush().unwrap();
}

fn recv(r: &mut impl Read) -> Option<Value> {
    let mut h = [0u8; 4];
    r.read_exact(&mut h).ok()?;
    let n = u32::from_ne_bytes(h) as usize;
    assert!(n < 1024 * 1024, "host sent a frame >= 1 MiB");
    let mut b = vec![0; n];
    r.read_exact(&mut b).unwrap();
    Some(serde_json::from_slice(&b).unwrap())
}

fn missing_app(dir: &Path) -> PathBuf {
    dir.join("no-such-prism-app.exe")
}

#[test]
fn relays_messages_long_lived_port() {
    let mock = start_mock();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ipc.json"), ipc_json(mock.port)).unwrap();
    let mut child = spawn_host(dir.path(), &missing_app(dir.path()));
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    // 1. download message → echoed with same id, latency measured
    let t = Instant::now();
    send(
        &mut stdin,
        &json!({"id": "m1", "type": "download", "url": "https://example.com/a.zip", "cookies": "a=b"}),
    );
    let r = recv(&mut stdout).unwrap();
    let first = t.elapsed();
    assert_eq!(r["id"], "m1");
    assert_eq!(r["ok"], true);
    assert_eq!(r["echo"]["url"], "https://example.com/a.zip");
    {
        let rec = mock.received.lock().unwrap();
        assert_eq!(rec[0].0, "/v1/message");
        assert_eq!(
            rec[0].1.as_deref(),
            Some(format!("Bearer {TOKEN}").as_str())
        );
        assert_eq!(rec[0].2["cookies"], "a=b");
    }

    // 2. second message on the same port (warm)
    let t = Instant::now();
    send(
        &mut stdin,
        &json!({"id": 2, "type": "hello", "version": "1.0.0", "browser": "chrome"}),
    );
    let r = recv(&mut stdout).unwrap();
    let warm = t.elapsed();
    assert_eq!(r["id"], 2);
    eprintln!("first reply {first:?}, warm reply {warm:?}");
    assert!(warm < Duration::from_millis(500));

    // 3. malformed JSON → error, host keeps running
    send_raw(&mut stdin, b"{not json");
    let r = recv(&mut stdout).unwrap();
    assert_eq!(r["id"], Value::Null);
    assert_eq!(r["error"]["code"], "bad_json");
    send(&mut stdin, &json!({"type": "hello"}));
    let r = recv(&mut stdout).unwrap();
    assert_eq!(r["error"]["code"], "bad_request");

    // 4. host-local ping
    send(&mut stdin, &json!({"id": "p", "type": "hostPing"}));
    let r = recv(&mut stdout).unwrap();
    assert_eq!(r["host"]["version"], env!("CARGO_PKG_VERSION"));
    // No data folder: its path holds the account name (GHSA-v59w-jfj8-7pf6).
    assert!(r["host"].get("dataDir").is_none(), "{r}");

    // 5. concurrency: a slow request does not block a fast one
    send(&mut stdin, &json!({"id": "slow", "type": "slow"}));
    send(&mut stdin, &json!({"id": "fast", "type": "status"}));
    let a = recv(&mut stdout).unwrap();
    let b = recv(&mut stdout).unwrap();
    assert_eq!(a["id"], "fast");
    assert_eq!(b["id"], "slow");

    // 6. oversized app reply → reply_too_large (frame stays < 1 MiB)
    send(&mut stdin, &json!({"id": "big", "type": "big"}));
    let r = recv(&mut stdout).unwrap();
    assert_eq!(r["error"]["code"], "reply_too_large");

    // 7. app error status with structured body is passed through
    send(&mut stdin, &json!({"id": "boom", "type": "boom"}));
    let r = recv(&mut stdout).unwrap();
    assert_eq!(r["ok"], false);
    assert_eq!(r["error"]["code"], "internal");

    // 8. large incoming message (2 MiB)
    let big = "y".repeat(2 * 1024 * 1024);
    send(
        &mut stdin,
        &json!({"id": "large", "type": "batch", "items": [], "pad": big}),
    );
    let r = recv(&mut stdout).unwrap();
    assert_eq!(r["id"], "large", "{r}");
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["padLen"], 2 * 1024 * 1024);

    // EOF → exit 0
    drop(stdin);
    assert!(recv(&mut stdout).is_none());
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(0));
}

#[test]
fn one_shot_and_app_not_running() {
    let dir = tempfile::tempdir().unwrap();
    // no ipc.json, noLaunch → app_not_running
    let mut child = spawn_host(dir.path(), &missing_app(dir.path()));
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    send(
        &mut stdin,
        &json!({"id": "a", "type": "status", "noLaunch": true}),
    );
    let r = recv(&mut stdout).unwrap();
    assert_eq!(r["error"]["code"], "app_not_running");
    // launch allowed, but the app binary is missing → app_unreachable (fast)
    let t = Instant::now();
    send(
        &mut stdin,
        &json!({"id": "b", "type": "download", "url": "https://x/y.zip"}),
    );
    let r = recv(&mut stdout).unwrap();
    assert_eq!(r["error"]["code"], "app_unreachable");
    assert!(t.elapsed() < Duration::from_secs(3));
    drop(stdin);
    assert_eq!(child.wait().unwrap().code(), Some(0));

    // stale ipc.json pointing at a closed port, noLaunch
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let dead = l.local_addr().unwrap().port();
    drop(l);
    std::fs::write(dir.path().join("ipc.json"), ipc_json(dead)).unwrap();
    let mut child = spawn_host(dir.path(), &missing_app(dir.path()));
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let t = Instant::now();
    send(
        &mut stdin,
        &json!({"id": "c", "type": "hello", "noLaunch": true}),
    );
    let r = recv(&mut stdout).unwrap();
    assert_eq!(r["error"]["code"], "app_not_running");
    eprintln!("stale port detection took {:?}", t.elapsed());
    drop(stdin);
    child.wait().unwrap();
}

/// Started for `caller` (the browser's arguments) with a working app behind it: whether the
/// host answers a status request, or exits at once.
fn answers_for(args: &[&str], extra_ids: Option<&str>) -> bool {
    let mock = start_mock();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ipc.json"), ipc_json(mock.port)).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_prism-native-host"));
    cmd.args(args)
        .env("PRISM_DATA_DIR", dir.path())
        .env("PRISM_APP_PATH", missing_app(dir.path()))
        .env_remove(prism_native_host::EXTRA_CALLERS_ENV)
        .env_remove("HTTP_PROXY")
        .env_remove("http_proxy")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(ids) = extra_ids {
        cmd.env(prism_native_host::EXTRA_CALLERS_ENV, ids);
    }
    let mut child = cmd.spawn().expect("spawn host");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    // The host may already be gone: a failed write is part of the answer.
    let body = serde_json::to_vec(&json!({"id": 1, "type": "status"})).unwrap();
    let _ = stdin
        .write_all(&(body.len() as u32).to_ne_bytes())
        .and_then(|_| stdin.write_all(&body))
        .and_then(|_| stdin.flush());
    let reply = recv(&mut stdout);
    drop(stdin);
    let status = child.wait().unwrap();
    match reply {
        Some(r) => r["ok"] == true,
        None => {
            assert!(
                !status.success(),
                "a turned-away caller exits with an error"
            );
            false
        }
    }
}

#[test]
fn only_prisms_extension_is_served() {
    // GHSA-v59w-jfj8-7pf6: an extension with the old Firefox ID (someone else's on Firefox
    // Add-ons) was let in by older manifests; the host turns away any caller but Prism's.
    let ff_manifest = "/home/u/.mozilla/native-messaging-hosts/dev.prism.host.json";
    assert!(answers_for(
        &[ff_manifest, prism_native_host::FIREFOX_EXTENSION_ID],
        None
    ));
    assert!(answers_for(
        &[
            "chrome-extension://mmcdeiglkbfbicdjboocmhehodogilfl/",
            "--parent-window=0"
        ],
        None
    ));
    assert!(!answers_for(&[ff_manifest, "prism@prism.dev"], None));
    assert!(!answers_for(
        &["chrome-extension://abcdefghijklmnopabcdefghijklmnop/"],
        None
    ));
    // An extension built with an ID of its own, named in PRISM_EXTENSION_IDS.
    assert!(answers_for(
        &[ff_manifest, "me@example.org"],
        Some("me@example.org")
    ));
}

#[test]
fn wrong_token_is_unauthorized() {
    let mock = start_mock();
    let dir = tempfile::tempdir().unwrap();
    let bad = json!({"port": mock.port, "token": "nope", "pid": 1, "version": "x"}).to_string();
    std::fs::write(dir.path().join("ipc.json"), bad).unwrap();
    let mut child = spawn_host(dir.path(), &missing_app(dir.path()));
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    send(&mut stdin, &json!({"id": 1, "type": "status"}));
    assert_eq!(recv(&mut stdout).unwrap()["error"]["code"], "unauthorized");
    drop(stdin);
    child.wait().unwrap();
}

/// The host launches the "app" (a script that publishes ipc.json for the mock)
/// when ipc.json is missing, then delivers the message.
#[test]
fn launches_app_when_missing() {
    let mock = start_mock();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ipc.ready"), ipc_json(mock.port)).unwrap();
    let marker = dir.path().join("launched-args.txt");
    #[cfg(windows)]
    let app = {
        let p = dir.path().join("fake-prism.cmd");
        std::fs::write(
            &p,
            "@echo off\r\necho %*> \"%~dp0launched-args.txt\"\r\ncopy /y \"%~dp0ipc.ready\" \"%~dp0ipc.json\" >nul\r\n",
        )
        .unwrap();
        p
    };
    #[cfg(unix)]
    let app = {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.path().join("fake-prism.sh");
        std::fs::write(&p, "#!/bin/sh\nd=$(dirname \"$0\")\necho \"$@\" > \"$d/launched-args.txt\"\ncp \"$d/ipc.ready\" \"$d/ipc.json\"\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    };
    let mut child = spawn_host(dir.path(), &app);
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let t = Instant::now();
    send(
        &mut stdin,
        &json!({"id": "L", "type": "download", "url": "https://x/launch.zip"}),
    );
    let r = recv(&mut stdout).unwrap();
    assert_eq!(r["id"], "L");
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["echo"]["url"], "https://x/launch.zip");
    eprintln!("launch + deliver took {:?}", t.elapsed());
    assert!(
        std::fs::read_to_string(&marker)
            .unwrap()
            .contains("--background")
    );
    drop(stdin);
    child.wait().unwrap();
}
