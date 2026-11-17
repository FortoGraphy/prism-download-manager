//! HLS and DASH integration tests. Synthetic fixtures verify byte-exact output (no remux);
//! real streams generated with ffmpeg verify the remux path (skipped when ffmpeg is missing).

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use bytes::Bytes;
use common::*;
use prism_engine::{
    DownloadState, EngineError, MediaKind, MediaRequest, ResumeKind, ResumeState, Url,
    VariantSelection,
};
use prism_testserver::{DashFixture, DashRepresentation, HlsFixture, HlsVariant, ServerConfig};

fn media_request(url: &str, dir: &Path, remux: bool) -> MediaRequest {
    let mut r = MediaRequest::new(Url::parse(url).unwrap(), dir);
    r.remux = remux;
    r.connections = 4;
    r
}

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hls_plain_variant_selection() {
    let fixture = HlsFixture::synthetic(3, 12, 64 * 1024 + 17, 1);
    let srv = server(ServerConfig {
        hls: Some(fixture.clone()),
        ..Default::default()
    })
    .await;
    for (sel, expected_variant) in [
        (VariantSelection::Best, 2),
        (VariantSelection::MaxHeight(400), 0),
        (VariantSelection::Index(1), 1),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut req = media_request(&srv.url("/hls/master.m3u8"), dir.path(), false);
        req.variant = sel;
        let h = engine().start_media(req, None, vec![]);
        let done = wait(&h, 60).await.unwrap();
        assert_eq!(done.path, dir.path().join("master.ts"));
        assert!(done.warning.is_some());
        assert_eq!(
            sha256_file(&done.path),
            sha256(&fixture.expected_variant_bytes(expected_variant)),
            "{sel:?}"
        );
        // The work directory is cleaned up.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hls_aes128_byte_ranges_and_audio_rendition() {
    for (explicit_iv, byte_ranges) in [(None, false), (Some([9u8; 16]), true)] {
        let mut fixture = HlsFixture::synthetic(2, 10, 50_000, 2);
        fixture.key = Some(*b"0123456789abcdef");
        fixture.explicit_iv = explicit_iv;
        fixture.byte_ranges = byte_ranges;
        fixture.media_sequence = 5;
        fixture.audio = Some(
            (0..10)
                .map(|i| Bytes::from(vec![i as u8; 3000 + i]))
                .collect(),
        );
        let srv = server(ServerConfig {
            hls: Some(fixture.clone()),
            ..Default::default()
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let h = engine().start_media(
            media_request(&srv.url("/hls/master.m3u8"), dir.path(), false),
            None,
            vec![],
        );
        let done = wait(&h, 60).await.unwrap();
        assert_eq!(
            sha256_file(&done.path),
            sha256(&fixture.expected_variant_bytes(1)),
            "iv {explicit_iv:?}"
        );
        assert_eq!(done.extra_files.len(), 1);
        assert_eq!(done.extra_files[0], dir.path().join("master.audio.aac"));
        assert_eq!(
            std::fs::read(&done.extra_files[0]).unwrap(),
            fixture.expected_audio_bytes()
        );
        // The key is fetched once, not per segment.
        assert_eq!(srv.stats().requests_to("/hls/key.bin"), 1);
    }
}

/// Issue #80: with `EXT-X-KEY` before `EXT-X-MAP` the fMP4 init section is encrypted too.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hls_aes128_encrypted_init_section() {
    for (explicit_iv, byte_ranges, encrypt_init) in [
        (None, false, true),
        (Some([9u8; 16]), false, true),
        (None, true, true),
        // MAP before KEY: the init section is served plain and must be left alone.
        (None, false, false),
    ] {
        let mut fixture = HlsFixture::synthetic(1, 8, 20_000, 80);
        // Starts like a real init section; 733 bytes is not a multiple of the block size.
        let mut init = b"\0\0\0\x1cftypiso5\0\0\x02\0iso5iso6mp41".to_vec();
        init.resize(733, 0x5a);
        fixture.variants[0].init = Some(Bytes::from(init));
        fixture.key = Some(*b"fedcba9876543210");
        fixture.explicit_iv = explicit_iv;
        fixture.byte_ranges = byte_ranges;
        fixture.media_sequence = 3;
        fixture.encrypt_init = encrypt_init;
        let srv = server(ServerConfig {
            hls: Some(fixture.clone()),
            ..Default::default()
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let h = engine().start_media(
            media_request(&srv.url("/hls/master.m3u8"), dir.path(), false),
            None,
            vec![],
        );
        let done = wait(&h, 60).await.unwrap();
        let case = format!("iv {explicit_iv:?} ranges {byte_ranges} encrypted {encrypt_init}");
        let got = std::fs::read(&done.path).unwrap();
        assert!(got.starts_with(b"\0\0\0\x1cftyp"), "{case}");
        assert!(got == fixture.expected_variant_bytes(0), "{case}");
        // The init section reuses the cached key.
        assert_eq!(srv.stats().requests_to("/hls/key.bin"), 1, "{case}");
        assert_eq!(srv.stats().requests_to("/hls/v0/init.mp4"), 1, "{case}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hls_pause_and_resume_skips_completed_segments() {
    let fixture = HlsFixture::synthetic(1, 30, 64 * 1024, 3);
    let srv = server(ServerConfig {
        hls: Some(fixture.clone()),
        ..Default::default()
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut req = media_request(&srv.url("/hls/master.m3u8"), dir.path(), false);
    req.speed_limit = 512 * 1024;
    let h = engine().start_media(req.clone(), None, vec![]);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let state = h.pause().await.expect("state");
    assert_eq!(h.state(), DownloadState::Paused);
    assert_eq!(state.kind, ResumeKind::Media);
    let media = state.media.clone().unwrap();
    assert!(
        !media.completed.is_empty() && media.completed.len() < 30,
        "{} completed",
        media.completed.len()
    );
    let state = ResumeState::from_json(&state.to_json()).unwrap();
    srv.reset_stats();
    req.speed_limit = 0;
    let done = wait(&engine().start_media(req, Some(state.clone()), vec![]), 60)
        .await
        .unwrap();
    assert_eq!(
        sha256_file(&done.path),
        sha256(&fixture.expected_variant_bytes(0))
    );
    let refetched = srv
        .stats()
        .requests
        .iter()
        .filter(|r| r.uri.contains("/seg"))
        .count();
    assert_eq!(
        refetched,
        30 - media.completed.len(),
        "completed segments must not be downloaded again"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dash_synthetic_template_and_timeline() {
    for timeline in [false, true] {
        let mut fixture = DashFixture::synthetic(2, 8, 40_000, 4);
        fixture.timeline = timeline;
        let srv = server(ServerConfig {
            dash: Some(fixture.clone()),
            ..Default::default()
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let mut req = media_request(&srv.url("/dash/manifest.mpd"), dir.path(), false);
        req.kind = MediaKind::Auto;
        let done = wait(&engine().start_media(req, None, vec![]), 60)
            .await
            .unwrap();
        assert_eq!(done.path, dir.path().join("manifest.mp4"));
        assert_eq!(
            sha256_file(&done.path),
            sha256(&DashFixture::expected_bytes(&fixture.video[1])),
            "timeline {timeline}"
        );
        assert_eq!(done.extra_files.len(), 1);
        assert_eq!(
            std::fs::read(&done.extra_files[0]).unwrap(),
            DashFixture::expected_bytes(&fixture.audio[0])
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unsupported_streams_and_cancel() {
    let srv = default_server().await;
    let dir = tempfile::tempdir().unwrap();
    let h = engine().start_media(
        media_request(&srv.url("/file/x.bin?size=100"), dir.path(), false),
        None,
        vec![],
    );
    assert!(matches!(
        wait(&h, 30).await.unwrap_err(),
        EngineError::Unsupported { .. }
    ));
    let h = engine().start_media(
        media_request(&srv.url("/status/404"), dir.path(), false),
        None,
        vec![],
    );
    assert_eq!(
        wait(&h, 30).await.unwrap_err(),
        EngineError::Http { status: 404 }
    );

    let fixture = HlsFixture::synthetic(1, 20, 64 * 1024, 5);
    let srv = server(ServerConfig {
        hls: Some(fixture),
        ..Default::default()
    })
    .await;
    let mut req = media_request(&srv.url("/hls/master.m3u8"), dir.path(), false);
    req.speed_limit = 256 * 1024;
    let h = engine().start_media(req, None, vec![]);
    tokio::time::sleep(Duration::from_millis(600)).await;
    let work = h.snapshot().path.unwrap().with_extension("ts.prismpart.d");
    assert!(work.is_dir(), "{}", work.display());
    h.cancel(true);
    assert_eq!(wait(&h, 10).await.unwrap_err(), EngineError::Cancelled);
    assert!(!work.exists());
}

/// Runs ffmpeg with the given arguments in `dir`.
fn ffmpeg(dir: &Path, args: &[&str]) {
    let out = Command::new("ffmpeg")
        .current_dir(dir)
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "ffmpeg failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn ffprobe_streams(path: &Path) -> Vec<String> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.split(',').next().unwrap_or("").trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

fn sorted_files(dir: &Path, prefix: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with(prefix))
        .collect();
    v.sort_by_key(|p| {
        let n: String = p
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect();
        n.parse::<u64>().unwrap_or(0)
    });
    v
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hls_real_stream_encrypted_remux_with_ffmpeg() {
    if !ffmpeg_available() {
        eprintln!("SKIPPED: ffmpeg not found");
        return;
    }
    let src = tempfile::tempdir().unwrap();
    ffmpeg(
        src.path(),
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=6:size=320x240:rate=25",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=6",
            // mpeg2video is built into every ffmpeg (mpeg4 part 2 in TS lacks in-band dimensions).
            "-c:v",
            "mpeg2video",
            "-c:a",
            "aac",
            "-f",
            "hls",
            "-hls_time",
            "2",
            "-hls_list_size",
            "0",
            "-hls_segment_filename",
            "seg%d.ts",
            "index.m3u8",
        ],
    );
    let segments: Vec<Bytes> = sorted_files(src.path(), "seg")
        .iter()
        .map(|p| Bytes::from(std::fs::read(p).unwrap()))
        .collect();
    assert!(segments.len() >= 3);
    let fixture = HlsFixture {
        variants: vec![HlsVariant {
            bandwidth: 500_000,
            resolution: Some((320, 240)),
            segments,
            init: None,
        }],
        audio: None,
        segment_duration: 2.0,
        media_sequence: 0,
        key: Some([42u8; 16]),
        explicit_iv: None,
        byte_ranges: false,
        encrypt_init: false,
    };
    let srv = server(ServerConfig {
        hls: Some(fixture),
        ..Default::default()
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let h = engine().start_media(
        media_request(&srv.url("/hls/master.m3u8"), dir.path(), true),
        None,
        vec![],
    );
    let done = wait(&h, 120).await.unwrap();
    assert_eq!(done.path, dir.path().join("master.mp4"));
    assert!(done.warning.is_none());
    let streams = ffprobe_streams(&done.path);
    assert!(
        streams.contains(&"video".to_string()) && streams.contains(&"audio".to_string()),
        "{streams:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dash_real_stream_mux_with_ffmpeg() {
    if !ffmpeg_available() {
        eprintln!("SKIPPED: ffmpeg not found");
        return;
    }
    let src = tempfile::tempdir().unwrap();
    ffmpeg(
        src.path(),
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=6:size=320x240:rate=25",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=6",
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-c:v",
            "mpeg4",
            "-c:a",
            "aac",
            "-f",
            "dash",
            "-seg_duration",
            "2",
            "-use_template",
            "1",
            "-use_timeline",
            "0",
            "-init_seg_name",
            "init-$RepresentationID$.m4s",
            "-media_seg_name",
            "chunk-$RepresentationID$-$Number%05d$.m4s",
            "out.mpd",
        ],
    );
    let rep = |id: &str, bandwidth: u64, height: Option<u32>| DashRepresentation {
        id: if id == "0" { "v0".into() } else { "a0".into() },
        bandwidth,
        width: height.map(|h| h * 4 / 3),
        height,
        init: Bytes::from(std::fs::read(src.path().join(format!("init-{id}.m4s"))).unwrap()),
        segments: sorted_files(src.path(), &format!("chunk-{id}-"))
            .iter()
            .map(|p| Bytes::from(std::fs::read(p).unwrap()))
            .collect(),
    };
    let video = rep("0", 500_000, Some(240));
    let audio = rep("1", 128_000, None);
    // Both representations must have the same number of segments for the fixture's manifest.
    let n = video.segments.len().min(audio.segments.len());
    let trim = |mut r: DashRepresentation| {
        r.segments.truncate(n);
        r
    };
    let fixture = DashFixture {
        video: vec![trim(video)],
        audio: vec![trim(audio)],
        segment_duration: 2,
        timeline: false,
    };
    let srv = server(ServerConfig {
        dash: Some(fixture),
        ..Default::default()
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let done = wait(
        &engine().start_media(
            media_request(&srv.url("/dash/manifest.mpd"), dir.path(), true),
            None,
            vec![],
        ),
        120,
    )
    .await
    .unwrap();
    assert!(done.warning.is_none());
    let streams = ffprobe_streams(&done.path);
    assert!(
        streams.contains(&"video".to_string()) && streams.contains(&"audio".to_string()),
        "{streams:?}"
    );
}
