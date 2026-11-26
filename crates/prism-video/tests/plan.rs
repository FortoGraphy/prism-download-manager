mod common;

use std::path::PathBuf;

use common::video;
use prism_video::plan::{
    accelerated_blocker, cookie_header_from_ytdlp, format_selector, lang_matches, plan, plan_ytdlp,
    select_subtitles,
};
use prism_video::probe::{ToolPaths, context_args, global_args, netscape_cookies, probe_args};
use prism_video::template::{render_template, sanitize_component};
use prism_video::{
    AudioFormat, Choice, Container, OutputOptions, Plan, RequestContext, Section, SponsorBlockMode,
    StreamRole, VideoOptions,
};

fn opts() -> OutputOptions {
    OutputOptions {
        output_dir: PathBuf::from("out"),
        ..Default::default()
    }
}

fn choice<'a>(v: &'a prism_video::VideoInfo, id: &str) -> &'a Choice {
    v.choices
        .iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("no choice {id}"))
}

#[test]
fn accelerated_plan_for_progressive_youtube_formats() {
    let v = video("youtube_bbb.json");
    let c = choice(&v, "v1080p60");
    let ctx = RequestContext {
        cookies: Some("SID=abc".into()),
        ..Default::default()
    };
    let o = OutputOptions {
        embed_subtitles: true,
        ..opts()
    };
    let Plan::Accelerated(p) = plan(&v, c, &o, &ctx, None, true, 8) else {
        panic!("expected accelerated")
    };
    let roles: Vec<StreamRole> = p.streams.iter().map(|s| s.role).collect();
    // No subtitles in this fixture; thumbnail embedded by default.
    assert_eq!(
        roles,
        [StreamRole::Video, StreamRole::Audio, StreamRole::Thumbnail]
    );
    let vs = &p.streams[0];
    assert_eq!(vs.format_id.as_deref(), Some("299"));
    assert_eq!(vs.chunk_size, Some(10_485_760));
    assert_eq!(vs.size, Some(257_619_653));
    assert!(vs.size_exact);
    assert_eq!(vs.file_name, "aqz-KE-bpKQ.f299.mp4");
    assert!(
        vs.headers
            .iter()
            .any(|(k, v)| k == "User-Agent" && v.contains("Chrome"))
    );
    // Page cookies are for youtube.com, the stream is on googlevideo.com: not forwarded.
    assert!(!vs.headers.iter().any(|(k, _)| k == "Cookie"));
    assert_eq!(p.streams[2].file_name, "aqz-KE-bpKQ.thumb.webp");
    assert!(p.streams[2].optional);

    let post = &p.post;
    assert_eq!(post.inputs.len(), 3);
    assert_eq!(post.inputs[0].file_name, vs.file_name);
    assert_eq!(post.container, Container::Mp4);
    assert!(post.audio_conversion.is_none());
    assert_eq!(
        post.output_path,
        PathBuf::from("out").join(
            "Big Buck Bunny 60fps 4K - Official Blender Foundation Short Film [aqz-KE-bpKQ].mp4"
        )
    );
    let meta = post.metadata.as_ref().unwrap();
    assert_eq!(meta.artist.as_deref(), Some("Blender"));
    assert_eq!(
        meta.comment.as_deref(),
        Some("https://www.youtube.com/watch?v=aqz-KE-bpKQ")
    );
    assert!(post.delete_inputs);
}

#[test]
fn hls_live_drm_and_sponsorblock_remove_fall_back_to_ytdlp() {
    let v = video("synthetic_hdr.json");
    let o = opts();
    // HLS-only 720p.
    let c720 = v.choices.iter().find(|c| c.height == Some(720)).unwrap();
    let reason = accelerated_blocker(&v, c720, &o).unwrap();
    assert!(reason.contains("m3u8_native"), "{reason}");
    assert!(matches!(
        plan(&v, c720, &o, &RequestContext::default(), None, true, 8),
        Plan::YtDlp(_)
    ));

    // Live.
    let mut live = v.clone();
    live.is_live = true;
    let c = v.choices.iter().find(|c| c.height == Some(1080)).unwrap();
    assert_eq!(
        accelerated_blocker(&live, c, &o).as_deref(),
        Some("live stream")
    );

    // SponsorBlock remove.
    let mut o2 = opts();
    o2.sponsorblock.mode = SponsorBlockMode::Remove;
    assert!(
        accelerated_blocker(&v, c, &o2)
            .unwrap()
            .contains("SponsorBlock")
    );

    // SponsorBlock mark is fine (done via the API + chapters).
    o2.sponsorblock.mode = SponsorBlockMode::Mark;
    let Plan::Accelerated(p) = plan(&v, c, &o2, &RequestContext::default(), None, true, 8) else {
        panic!()
    };
    let sb = p.post.sponsorblock.as_ref().unwrap();
    assert_eq!(sb.video_id, "HDRtest0001");

    // Unknown format id.
    let mut bogus = c.clone();
    bogus.video_format_id = Some("nope".into());
    assert!(
        accelerated_blocker(&v, &bogus, &o)
            .unwrap()
            .contains("not found")
    );
}

