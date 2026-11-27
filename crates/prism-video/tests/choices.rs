mod common;

use common::{fixture, video};
use prism_video::choices::{AudioCodec, VideoCodec, compute_choices, human_size};
use prism_video::model::parse_probe_json;
use prism_video::{AudioFormat, ChoiceKind, Container, FormatKind, Probe};

#[test]
fn parses_real_youtube_fixture() {
    let v = video("youtube_bbb.json");
    assert_eq!(v.id, "aqz-KE-bpKQ");
    assert!(v.title.starts_with("Big Buck Bunny"));
    assert_eq!(v.extractor, "youtube");
    assert_eq!(v.duration, Some(635.0));
    assert_eq!(v.upload_date.as_deref(), Some("20141110"));
    assert_eq!(
        v.thumbnail.as_deref(),
        Some("https://i.ytimg.com/vi_webp/aqz-KE-bpKQ/maxresdefault.webp")
    );
    assert!(!v.is_live);
    assert!(!v.has_drm);
    // Storyboard is classified as "other" and unusable.
    let sb = v.formats.iter().find(|f| f.format_id == "sb0").unwrap();
    assert_eq!(sb.kind, FormatKind::Other);
    assert!(!sb.usable);
    let f140 = v.formats.iter().find(|f| f.format_id == "140").unwrap();
    assert_eq!(f140.kind, FormatKind::Audio);
    assert_eq!(f140.http_chunk_size, Some(10_485_760));
    assert!(f140.is_progressive_http());
    let hls = v.formats.iter().find(|f| f.format_id == "230").unwrap();
    assert!(hls.fragmented);
    assert!(!hls.is_progressive_http());
    let f18 = v.formats.iter().find(|f| f.format_id == "18").unwrap();
    assert_eq!(f18.kind, FormatKind::Muxed);
    // 773 has null codecs but "audio only" resolution -> audio.
    let f773 = v.formats.iter().find(|f| f.format_id == "773").unwrap();
    assert_eq!(f773.kind, FormatKind::Audio);
}

#[test]
fn youtube_mp4_choices_prefer_h264_and_aac() {
    let v = video("youtube_bbb.json");
    let video: Vec<_> = v
        .choices
        .iter()
        .filter(|c| c.kind == ChoiceKind::Video)
        .collect();
    let labels: Vec<&str> = video.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "2160p60 · AV1",
            "1080p60 · H.264",
            "720p60 · H.264",
            "360p · H.264",
            "144p · H.264"
        ]
    );
    let c1080 = video.iter().find(|c| c.height == Some(1080)).unwrap();
    assert_eq!(c1080.video_format_id.as_deref(), Some("299"));
    // Best AAC: 258 (6ch 388 kbps), not DRC, not the unknown-codec 773.
    assert_eq!(c1080.audio_format_id.as_deref(), Some("258"));
    assert_eq!(c1080.container, Container::Mp4);
    assert!(c1080.accelerable);
    assert_eq!(c1080.est_size, Some(257_619_653 + 30_767_611));
    assert_eq!(c1080.sub_label, "MP4 · ~288 MB");
    // 360p: separate h264 video-only (134) beats the pre-muxed format 18.
    let c360 = video.iter().find(|c| c.height == Some(360)).unwrap();
    assert_eq!(c360.video_format_id.as_deref(), Some("134"));
    assert!(c360.format_id.is_none());
    // 720p: 60 fps (298) beats 30 fps (136).
    let c720 = video.iter().find(|c| c.height == Some(720)).unwrap();
    assert_eq!(c720.video_format_id.as_deref(), Some("298"));
    assert_eq!(c720.id, "v720p60");
}

