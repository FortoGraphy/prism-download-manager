//! The quality picker's probe: a page probed a moment ago answers from memory, and two requests
//! for the same page while it is being probed run yt-dlp once.

use std::path::{Path, PathBuf};

use prism_core::manager::{Core, CoreOptions};

/// A stand-in yt-dlp that appends a line to `runs.txt` for every probe (not `--version`), takes
/// ~1 s, and prints a probe result.
fn fake_ytdlp(dir: &Path) -> PathBuf {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../prism-video/tests/fixtures/generic_direct.json");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture).unwrap()).unwrap();
    std::fs::write(dir.join("probe.json"), json.to_string()).unwrap();
    #[cfg(windows)]
    {
        let exe = dir.join("yt-dlp.cmd");
        std::fs::write(
            &exe,
            "@echo off\r\n\
             if \"%1\"==\"--version\" (echo 2026.10.10& exit /b 0)\r\n\
             echo run>>\"%~dp0runs.txt\"\r\n\
             ping -n 2 127.0.0.1 >nul\r\n\
             type \"%~dp0probe.json\"\r\n",
        )
        .unwrap();
        exe
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let exe = dir.join("yt-dlp");
        std::fs::write(
            &exe,
            "#!/bin/sh\n[ \"$1\" = --version ] && echo 2026.10.10 && exit 0\nd=$(dirname \"$0\")\necho run >> \"$d/runs.txt\"\nsleep 1\ncat \"$d/probe.json\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        exe
    }
}

fn runs(dir: &Path) -> usize {
    std::fs::read_to_string(dir.join("runs.txt"))
        .map(|t| t.lines().count())
        .unwrap_or(0)
}

#[tokio::test]
async fn probe_is_shared_while_running_and_reused_afterwards() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let ytdlp = fake_ytdlp(&bin);
    let data = tmp.path().join("data");
    // A managed deno so the probe doesn't go and install the video tools.
    let tools = data.join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    std::fs::write(
        tools.join(format!("deno{}", std::env::consts::EXE_SUFFIX)),
        b"",
    )
    .unwrap();
    let core = Core::start(CoreOptions {
        data_dir: data,
        ipc: false,
        clipboard: false,
    })
    .await
    .unwrap();
    core.update_settings(&serde_json::json!({"video": {"ytdlpPath": ytdlp, "autoUpdate": false}}))
        .unwrap();

    let url = "https://example.com/watch/1";
    let (a, b) = tokio::join!(
        core.probe_video(url, Default::default()),
        core.probe_video(url, Default::default())
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(a, b);
    assert_eq!(a["title"], "Big_Buck_Bunny_360_10s_1MB", "{a}");
    assert_eq!(runs(&bin), 1, "two requests at once ran yt-dlp once");

    let t = std::time::Instant::now();
    core.probe_video(url, Default::default()).await.unwrap();
    assert!(
        t.elapsed() < std::time::Duration::from_millis(500),
        "{:?}",
        t.elapsed()
    );
    assert_eq!(runs(&bin), 1, "a recent probe answers from memory");

    core.probe_video("https://example.com/watch/2", Default::default())
        .await
        .unwrap();
    assert_eq!(runs(&bin), 2, "another page is probed");
}