#[test]
fn subtitles_and_audio_conversion_in_accelerated_plan() {
    let v = video("synthetic_hdr.json");
    let c = v.choices.iter().find(|c| c.height == Some(1080)).unwrap();
    let o = OutputOptions {
        embed_subtitles: true,
        sub_langs: vec!["en".into(), "cs".into(), "de".into()],
        auto_subtitles: true,
        ..opts()
    };
    let Plan::Accelerated(p) = plan(&v, c, &o, &RequestContext::default(), None, false, 8) else {
        panic!()
    };
    let subs: Vec<_> = p
        .streams
        .iter()
        .filter(|s| s.role == StreamRole::Subtitle)
        .collect();
    let names: Vec<&str> = subs.iter().map(|s| s.file_name.as_str()).collect();
    // en: manual vtt preferred over json3; cs: srt; de: auto caption.
    assert_eq!(
        names,
        [
            "HDRtest0001.en.vtt",
            "HDRtest0001.cs.srt",
            "HDRtest0001.de.auto.vtt"
        ]
    );
    assert!(p.post.embed_subtitles);
    assert!(!p.post.delete_inputs);
    assert_eq!(p.post.chapters.len(), 3);

    // Audio only -> mp3 conversion of the best source (opus 251).
    let mp3 = v.choices.iter().find(|c| c.id == "a-mp3").unwrap();
    let Plan::Accelerated(p) = plan(&v, mp3, &opts(), &RequestContext::default(), None, true, 8)
    else {
        panic!()
    };
    assert_eq!(p.streams[0].role, StreamRole::Audio);
    assert_eq!(p.streams[0].format_id.as_deref(), Some("251"));
    let conv = p.post.audio_conversion.as_ref().unwrap();
    assert_eq!(conv.format, AudioFormat::Mp3);
    assert_eq!(conv.bitrate_kbps, Some(320));
    assert_eq!(conv.source_codec.as_deref(), Some("opus"));
    assert_eq!(p.post.output_path.extension().unwrap(), "mp3");
    assert!(
        p.post
            .output_path
            .to_string_lossy()
            .contains("HDR Test： ＂Colors＂ ⧸ 8K？")
    );
}

#[test]
fn ytdlp_args_video() {
    let v = video("synthetic_hdr.json");
    let c = v.choices.iter().find(|c| c.hdr).unwrap();
    let mut o = OutputOptions {
        embed_subtitles: true,
        write_subtitles: true,
        sub_langs: vec!["en".into(), "cs".into()],
        write_thumbnail: true,
        ..opts()
    };
    o.sponsorblock.mode = SponsorBlockMode::Remove;
    let job = plan_ytdlp(&v, c, &o, &RequestContext::default(), 8, "test".into());
    let a = job.args.join(" ");
    assert!(
        a.starts_with("-f 701+140/bv*[height<=2160]+ba/b[height<=2160]/b"),
        "{a}"
    );
    assert!(a.contains("--merge-output-format mp4 --remux-video mp4"));
    assert!(a.contains("-N 8"));
    assert!(a.contains("--sub-langs en,cs --write-subs --embed-subs"));
    assert!(a.contains("--embed-thumbnail --convert-thumbnails jpg --write-thumbnail"));
    assert!(a.contains("--embed-metadata --embed-chapters"));
    assert!(a.contains("--sponsorblock-remove sponsor,selfpromo,interaction"));
    assert!(a.contains("--progress-template download:PRISM %(progress)j"));
    assert!(a.contains("--progress-template postprocess:PRISMPP %(progress)j"));
    assert!(a.contains("--print after_move:PRISMFILE %(filepath)j"));
    assert!(a.contains("--no-mtime --windows-filenames --no-playlist"));
    assert!(a.ends_with("-P out -o %(title)s [%(id)s].%(ext)s"));
    assert_eq!(job.url, "https://www.youtube.com/watch?v=HDRtest0001");
}