#[test]
fn a_preferred_codec_wins_at_the_same_quality() {
    use prism_video::choices::compute_choices_preferring;
    let v = video("youtube_bbb.json");
    let at = |container, prefer, height| {
        compute_choices_preferring(&v, container, prefer)
            .into_iter()
            .find(|c| c.kind == ChoiceKind::Video && c.height == Some(height))
            .unwrap()
    };
    // #152: 1080p60 comes as H.264 (299), VP9 (303) and AV1 (399).
    assert_eq!(at(Container::Mp4, None, 1080).label, "1080p60 · H.264");
    let av1 = at(Container::Mp4, Some(VideoCodec::Av1), 1080);
    assert_eq!(av1.video_format_id.as_deref(), Some("399"));
    assert_eq!(av1.label, "1080p60 · AV1");
    assert_eq!(
        at(Container::Mp4, Some(VideoCodec::Vp9), 720).label,
        "720p60 · VP9"
    );
    // The quality stays: 2160p has no H.264, so it keeps its usual codec.
    assert_eq!(
        at(Container::Mp4, Some(VideoCodec::H264), 2160).label,
        "2160p60 · AV1"
    );
    // A codec the container can't hold is no preference: WebM has no H.264.
    assert_eq!(
        at(Container::Webm, Some(VideoCodec::H264), 1080).label,
        "1080p60 · VP9"
    );
    // The setting's names, and anything else meaning no preference.
    assert_eq!(VideoCodec::preferred(Some("AV1")), Some(VideoCodec::Av1));
    assert_eq!(VideoCodec::preferred(Some("h264")), Some(VideoCodec::H264));
    assert_eq!(VideoCodec::preferred(Some("vp9")), Some(VideoCodec::Vp9));
    assert_eq!(VideoCodec::preferred(Some("")), None);
    assert_eq!(VideoCodec::preferred(None), None);
}

#[test]
fn youtube_mkv_and_webm_prefer_vp9_and_opus() {
    let v = video("youtube_bbb.json");
    for container in [Container::Mkv, Container::Webm] {
        let ch = compute_choices(&v, container);
        let c1080 = ch.iter().find(|c| c.height == Some(1080)).unwrap();
        assert_eq!(
            c1080.video_format_id.as_deref(),
            Some("303"),
            "{container:?}"
        );
        assert_eq!(
            c1080.audio_format_id.as_deref(),
            Some("251"),
            "{container:?}"
        );
        assert_eq!(c1080.container, container);
        assert_eq!(c1080.label, "1080p60 · VP9");
    }
}

#[test]
fn audio_only_choices() {
    let v = video("youtube_bbb.json");
    let audio: Vec<_> = v
        .choices
        .iter()
        .filter(|c| c.kind == ChoiceKind::Audio)
        .collect();
    let labels: Vec<&str> = audio.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Audio · MP3 320 kbps",
            "Audio · M4A (original AAC)",
            "Audio · Opus (original)",
            "Audio · FLAC (lossless container)"
        ]
    );
    let mp3 = &audio[0];
    assert_eq!(mp3.audio_format, Some(AudioFormat::Mp3));
    assert_eq!(mp3.audio_bitrate_kbps, Some(320));
    assert_eq!(mp3.est_size, Some((320_000.0 / 8.0 * 635.0) as u64));
    let m4a = &audio[1];
    assert_eq!(m4a.audio_format_id.as_deref(), Some("258"));
    let opus = &audio[2];
    assert_eq!(opus.audio_format_id.as_deref(), Some("251"));
    assert!(audio.iter().all(|c| c.accelerable));
}

