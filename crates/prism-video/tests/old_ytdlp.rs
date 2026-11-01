//! #43: a yt-dlp from a Linux distribution's packages, older than `--js-runtimes`, stopped every
//! download with "no such option: --js-runtimes". Prism checks the version of a yt-dlp that isn't
//! its own, leaves the option out for an old one, and installs its own yt-dlp in its place (see
//! `VideoService::needs_tools`).
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use prism_video::{VideoOptions, VideoService, YtDlpJob};

/// A stand-in yt-dlp of `version` that fails every real run.
fn script(dir: &Path, name: &str, version: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(
        &p,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo {version}; exit 0; fi\n\
             echo 'yt-dlp: error: no such option: --js-runtimes' >&2; exit 2\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

fn job(dir: &Path) -> YtDlpJob {
    YtDlpJob {
        url: "https://example.com/v/1".into(),
        args: vec![],
        output_dir: dir.to_path_buf(),
        context: Default::default(),
        reason: "test".into(),
        section: None,
    }
}

fn has_js_runtimes(svc: &VideoService, dir: &Path) -> bool {
    svc.ytdlp_command_line(&job(dir), None)
        .iter()
        .any(|a| a == "--js-runtimes")
}

#[tokio::test]
async fn an_old_ytdlp_gets_no_js_runtime_option() {
    let dir = tempfile::tempdir().unwrap();
    let deno = script(dir.path(), "deno", "2.5.6");
    let svc = |ytdlp: PathBuf| {
        VideoService::new(
            dir.path().join("tools"),
            VideoOptions {
                ytdlp_path: Some(ytdlp),
                deno_path: Some(deno.clone()),
                ..Default::default()
            },
        )
    };

    let old = svc(script(dir.path(), "yt-dlp-old", "2024.04.09"));
    // Before its version is known, nothing changes.
    assert!(has_js_runtimes(&old, dir.path()));
    // A probe asks for it first (and then fails: the script is no real yt-dlp)…
    let _ = old
        .probe("https://example.com/v/1", &Default::default())
        .await;
    // …and the old yt-dlp no longer gets the option it doesn't know.
    assert!(!has_js_runtimes(&old, dir.path()));

    let current = svc(script(dir.path(), "yt-dlp-new", "2026.10.29"));
    let _ = current
        .probe("https://example.com/v/1", &Default::default())
        .await;
    assert!(has_js_runtimes(&current, dir.path()));
}

/// A stand-in ffmpeg (or ffprobe) that runs, or one that fails like a broken build (#45).
fn ffmpeg(dir: &Path, name: &str, works: bool) {
    let p = dir.join(name);
    let body = if works {
        "#!/bin/sh\necho 'ffmpeg version 7.1 Copyright (c) 2000-2026 the FFmpeg developers'\n"
    } else {
        "#!/bin/sh\necho 'error while loading shared libraries: libavdevice.so.61' >&2; exit 127\n"
    };
    std::fs::write(&p, body).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[tokio::test]
async fn system_tools_that_cant_do_the_job_get_prisms_own_installed() {
    // The tools come from PATH (a distribution's packages, /usr/local/bin); Prism has none yet.
    let dir = tempfile::tempdir().unwrap();
    let bin = |name: &str, ytdlp: &str, ffmpeg_works: bool| {
        let b = dir.path().join(name);
        std::fs::create_dir_all(&b).unwrap();
        script(&b, "yt-dlp", ytdlp);
        ffmpeg(&b, "ffmpeg", ffmpeg_works);
        ffmpeg(&b, "ffprobe", true);
        b
    };
    let old_ytdlp = bin("old", "2025.09.26", true);
    let broken_ffmpeg = bin("broken", "2026.10.29", false);
    let good = bin("good", "2025.11.12", true);
    let deno = script(dir.path(), "deno", "2.5.6");
    let svc = VideoService::new(
        dir.path().join("tools"),
        VideoOptions {
            deno_path: Some(deno),
            ..Default::default()
        },
    );
    // SAFETY: the only test in this binary that reads PATH.
    unsafe { std::env::set_var("PATH", &old_ytdlp) };
    assert!(svc.needs_tools().await, "#43: an old yt-dlp is replaced");
    unsafe { std::env::set_var("PATH", &broken_ffmpeg) };
    assert!(
        svc.needs_tools().await,
        "#45: an ffmpeg that doesn't run is replaced"
    );
    unsafe { std::env::set_var("PATH", &good) };
    assert!(
        !svc.needs_tools().await,
        "current tools that run are used as they are"
    );
}
