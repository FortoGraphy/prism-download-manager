//! Videos picked in the video dialog: playlist entries named after their own titles (numbered
//! when Settings › Video asks), the "One at a time" queue, the container set in Settings, and
//! a part of a video ("Only a part").

use std::path::{Path, PathBuf};
use std::time::Duration;

use prism_core::events::CoreEvent;
use prism_core::manager::{Core, CoreOptions, ONE_AT_A_TIME_QUEUE, VideoDecision};
use serde_json::json;

const PLAYLIST: &str = "https://www.youtube.com/playlist?list=PLtest";
const A: &str = "https://www.youtube.com/watch?v=aaaaaaaaaaa";
const B: &str = "https://www.youtube.com/watch?v=bbbbbbbbbbb";
const C: &str = "https://www.youtube.com/watch?v=ccccccccccc";

async fn core_in(dir: &Path) -> Core {
    let core = Core::start(CoreOptions {
        data_dir: dir.join("data"),
        ipc: false,
        clipboard: false,
    })
    .await
    .unwrap();
    core.update_settings(&json!({
        "downloads": {"defaultDir": dir.join("dl"), "useCategoryFolders": false},
        "general": {"showCaptureDialog": false},
        "antivirus": {"enabled": false},
    }))
    .unwrap();
    core
}

/// What the video dialog sends for a playlist (MKV chosen in Settings, so the dialog asks for it).
fn decision(entries: serde_json::Value, extra: serde_json::Value) -> VideoDecision {
    let mut d = json!({
        "pageUrl": PLAYLIST,
        "title": "My playlist",
        "choice": {"id": "v1080", "kind": "video", "label": "1080p", "subLabel": "", "container": "mp4", "height": 1080},
        "options": {"container": "mkv"},
        "entries": entries,
        "mode": {"type": "later"},
    });
    d.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    serde_json::from_value(d).unwrap()
}