#[test]
fn ytdlp_args_audio() {
    let v = video("youtube_bbb.json");
    let mp3 = choice(&v, "a-mp3");
    let job = plan_ytdlp(&v, mp3, &opts(), &RequestContext::default(), 4, "x".into());
    let a = job.args.join(" ");
    assert!(
        a.starts_with("-f 258/ba/b -x --audio-format mp3 --audio-quality 320K -N 4"),
        "{a}"
    );
    assert!(!a.contains("--merge-output-format"));
    let flac = choice(&v, "a-flac");
    let a = plan_ytdlp(&v, flac, &opts(), &RequestContext::default(), 4, "x".into())
        .args
        .join(" ");
    assert!(
        a.contains("-x --audio-format flac --audio-quality 0"),
        "{a}"
    );
}

#[test]
fn format_selectors() {
    let v = video("youtube_bbb.json");
    assert_eq!(
        format_selector(choice(&v, "v360p")),
        "134+258/bv*[height<=360]+ba/b[height<=360]/b"
    );
    assert_eq!(format_selector(choice(&v, "a-m4a")), "258/ba/b");
    let g = video("generic_direct.json");
    assert_eq!(format_selector(&g.choices[0]), "mp4/bv*+ba/b/b");
}

#[test]
fn probe_and_global_args() {
    let ctx = RequestContext {
        want_playlist: true,
        ..Default::default()
    };
    assert_eq!(
        probe_args("https://www.youtube.com/watch?v=x&list=PL1", &ctx),
        [
            "-J",
            "--no-warnings",
            "--flat-playlist",
            "--yes-playlist",
            "--",
            "https://www.youtube.com/watch?v=x&list=PL1"
        ]
    );
    assert_eq!(
        probe_args("u", &RequestContext::default())[3],
        "--no-playlist"
    );

    let dir = PathBuf::from("tools");
    let tools = ToolPaths {
        ytdlp: Some(dir.join("yt-dlp.exe")),
        ffmpeg: Some(dir.join("ffmpeg.exe")),
        ffprobe: Some(dir.join("ffprobe.exe")),
        deno: Some(dir.join("deno.exe")),
    };
    let o = VideoOptions {
        proxy: Some("socks5://127.0.0.1:9050".into()),
        cookies_from_browser: Some("firefox".into()),
        ..Default::default()
    };
    let g = global_args(&tools, &o);
    let deno = format!("deno:{}", dir.join("deno.exe").display());
    assert_eq!(
        g,
        [
            "--ignore-config",
            "--ffmpeg-location",
            "tools",
            "--js-runtimes",
            deno.as_str(),
            "--proxy",
            "socks5://127.0.0.1:9050",
            "--cookies-from-browser",
            "firefox"
        ]
    );
    // ffprobe elsewhere -> pass the ffmpeg binary itself.
    let t2 = ToolPaths {
        ffprobe: Some(PathBuf::from("other/ffprobe")),
        ..tools.clone()
    };
    assert_eq!(
        global_args(&t2, &VideoOptions::default())[2],
        dir.join("ffmpeg.exe").to_string_lossy()
    );
}

#[test]
fn context_args_and_cookies() {
    let ctx = RequestContext {
        cookies: Some("a=1; b=two".into()),
        referer: Some("https://example.com/page".into()),
        user_agent: Some("Mozilla/5.0 Browser".into()),
        headers: vec![
            ("X-Test".into(), "1".into()),
            ("Cookie".into(), "ignored".into()),
        ],
        want_playlist: false,
    };
    let o = VideoOptions::default();
    let cf = PathBuf::from("c.txt");
    let a = context_args("https://example.com/v/1", &ctx, &o, Some(&cf));
    assert_eq!(
        a,
        [
            "--cookies",
            "c.txt",
            "--add-headers",
            "Referer:https://example.com/page",
            "--add-headers",
            "User-Agent:Mozilla/5.0 Browser",
            "--add-headers",
            "X-Test:1"
        ]
    );
    // No forced UA on YouTube.
    let a = context_args("https://www.youtube.com/watch?v=1", &ctx, &o, None);
    assert!(!a.iter().any(|x| x.starts_with("User-Agent:")));

    let txt = netscape_cookies(
        "https://www.example.com/x",
        "a=1; b=two; junk",
        1_800_000_000,
    )
    .unwrap();
    assert!(txt.starts_with("# Netscape HTTP Cookie File"));
    assert!(txt.contains(".example.com\tTRUE\t/\tFALSE\t1800000000\ta\t1\n"));
    assert!(txt.contains(".example.com\tTRUE\t/\tFALSE\t1800000000\tb\ttwo\n"));
    assert!(!txt.contains("junk"));
    assert!(netscape_cookies("https://example.com", "", 0).is_none());
}

