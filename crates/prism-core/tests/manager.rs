//! End-to-end tests of the download manager against the local test server:
//! adding, capture dialog flow, pause/resume across an app restart, queues, browser IPC
//! messages and the trash.

use prism_core::events::{CaptureSource, CoreEvent};
use prism_core::ipc::IpcHandler;
use prism_core::manager::{CaptureDecision, Core, CoreOptions, NewItem, StartMode, VideoDecision};
use prism_core::model::{DownloadItem, ItemKind, Queue, Status};
use prism_core::settings::ConflictAction;
use prism_testserver::{HlsFixture, ServerConfig, TestServer, expected_sha256, sha256_hex};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::broadcast;

async fn core_in(dir: &Path) -> Core {
    let core = Core::start(CoreOptions {
        data_dir: dir.join("data"),
        ipc: false,
        clipboard: false,
    })
    .await
    .unwrap();
    core.update_settings(&serde_json::json!({
        "downloads": {"defaultDir": dir.join("dl"), "useCategoryFolders": false},
        "general": {"showCaptureDialog": false},
        "antivirus": {"enabled": false},
    }))
    .unwrap();
    core
}

fn sha256_file(path: &Path) -> String {
    let data = std::fs::read(path).unwrap();
    sha256_hex(&data)
}

/// Wait until `pred` matches an event (with timeout).
async fn wait_event<T>(
    rx: &mut broadcast::Receiver<CoreEvent>,
    secs: u64,
    mut pred: impl FnMut(&CoreEvent) -> Option<T>,
) -> T {
    tokio::time::timeout(Duration::from_secs(secs), async {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    if let Some(v) = pred(&ev) {
                        return v;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(e) => panic!("event stream closed: {e}"),
            }
        }
    })
    .await
    .expect("timed out waiting for event")
}

async fn wait_completed(rx: &mut broadcast::Receiver<CoreEvent>, id: &str) -> PathBuf {
    wait_event(rx, 60, |ev| match ev {
        CoreEvent::Completed(item) if item.id == id => Some(item.path()),
        CoreEvent::Failed(item) if item.id == id => panic!("download failed: {:?}", item.error),
        _ => None,
    })
    .await
}