#[test]
fn hdr_8k_drm_and_container_fallback() {
    let v = video("synthetic_hdr.json");
    let labels: Vec<&str> = v
        .choices
        .iter()
        .filter(|c| c.kind == ChoiceKind::Video)
        .map(|c| c.label.as_str())
        .collect();
    assert_eq!(
        labels,
        [
            "4320p60 · AV1",
            "2160p60 · HDR · AV1",
            "2160p60 · AV1",
            "1440p · H.264",
            "1080p · H.264",
            "720p · H.264",
            "360p · H.264"
        ]
    );
    let hdr = v.choices.iter().find(|c| c.hdr).unwrap();
    assert_eq!(hdr.video_format_id.as_deref(), Some("701"));
    assert_eq!(hdr.dynamic_range.as_deref(), Some("HDR10"));
    assert_eq!(hdr.id, "v2160p60-hdr");
    // 1080p: the DRM 60fps format is excluded, h264 137 wins in mp4.
    let c1080 = v.choices.iter().find(|c| c.height == Some(1080)).unwrap();
    assert_eq!(c1080.video_format_id.as_deref(), Some("137"));
    // Non-DRC AAC preferred.
    assert_eq!(c1080.audio_format_id.as_deref(), Some("140"));
    // 720p only exists as HLS: muxed, not accelerable.
    let c720 = v.choices.iter().find(|c| c.height == Some(720)).unwrap();
    assert_eq!(c720.format_id.as_deref(), Some("hls-720"));
    assert!(!c720.accelerable);
    // "maybe" DRM is not treated as DRM.
    assert!(
        v.formats
            .iter()
            .find(|f| f.format_id == "maybe-drm")
            .unwrap()
            .usable
    );
    assert!(
        !v.formats
            .iter()
            .find(|f| f.format_id == "drm-1080")
            .unwrap()
            .usable
    );

    // MKV prefers VP9 for HDR.
    let mkv = compute_choices(&v, Container::Mkv);
    let hdr_mkv = mkv.iter().find(|c| c.hdr).unwrap();
    assert_eq!(hdr_mkv.video_format_id.as_deref(), Some("337"));
    assert_eq!(hdr_mkv.label, "2160p60 · HDR · VP9");
    assert_eq!(hdr_mkv.audio_format_id.as_deref(), Some("251"));

    // WebM cannot hold H.264: those entries fall back to MKV.
    let webm = compute_choices(&v, Container::Webm);
    let c360 = webm.iter().find(|c| c.height == Some(360)).unwrap();
    assert_eq!(c360.container, Container::Mkv);
    let c1080w = webm.iter().find(|c| c.height == Some(1080)).unwrap();
    assert_eq!(c1080w.video_format_id.as_deref(), Some("248"));
    assert_eq!(c1080w.container, Container::Webm);
}

#[test]
fn all_drm_yields_no_choices() {
    let mut json: serde_json::Value = serde_json::from_str(&fixture("synthetic_hdr.json")).unwrap();
    for f in json["formats"].as_array_mut().unwrap() {
        f["has_drm"] = serde_json::Value::Bool(true);
    }
    let Probe::Video(v) = parse_probe_json(&json.to_string()).unwrap() else {
        panic!()
    };
    assert!(v.has_drm);
    assert!(v.choices.is_empty());
}

#[test]
fn subtitles_and_chapters_parsed() {
    let v = video("synthetic_hdr.json");
    let manual: Vec<_> = v
        .subtitles
        .iter()
        .filter(|s| !s.auto)
        .map(|s| s.lang.as_str())
        .collect();
    assert_eq!(manual, ["cs", "en"]); // live_chat dropped
    let auto: Vec<_> = v
        .subtitles
        .iter()
        .filter(|s| s.auto)
        .map(|s| s.lang.as_str())
        .collect();
    assert_eq!(auto, ["de", "de-orig", "en"]);
    assert_eq!(v.chapters.len(), 3);
    assert_eq!(v.chapters[1].title, "Main = part");
    // `thumbnail` absent -> highest preference thumbnail.
    assert_eq!(
        v.thumbnail.as_deref(),
        Some("https://i.ytimg.com/vi_webp/HDRtest0001/maxresdefault.webp")
    );
}