fn names(items: &[prism_core::model::DownloadItem]) -> Vec<String> {
    items.iter().map(|i| i.file_name.clone()).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn playlist_videos_keep_their_titles_and_are_numbered_when_asked() {
    let tmp = tempfile::tempdir().unwrap();
    let core = core_in(tmp.path()).await;

    let items = core
        .confirm_video(decision(
            json!([
                {"url": A, "title": "First clip", "index": 1},
                {"url": B, "title": "Second clip", "index": 12},
                C,
            ]),
            json!({}),
        ))
        .unwrap();
    assert_eq!(
        names(&items),
        [
            "First clip [aaaaaaaaaaa].mkv",
            "Second clip [bbbbbbbbbbb].mkv",
            "Video [ccccccccccc].mkv"
        ]
    );
    let index = |i: usize| items[i].media.as_ref().unwrap().playlist_index;
    assert_eq!((index(0), index(1), index(2)), (Some(1), Some(12), None));
    assert!(
        items
            .iter()
            .all(|i| i.media.as_ref().unwrap().title.as_deref() != Some("My playlist")),
        "the playlist's title is not a video's"
    );

    // One video picked from a playlist is named after that video, not the playlist.
    let one = core
        .confirm_video(decision(
            json!([{"url": A, "title": "Only this one", "index": 3}]),
            json!({}),
        ))
        .unwrap();
    assert_eq!(names(&one), ["Only this one [aaaaaaaaaaa].mkv"]);

    core.update_settings(&json!({"video": {"numberPlaylistVideos": true}}))
        .unwrap();
    let items = core
        .confirm_video(decision(
            json!([
                {"url": A, "title": "First clip", "index": 1},
                {"url": B, "title": "Second clip", "index": 12},
            ]),
            json!({}),
        ))
        .unwrap();
    assert_eq!(
        names(&items),
        [
            "01 - First clip [aaaaaaaaaaa].mkv",
            "12 - Second clip [bbbbbbbbbbb].mkv"
        ]
    );
    // As wide as the last position.
    let items = core
        .confirm_video(decision(
            json!([
                {"url": A, "title": "First clip", "index": 1},
                {"url": B, "title": "Last clip", "index": 120},
            ]),
            json!({}),
        ))
        .unwrap();
    assert_eq!(
        names(&items),
        [
            "001 - First clip [aaaaaaaaaaa].mkv",
            "120 - Last clip [bbbbbbbbbbb].mkv"
        ]
    );
    // A single video (not from a playlist) is never numbered.
    let single = core
        .confirm_video(
            serde_json::from_value(json!({
                "pageUrl": A, "title": "Just a video",
                "choice": {"id": "v1080", "kind": "video", "label": "1080p", "subLabel": "", "container": "mp4", "height": 1080},
                "options": {"container": "mkv"}, "mode": {"type": "later"},
            }))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(names(&single), ["Just a video [aaaaaaaaaaa].mkv"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_at_a_time_puts_the_playlist_into_a_one_download_queue() {
    let tmp = tempfile::tempdir().unwrap();
    let core = core_in(tmp.path()).await;
    let entries = json!([{"url": A, "title": "First", "index": 1}, {"url": B, "title": "Second", "index": 2}]);

    let items = core
        .confirm_video(decision(entries.clone(), json!({"oneAtATime": true})))
        .unwrap();
    assert!(
        items
            .iter()
            .all(|i| i.queue_id.as_deref() == Some(ONE_AT_A_TIME_QUEUE)),
        "{items:?}"
    );
    let queue = || {
        core.queues()
            .into_iter()
            .find(|q| q.id == ONE_AT_A_TIME_QUEUE)
            .unwrap()
    };
    assert_eq!(queue().max_concurrent, 1);
    assert_eq!(queue().name, "One at a time");
    assert!(!queue().running, "\"Later\" doesn't start it");

    // Edited to more downloads at once: using it again makes it one at a time again.
    let mut q = queue();
    q.max_concurrent = 3;
    core.upsert_queue(q).unwrap();
    core.confirm_video(decision(entries.clone(), json!({"oneAtATime": true})))
        .unwrap();
    assert_eq!(queue().max_concurrent, 1);
    assert_eq!(
        core.queues()
            .iter()
            .filter(|q| q.id == ONE_AT_A_TIME_QUEUE)
            .count(),
        1
    );

    // Without the option the entries don't join it.
    let items = core.confirm_video(decision(entries, json!({}))).unwrap();
    assert!(items.iter().all(|i| i.queue_id.is_none()));
}

/// A stand-in yt-dlp printing `fixture` (after ~1 s) for anything but `--version`. Each call's
/// arguments are added to `args.txt` next to it.
fn fake_ytdlp(dir: &Path, fixture: &str) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../prism-video/tests/fixtures")
        .join(fixture);
    // One line, as yt-dlp prints it.
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(src).unwrap()).unwrap();
    std::fs::write(dir.join("probe.json"), json.to_string()).unwrap();
    #[cfg(windows)]
    {
        let exe = dir.join("yt-dlp.cmd");
        std::fs::write(
            &exe,
            "@echo off\r\n\
             if \"%1\"==\"--version\" (echo 2026.10.10& exit /b 0)\r\n\
             echo %*>>\"%~dp0args.txt\"\r\n\
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
            "#!/bin/sh\n[ \"$1\" = --version ] && echo 2026.10.10 && exit 0\necho \"$*\" >> \"$(dirname \"$0\")/args.txt\"\nsleep 1\ncat \"$(dirname \"$0\")/probe.json\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        exe
    }
}

/// A core whose yt-dlp is the fake above (and whose other video tools exist), so nothing is
/// downloaded from the internet.
async fn core_with_fake_tools(dir: &Path, fixture: &str) -> Core {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let ytdlp = fake_ytdlp(&bin, fixture);
    let tools = dir.join("data").join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    let suffix = std::env::consts::EXE_SUFFIX;
    std::fs::write(tools.join(format!("deno{suffix}")), b"").unwrap();
    let ffmpeg = bin.join(format!("ffmpeg{suffix}"));
    std::fs::write(&ffmpeg, b"").unwrap();
    std::fs::write(bin.join(format!("ffprobe{suffix}")), b"").unwrap();
    let core = core_in(dir).await;
    core.update_settings(
        &json!({"video": {"ytdlpPath": ytdlp, "ffmpegPath": ffmpeg, "autoUpdate": false}}),
    )
    .unwrap();
    core
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_quality_list_is_for_the_container_set_in_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let core = core_with_fake_tools(tmp.path(), "youtube_bbb.json").await;
    let containers = |p: &serde_json::Value| -> Vec<String> {
        p["choices"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["kind"] == "video")
            .map(|c| c["container"].as_str().unwrap().to_string())
            .collect()
    };
    let mp4 = core
        .probe_video(
            "https://www.youtube.com/watch?v=aqz-KE-bpKQ",
            Default::default(),
        )
        .await
        .unwrap();
    assert!(
        containers(&mp4).iter().any(|c| c == "mp4"),
        "{:?}",
        containers(&mp4)
    );

    core.update_settings(&json!({"video": {"container": "mkv"}}))
        .unwrap();
    // Another page: the one above is answered from memory for a few minutes.
    let mkv = core
        .probe_video(
            "https://www.youtube.com/watch?v=aqz-KE-bpKQ&t=1",
            Default::default(),
        )
        .await
        .unwrap();
    let list = containers(&mkv);
    assert!(
        !list.is_empty() && list.iter().all(|c| c == "mkv"),
        "{list:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_playlist_video_without_a_title_is_named_once_its_page_is_read() {
    let tmp = tempfile::tempdir().unwrap();
    let core = core_with_fake_tools(tmp.path(), "youtube_bbb.json").await;
    core.update_settings(&json!({"video": {"numberPlaylistVideos": true, "accelerate": false}}))
        .unwrap();
    let mut rx = core.subscribe();
    let items = core
        .confirm_video(decision(
            json!([{"url": "https://www.youtube.com/watch?v=aqz-KE-bpKQ", "index": 3}]),
            json!({"mode": {"type": "now"}}),
        ))
        .unwrap();
    assert_eq!(names(&items), ["03 - Video [aqz-KE-bpKQ].mkv"]);
    let id = items[0].id.clone();
    let renamed = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match rx.recv().await {
                Ok(CoreEvent::ItemUpdated(i)) if i.id == id && !i.file_name.contains("Video [") => {
                    return i;
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(e) => panic!("{e}"),
            }
        }
    })
    .await
    .expect("the entry was not renamed");
    assert_eq!(
        renamed.file_name,
        "03 - Big Buck Bunny 60fps 4K - Official Blender Foundation Short Film [aqz-KE-bpKQ].mkv"
    );
    let media = renamed.media.unwrap();
    assert_eq!(
        media.title.as_deref(),
        Some("Big Buck Bunny 60fps 4K - Official Blender Foundation Short Film")
    );
    assert_eq!(media.playlist_index, Some(3));
    core.pause_items(&[id]);
}

const BBB: &str = "https://www.youtube.com/watch?v=aqz-KE-bpKQ";

/// What the video dialog sends for "Only a part" of Big Buck Bunny (10:35).
fn part_decision(section: serde_json::Value) -> VideoDecision {
    serde_json::from_value(json!({
        "pageUrl": BBB,
        "title": "Big Buck Bunny",
        "duration": 635,
        "choice": {"id": "v1080p60", "kind": "video", "label": "1080p60 · H.264", "subLabel": "", "container": "mp4", "height": 1080},
        "options": {"container": "mp4"},
        "section": section,
        "mode": {"type": "later"},
    }))
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_part_of_a_video_keeps_its_range_across_a_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let part = prism_video::Section::new(83.0, 165.0);
    let id = {
        let core = core_with_fake_tools(tmp.path(), "youtube_bbb.json").await;
        let items = core
            .confirm_video(part_decision(json!({"start": 83, "end": "2:45"})))
            .unwrap();
        // Named apart from the whole video.
        assert_eq!(
            names(&items),
            ["Big Buck Bunny [aqz-KE-bpKQ] (1m23s–2m45s).mp4"]
        );
        assert_eq!(items[0].media.as_ref().unwrap().section, Some(part));
        // A paused cut starts over: honest about it.
        assert_eq!(items[0].resumable, Some(false));
        // The whole video, picked as a "part", is just the video.
        let whole = core
            .confirm_video(part_decision(json!({"start": 0, "end": 635})))
            .unwrap();
        assert_eq!(names(&whole), ["Big Buck Bunny [aqz-KE-bpKQ].mp4"]);
        assert_eq!(whole[0].media.as_ref().unwrap().section, None);
        core.shutdown().await;
        items[0].id.clone()
    };

    // The app restarts; the job still holds the range.
    let core = core_in(tmp.path()).await;
    let item = core.item(&id).expect("item persisted");
    assert_eq!(item.media.as_ref().unwrap().section, Some(part));
    assert_eq!(
        item.request.as_ref().unwrap()["section"],
        json!({"start": 83.0, "end": 165.0})
    );
    // Started now, yt-dlp is asked for the same part (not the accelerated whole file).
    let args_file = tmp.path().join("bin").join("args.txt");
    core.start_items(std::slice::from_ref(&id));
    let args = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let text = std::fs::read_to_string(&args_file).unwrap_or_default();
            if let Some(line) = text.lines().find(|l| l.contains("--download-sections")) {
                return line.to_string();
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("yt-dlp was not asked for the part");
    assert!(args.contains("--download-sections *83-165"), "{args}");
    assert!(args.contains("--force-keyframes-at-cuts"), "{args}");
    assert!(args.contains("(1m23s"), "{args}");
    core.shutdown().await;
}