#[test]
fn ytdlp_cookie_field_parsing() {
    let s = "VISITOR_INFO1_LIVE=abc; Domain=.youtube.com; Path=/; Secure; Expires=1800000000; PREF=f6=40; Domain=.youtube.com; Path=/";
    assert_eq!(
        cookie_header_from_ytdlp(s).as_deref(),
        Some("VISITOR_INFO1_LIVE=abc; PREF=f6=40")
    );
    assert_eq!(cookie_header_from_ytdlp("Domain=x; Path=/"), None);
}

#[test]
fn subtitle_language_matching() {
    assert!(lang_matches("en", "en"));
    assert!(lang_matches("en", "en-US"));
    assert!(!lang_matches("en", "eng"));
    assert!(lang_matches("en.*", "en-GB"));
    assert!(lang_matches("all", "xx"));
    assert!(!lang_matches("-live_chat", "live_chat"));
    let v = video("synthetic_hdr.json");
    let picked = select_subtitles(&v.subtitles, &["all".into()], true);
    assert_eq!(
        picked.iter().map(|t| t.lang.as_str()).collect::<Vec<_>>(),
        ["cs", "en"]
    );
    let picked = select_subtitles(&v.subtitles, &["de".into(), "fr".into()], false);
    assert!(picked.is_empty());
}

#[test]
fn templates_and_sanitizing() {
    let v = video("synthetic_hdr.json");
    assert_eq!(
        render_template("%(title)s [%(id)s].%(ext)s", &v, "mkv"),
        PathBuf::from("HDR Test： ＂Colors＂ ⧸ 8K？ [HDRtest0001].mkv")
    );
    assert_eq!(
        render_template(
            "%(uploader)s/%(upload_date)s - %(nope)s 100%%.%(ext)s",
            &v,
            "mp4"
        ),
        PathBuf::from("Prism Test Channel").join("20260901 - NA 100%.mp4")
    );
    assert_eq!(
        render_template("../%(id)s.%(ext)s", &v, "mp4"),
        PathBuf::from("HDRtest0001.mp4")
    );
    assert_eq!(sanitize_component("CON.txt"), "_CON.txt");
    assert_eq!(sanitize_component("a\tb\nc. "), "a b c");
    assert_eq!(sanitize_component("   "), "_");
    assert_eq!(sanitize_component(&"x".repeat(300)).chars().count(), 180);
}

fn part(start: f64, end: f64) -> OutputOptions {
    OutputOptions {
        section: Some(Section::new(start, end)),
        ..opts()
    }
}

fn ytdlp_args(v: &prism_video::VideoInfo, c: &Choice, o: &OutputOptions) -> String {
    plan_ytdlp(v, c, o, &RequestContext::default(), 8, "x".into())
        .args
        .join(" ")
}