/// Wait until a running download has received at least `bytes` (a fixed sleep is too short
/// on a busy CI runner and lets a fast machine get too far).
async fn wait_downloaded(core: &Core, id: &str, bytes: u64) {
    tokio::time::timeout(Duration::from_secs(30), async {
        while core.item(id).unwrap().downloaded < bytes {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("no progress");
}

/// Wait until the item's job has really stopped and left it paused with a resume point.
/// Pausing (or trashing) only asks the job to stop: it flushes and reports back in its own
/// time, and until then a periodic checkpoint can already have stored a resume point.
async fn wait_paused(core: &Core, id: &str) -> DownloadItem {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let details = core.item_details(id).expect("item exists");
        let running = details.progress.is_some();
        let item = details.item;
        if !running && item.status == Status::Paused && item.resume.is_some() {
            return item;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "not paused with a resume point: status {:?}, job running {running}, resume {}",
            item.status,
            item.resume.is_some()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn add_download_and_verify_content() {
    let server = TestServer::start(ServerConfig::default()).await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let core = core_in(tmp.path()).await;
    let mut rx = core.subscribe();
    let size = 8 << 20;
    let url = server.url(&format!("/file/data.bin?size={size}&seed=7"));
    let items = core
        .add_items(
            vec![NewItem {
                url,
                ..Default::default()
            }],
            StartMode::Now,
        )
        .unwrap();
    let path = wait_completed(&mut rx, &items[0].id).await;
    assert_eq!(sha256_file(&path), expected_sha256(7, size));
    let item = core.item(&items[0].id).unwrap();
    assert_eq!(item.status, Status::Completed);
    assert_eq!(item.size, Some(size));
    assert!(
        server.stats().total_connections() > 1,
        "expected a multi-connection download"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn capture_dialog_flow_uses_probed_name() {
    let server = TestServer::start(ServerConfig::default()).await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let core = core_in(tmp.path()).await;
    let mut rx = core.subscribe();
    let url = server
        .url("/file/x?size=300000&seed=3&cd=attachment%3B%20filename%3D%22report%20final.pdf%22");
    let info = core.capture(
        NewItem {
            url,
            ..Default::default()
        },
        CaptureSource::Browser,
    );
    assert!(info.probing);
    let probed = wait_event(&mut rx, 20, |ev| match ev {
        CoreEvent::CaptureUpdated(c) if c.id == info.id => Some(c.clone()),
        _ => None,
    })
    .await;
    assert_eq!(probed.file_name, "report final.pdf");
    assert_eq!(probed.size, Some(300_000));
    assert_eq!(probed.category, "documents");
    assert_eq!(probed.resumable, Some(true));
    let item = core
        .confirm_capture(CaptureDecision {
            id: info.id.clone(),
            description: Some("Q3".into()),
            ..Default::default()
        })
        .unwrap();
    let path = wait_completed(&mut rx, &item.id).await;
    assert_eq!(path.file_name().unwrap(), "report final.pdf");
    assert_eq!(sha256_file(&path), expected_sha256(3, 300_000));
    assert_eq!(core.item(&item.id).unwrap().description, "Q3");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_then_resume_after_app_restart() {
    let server = TestServer::start(ServerConfig::default()).await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let size: u64 = 6 << 20;
    // 512 KiB/s per connection: slow enough to pause mid-way.
    let url = server.url(&format!(
        "/file/big.bin?size={size}&seed=11&throttle=524288"
    ));
    let id = {
        let core = core_in(tmp.path()).await;
        let items = core
            .add_items(
                vec![NewItem {
                    url,
                    ..Default::default()
                }],
                StartMode::Now,
            )
            .unwrap();
        let id = items[0].id.clone();
        wait_downloaded(&core, &id, 512 * 1024).await;
        core.pause_items(std::slice::from_ref(&id));
        let item = wait_paused(&core, &id).await;
        assert!(
            item.downloaded > 0 && item.downloaded < size,
            "downloaded {}",
            item.downloaded
        );
        core.shutdown().await;
        id
    };
    // A fresh core over the same data directory = app restart.
    let core = core_in(tmp.path()).await;
    let item = core.item(&id).expect("item persisted");
    assert_eq!(item.status, Status::Paused);
    let before = item.downloaded;
    assert!(before > 0);
    server.reset_stats();
    let mut rx = core.subscribe();
    core.start_items(std::slice::from_ref(&id));
    let path = wait_completed(&mut rx, &id).await;
    assert_eq!(sha256_file(&path), expected_sha256(11, size));
    // The resumed run must not re-download what was already there.
    let resent: u64 = server
        .stats()
        .connections
        .iter()
        .map(|c| c.bytes_sent)
        .sum();
    assert!(
        resent < size - before / 2,
        "resent {resent} bytes, had {before}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queue_respects_concurrency_and_finishes() {
    let server = TestServer::start(ServerConfig::default()).await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let core = core_in(tmp.path()).await;
    let mut queue: Queue = core.queues().into_iter().find(|q| q.id == "main").unwrap();
    queue.max_concurrent = 2;
    core.upsert_queue(queue).unwrap();
    let entries: Vec<NewItem> = (0..4)
        .map(|i| NewItem {
            url: server.url(&format!(
                "/file/q{i}.bin?size=1500000&seed={i}&throttle=1048576"
            )),
            ..Default::default()
        })
        .collect();
    let items = core
        .add_items(entries, StartMode::Queue("main".into()))
        .unwrap();
    assert!(
        items
            .iter()
            .all(|i| core.item(&i.id).unwrap().status == Status::Idle)
    );
    let mut rx = core.subscribe();
    core.start_queue("main");
    let mut max_active = 0;
    let finished = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            match rx.recv().await {
                Ok(CoreEvent::Progress(list)) => max_active = max_active.max(list.len()),
                Ok(CoreEvent::QueueFinished { queue_id, .. }) if queue_id == "main" => break true,
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break false,
            }
        }
    })
    .await
    .expect("queue did not finish");
    assert!(finished);
    assert!(max_active <= 2, "queue ran {max_active} downloads at once");
    for (i, item) in items.iter().enumerate() {
        let item = core.item(&item.id).unwrap();
        assert_eq!(item.status, Status::Completed);
        assert_eq!(
            sha256_file(&item.path()),
            expected_sha256(i as u64, 1_500_000)
        );
    }
    assert!(
        !core
            .queues()
            .iter()
            .find(|q| q.id == "main")
            .unwrap()
            .running
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_empty_queue_run_does_not_run_its_finish_action() {
    // #41: "Run now" on a queue with nothing to download put the computer to sleep at once.
    // (The finish action here is the queue sound, so a failure only plays a sound.)
    let tmp = tempfile::tempdir().unwrap();
    let core = core_in(tmp.path()).await;
    let mut queue: Queue = core.queues().into_iter().find(|q| q.id == "main").unwrap();
    queue.schedule.on_complete = prism_core::model::PostAction::Sound;
    core.upsert_queue(queue).unwrap();
    let mut rx = core.subscribe();
    core.start_queue("main");
    let mut finished = false;
    let mut finish_action = false;
    let _ = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            match rx.recv().await {
                Ok(CoreEvent::QueueFinished { queue_id, .. }) if queue_id == "main" => {
                    finished = true
                }
                Ok(CoreEvent::PostActionPending { .. }) => finish_action = true,
                Ok(CoreEvent::PlaySound { kind }) if kind == "queue" => finish_action = true,
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
    })
    .await;
    assert!(finished, "the queue says it has nothing to download");
    assert!(!finish_action, "the finish action ran for an empty queue");
    let main = core.queues().into_iter().find(|q| q.id == "main").unwrap();
    assert!(!main.running);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn browser_messages_follow_protocol() {
    let server = TestServer::start(ServerConfig::default()).await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let core = core_in(tmp.path()).await;
    let hello = core
        .handle(serde_json::json!({"id": "1", "type": "hello", "protocol": 1, "browser": "chrome"}))
        .await;
    assert_eq!(hello["ok"], true);
    assert_eq!(hello["protocol"], 1);
    assert!(
        hello["settings"]["extensions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e == "ZIP")
    );

    let mut rx = core.subscribe();
    let url = server.url("/file/pkg.zip?size=200000&seed=5");
    let reply = core
        .handle(serde_json::json!({
            "id": "2", "type": "download", "url": url, "referrer": "https://example.com/page",
            "cookies": "session=abc", "userAgent": "TestBrowser/1.0", "filename": "pkg.zip",
            "fileSize": 200000, "method": "GET", "source": "capture",
        }))
        .await;
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["accepted"], true);
    let id = reply["downloadId"].as_str().unwrap().to_string();
    let path = wait_completed(&mut rx, &id).await;
    assert_eq!(sha256_file(&path), expected_sha256(5, 200_000));
    // Browser context is forwarded to the server.
    let log = server.stats().requests;
    assert!(
        log.iter()
            .all(|r| r.cookie.as_deref() == Some("session=abc"))
    );
    assert!(
        log.iter()
            .all(|r| r.referer.as_deref() == Some("https://example.com/page"))
    );
    assert!(
        log.iter()
            .all(|r| r.user_agent.as_deref() == Some("TestBrowser/1.0"))
    );

    let batch = core
        .handle(serde_json::json!({"id": "3", "type": "batch", "pageUrl": "https://example.com", "title": "t",
            "kind": "links", "items": [{"url": "https://example.com/a.zip"}, {"url": "https://example.com/b.pdf"}]}))
        .await;
    assert_eq!(batch["count"], 2);
    let unknown = core
        .handle(serde_json::json!({"id": "4", "type": "nope"}))
        .await;
    assert_eq!(unknown["ok"], false);
    assert_eq!(unknown["error"]["code"], "bad_request");
}

/// Picking a quality from the video panel (a sniffed stream or a probed format) must open the
/// same "Download File" dialog other captures do, not start downloading on its own — the user
/// may want to rename the file or change where it saves first.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn video_quality_pick_opens_a_capture_dialog_instead_of_starting() {
    let server = TestServer::start(ServerConfig::default()).await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let core = core_in(tmp.path()).await;
    core.update_settings(&serde_json::json!({"general": {"showCaptureDialog": true}}))
        .unwrap();
    let mut rx = core.subscribe();
    let url = server.url("/file/clip.mp4?size=1000");
    let reply = core
        .handle(serde_json::json!({
            "id": "1", "type": "video", "pageUrl": "https://example.com/watch",
            "streams": [{"url": url, "kind": "direct", "quality": "720p"}],
        }))
        .await;
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["accepted"], true);
    let info = wait_event(&mut rx, 5, |ev| match ev {
        CoreEvent::CaptureRequested(info) => Some(info.clone()),
        _ => None,
    })
    .await;
    // Pending confirmation, not an in-progress download.
    assert!(core.pending_capture(&info.id).is_some());
    assert!(core.item(&info.id).is_none());
    // Confirming it starts the download, same as any other capture.
    let decision = CaptureDecision {
        id: info.id,
        mode: StartMode::Now,
        ..Default::default()
    };
    assert!(core.confirm_capture(decision).is_ok());
}

/// A format picked in the video menu of a site Prism reads (YouTube and the like) opens the
/// Download video window with that quality picked ("Only a part", the folder, the options are
/// there), not the plain file dialog; the download keeps the tab's session (cookies, user agent).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_picked_video_format_opens_the_video_window_with_it_picked() {
    let tmp = tempfile::tempdir().unwrap();
    let core = core_in(tmp.path()).await;
    core.update_settings(&serde_json::json!({"general": {"showCaptureDialog": true}}))
        .unwrap();
    let mut rx = core.subscribe();
    let page = "https://www.youtube.com/watch?v=jNQXAC9IVRw";
    let reply = core
        .handle(serde_json::json!({
            "id": "1", "type": "video", "pageUrl": page, "title": "Me at the zoo",
            "formatId": "v720", "formatLabel": "720p", "streams": [],
            "cookies": "PREF=1", "userAgent": "test UA",
        }))
        .await;
    assert_eq!(reply["accepted"], true, "{reply}");
    let v = wait_event(&mut rx, 5, |ev| match ev {
        CoreEvent::VideoRequested(v) => Some(v.clone()),
        CoreEvent::CaptureRequested(_) => panic!("the plain file dialog opened"),
        _ => None,
    })
    .await;
    assert_eq!(v.page_url, page);
    assert_eq!(v.title.as_deref(), Some("Me at the zoo"));
    assert_eq!(v.preselected.as_deref(), Some("v720"));
    assert_eq!(reply["downloadId"], v.id.as_str());
    assert!(
        core.bootstrap().items.is_empty(),
        "nothing starts before the window"
    );
    // The session stays in the core: the window's copy has none.
    let shown = serde_json::to_value(core.pending_video(&v.id).unwrap()).unwrap();
    assert!(shown.get("browser").is_none(), "{shown}");
    // Confirmed in the window (which sends no context): the tab's cookies come along.
    let decision: VideoDecision = serde_json::from_value(serde_json::json!({
        "captureId": v.id, "pageUrl": page, "title": "Me at the zoo",
        "choice": {"id": "v720", "kind": "video", "label": "720p", "subLabel": "", "container": "mp4", "height": 720},
        "options": {"container": "mp4"},
        "mode": {"type": "later"},
    }))
    .unwrap();
    let items = core.confirm_video(decision).unwrap();
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item.kind, ItemKind::Video);
    let request = item.request.clone().unwrap();
    assert_eq!(request["pageUrl"], page);
    assert_eq!(request["choice"]["id"], "v720");
    assert_eq!(request["context"]["cookies"], "PREF=1");
    assert_eq!(request["context"]["userAgent"], "test UA");
    assert!(
        core.pending_video(&v.id).is_none(),
        "the request is used up"
    );
}

/// With "Show the Download File dialog" off, a format picked in the video menu starts right away,
/// as before.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_picked_video_format_starts_right_away_without_the_confirmation() {
    let tmp = tempfile::tempdir().unwrap();
    let core = core_in(tmp.path()).await;
    core.update_settings(&serde_json::json!({"general": {"showCaptureDialog": false}, "downloads": {"autoStart": false}}))
        .unwrap();
    let mut rx = core.subscribe();
    let page = "https://www.youtube.com/watch?v=jNQXAC9IVRw";
    let reply = core
        .handle(serde_json::json!({
            "id": "1", "type": "video", "pageUrl": page, "title": "Me at the zoo",
            "formatId": "best", "formatLabel": "Best quality", "streams": [],
            "cookies": "PREF=1", "userAgent": "test UA",
        }))
        .await;
    assert_eq!(reply["accepted"], true, "{reply}");
    let items = core.bootstrap().items;
    assert_eq!(items.len(), 1, "added at once");
    assert_eq!(items[0].kind, ItemKind::Video);
    assert_eq!(
        items[0].request.as_ref().unwrap()["context"]["cookies"],
        "PREF=1"
    );
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    while let Ok(ev) = rx.try_recv() {
        assert!(
            !matches!(
                ev,
                CoreEvent::VideoRequested(_) | CoreEvent::CaptureRequested(_)
            ),
            "no window: {ev:?}"
        );
    }
}

/// A sniffed HLS stream's qualities are listed for the page menu, and the one picked there is the
/// one downloaded, under the name and overwrite choice made in the "Download File" dialog.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_quality_picked_from_a_stream_downloads_that_variant() {
    let fixture = HlsFixture::synthetic(3, 4, 20_000, 7);
    let server = TestServer::start(ServerConfig {
        hls: Some(fixture.clone()),
        ..Default::default()
    })
    .await
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let core = core_in(tmp.path()).await;
    // The fixture's segments are not real video: kept as downloaded (no ffmpeg remux).
    core.update_settings(&serde_json::json!({
        "general": {"showCaptureDialog": true},
        "video": {"ffmpegPath": tmp.path().join("no-ffmpeg.exe")},
    }))
    .unwrap();
    let master = server.url("/hls/master.m3u8");

    let reply = core
        .handle(serde_json::json!({
            "id": "1", "type": "probeStream", "url": master, "kind": "hls",
            "pageUrl": "https://example.com/watch", "userAgent": "test",
        }))
        .await;
    assert_eq!(reply["ok"], true, "{reply}");
    let listed: Vec<(u64, u64)> = reply["variants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v["index"].as_u64().unwrap(), v["height"].as_u64().unwrap()))
        .collect();
    assert_eq!(listed, [(0, 360), (1, 720), (2, 1080)]);

    // A file of the chosen name already exists: the dialog says to overwrite it.
    let dl = tmp.path().join("dl");
    std::fs::create_dir_all(&dl).unwrap();
    std::fs::write(dl.join("Picked.ts"), b"old").unwrap();
    server.reset_stats();
    let mut rx = core.subscribe();
    let reply = core
        .handle(serde_json::json!({
            "id": "2", "type": "video", "pageUrl": "https://example.com/watch", "title": "Clip",
            "streams": [{"url": master, "kind": "hls", "variantIndex": 1}],
        }))
        .await;
    assert_eq!(reply["accepted"], true, "{reply}");
    let info = wait_event(&mut rx, 5, |ev| match ev {
        CoreEvent::CaptureRequested(info) => Some(info.clone()),
        _ => None,
    })
    .await;
    let item = core
        .confirm_capture(CaptureDecision {
            id: info.id,
            file_name: Some("Picked.ts".into()),
            save_dir: Some(dl.clone()),
            conflict: Some(ConflictAction::Overwrite),
            mode: StartMode::Now,
            ..Default::default()
        })
        .unwrap();
    let path = wait_completed(&mut rx, &item.id).await;
    assert_eq!(path, dl.join("Picked.ts"));
    assert_eq!(
        sha256_file(&path),
        sha256_hex(&fixture.expected_variant_bytes(1))
    );
    let stats = server.stats();
    assert!(stats.requests_to("/hls/v1/") > 0);
    assert_eq!(stats.requests_to("/hls/v0/"), 0);
    assert_eq!(stats.requests_to("/hls/v2/"), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn trash_restore_and_permanent_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let core = core_in(tmp.path()).await;
    let items = core
        .add_items(
            vec![NewItem {
                url: "https://example.invalid/a.zip".into(),
                ..Default::default()
            }],
            StartMode::Later,
        )
        .unwrap();
    let id = items[0].id.clone();
    core.delete_items(std::slice::from_ref(&id), false, false)
        .await
        .unwrap();
    assert!(core.item(&id).unwrap().trashed);
    core.restore_items(std::slice::from_ref(&id));
    assert!(!core.item(&id).unwrap().trashed);
    core.delete_items(std::slice::from_ref(&id), false, true)
        .await
        .unwrap();
    assert!(core.item(&id).is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_continue_after_restart_for_update() {
    let server = TestServer::start(ServerConfig::default()).await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let size: u64 = 6 << 20;
    let url = server.url(&format!(
        "/file/upd.bin?size={size}&seed=14&throttle=524288"
    ));
    let (running, idle) = {
        let core = core_in(tmp.path()).await;
        let running = core
            .add_items(
                vec![NewItem {
                    url,
                    ..Default::default()
                }],
                StartMode::Now,
            )
            .unwrap()[0]
            .id
            .clone();
        let idle = core
            .add_items(
                vec![NewItem {
                    url: "https://example.invalid/later.zip".into(),
                    ..Default::default()
                }],
                StartMode::Later,
            )
            .unwrap()[0]
            .id
            .clone();
        tokio::time::sleep(Duration::from_millis(1500)).await;
        core.shutdown_for_restart().await;
        (running, idle)
    };
    // New version starts: the running download continues by itself, the idle one doesn't.
    let core = core_in(tmp.path()).await;
    let mut rx = core.subscribe();
    let item = core.item(&running).unwrap();
    assert_ne!(
        item.status,
        Status::Paused,
        "should continue without a click"
    );
    let path = wait_completed(&mut rx, &running).await;
    assert_eq!(sha256_file(&path), expected_sha256(14, size));
    assert_ne!(core.item(&idle).unwrap().status, Status::Downloading);
    // A normal restart afterwards doesn't start anything again.
    assert!(
        !tmp.path()
            .join("data")
            .join("resume-after-restart.json")
            .exists()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn running_download_survives_trash_and_restore() {
    let server = TestServer::start(ServerConfig::default()).await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let core = core_in(tmp.path()).await;
    let size: u64 = 6 << 20;
    let url = server.url(&format!(
        "/file/trash.bin?size={size}&seed=12&throttle=524288"
    ));
    let items = core
        .add_items(
            vec![NewItem {
                url,
                ..Default::default()
            }],
            StartMode::Now,
        )
        .unwrap();
    let id = items[0].id.clone();
    wait_downloaded(&core, &id, 512 * 1024).await;
    // Trash it while it is downloading: the job pauses (not cancels) and keeps its resume point.
    core.delete_items(std::slice::from_ref(&id), false, false)
        .await
        .unwrap();
    let item = wait_paused(&core, &id).await;
    assert!(item.trashed);
    assert!(item.downloaded > 0, "trashing must keep the resume point");
    let before = item.downloaded;
    // Restore and continue: same file, not restarted.
    core.restore_items(std::slice::from_ref(&id));
    assert!(!core.item(&id).unwrap().trashed);
    server.reset_stats();
    let mut rx = core.subscribe();
    core.start_items(std::slice::from_ref(&id));
    let path = wait_completed(&mut rx, &id).await;
    assert_eq!(path.file_name().unwrap(), "trash.bin");
    assert_eq!(sha256_file(&path), expected_sha256(12, size));
    let resent: u64 = server
        .stats()
        .connections
        .iter()
        .map(|c| c.bytes_sent)
        .sum();
    assert!(
        resent < size - before / 2,
        "resent {resent} bytes, had {before}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn removing_an_unfinished_download_removes_its_partial_file() {
    let server = TestServer::start(ServerConfig::default()).await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let core = core_in(tmp.path()).await;
    let size: u64 = 6 << 20;
    let url = server.url(&format!(
        "/file/gone.bin?size={size}&seed=13&throttle=524288"
    ));
    let id = core
        .add_items(
            vec![NewItem {
                url,
                ..Default::default()
            }],
            StartMode::Now,
        )
        .unwrap()[0]
        .id
        .clone();
    wait_downloaded(&core, &id, 512 * 1024).await;
    core.pause_items(std::slice::from_ref(&id));
    wait_paused(&core, &id).await;
    let part = tmp.path().join("dl").join("gone.bin.prismpart");
    assert!(part.exists(), "partial file expected at {}", part.display());
    // Trash, then empty the trash ("delete files" not ticked).
    core.delete_items(std::slice::from_ref(&id), false, false)
        .await
        .unwrap();
    assert!(
        part.exists(),
        "the trash keeps the partial file for restoring"
    );
    core.empty_trash().await.unwrap();
    assert!(core.item(&id).is_none());
    assert!(
        !part.exists(),
        "an unfinished download's partial file must not be left behind"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn batch_pattern_expands() {
    let tmp = tempfile::tempdir().unwrap();
    let core = core_in(tmp.path()).await;
    let batch = core
        .batch_from_pattern("https://example.com/file[01-12].zip")
        .unwrap();
    assert_eq!(batch.entries.len(), 12);
    assert_eq!(batch.entries[0].file_name, "file01.zip");
    assert_eq!(batch.entries[0].category, "compressed");
}

#[tokio::test]
async fn stale_extension_reminder_follows_the_install_type() {
    let tmp = tempfile::tempdir().unwrap();
    // As Prism 1.0.6 left it: the version alone.
    std::fs::create_dir_all(tmp.path().join("data")).unwrap();
    std::fs::write(tmp.path().join("data").join("extension-version"), "1.0.0").unwrap();
    let core = core_in(tmp.path()).await;
    assert_eq!(core.extension_version().as_deref(), Some("1.0.0"));
    assert_eq!(core.extension_install_type(), None);
    assert!(core.extension_outdated());

    let hello = |version: &str, kind: Option<&str>| {
        let mut m = serde_json::json!({"id": "1", "type": "hello", "protocol": 1, "version": version, "browser": "chrome"});
        if let Some(k) = kind {
            m["installType"] = serde_json::json!(k);
        }
        m
    };
    // From a store or by policy: the browser updates it by itself (later than the app).
    for kind in ["normal", "admin"] {
        assert_eq!(core.handle(hello("1.0.0", Some(kind))).await["ok"], true);
        assert!(!core.extension_outdated(), "{kind}");
    }
    // Loaded unpacked, or an extension that doesn't say (before 1.0.7): it has to be reloaded.
    core.handle(hello("1.0.0", Some("development"))).await;
    assert!(core.extension_outdated());
    core.handle(hello("1.0.0", None)).await;
    assert_eq!(core.extension_install_type(), None);
    assert!(core.extension_outdated());
    // The app's own version is never outdated; what was seen last is kept across a restart.
    core.handle(hello(prism_core::APP_VERSION, Some("development")))
        .await;
    assert!(!core.extension_outdated());
    core.shutdown().await;
    let core = core_in(tmp.path()).await;
    assert_eq!(
        core.extension_version().as_deref(),
        Some(prism_core::APP_VERSION)
    );
    assert_eq!(
        core.extension_install_type().as_deref(),
        Some("development")
    );
    core.shutdown().await;
}

/// "Ask before adding copied links" off: copied file links go into the list as "Download
/// later" items, nothing starts and no window opens.
#[tokio::test]
async fn copied_links_go_quietly_into_the_list() {
    let tmp = tempfile::tempdir().unwrap();
    let core = core_in(tmp.path()).await;
    let mut rx = core.subscribe();
    let urls = [
        "https://example.invalid/files/a.zip".to_string(),
        "https://example.invalid/b.iso".to_string(),
    ];
    let items = core.add_copied_links(&urls).unwrap();
    let names: Vec<_> = items.iter().map(|i| i.file_name.as_str()).collect();
    assert_eq!(names, ["a.zip", "b.iso"]);
    for i in &items {
        let now = core.item(&i.id).unwrap();
        assert!(
            !now.status.is_active() && now.status != Status::Queued,
            "{:?}",
            now.status
        );
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    while let Ok(ev) = rx.try_recv() {
        assert!(
            !matches!(
                ev,
                CoreEvent::CaptureRequested(_) | CoreEvent::BatchRequested(_)
            ),
            "no window may open: {ev:?}"
        );
    }
    core.shutdown().await;
}

/// #14 (softpedia.com): a browser download behind a bot check, or answered with a web page
/// instead of the `.exe`, is left to the browser (`accepted: false`: the extension lets the
/// browser carry on) and nothing is added; a normal one is taken over as before, and the
/// Download File dialog already knows the size from that first try.
#[tokio::test]
async fn browser_downloads_only_a_browser_can_get_are_left_to_it() {
    let server = TestServer::start(ServerConfig::default()).await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let core = core_in(tmp.path()).await;
    let mut rx = core.subscribe();
    let msg = |url: String, name: &str| {
        serde_json::json!({
            "id": "1", "type": "download", "url": url, "filename": name,
            "cookies": "cf_clearance=x", "userAgent": "TestBrowser/1.0", "method": "GET", "source": "capture",
        })
    };
    for dialog in [false, true] {
        core.update_settings(&serde_json::json!({"general": {"showCaptureDialog": dialog}}))
            .unwrap();
        for (url, name) in [
            (server.url("/challenge"), "Nero-7.11.10.0.exe"),
            (
                server.url("/file/x?size=11389&seed=1&type=text/html"),
                "Visual_Paradigm.exe",
            ),
            // A plain 403 for a link the browser itself just got: only the browser is let in.
            (server.url("/status/403"), "Nero-8.3.2.1.exe"),
        ] {
            let reply = core.handle(msg(url, name)).await;
            assert_eq!(reply["ok"], true, "{reply}");
            assert_eq!(reply["accepted"], false, "{name}: {reply}");
        }
    }
    assert!(core.bootstrap().items.is_empty(), "nothing added");
    let mut left = 0;
    while let Ok(ev) = rx.try_recv() {
        assert!(
            !matches!(ev, CoreEvent::CaptureRequested(_)),
            "no Download File window"
        );
        if matches!(ev, CoreEvent::LeftToBrowser { .. }) {
            left += 1;
        }
    }
    assert_eq!(left, 6);

    // Other answers are Prism's to show, as before (the dialog explains a 404).
    let reply = core
        .handle(msg(server.url("/status/404"), "gone.zip"))
        .await;
    assert_eq!(reply["accepted"], true, "{reply}");

    // A normal download: taken over, and the dialog has the size right away.
    let url = server.url("/file/pkg.zip?size=200000&seed=5");
    let reply = core.handle(msg(url, "pkg.zip")).await;
    assert_eq!(reply["accepted"], true, "{reply}");
    let info = core
        .pending_capture(reply["downloadId"].as_str().unwrap())
        .unwrap();
    assert!(!info.probing);
    assert_eq!(info.size, Some(200_000));
    core.shutdown().await;
}

/// Settings › General › "Remove downloads whose files were deleted" (like ABDM): off by default;
/// when on, a finished download whose file was deleted leaves the list, while one whose whole
/// folder is gone (an unplugged drive) and one whose file is still there stay.
#[tokio::test]
async fn downloads_whose_files_were_deleted_leave_the_list_when_asked() {
    let server = TestServer::start(ServerConfig::default()).await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let core = core_in(tmp.path()).await;
    let drive = tmp.path().join("usb");
    let new = |name: &str, dir: Option<PathBuf>| NewItem {
        url: server.url(&format!("/file/{name}?size=4000&seed=2")),
        save_dir: dir,
        ..Default::default()
    };
    let items = core
        .add_items(
            vec![
                new("deleted.bin", None),
                new("kept.bin", None),
                new("on-usb.bin", Some(drive.clone())),
            ],
            StartMode::Now,
        )
        .unwrap();
    // All three finish (in any order).
    let paths: Vec<PathBuf> = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let now: Vec<DownloadItem> = items.iter().map(|i| core.item(&i.id).unwrap()).collect();
            if now.iter().all(|i| i.status == Status::Completed) {
                break now.iter().map(|i| i.path()).collect();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the downloads finish");
    std::fs::remove_file(&paths[0]).unwrap();
    std::fs::remove_dir_all(&drive).unwrap();

    assert_eq!(core.remove_deleted_files().await, 0, "off by default");
    assert_eq!(core.bootstrap().items.len(), 3);

    core.update_settings(&serde_json::json!({"general": {"removeDeletedFiles": true}}))
        .unwrap();
    assert_eq!(core.remove_deleted_files().await, 1);
    let left: Vec<String> = core
        .bootstrap()
        .items
        .into_iter()
        .map(|i| i.file_name)
        .collect();
    assert_eq!(left.len(), 2, "{left:?}");
    assert!(left.contains(&"kept.bin".to_string()) && left.contains(&"on-usb.bin".to_string()));
    assert!(paths[1].exists(), "files are never touched");
    core.shutdown().await;
}

/// A tiny but well-formed `.torrent` file (one 3-byte file).
fn tiny_torrent() -> Vec<u8> {
    let mut t = b"d8:announce20:http://tracker.test/4:infod6:lengthi3e4:name5:a.txt12:piece lengthi16384e6:pieces20:".to_vec();
    t.extend_from_slice(&[7u8; 20]);
    t.extend_from_slice(b"ee");
    t
}

/// A site that hands out its `.torrent` file only with the visitor's cookie (else a web page),
/// and a "`.torrent`" link that is really a page.
async fn torrent_site() -> String {
    use axum::{Router, http::HeaderMap, response::Html, response::IntoResponse, routing::get};
    let torrent = tiny_torrent();
    let app = Router::new()
        .route(
            "/dl/movie.torrent",
            get(move |headers: HeaderMap| async move {
                let cookie = headers
                    .get("cookie")
                    .and_then(|c| c.to_str().ok())
                    .unwrap_or("");
                if cookie.contains("session=ok") {
                    ([("content-type", "application/x-bittorrent")], torrent).into_response()
                } else {
                    Html("<html>Please log in</html>").into_response()
                }
            }),
        )
        .route(
            "/page/fake.torrent",
            get(|| async { Html("<html>not a torrent</html>") }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// #20: a browser download of a link to a `.torrent` file failed with "missing torrent source".
/// Now Prism fetches the `.torrent` file with the browser's cookies and opens "Add torrent"; a
/// link that turns out to be a page is downloaded as a plain file; with the Download File dialog
/// off the torrent goes straight into the list.
#[tokio::test]
async fn a_browser_link_to_a_torrent_file_opens_add_torrent() {
    let site = torrent_site().await;
    let tmp = tempfile::tempdir().unwrap();
    let core = core_in(tmp.path()).await;
    core.update_settings(&serde_json::json!({"general": {"showCaptureDialog": true}}))
        .unwrap();
    let mut rx = core.subscribe();
    let msg = |url: String, name: &str| {
        serde_json::json!({
            "id": "1", "type": "download", "url": url, "filename": name, "cookies": "session=ok",
            "referrer": "https://site.test/page", "userAgent": "TestBrowser/1.0", "method": "GET",
        })
    };

    let reply = core
        .handle(msg(
            format!("{site}/dl/movie.torrent"),
            "Movie Pack.torrent",
        ))
        .await;
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["accepted"], true, "{reply}");
    let path = wait_event(&mut rx, 10, |ev| match ev {
        CoreEvent::OpenTorrent { source } => Some(source.clone()),
        CoreEvent::CaptureRequested(c) => panic!("no Download File window: {c:?}"),
        _ => None,
    })
    .await;
    assert!(path.ends_with("Movie Pack.torrent"), "{path}");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        tiny_torrent(),
        "fetched with the cookie"
    );
    assert!(
        core.bootstrap().items.is_empty(),
        "nothing added before Add torrent"
    );

    // A ".torrent" link that is really a web page: the usual Download File dialog, as a file.
    let reply = core
        .handle(msg(format!("{site}/page/fake.torrent"), "fake.torrent"))
        .await;
    assert_eq!(reply["accepted"], true, "{reply}");
    let info = wait_event(&mut rx, 10, |ev| match ev {
        CoreEvent::CaptureRequested(c) => Some(c.clone()),
        CoreEvent::OpenTorrent { .. } => panic!("not a torrent"),
        _ => None,
    })
    .await;
    assert_eq!(info.kind, ItemKind::Http);
    assert_eq!(info.file_name, "fake.torrent");

    // Dialog off: the torrent itself goes into the list, with the fetched .torrent inside.
    core.update_settings(&serde_json::json!({
        "general": {"showCaptureDialog": false}, "downloads": {"autoStart": false}
    }))
    .unwrap();
    core.handle(msg(
        format!("{site}/dl/movie.torrent"),
        "Movie Pack.torrent",
    ))
    .await;
    let item = wait_event(&mut rx, 10, |ev| match ev {
        CoreEvent::ItemsAdded(items) => items.first().cloned(),
        _ => None,
    })
    .await;
    assert_eq!(item.kind, ItemKind::Torrent);
    assert_eq!(item.file_name, "Movie Pack");
    assert_eq!(item.status, Status::Idle);
    let source = &item.request.as_ref().unwrap()["source"];
    assert_eq!(source["type"], "file", "{source}");
    core.shutdown().await;
}

/// Torrent links that skip "Add torrent" still carry their torrent: a magnet copied with
/// "Ask before adding" on opens the window, and links added straight to the list (batches,
/// quiet copied links) let the torrent engine read the link itself.
#[tokio::test]
async fn torrent_links_always_have_a_torrent_source() {
    let tmp = tempfile::tempdir().unwrap();
    let core = core_in(tmp.path()).await;
    let mut rx = core.subscribe();
    let magnet = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=x";
    core.capture(
        NewItem {
            url: magnet.into(),
            ..Default::default()
        },
        CaptureSource::Clipboard,
    );
    let source = wait_event(&mut rx, 10, |ev| match ev {
        CoreEvent::OpenTorrent { source } => Some(source.clone()),
        _ => None,
    })
    .await;
    assert_eq!(source, magnet);

    let url = "https://example.invalid/files/linux.iso.torrent";
    let items = core
        .add_items(
            vec![NewItem {
                url: url.into(),
                ..Default::default()
            }],
            StartMode::Later,
        )
        .unwrap();
    assert_eq!(items[0].kind, ItemKind::Torrent);
    assert_eq!(
        items[0].request.as_ref().unwrap()["source"],
        serde_json::json!({"type": "url", "value": url})
    );
    core.shutdown().await;
}

type Seen<T> = std::sync::Arc<std::sync::Mutex<Vec<T>>>;

/// A Hugging Face lookalike: the model API, `/resolve/` links that need the token and redirect
/// to a "CDN" on another server (signed links, ranges), and what each side was sent.
struct HubSite {
    hub: String,
    /// Authorization headers the hub's `/resolve/` links got.
    hub_auth: Seen<Option<String>>,
    /// Whether each request the CDN got carried an Authorization header.
    cdn_auth: Seen<bool>,
    files: Vec<(String, Vec<u8>)>,
}

fn ranged(data: &[u8], headers: &axum::http::HeaderMap) -> axum::response::Response {
    use axum::{http::StatusCode, response::IntoResponse};
    let total = data.len();
    let range = headers
        .get("range")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("bytes="))
        .and_then(|v| v.split_once('-'))
        .map(|(a, b)| {
            let a: usize = a.parse().unwrap_or(0);
            let b = b
                .parse::<usize>()
                .map(|b| b.min(total - 1))
                .unwrap_or(total - 1);
            (a, b)
        });
    match range {
        Some((a, b)) if a <= b && a < total => (
            StatusCode::PARTIAL_CONTENT,
            [
                ("content-range", format!("bytes {a}-{b}/{total}")),
                ("accept-ranges", "bytes".to_string()),
                ("content-type", "application/octet-stream".to_string()),
            ],
            data[a..=b].to_vec(),
        )
            .into_response(),
        _ => (
            [
                ("accept-ranges", "bytes".to_string()),
                ("content-type", "application/octet-stream".to_string()),
            ],
            data.to_vec(),
        )
            .into_response(),
    }
}

async fn hub_site() -> HubSite {
    use axum::{
        Router,
        extract::Path as AxPath,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::get,
    };
    let blob = |n: usize, seed: u8| {
        (0..n)
            .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
            .collect::<Vec<u8>>()
    };
    let files = vec![
        (
            "config.json".to_string(),
            br#"{"model_type":"llama"}"#.to_vec(),
        ),
        ("model-Q4_K_M.gguf".to_string(), blob(3 << 20, 7)),
        ("model-Q8_0.gguf".to_string(), blob(5 << 20, 9)),
    ];
    let by_name: std::sync::Arc<std::collections::HashMap<String, Vec<u8>>> =
        std::sync::Arc::new(files.iter().cloned().collect());

    // The CDN: ranges, no credentials needed.
    let cdn_auth: Seen<bool> = Default::default();
    let cdn_seen = cdn_auth.clone();
    let cdn = Router::new().route(
        "/blob/{name}",
        get(move |AxPath(name): AxPath<String>, headers: HeaderMap| {
            let files = by_name.clone();
            let seen = cdn_seen.clone();
            async move {
                seen.lock()
                    .unwrap()
                    .push(headers.contains_key("authorization"));
                match files.get(&name) {
                    Some(data) => ranged(data, &headers),
                    None => StatusCode::NOT_FOUND.into_response(),
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cdn_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, cdn).await.unwrap() });

    // The hub: the model's file list and its /resolve/ links.
    let siblings: Vec<serde_json::Value> = files
        .iter()
        .map(|(name, data)| {
            if name.ends_with(".gguf") {
                serde_json::json!({"rfilename": name, "size": data.len(),
                    "lfs": {"sha256": sha256_hex(data), "size": data.len()}})
            } else {
                serde_json::json!({"rfilename": name, "size": data.len()})
            }
        })
        .collect();
    let info = serde_json::json!({"id": "owner/tiny-GGUF", "sha": "c0ffee", "gated": "auto",
        "downloads": 12, "likes": 3, "tags": ["gguf"], "siblings": siblings});
    let hub_auth: Seen<Option<String>> = Default::default();
    let seen = hub_auth.clone();
    let hub = Router::new()
        .route(
            "/api/models/owner/tiny-GGUF/revision/main",
            get(move |headers: HeaderMap| {
                let info = info.clone();
                async move {
                    match headers.get("authorization").and_then(|v| v.to_str().ok()) {
                        Some("Bearer hf_test") => axum::Json(info).into_response(),
                        Some(_) => (StatusCode::UNAUTHORIZED, "bad token").into_response(),
                        None => (
                            StatusCode::UNAUTHORIZED,
                            [("x-error-code", "GatedRepo")],
                            "gated",
                        )
                            .into_response(),
                    }
                }
            }),
        )
        .route(
            "/owner/tiny-GGUF/resolve/{rev}/{name}",
            get(
                move |AxPath((rev, name)): AxPath<(String, String)>, headers: HeaderMap| {
                    let seen = seen.clone();
                    let cdn_url = cdn_url.clone();
                    async move {
                        let auth = headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string);
                        seen.lock().unwrap().push(auth.clone());
                        if auth.as_deref() != Some("Bearer hf_test") || rev != "c0ffee" {
                            return (
                                StatusCode::UNAUTHORIZED,
                                [("x-error-code", "GatedRepo")],
                                "",
                            )
                                .into_response();
                        }
                        (
                            StatusCode::FOUND,
                            [(
                                "location",
                                format!("{cdn_url}/blob/{name}?Expires=9&Signature=x"),
                            )],
                        )
                            .into_response()
                    }
                },
            ),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hub_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, hub).await.unwrap() });
    HubSite {
        hub: hub_url,
        hub_auth,
        cdn_auth,
        files,
    }
}

/// #29: a gated model from Hugging Face. Without a token Prism says one is needed; with it the
/// suggested GGUF version goes into the "Hugging Face" queue, which starts, downloads every file
/// through the redirect (the token goes to the hub only, never to the CDN), checks the SHA-256
/// and then knows the files are there. A file whose SHA-256 doesn't match fails.
#[tokio::test]
async fn a_hugging_face_model_downloads_into_its_queue() {
    use prism_core::huggingface::{AddModel, RepoKind};
    let site = hub_site().await;
    let tmp = tempfile::tempdir().unwrap();
    let core = core_in(tmp.path()).await;
    core.update_settings(&serde_json::json!({"huggingFace": {"endpoint": site.hub, "token": ""}}))
        .unwrap();
    // No token (unless the Hugging Face tools saved one on this machine).
    if prism_core::huggingface::token(&core.settings().hugging_face).is_none() {
        let err = core
            .hf_repo("owner/tiny-GGUF")
            .await
            .unwrap_err()
            .to_string();
        assert_eq!(err, "hf:needsToken");
    }
    core.update_settings(&serde_json::json!({"huggingFace": {"token": "hf_test"}}))
        .unwrap();
    let repo = core
        .hf_repo(&format!("{}/owner/tiny-GGUF", site.hub))
        .await
        .unwrap();
    assert_eq!(repo.sha, "c0ffee");
    assert!(repo.gated && repo.gguf);
    assert_eq!(repo.suggestion.variant.as_deref(), Some("Q4_K_M"));
    assert_eq!(repo.suggestion.files, vec!["model-Q4_K_M.gguf"]);
    let folder = PathBuf::from(&repo.folder);
    assert_eq!(
        folder,
        tmp.path().join("dl").join("Models").join("owner/tiny-GGUF")
    );

    let wanted: Vec<_> = repo
        .files
        .iter()
        .filter(|f| f.path != "model-Q8_0.gguf")
        .cloned()
        .collect();
    let mut rx = core.subscribe();
    let added = core
        .hf_add(AddModel {
            kind: RepoKind::Model,
            id: repo.id.clone(),
            sha: repo.sha.clone(),
            files: wanted,
            folder: folder.clone(),
            mode: None,
        })
        .unwrap();
    assert_eq!(added, 2);
    let queue = core
        .queues()
        .into_iter()
        .find(|q| q.id == "huggingface")
        .expect("queue made");
    assert_eq!(
        (queue.name.as_str(), queue.max_concurrent),
        ("Hugging Face", 2)
    );
    let items = core.bootstrap().items;
    assert_eq!(items.len(), 2);
    assert!(
        items
            .iter()
            .all(|i| i.queue_id.as_deref() == Some("huggingface"))
    );
    let mut done = 0;
    wait_event(&mut rx, 60, |ev| match ev {
        CoreEvent::Completed(_) => {
            done += 1;
            (done == 2).then_some(())
        }
        CoreEvent::Failed(item) => panic!("failed: {:?}", item.error),
        _ => None,
    })
    .await;
    for (name, data) in &site.files[..2] {
        assert_eq!(&std::fs::read(folder.join(name)).unwrap(), data, "{name}");
    }
    let cdn = site.cdn_auth.lock().unwrap().clone();
    assert!(cdn.len() >= 2, "{cdn:?}");
    assert!(
        cdn.iter().all(|a| !a),
        "the token never reaches the CDN: {cdn:?}"
    );
    assert!(
        site.hub_auth
            .lock()
            .unwrap()
            .iter()
            .all(|a| a.as_deref() == Some("Bearer hf_test")),
        "the hub's links get the token"
    );
    let item = core
        .bootstrap()
        .items
        .into_iter()
        .find(|i| i.file_name == "model-Q4_K_M.gguf")
        .unwrap();
    assert!(
        item.request
            .as_ref()
            .is_some_and(|r| !r.to_string().contains("hf_test")),
        "the token isn't stored"
    );

    // The window now knows what's there.
    let present = core.hf_present(RepoKind::Model, &repo.id, &repo.sha, &folder, &repo.files);
    assert_eq!(present.listed.len(), 2, "{present:?}");
    core.delete_items(
        &core
            .bootstrap()
            .items
            .iter()
            .map(|i| i.id.clone())
            .collect::<Vec<_>>(),
        false,
        true,
    )
    .await
    .unwrap();
    let present = core.hf_present(RepoKind::Model, &repo.id, &repo.sha, &folder, &repo.files);
    assert_eq!(present.on_disk.len(), 2, "{present:?}");

    // A file that doesn't match its SHA-256 fails instead of passing as the model.
    let mut bad = repo
        .files
        .iter()
        .find(|f| f.path == "model-Q8_0.gguf")
        .unwrap()
        .clone();
    bad.sha256 = Some("0".repeat(64));
    core.hf_add(AddModel {
        kind: RepoKind::Model,
        id: repo.id.clone(),
        sha: repo.sha.clone(),
        files: vec![bad],
        folder: folder.clone(),
        mode: Some(StartMode::Now),
    })
    .unwrap();
    wait_event(&mut rx, 60, |ev| match ev {
        CoreEvent::Failed(item) if item.file_name == "model-Q8_0.gguf" => Some(()),
        CoreEvent::Completed(item) if item.file_name == "model-Q8_0.gguf" => {
            panic!("a wrong file passed")
        }
        _ => None,
    })
    .await;
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_expired_link_is_refreshed_and_the_download_resumes_where_it_stopped() {
    let server = TestServer::start(ServerConfig::default()).await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let core = core_in(tmp.path()).await;
    let size = 24u64 << 20;
    let q = format!("size={size}&seed=70&throttle=4M");
    // A signed link that stops working 1.5 s after its first use (410 Gone).
    let url = server.url(&format!("/expiring/iso.bin?{q}&ttl_ms=1500"));
    let items = core
        .add_items(
            vec![NewItem {
                url,
                ..Default::default()
            }],
            StartMode::Now,
        )
        .unwrap();
    let id = &items[0].id;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while core
        .item_details(id)
        .unwrap()
        .progress
        .map_or(0, |p| p.downloaded)
        < 4 << 20
    {
        assert!(tokio::time::Instant::now() < deadline, "no progress");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // Paused (sleep, a restart, a dropped network) until the link has expired.
    core.pause_items(std::slice::from_ref(id));
    let paused = wait_paused(&core, id).await;
    tokio::time::sleep(Duration::from_millis(1600)).await;

    // Resuming tells the user the link expired, and keeps what was downloaded.
    core.start_items(std::slice::from_ref(id));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let failed = loop {
        let d = core.item_details(id).unwrap();
        if d.progress.is_none() && d.item.status == Status::Failed {
            break d.item;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "status {:?}",
            d.item.status
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    eprintln!("after expiry: error {:?}", failed.error);
    assert!(
        failed
            .error
            .as_deref()
            .unwrap_or("")
            .to_lowercase()
            .contains("expired"),
        "error: {:?}",
        failed.error
    );
    assert!(failed.downloaded >= paused.downloaded, "progress kept");
    assert!(failed.resume.is_some(), "resume point kept");

    // Refresh download address with a fresh link to the same file, then resume.
    let fresh = format!("/file/iso.bin?{q}");
    core.update_item(
        id,
        prism_core::manager::ItemPatch {
            url: Some(server.url(&fresh)),
            ..Default::default()
        },
    )
    .unwrap();
    let mut rx = core.subscribe();
    core.start_items(std::slice::from_ref(id));
    let path = wait_completed(&mut rx, id).await;
    assert_eq!(sha256_file(&path), expected_sha256(70, size));
    // It went on from where it stopped: no data request on the fresh link starts at 0
    // (only the 1-byte check that it is the same file).
    let from_zero: Vec<_> = server
        .stats()
        .requests
        .iter()
        .filter(|r| r.uri.starts_with("/file/iso.bin"))
        .filter(|r| {
            r.range
                .as_deref()
                .is_none_or(|r| r.starts_with("bytes=0-") && r != "bytes=0-0")
        })
        .map(|r| r.range.clone())
        .collect();
    assert!(
        from_zero.is_empty(),
        "downloaded again from the start: {from_zero:?}"
    );
}
