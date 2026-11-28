//! HTTP integration tests against prism-testserver. Every downloaded file is verified by
//! SHA-256 against the server's deterministic content.

mod common;

use std::time::{Duration, Instant};

use common::*;
use prism_engine::{
    AuthScheme, ChecksumAlgo, ConflictPolicy, Credentials, DownloadEvent, DownloadState,
    EngineError, ExpectedChecksum, HttpMethod, ResumeState, SegmentStatus, Url,
};
use prism_testserver::{LimitMode, ServerConfig, expected_bytes, expected_sha256, versioned_seed};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multi_segment_8_connections_with_keepalive_reuse() {
    init_tracing();
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 64 * MIB;
    // Throttled so the transfer lasts long enough to observe all connections.
    let h = engine().start(
        request(
            &srv.url(&format!("/file/a.bin?size={size}&seed=11&throttle=6M")),
            dir.path(),
            8,
        ),
        None,
        vec![],
    );
    let done = wait(&h, 120).await.unwrap();
    assert_eq!(done.size, size);
    assert_eq!(done.path, dir.path().join("a.bin"));
    assert_eq!(sha256_file(&done.path), expected_sha256(11, size));
    assert!(!dir.path().join("a.bin.prismpart").exists());
    let stats = srv.stats();
    assert!(
        stats.max_concurrent >= 6,
        "used {} connections at most",
        stats.max_concurrent
    );
    assert!(
        stats.total_connections() <= 24,
        "too many connections: {}",
        stats.total_connections()
    );
    assert!(
        stats.max_requests_per_connection() >= 2,
        "bounded ranges should reuse keep-alive connections: {:?}",
        stats
            .connections
            .iter()
            .map(|c| c.requests)
            .collect::<Vec<_>>()
    );
    // Accept-Encoding: identity on every request.
    assert!(
        stats
            .requests
            .iter()
            .all(|r| r.accept_encoding.as_deref() == Some("identity"))
    );
    let snap = h.snapshot();
    assert_eq!(snap.state, DownloadState::Completed);
    assert!(
        snap.segments
            .iter()
            .all(|s| s.status == SegmentStatus::Done)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multi_segment_32_connections() {
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 64 * MIB + 12_345;
    let h = engine().start(
        request(
            &srv.url(&format!("/file/b.bin?size={size}&seed=12&throttle=2M")),
            dir.path(),
            32,
        ),
        None,
        vec![],
    );
    let done = wait(&h, 120).await.unwrap();
    assert_eq!(sha256_file(&done.path), expected_sha256(12, size));
    assert!(
        srv.stats().max_concurrent >= 16,
        "max concurrent {}",
        srv.stats().max_concurrent
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn throttled_server_benefits_from_connections() {
    let srv = default_server().await;
    let size = 16 * MIB;
    let url = srv.url(&format!("/file/t.bin?size={size}&seed=3&throttle=2M"));
    let dir1 = tempfile::tempdir().unwrap();
    let t = Instant::now();
    let h1 = engine().start(request(&url, dir1.path(), 1), None, vec![]);
    let single = transfer_time(&h1, t, 60).await;
    let d1 = wait(&h1, 60).await.unwrap();
    assert_eq!(sha256_file(&d1.path), expected_sha256(3, size));
    srv.reset_stats();
    let dir8 = tempfile::tempdir().unwrap();
    let t = Instant::now();
    let h8 = engine().start(request(&url, dir8.path(), 8), None, vec![]);
    let multi = transfer_time(&h8, t, 60).await;
    let d8 = wait(&h8, 60).await.unwrap();
    assert_eq!(sha256_file(&d8.path), expected_sha256(3, size));
    assert!(
        srv.stats().max_concurrent >= 6,
        "max concurrent {}",
        srv.stats().max_concurrent
    );
    assert!(
        single.as_secs_f64() / multi.as_secs_f64() >= 3.0,
        "speed-up too small: 1 conn {single:?}, 8 conns {multi:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_connection_tail_is_split_and_raced() {
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 16 * MIB;
    // Connection #1 (the first worker after the probe) crawls at 32 KiB/s; alone it would need
    // minutes for its piece.
    let url = srv.url(&format!(
        "/file/s.bin?size={size}&seed=5&throttle=8M&slow_conn=1&slow_rate=32K"
    ));
    let h = engine().start(request(&url, dir.path(), 4), None, vec![]);
    let events = collect_events(&h);
    let t = Instant::now();
    let done = wait(&h, 60).await.unwrap();
    assert_eq!(sha256_file(&done.path), expected_sha256(5, size));
    assert!(
        t.elapsed() < Duration::from_secs(30),
        "tail not taken over: {:?}",
        t.elapsed()
    );
    let splits = events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, DownloadEvent::SegmentSplit { .. }))
        .count();
    assert!(splits >= 3, "expected dynamic splits, got {splits}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_range_server_uses_single_connection() {
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 5 * MIB + 7;
    let h = engine().start(
        request(
            &srv.url(&format!("/file/n.bin?size={size}&seed=6&ranges=0")),
            dir.path(),
            8,
        ),
        None,
        vec![],
    );
    let events = collect_events(&h);
    let done = wait(&h, 60).await.unwrap();
    assert_eq!(sha256_file(&done.path), expected_sha256(6, size));
    assert_eq!(srv.stats().total_requests(), 1);
    assert!(!h.snapshot().resumable);
    let probed = events.lock().unwrap().iter().find_map(|e| match e {
        DownloadEvent::Probed(p) => Some(p.clone()),
        _ => None,
    });
    let p = probed.expect("probed event");
    assert!(!p.resumable && !p.accept_ranges);
    assert_eq!(p.size, Some(size));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_range_server_restarts_after_drop() {
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 3 * MIB;
    // Random drops (1% per 64 KiB block): an attempt fails ~40% of the time and the
    // non-resumable stream must restart from zero.
    let url = srv.url(&format!(
        "/file/nd.bin?size={size}&seed=9&ranges=0&drop_prob=0.01"
    ));
    let h = engine().start(request(&url, dir.path(), 1), None, vec![]);
    let done = wait(&h, 120).await.unwrap();
    assert_eq!(sha256_file(&done.path), expected_sha256(9, size));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn redirect_chain_is_followed_and_final_url_reused() {
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 9 * MIB;
    let target = format!("/file/r.bin?size={size}&seed=7");
    let enc: String = url::form_urlencoded::byte_serialize(target.as_bytes()).collect();
    let h = engine().start(
        request(
            &srv.url(&format!("/redirect/3?code=307&to={enc}")),
            dir.path(),
            4,
        ),
        None,
        vec![],
    );
    let done = wait(&h, 60).await.unwrap();
    assert_eq!(done.path.file_name().unwrap(), "r.bin");
    assert_eq!(sha256_file(&done.path), expected_sha256(7, size));
    // Only the probe went through the redirect chain; workers used the final URL directly.
    assert_eq!(srv.stats().requests_to("/redirect/3"), 1);
    assert!(
        h.snapshot()
            .final_url
            .unwrap()
            .path()
            .ends_with("/file/r.bin")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_byte_window_is_saved_as_the_whole_file() {
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 6 * MIB;
    let (from, to) = (MIB + 12_345, 5 * MIB + 777);
    let mut req = request(
        &srv.url(&format!("/file/w.bin?size={size}&seed=41")),
        dir.path(),
        4,
    );
    // Small requests (like YouTube's 10 MiB chunks), so the window is fetched in many ranges.
    req.max_request_size = Some(256 * 1024);
    req.window = Some((from, to));
    let h = engine().start(req, None, vec![]);
    let done = wait(&h, 60).await.unwrap();
    assert_eq!(done.size, to - from);
    assert!(
        std::fs::read(&done.path).unwrap() == expected_bytes(41, from..to),
        "the saved file is not the window's bytes"
    );
    // Every request asked for bytes inside the window, over several connections.
    let ranges: Vec<(u64, u64)> = srv
        .stats()
        .requests
        .iter()
        .filter(|r| r.uri.starts_with("/file/w.bin"))
        .map(|r| {
            let v = r.range.as_deref().expect("a range request");
            let (a, b) = v.trim_start_matches("bytes=").split_once('-').unwrap();
            (a.parse().unwrap(), b.parse().unwrap())
        })
        .collect();
    assert!(ranges.len() > 4, "{ranges:?}");
    assert!(
        ranges.iter().all(|&(a, b)| a >= from && b < to),
        "{ranges:?}"
    );

    // A server that ignores ranges would send the whole file: that's an error, not a file.
    let mut req = request(
        &srv.url(&format!("/file/w2.bin?size={size}&seed=42&ranges=0")),
        dir.path(),
        4,
    );
    req.window = Some((from, to));
    let h = engine().start(req, None, vec![]);
    assert!(matches!(
        wait(&h, 60).await,
        Err(EngineError::RangeNotSupported)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expired_signed_redirect_target_falls_back_to_original_url() {
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 8 * MIB;
    // Each pass through /redirect yields a fresh /expiring/k{n} URL valid for 700 ms.
    let target = format!("/expiring/k{{n}}?size={size}&seed=8&throttle=2M&ttl_ms=700");
    let enc: String = url::form_urlencoded::byte_serialize(target.as_bytes()).collect();
    let mut req = request(&srv.url(&format!("/redirect/1?to={enc}")), dir.path(), 4);
    // Small requests (like YouTube's 10 MiB chunks) so workers keep issuing new ones.
    req.max_request_size = Some(512 * 1024);
    let h = engine().start(req, None, vec![]);
    let done = wait(&h, 60).await.unwrap();
    assert_eq!(sha256_file(&done.path), expected_sha256(8, size));
    assert!(
        srv.stats().requests_to("/redirect/1") >= 2,
        "should have re-resolved the redirect"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn basic_and_digest_auth() {
    let srv = default_server().await;
    let size = 3 * MIB;
    for (mode, scheme) in [
        ("basic", AuthScheme::Basic),
        ("basic", AuthScheme::Auto),
        ("digest", AuthScheme::Auto),
        ("digest", AuthScheme::Digest),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut req = request(
            &srv.url(&format!("/file/auth.bin?size={size}&seed=13&auth={mode}")),
            dir.path(),
            4,
        );
        req.auth = Some(Credentials {
            username: "user".into(),
            password: "pass".into(),
            scheme,
        });
        let done = wait(&engine().start(req, None, vec![]), 60)
            .await
            .unwrap_or_else(|e| panic!("{mode}/{scheme:?}: {e}"));
        assert_eq!(sha256_file(&done.path), expected_sha256(13, size), "{mode}");
    }
    // Wrong or missing credentials fail fast with AuthRequired.
    for creds in [
        None,
        Some(Credentials {
            username: "user".into(),
            password: "nope".into(),
            scheme: AuthScheme::Auto,
        }),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut req = request(
            &srv.url("/file/auth.bin?size=1000&auth=digest"),
            dir.path(),
            4,
        );
        req.auth = creds;
        assert_eq!(
            wait(&engine().start(req, None, vec![]), 30)
                .await
                .unwrap_err(),
            EngineError::AuthRequired
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_connection_limit_is_detected() {
    for mode in [LimitMode::Status503, LimitMode::Reset] {
        let srv = server(ServerConfig {
            max_connections: Some(4),
            limit_mode: mode,
            ..Default::default()
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let size = 24 * MIB;
        let eng = engine();
        let h = eng.start(
            request(
                &srv.url(&format!("/file/l.bin?size={size}&seed=14&throttle=4M")),
                dir.path(),
                16,
            ),
            None,
            vec![],
        );
        let events = collect_events(&h);
        let done = wait(&h, 120)
            .await
            .unwrap_or_else(|e| panic!("{mode:?}: {e}"));
        assert_eq!(sha256_file(&done.path), expected_sha256(14, size));
        let limit = events.lock().unwrap().iter().find_map(|e| match e {
            DownloadEvent::ServerLimitDetected { connections } => Some(*connections),
            _ => None,
        });
        let limit = limit.unwrap_or_else(|| panic!("{mode:?}: no limit detected"));
        assert!((1..=4).contains(&limit), "{mode:?}: limit {limit}");
        let learned = eng.host_limits();
        assert_eq!(learned.len(), 1);
        assert!(learned[0].connections <= 5, "{learned:?}");
        assert!(srv.stats().max_concurrent <= 4);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn random_connection_drops_are_survived() {
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 32 * MIB;
    let h = engine().start(
        request(
            &srv.url(&format!("/file/d.bin?size={size}&seed=15&drop_prob=0.01")),
            dir.path(),
            8,
        ),
        None,
        vec![],
    );
    let events = collect_events(&h);
    let done = wait(&h, 120).await.unwrap();
    assert_eq!(sha256_file(&done.path), expected_sha256(15, size));
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, DownloadEvent::Retrying { .. })),
        "drops should cause retries"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn frequent_drops_keep_throughput_and_connections() {
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 96 * MIB;
    // 16 connections × 3 MiB/s = 48 MiB/s nominal; 2% of the 16 KiB blocks drop the connection.
    let url = srv.url(&format!(
        "/file/fd.bin?size={size}&seed=28&throttle=3M&drop_prob=0.02"
    ));
    let h = engine().start(request(&url, dir.path(), 16), None, vec![]);
    let events = collect_events(&h);
    let t = Instant::now();
    let done = wait(&h, 120).await.unwrap();
    let secs = t.elapsed().as_secs_f64();
    assert_eq!(sha256_file(&done.path), expected_sha256(28, size));
    let rate = size as f64 / secs / MIB as f64;
    // Shared CI runners are often CPU-starved; locally the bar is 40% of nominal.
    let bar = if std::env::var_os("CI").is_some() {
        0.15
    } else {
        0.4
    };
    assert!(
        rate >= bar * 48.0,
        "throughput {rate:.1} MiB/s (< {:.0}% of 48 MiB/s) in {secs:.1}s",
        bar * 100.0
    );
    assert!(
        !events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, DownloadEvent::ServerLimitDetected { .. })),
        "dropped connections must not be mistaken for a connection limit"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_and_resume_in_new_engine() {
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 32 * MIB;
    let url = srv.url(&format!("/file/p.bin?size={size}&seed=16&throttle=4M"));
    let h = engine().start(request(&url, dir.path(), 4), None, vec![]);
    wait_for_progress(&h, 6 * MIB).await;
    let state = h.pause().await.expect("resume state");
    assert_eq!(wait(&h, 5).await.unwrap_err(), EngineError::Paused);
    assert_eq!(h.state(), DownloadState::Paused);
    assert!(state.resumable);
    assert!(
        state.downloaded() >= 6 * MIB,
        "state claims {}",
        state.downloaded()
    );
    assert!(state.part_path.exists());
    // Persist as JSON like the app does, then "restart the app".
    let json = state.to_json();
    drop(h);
    let state = ResumeState::from_json(&json).unwrap();
    state.verify_against_disk().unwrap();
    srv.reset_stats();
    let h2 = engine().start(request(&url, dir.path(), 4), Some(state.clone()), vec![]);
    let done = wait(&h2, 120).await.unwrap();
    assert_eq!(done.path, state.final_path);
    assert_eq!(sha256_file(&done.path), expected_sha256(16, size));
    assert!(
        done.transferred <= size - state.downloaded() + 2 * MIB,
        "re-downloaded too much: {}",
        done.transferred
    );
    let first = srv.stats().requests.into_iter().next().unwrap();
    assert!(first.if_range.is_some(), "resume must send If-Range");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resume_after_resource_change_is_rejected() {
    for (query, name) in [("", "v.bin"), ("&etag=0&lm=0", "w.bin")] {
        let srv = default_server().await;
        let dir = tempfile::tempdir().unwrap();
        let size = 16 * MIB;
        let url = srv.url(&format!(
            "/file/{name}?size={size}&seed=17&throttle=4M{query}"
        ));
        let h = engine().start(request(&url, dir.path(), 4), None, vec![]);
        wait_for_progress(&h, 3 * MIB).await;
        let state = h.pause().await.unwrap();
        // Same resource: resumes fine (validators or overlap check pass) — tested elsewhere;
        // here the content changes on the server.
        srv.set_version(name, 1);
        let h2 = engine().start(request(&url, dir.path(), 4), Some(state), vec![]);
        assert_eq!(
            wait(&h2, 60).await.unwrap_err(),
            EngineError::ValidatorMismatch,
            "{query}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resume_without_validators_uses_overlap_check() {
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 16 * MIB;
    let url = srv.url(&format!(
        "/file/nv.bin?size={size}&seed=18&throttle=4M&etag=0&lm=0"
    ));
    let h = engine().start(request(&url, dir.path(), 4), None, vec![]);
    wait_for_progress(&h, 3 * MIB).await;
    let state = h.pause().await.unwrap();
    assert!(state.etag.is_none() && state.last_modified.is_none());
    srv.reset_stats();
    let done = wait(
        &engine().start(request(&url, dir.path(), 4), Some(state), vec![]),
        60,
    )
    .await
    .unwrap();
    assert_eq!(sha256_file(&done.path), expected_sha256(18, size));
    let first = &srv.stats().requests[0];
    assert!(first.if_range.is_none());
}

#[test]
fn crash_and_resume_from_checkpoint() {
    let srv_rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let srv = srv_rt.block_on(default_server());
    let dir = tempfile::tempdir().unwrap();
    let size = 24 * MIB;
    let url = srv.url(&format!("/file/c.bin?size={size}&seed=19&throttle=3M"));

    // "Process" 1: download, checkpoint periodically, then die without pausing.
    let crash_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let checkpoint = crash_rt.block_on(async {
        let h = engine().start(request(&url, dir.path(), 4), None, vec![]);
        // There is nothing to checkpoint until the job has probed and created its file, which
        // can take a while on a busy runner; from then on every checkpoint must succeed.
        tokio::time::timeout(Duration::from_secs(60), async {
            let mut taken = 0;
            loop {
                tokio::time::sleep(Duration::from_millis(250)).await;
                match h.checkpoint().await {
                    Some(c) => {
                        taken += 1;
                        if taken >= 3 && c.downloaded() > 0 {
                            assert!(!h.is_finished(), "finished before the crash");
                            return c;
                        }
                    }
                    None => assert_eq!(taken, 0, "checkpoint failed mid-download"),
                }
            }
        })
        .await
        .expect("no checkpoint with data")
    });
    crash_rt.shutdown_background();
    assert!(checkpoint.downloaded() > 0);
    // The durable state must describe bytes that really are on disk.
    let on_disk = std::fs::read(&checkpoint.part_path).unwrap();
    for s in &checkpoint.segments {
        let want = prism_testserver::expected_bytes(19, s.start..s.pos);
        assert_eq!(
            &on_disk[s.start as usize..s.pos as usize],
            &want[..],
            "segment {s:?} not on disk"
        );
    }

    // "Process" 2: resume from the persisted checkpoint.
    let json = checkpoint.to_json();
    let done = srv_rt.block_on(async {
        let state = ResumeState::from_json(&json).unwrap();
        wait(
            &engine().start(request(&url, dir.path(), 4), Some(state), vec![]),
            120,
        )
        .await
    });
    let done = done.unwrap();
    assert_eq!(sha256_file(&done.path), expected_sha256(19, size));
    drop(srv);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn change_url_mid_download() {
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 16 * MIB;
    let q = format!("size={size}&seed=20&throttle=3M");
    let h = engine().start(
        request(&srv.url(&format!("/file/old.bin?{q}")), dir.path(), 4),
        None,
        vec![],
    );
    let events = collect_events(&h);
    wait_for_progress(&h, 2 * MIB).await;
    // A different size is rejected...
    let bad = Url::parse(&srv.url(&format!("/file/x.bin?size={}&seed=20", size + 1))).unwrap();
    assert_eq!(
        h.change_url(bad).await.unwrap_err(),
        EngineError::ValidatorMismatch
    );
    // ...the same file elsewhere is accepted. Connections reopen on it at once, possibly
    // before `change_url` has returned here.
    let new = Url::parse(&srv.url(&format!("/file/new.bin?{q}"))).unwrap();
    let before = srv.stats().requests.len();
    h.change_url(new.clone()).await.unwrap();
    let mark = srv.stats().requests.len();
    let done = wait(&h, 60).await.unwrap();
    assert_eq!(sha256_file(&done.path), expected_sha256(20, size));
    let requests = srv.stats().requests;
    let after: Vec<_> = requests[mark..].iter().map(|r| r.uri.clone()).collect();
    assert!(
        after.iter().all(|u| !u.starts_with("/file/old.bin")),
        "old URL used after change: {after:?}"
    );
    // Data requests, not the validation probe of the first byte.
    assert!(
        requests[before..]
            .iter()
            .any(|r| r.uri.starts_with("/file/new.bin") && r.range.as_deref() != Some("bytes=0-0")),
        "new URL not used"
    );
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, DownloadEvent::UrlChanged { .. }))
    );
    assert_eq!(h.resume_state().map(|s| s.url), Some(new));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mirrors_are_validated_and_used() {
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 32 * MIB;
    let q = format!("size={size}&seed=21&throttle=4M");
    let mut req = request(&srv.url(&format!("/file/primary.bin?{q}")), dir.path(), 8);
    // Mirrors are validated in the background and join the rotation when a worker issues its
    // next request. Small requests make every worker pick a source many times; with whole
    // segments a late validation (busy machine) would leave the mirror unused.
    req.max_request_size = Some(512 * 1024);
    req.mirrors = vec![
        Url::parse(&srv.url(&format!("/file/mirror.bin?{q}"))).unwrap(),
        Url::parse(&srv.url(&format!("/file/wrong.bin?size={}&seed=21", size / 2))).unwrap(),
    ];
    let done = wait(&engine().start(req, None, vec![]), 60).await.unwrap();
    assert_eq!(done.path.file_name().unwrap(), "primary.bin");
    assert_eq!(sha256_file(&done.path), expected_sha256(21, size));
    let stats = srv.stats();
    let ranged = |p: &str| {
        stats
            .requests
            .iter()
            .filter(|r| r.uri.starts_with(p) && r.range.as_deref() != Some("bytes=0-0"))
            .count()
    };
    assert!(ranged("/file/mirror.bin") >= 1, "mirror unused");
    assert!(ranged("/file/primary.bin") >= 1);
    assert_eq!(ranged("/file/wrong.bin"), 0, "invalid mirror used");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn speed_limit_is_respected_and_live_adjustable() {
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 6 * MIB;
    let mut req = request(
        &srv.url(&format!("/file/sl.bin?size={size}&seed=22")),
        dir.path(),
        8,
    );
    req.speed_limit = 2 * MIB;
    let t = Instant::now();
    let h = engine().start(req, None, vec![]);
    // Measure the transfer only (finalizing fsyncs, which is slow on some CI file systems).
    let mut events = h.events();
    let (transfer, races) = tokio::time::timeout(Duration::from_secs(60), async {
        let mut races = 0;
        loop {
            match events.recv().await {
                Ok(DownloadEvent::StateChanged {
                    state: DownloadState::Finalizing,
                }) => return (t.elapsed(), races),
                Ok(DownloadEvent::SegmentSplit { race: true, .. }) => races += 1,
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    let done = wait(&h, 60).await.unwrap();
    let secs = transfer.as_secs_f64();
    assert_eq!(sha256_file(&done.path), expected_sha256(22, size));
    // Racing a tail duplicates bytes, which under a cap only burns the shared budget.
    assert_eq!(races, 0, "tail races while rate-limited");
    // 3 s nominal minus the 250 ms burst allowance, plus CI jitter.
    assert!(
        (2.4..=3.9).contains(&secs),
        "6 MiB at 2 MiB/s took {secs:.2}s"
    );

    // Live change: start at 1 MiB/s, lift the limit after 1 s.
    let dir = tempfile::tempdir().unwrap();
    let mut req = request(
        &srv.url(&format!("/file/sl2.bin?size={}&seed=22", 16 * MIB)),
        dir.path(),
        4,
    );
    req.speed_limit = MIB;
    let t = Instant::now();
    let h = engine().start(req, None, vec![]);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(h.snapshot().downloaded < 2 * MIB);
    h.set_speed_limit(0);
    wait(&h, 60).await.unwrap();
    assert!(
        t.elapsed() < Duration::from_secs(6),
        "limit not lifted: {:?}",
        t.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn global_and_queue_limiters_apply() {
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let eng = engine();
    eng.global_limiter().set_rate(4 * MIB);
    let queue = std::sync::Arc::new(prism_engine::RateLimiter::new(2 * MIB));
    let t = Instant::now();
    let h = eng.start(
        request(
            &srv.url(&format!("/file/q.bin?size={}&seed=1", 4 * MIB)),
            dir.path(),
            4,
        ),
        None,
        vec![queue],
    );
    let secs = transfer_time(&h, t, 60).await.as_secs_f64();
    wait(&h, 60).await.unwrap();
    assert!(
        (1.7..=2.6).contains(&secs),
        "queue limit (2 MiB/s) not applied: {secs:.2}s"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn checksum_verification() {
    let srv = default_server().await;
    let size = 2 * MIB;
    let url = srv.url(&format!("/file/sum.bin?size={size}&seed=23"));
    let dir = tempfile::tempdir().unwrap();
    let mut req = request(&url, dir.path(), 4);
    req.checksum = Some(ExpectedChecksum {
        algo: ChecksumAlgo::Sha256,
        hex: expected_sha256(23, size).to_uppercase(),
    });
    let done = wait(&engine().start(req, None, vec![]), 60).await.unwrap();
    assert_eq!(done.checksum_ok, Some(true));

    let dir = tempfile::tempdir().unwrap();
    let mut req = request(&url, dir.path(), 4);
    req.checksum = Some(ExpectedChecksum {
        algo: ChecksumAlgo::Md5,
        hex: "00".repeat(16),
    });
    match wait(&engine().start(req, None, vec![]), 60)
        .await
        .unwrap_err()
    {
        EngineError::ChecksumMismatch {
            expected,
            actual,
            path,
        } => {
            assert_eq!(expected, "00".repeat(16));
            assert_eq!(actual.len(), 32);
            assert!(path.exists(), "file is kept for inspection");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn file_name_from_content_disposition_and_conflict_rename() {
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let cd = "attachment; filename=\"fallback.bin\"; filename*=UTF-8''r%C3%A9sum%C3%A9%20v2.pdf";
    let enc: String = url::form_urlencoded::byte_serialize(cd.as_bytes()).collect();
    let url = srv.url(&format!("/file/download?size=1000&seed=24&cd={enc}"));
    let first = wait(
        &engine().start(request(&url, dir.path(), 2), None, vec![]),
        30,
    )
    .await
    .unwrap();
    assert_eq!(first.path, dir.path().join("résumé v2.pdf"));
    let second = wait(
        &engine().start(request(&url, dir.path(), 2), None, vec![]),
        30,
    )
    .await
    .unwrap();
    assert_eq!(second.path, dir.path().join("résumé v2 (1).pdf"));
    assert_eq!(sha256_file(&second.path), expected_sha256(24, 1000));
    // Overwrite and Fail policies.
    let mut req = request(&url, dir.path(), 2);
    req.destination.conflict = ConflictPolicy::Overwrite;
    assert_eq!(
        wait(&engine().start(req, None, vec![]), 30)
            .await
            .unwrap()
            .path,
        first.path
    );
    let mut req = request(&url, dir.path(), 2);
    req.destination.conflict = ConflictPolicy::Fail;
    assert!(matches!(
        wait(&engine().start(req, None, vec![]), 30)
            .await
            .unwrap_err(),
        EngineError::FileExists { .. }
    ));
    // MIME-derived extension and explicit names.
    let u2 = srv.url("/file/getfile?size=10&type=application/zip");
    assert_eq!(
        wait(
            &engine().start(request(&u2, dir.path(), 1), None, vec![]),
            30
        )
        .await
        .unwrap()
        .path,
        dir.path().join("getfile.zip")
    );
    let mut req = request(&u2, dir.path(), 1);
    req.destination.file_name = Some("my:name?.zip".into());
    assert_eq!(
        wait(&engine().start(req, None, vec![]), 30)
            .await
            .unwrap()
            .path,
        dir.path().join("my_name_.zip")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn not_found_is_fatal_and_fast() {
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let t = Instant::now();
    let h = engine().start(
        request(&srv.url("/status/404"), dir.path(), 8),
        None,
        vec![],
    );
    assert_eq!(
        wait(&h, 30).await.unwrap_err(),
        EngineError::Http { status: 404 }
    );
    assert!(t.elapsed() < Duration::from_secs(3));
    assert_eq!(h.state(), DownloadState::Failed);
    assert!(
        std::fs::read_dir(dir.path()).unwrap().next().is_none(),
        "no file created"
    );
    let h = engine().start(
        request(&srv.url("/status/410"), dir.path(), 8),
        None,
        vec![],
    );
    assert_eq!(
        wait(&h, 30).await.unwrap_err(),
        EngineError::Http { status: 410 }
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chunked_response_without_length() {
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 3 * MIB + 333;
    let h = engine().start(
        request(
            &srv.url(&format!("/file/ch.bin?size={size}&seed=25&chunked=1")),
            dir.path(),
            8,
        ),
        None,
        vec![],
    );
    let done = wait(&h, 60).await.unwrap();
    assert_eq!(done.size, size);
    assert_eq!(sha256_file(&done.path), expected_sha256(25, size));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn empty_file_post_and_headers() {
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let done = wait(
        &engine().start(
            request(&srv.url("/file/empty.txt?size=0"), dir.path(), 4),
            None,
            vec![],
        ),
        30,
    )
    .await
    .unwrap();
    assert_eq!(done.size, 0);
    assert!(done.path.exists());

    let size = 2 * MIB;
    let mut req = request(
        &srv.url(&format!("/file/post.bin?size={size}&seed=26")),
        dir.path(),
        8,
    );
    req.method = HttpMethod::Post;
    req.body = Some("a=1&b=2".into());
    req.cookies = Some("sid=abc".into());
    req.referer = Some("https://example.com/page".into());
    req.user_agent = Some("PrismTest/1".into());
    req.headers = vec![("X-Custom".into(), "1".into())];
    let done = wait(&engine().start(req, None, vec![]), 30).await.unwrap();
    assert_eq!(sha256_file(&done.path), expected_sha256(26, size));
    let stats = srv.stats();
    let posts: Vec<_> = stats
        .requests
        .iter()
        .filter(|r| r.method == "POST")
        .collect();
    assert_eq!(posts.len(), 1, "POST downloads use a single connection");
    assert_eq!(posts[0].cookie.as_deref(), Some("sid=abc"));
    assert_eq!(
        posts[0].referer.as_deref(),
        Some("https://example.com/page")
    );
    assert_eq!(posts[0].user_agent.as_deref(), Some("PrismTest/1"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn set_connections_and_cancel() {
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 32 * MIB;
    let mut req = request(
        &srv.url(&format!("/file/sc.bin?size={size}&seed=27&throttle=1M")),
        dir.path(),
        2,
    );
    // Surplus workers retire between requests; at 1 MiB/s a default (≥ 4 MiB) request would
    // keep all 8 connections busy long enough to finish the file before they are lowered.
    req.max_request_size = Some(512 * 1024);
    let h = engine().start(req, None, vec![]);
    wait_for_progress(&h, 256 * 1024).await;
    h.set_connections(8);
    let snap = wait_until(&h, "connections raised to 6+", |s| {
        s.active_connections >= 6
    })
    .await;
    assert_eq!(snap.target_connections, 8);
    h.set_connections(2);
    wait_until(&h, "connections lowered to 3-", |s| {
        s.active_connections <= 3
    })
    .await;
    assert!(
        !h.is_finished(),
        "finished before the connections were lowered"
    );
    let part = h.snapshot().path.unwrap().with_extension("bin.prismpart");
    assert!(part.exists());
    h.cancel(true);
    assert_eq!(wait(&h, 10).await.unwrap_err(), EngineError::Cancelled);
    assert!(!part.exists(), "part file deleted on cancel");
    assert_eq!(h.state(), DownloadState::Cancelled);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn probe_reports_metadata() {
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let info = engine()
        .probe(&request(
            &srv.url("/file/pr.bin?size=12345&type=application/pdf"),
            dir.path(),
            1,
        ))
        .await
        .unwrap();
    assert_eq!(info.size, Some(12345));
    assert!(info.resumable && info.accept_ranges);
    assert_eq!(info.file_name, "pr.bin");
    assert_eq!(info.mime.as_deref(), Some("application/pdf"));
    assert!(info.etag.is_some() && info.last_modified.is_some());
    assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    let _ = versioned_seed(1, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn engine_config_is_live_updatable() {
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let eng = engine();
    let mut cfg = (*eng.config()).clone();
    cfg.user_agent = "Updated/2".into();
    cfg.default_connections = 3;
    eng.update_config(cfg);
    assert_eq!(eng.config().user_agent, "Updated/2");
    wait(
        &eng.start(
            request(&srv.url("/file/u.bin?size=100"), dir.path(), 0),
            None,
            vec![],
        ),
        30,
    )
    .await
    .unwrap();
    assert!(
        srv.stats()
            .requests
            .iter()
            .all(|r| r.user_agent.as_deref() == Some("Updated/2"))
    );
}

/// Cloudflare's bot check (#14, softpedia.com): it fails at once with `BrowserCheck` (no retries,
/// nothing written), since only the browser can pass it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn browser_check_fails_at_once_without_a_file() {
    init_tracing();
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let t = Instant::now();
    let h = engine().start(request(&srv.url("/challenge"), dir.path(), 8), None, vec![]);
    assert_eq!(wait(&h, 30).await.unwrap_err(), EngineError::BrowserCheck);
    assert!(t.elapsed() < Duration::from_secs(3));
    assert!(
        std::fs::read_dir(dir.path()).unwrap().next().is_none(),
        "no file created"
    );
    let probe = engine()
        .probe(&request(&srv.url("/challenge"), dir.path(), 8))
        .await;
    assert_eq!(probe.unwrap_err(), EngineError::BrowserCheck);
}

/// A small web page where a file (`setup.exe`) was expected fails with `WebPage` instead of being
/// saved as `setup.exe`; without that expectation, and for big files with a wrong `text/html`
/// type, nothing changes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn web_page_instead_of_the_file_is_an_error() {
    init_tracing();
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let page = srv.url("/file/setup.exe?size=11389&seed=3&type=text/html");

    let mut req = request(&page, dir.path(), 8);
    req.expect_file = true;
    let h = engine().start(req, None, vec![]);
    assert_eq!(wait(&h, 30).await.unwrap_err(), EngineError::WebPage);
    assert!(!dir.path().join("setup.exe").exists());

    // Same answer, but nothing said it must be a file: downloaded as before.
    let done = wait(
        &engine().start(request(&page, dir.path(), 8), None, vec![]),
        30,
    )
    .await
    .unwrap();
    assert_eq!(sha256_file(&done.path), expected_sha256(3, 11389));
    std::fs::remove_file(&done.path).unwrap();

    // A big file sent as text/html is still the file.
    let size = 3 * MIB;
    let mut req = request(
        &srv.url(&format!("/file/big.iso?size={size}&seed=4&type=text/html")),
        dir.path(),
        8,
    );
    req.expect_file = true;
    let done = wait(&engine().start(req, None, vec![]), 60).await.unwrap();
    assert_eq!(sha256_file(&done.path), expected_sha256(4, size));
}

/// Parts of a file read with range requests: the first read learns the size, a transfer cut off
/// on the way continues where it stopped, a server without ranges is told apart, and a file
/// that changes between two reads is an error, not mixed bytes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_file_reads_parts_and_continues_where_it_stopped() {
    init_tracing();
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let open = |path: &str| {
        engine()
            .remote_file(&request(&srv.url(path), dir.path(), 1))
            .unwrap()
    };
    let size = 4 * MIB;
    // Throttled, so the client has read most of a response when the server drops it (a reset
    // throws away whatever the client hasn't read yet).
    let mut file = open(&format!(
        "/file/r.bin?size={size}&seed=21&drop_after=300K&throttle=4M"
    ));
    assert_eq!(file.size(), None);
    let tail = file.read(size - 1000, 1000).await.unwrap();
    assert_eq!(tail, expected_bytes(21, size - 1000..size));
    assert_eq!(file.size(), Some(size));
    // 1 MiB from a server that drops every response after 300 KiB.
    let part = file.new_reader().read(1000, MIB).await.unwrap();
    assert_eq!(part, expected_bytes(21, 1000..1000 + MIB));
    assert!(srv.stats().requests_to("/file/r.bin") >= 5);

    let mut whole = open("/file/w.bin?size=100000&ranges=0");
    assert_eq!(
        whole.read(0, 10).await.unwrap_err(),
        EngineError::RangeNotSupported
    );

    let mut changing = open("/file/v.bin?size=200000");
    changing.read(0, 10).await.unwrap();
    srv.set_version("v.bin", 1);
    assert_eq!(
        changing.read(10, 10).await.unwrap_err(),
        EngineError::ValidatorMismatch
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cross_origin_redirect_strips_cookie_and_auth() {
    init_tracing();
    let origin = default_server().await;
    let cdn = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 2 * MIB;
    let target = cdn.url(&format!("/file/cdn.bin?size={size}&seed=31"));
    let enc: String = url::form_urlencoded::byte_serialize(target.as_bytes()).collect();
    let mut req = request(
        &origin.url(&format!("/redirect/1?code=302&to={enc}")),
        dir.path(),
        4,
    );
    req.cookies = Some("sid=secret".into());
    req.auth = Some(Credentials {
        username: "u".into(),
        password: "p".into(),
        scheme: AuthScheme::Basic,
    });
    let done = wait(&engine().start(req, None, vec![]), 60).await.unwrap();
    assert_eq!(sha256_file(&done.path), expected_sha256(31, size));

    let cdn_stats = cdn.stats();
    assert!(!cdn_stats.requests.is_empty());
    for r in &cdn_stats.requests {
        assert_eq!(r.cookie, None, "cross-origin redirect leaked Cookie: {r:?}");
        assert!(
            !r.authorization,
            "cross-origin redirect leaked Authorization: {r:?}"
        );
    }
}