#[test]
fn generic_direct_file() {
    let v = video("generic_direct.json");
    assert_eq!(v.formats.len(), 1);
    assert_eq!(v.formats[0].kind, FormatKind::Muxed);
    let video: Vec<_> = v
        .choices
        .iter()
        .filter(|c| c.kind == ChoiceKind::Video)
        .collect();
    assert_eq!(video.len(), 1);
    assert_eq!(video[0].label, "Original quality");
    assert_eq!(video[0].format_id.as_deref(), Some("mp4"));
    assert_eq!(video[0].container, Container::Mp4);
    assert!(video[0].accelerable);
    // Audio choices extract from the muxed file.
    let mp3 = v.choices.iter().find(|c| c.id == "a-mp3").unwrap();
    assert_eq!(mp3.format_id.as_deref(), Some("mp4"));
}

#[test]
fn flat_playlist_fixture() {
    let Probe::Playlist(p) = parse_probe_json(&fixture("youtube_flat_playlist.json")).unwrap()
    else {
        panic!("expected playlist")
    };
    assert_eq!(p.title.as_deref(), Some("Blender - Videos"));
    assert_eq!(p.entries.len(), 3);
    assert_eq!(p.entries[0].id.as_deref(), Some("sO_lhW8av3U"));
    assert_eq!(
        p.entries[0].url,
        "https://www.youtube.com/watch?v=sO_lhW8av3U"
    );
    assert_eq!(p.entries[0].duration, Some(149.0));
    assert!(p.entries[0].thumbnail.as_deref().unwrap().contains("hq720"));
}

#[test]
fn tolerant_parsing() {
    let json = r#"{"id": 123, "title": null, "duration": "12.5", "formats": [
        {"format_id": "a", "url": "https://x/a.mp4", "height": "720", "fps": null, "filesize": 1.5e6, "has_drm": "maybe"},
        null, 5, {"weird": true}
    ], "entries": null, "chapters": "nope", "subtitles": []}"#;
    let Probe::Video(v) = parse_probe_json(json).unwrap() else {
        panic!()
    };
    assert_eq!(v.id, "123");
    assert_eq!(v.title, "123");
    assert_eq!(v.duration, Some(12.5));
    assert_eq!(v.formats[0].height, Some(720));
    assert_eq!(v.formats[0].filesize, Some(1_500_000));
    assert!(v.chapters.is_empty());
}

#[test]
fn codec_families_and_sizes() {
    assert_eq!(
        VideoCodec::from_codec(Some("avc1.64002A")),
        VideoCodec::H264
    );
    assert_eq!(
        VideoCodec::from_codec(Some("vp09.02.51.10")),
        VideoCodec::Vp9
    );
    assert_eq!(
        VideoCodec::from_codec(Some("av01.0.13M.10")),
        VideoCodec::Av1
    );
    assert_eq!(
        VideoCodec::from_codec(Some("hvc1.2.4.L153")),
        VideoCodec::Hevc
    );
    assert_eq!(AudioCodec::from_codec(Some("mp4a.40.5")), AudioCodec::Aac);
    assert_eq!(AudioCodec::from_codec(Some("ec-3")), AudioCodec::Eac3);
    assert_eq!(AudioCodec::from_codec(None), AudioCodec::Unknown);
    assert_eq!(human_size(999), "999 B");
    assert_eq!(human_size(1_500_000), "1.5 MB");
    assert_eq!(human_size(712_445_280), "712 MB");
}

#[test]
fn serde_is_camel_case() {
    let v = video("synthetic_hdr.json");
    let s = serde_json::to_string(&Probe::Video(Box::new(v))).unwrap();
    assert!(s.contains(r#""kind":"video""#));
    assert!(s.contains(r#""webpageUrl""#));
    assert!(s.contains(r#""videoFormatId""#));
    assert!(s.contains(r#""estSize""#));
    assert!(s.contains(r#""httpChunkSize""#));
    let back: Probe = serde_json::from_str(&s).unwrap();
    assert!(matches!(back, Probe::Video(_)));
}