#[test]
fn a_part_is_downloaded_by_the_engine_and_cut_by_ffmpeg() {
    let v = video("youtube_bbb.json");
    let c = choice(&v, "v1080p60");
    let o = OutputOptions {
        embed_subtitles: true,
        write_subtitles: true,
        embed_chapters: true,
        ..part(83.0, 165.0)
    };
    // The core downloads only the pieces around the part (over many connections) and ffmpeg
    // cuts it out of them.
    assert!(accelerated_blocker(&v, c, &o).is_none());
    let Plan::Accelerated(p) = plan(&v, c, &o, &RequestContext::default(), None, true, 8) else {
        panic!("expected the accelerated plan")
    };
    let cut = p.post.cut.as_ref().expect("a cut");
    assert_eq!((cut.start, cut.length), (83.0, 82.0));
    assert_eq!(p.post.duration, Some(82.0));
    let roles: Vec<StreamRole> = p.streams.iter().map(|s| s.role).collect();
    assert!(roles.contains(&StreamRole::Video) && roles.contains(&StreamRole::Audio));
    // Subtitles and chapters are timed to the whole video: a part gets none.
    assert!(!roles.contains(&StreamRole::Subtitle), "{roles:?}");
    assert!(!p.post.embed_subtitles && !p.post.write_subtitles);
    assert!(!p.post.embed_chapters && p.post.chapters.is_empty());
    // The whole video has no cut.
    let Plan::Accelerated(w) = plan(&v, c, &opts(), &RequestContext::default(), None, true, 8)
    else {
        panic!("expected the accelerated plan")
    };
    assert!(w.post.cut.is_none());
}

#[test]
fn a_part_falls_back_to_ytdlp_which_copies_it() {
    let v = video("youtube_bbb.json");
    let c = choice(&v, "v1080p60");
    let o = part(83.0, 165.0);
    // A stream without an index at its start: yt-dlp has ffmpeg fetch and cut the part.
    let job = plan_ytdlp(&v, c, &o, &RequestContext::default(), 8, "x".into());
    assert_eq!(job.section, Some(Section::new(83.0, 165.0)));
    let a = job.args.join(" ");
    assert!(
        a.contains(
            "-N 8 --download-sections *83-165 \
             --downloader-args ffmpeg:-progress pipe:1 -nostats"
        ),
        "{a}"
    );
    // Copied, not encoded again on every core (#135).
    assert!(!a.contains("--force-keyframes-at-cuts"), "{a}");
    assert!(a.contains("--embed-thumbnail") && a.contains("--embed-metadata"));
    // Chapters, SponsorBlock segments and subtitles are timed to the whole video.
    assert!(!a.contains("--embed-chapters"), "{a}");
    let mut o2 = OutputOptions {
        embed_subtitles: true,
        write_subtitles: true,
        auto_subtitles: true,
        ..part(83.0, 165.0)
    };
    o2.sponsorblock.mode = SponsorBlockMode::Remove;
    let a = ytdlp_args(&v, c, &o2);
    assert!(!a.contains("--sponsorblock"), "{a}");
    assert!(!a.contains("-subs") && !a.contains("--sub-langs"), "{a}");
}

#[test]
fn an_audio_part_is_copied_not_encoded_twice() {
    let v = video("youtube_bbb.json");
    let mp3 = choice(&v, "a-mp3");
    let job = plan_ytdlp(
        &v,
        mp3,
        &part(0.0, 10.5),
        &RequestContext::default(),
        8,
        "x".into(),
    );
    let a = job.args.join(" ");
    assert!(a.contains("-x --audio-format mp3"), "{a}");
    assert!(a.contains("--download-sections *0-10.5"), "{a}");
    assert!(!a.contains("--force-keyframes-at-cuts"), "{a}");
    assert_eq!(job.section.map(|s| s.length()), Some(10.5));
}

#[test]
fn a_part_is_cut_to_the_video() {
    let v = video("youtube_bbb.json"); // 635 s
    let c = choice(&v, "v720p60");
    // Past the end: up to the end.
    assert!(ytdlp_args(&v, c, &part(600.0, 900.0)).contains("--download-sections *600-635"));
    // The whole video (to the second) is a normal download.
    let whole = part(0.0, 634.6);
    assert!(prism_video::plan::part(&v, &whole).is_none());
    assert!(matches!(
        plan(&v, c, &whole, &RequestContext::default(), None, true, 8),
        Plan::Accelerated(_)
    ));
    let a = ytdlp_args(&v, c, &whole);
    assert!(!a.contains("--download-sections") && a.contains("--embed-chapters"));
    // Nothing left, or less than a second.
    assert!(prism_video::plan::part(&v, &part(700.0, 800.0)).is_none());
    assert!(prism_video::plan::part(&v, &part(10.0, 10.5)).is_none());
    // A live stream has no fixed length to cut.
    let mut live = v.clone();
    live.is_live = true;
    assert!(prism_video::plan::part(&live, &part(10.0, 20.0)).is_none());
    assert!(!ytdlp_args(&live, c, &part(10.0, 20.0)).contains("--download-sections"));
}
