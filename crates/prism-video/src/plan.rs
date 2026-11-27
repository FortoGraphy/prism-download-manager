//! Download planning: accelerated (engine downloads plain HTTP streams, then
//! ffmpeg post-processes) vs. the yt-dlp fallback.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ffmpeg::{AudioConversion, Cut, Metadata, PostInput, PostProcess, SponsorBlockRequest};
use crate::model::{
    AudioFormat, Choice, ChoiceKind, Container, Format, OutputOptions, RequestContext,
    SponsorBlockMode, SubtitleTrack, VideoInfo,
};
use crate::progress::{DL_TEMPLATE, FILE_PRINT, PP_TEMPLATE};
use crate::section::Section;
use crate::template::{render_template, temp_stem};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StreamRole {
    #[default]
    Video,
    Audio,
    /// One file carrying both video and audio.
    Muxed,
    Subtitle,
    Thumbnail,
}

/// One file the core should download with its segmented engine.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamSpec {
    pub role: StreamRole,
    pub url: String,
    /// Exactly these headers (yt-dlp's `http_headers` plus cookies/referer).
    pub headers: Vec<(String, String)>,
    /// Expected size (may be approximate when `size_exact` is false).
    pub size: Option<u64>,
    pub size_exact: bool,
    pub ext: String,
    /// Suggested temp file name (unique within the plan).
    pub file_name: String,
    pub format_id: Option<String>,
    /// Maximum bytes per range request (YouTube: 10 MiB; larger ranges get throttled / 403).
    pub chunk_size: Option<u64>,
    /// Subtitle language.
    pub language: Option<String>,
    /// Missing optional streams (subtitles, thumbnail) must not fail the download.
    pub optional: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceleratedPlan {
    pub streams: Vec<StreamSpec>,
    /// `post.inputs[i]` corresponds to `streams[i]`; call
    /// [`PostProcess::set_input_dir`] (or fill `path`s) after downloading.
    pub post: PostProcess,
}

/// A yt-dlp invocation. `args` are the job-specific options; the service adds
/// tool locations, proxy, cookies and request headers at run time
/// (see [`crate::VideoService::ytdlp_command_line`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YtDlpJob {
    pub url: String,
    pub args: Vec<String>,
    pub output_dir: PathBuf,
    pub context: RequestContext,
    /// Why the accelerated path was not used (for logs / UI).
    pub reason: String,
    /// The part of the video being cut (its progress is ffmpeg's, see [`crate::progress::PartProgress`]).
    #[serde(default)]
    pub section: Option<Section>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum Plan {
    Accelerated(AcceleratedPlan),
    YtDlp(YtDlpJob),
}

fn find<'a>(info: &'a VideoInfo, id: &Option<String>) -> Option<Option<&'a Format>> {
    match id {
        None => Some(None),
        Some(id) => info.formats.iter().find(|f| &f.format_id == id).map(Some),
    }
}

fn is_live(info: &VideoInfo) -> bool {
    info.is_live
        || matches!(
            info.live_status.as_deref(),
            Some("is_live" | "is_upcoming" | "post_live")
        )
}

/// The part of the video to download: the options' section, cut to the video's length.
/// `None` for the whole video, and for live streams (which have no fixed length).
pub fn part(info: &VideoInfo, opts: &OutputOptions) -> Option<Section> {
    if is_live(info) {
        return None;
    }
    opts.section?.part_of(info.duration)
}

/// Why the accelerated path cannot be used, or `None` if it can.
pub fn accelerated_blocker(
    info: &VideoInfo,
    choice: &Choice,
    opts: &OutputOptions,
) -> Option<String> {
    if is_live(info) {
        return Some("live stream".into());
    }
    if info.has_drm {
        return Some("DRM protected".into());
    }
    // A part: the core reads the index at the start of each stream and downloads only the
    // pieces around the part (see `crate::index`); a stream without one goes back to yt-dlp.
    if opts.sponsorblock.mode == SponsorBlockMode::Remove {
        return Some("SponsorBlock removal requires yt-dlp".into());
    }
    let ids = [
        &choice.video_format_id,
        &choice.audio_format_id,
        &choice.format_id,
    ];
    if ids.iter().all(|i| i.is_none()) {
        return Some("choice references no format".into());
    }
    for id in ids {
        match find(info, id) {
            None => return Some(format!("format {} not found", id.as_deref().unwrap_or("?"))),
            Some(Some(f)) if !f.is_progressive_http() => {
                return Some(format!(
                    "format {} uses {} (not a single HTTP file)",
                    f.format_id, f.protocol
                ));
            }
            _ => {}
        }
    }
    None
}

/// Parse yt-dlp's per-format `cookies` string (`a=b; Domain=..; Path=/; c=d; ...`) into a Cookie header.
pub fn cookie_header_from_ytdlp(s: &str) -> Option<String> {
    const ATTRS: [&str; 8] = [
        "domain", "path", "expires", "max-age", "secure", "httponly", "samesite", "version",
    ];
    let pairs: Vec<String> = s
        .split(';')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .filter_map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            let kl = k.trim().to_ascii_lowercase();
            if ATTRS.contains(&kl.as_str()) || !p.contains('=') {
                return None;
            }
            Some(format!("{}={}", k.trim(), v.trim()))
        })
        .collect();
    (!pairs.is_empty()).then(|| pairs.join("; "))
}

fn host(u: &str) -> Option<String> {
    url::Url::parse(u)
        .ok()?
        .host_str()
        .map(|h| h.to_ascii_lowercase())
}

/// Same "site" (last two labels) — decides whether page cookies may be sent to a stream URL.
pub(crate) fn same_site(a: &str, b: &str) -> bool {
    let tail = |h: &str| h.rsplit('.').take(2).collect::<Vec<_>>().join(".");
    match (host(a), host(b)) {
        (Some(x), Some(y)) => tail(&x) == tail(&y),
        _ => false,
    }
}

fn has_header(h: &[(String, String)], name: &str) -> bool {
    h.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
}

pub(crate) fn stream_headers(
    format_headers: &std::collections::BTreeMap<String, String>,
    format_cookies: Option<&str>,
    url: &str,
    info: &VideoInfo,
    ctx: &RequestContext,
    default_ua: Option<&str>,
) -> Vec<(String, String)> {
    let mut h: Vec<(String, String)> = format_headers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if !has_header(&h, "Cookie") {
        let cookie = format_cookies
            .and_then(cookie_header_from_ytdlp)
            .or_else(|| {
                ctx.cookies
                    .clone()
                    .filter(|_| same_site(url, &info.webpage_url))
            });
        if let Some(c) = cookie {
            h.push(("Cookie".into(), c));
        }
    }
    if !has_header(&h, "Referer")
        && let Some(r) = &ctx.referer
    {
        h.push(("Referer".into(), r.clone()));
    }
    if !has_header(&h, "User-Agent")
        && let Some(ua) = ctx.user_agent.as_deref().or(default_ua)
    {
        h.push(("User-Agent".into(), ua.to_string()));
    }
    for (k, v) in &ctx.headers {
        if !has_header(&h, k) {
            h.push((k.clone(), v.clone()));
        }
    }
    h
}

/// `en` matches `en`, `en-US`, `en-GB`; `en.*` is a prefix pattern; `all` matches every manual track.
pub fn lang_matches(pattern: &str, lang: &str) -> bool {
    let p = pattern.trim();
    if p.is_empty() || p.starts_with('-') {
        return false;
    }
    if p == "all" {
        return true;
    }
    if let Some(prefix) = p.strip_suffix(".*") {
        return lang.starts_with(prefix);
    }
    lang.eq_ignore_ascii_case(p)
        || lang
            .to_ascii_lowercase()
            .starts_with(&format!("{}-", p.to_ascii_lowercase()))
}

/// Pick subtitle tracks per pattern: manual tracks first, automatic captions only
/// when `auto` is enabled and no manual track matched that pattern.
pub fn select_subtitles<'a>(
    tracks: &'a [SubtitleTrack],
    patterns: &[String],
    auto: bool,
) -> Vec<&'a SubtitleTrack> {
    let mut out: Vec<&SubtitleTrack> = Vec::new();
    for p in patterns {
        let manual: Vec<&SubtitleTrack> = tracks
            .iter()
            .filter(|t| !t.auto && lang_matches(p, &t.lang))
            .collect();
        let picked = if !manual.is_empty() {
            manual
        } else if auto && p != "all" {
            // Auto captions: only the base language itself (YouTube offers ~100 machine
            // translations); fall back to the untranslated `<lang>-orig` track.
            let base = p.trim_end_matches(".*").to_ascii_lowercase();
            let orig = format!("{base}-orig");
            tracks
                .iter()
                .filter(|t| t.auto && t.lang.eq_ignore_ascii_case(&base))
                .chain(
                    tracks
                        .iter()
                        .filter(|t| t.auto && t.lang.eq_ignore_ascii_case(&orig)),
                )
                .take(1)
                .collect()
        } else {
            Vec::new()
        };
        for t in picked {
            if !out.iter().any(|o| o.lang == t.lang && o.auto == t.auto) {
                out.push(t);
            }
        }
    }
    out
}

fn thumb_ext(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "webp" => ext,
        _ => "jpg".into(),
    }
}

fn metadata_for(info: &VideoInfo) -> Metadata {
    let description = info.description.as_ref().map(|d| {
        let d = d.trim();
        if d.chars().count() > 2000 {
            format!("{}…", d.chars().take(2000).collect::<String>())
        } else {
            d.to_string()
        }
    });
    Metadata {
        title: Some(info.title.clone()),
        artist: info.uploader.clone().or_else(|| info.channel.clone()),
        date: info.upload_date.clone(),
        comment: (!info.webpage_url.is_empty()).then(|| info.webpage_url.clone()),
        description: description.filter(|d| !d.is_empty()),
    }
}

fn audio_format_of(choice: &Choice, opts: &OutputOptions) -> AudioFormat {
    choice.audio_format.unwrap_or(opts.audio_format)
}

/// Output extension for a choice.
pub fn output_ext(choice: &Choice, opts: &OutputOptions) -> &'static str {
    match choice.kind {
        ChoiceKind::Audio => audio_format_of(choice, opts).ext(),
        ChoiceKind::Video => choice.container.ext(),
    }
}

pub fn output_path(info: &VideoInfo, choice: &Choice, opts: &OutputOptions) -> PathBuf {
    opts.output_dir.join(render_template(
        &opts.file_template,
        info,
        output_ext(choice, opts),
    ))
}

/// Build the plan. `default_ua` is the service's configured User-Agent.
pub fn plan(
    info: &VideoInfo,
    choice: &Choice,
    opts: &OutputOptions,
    ctx: &RequestContext,
    default_ua: Option<&str>,
    delete_inputs: bool,
    concurrent_fragments: u8,
) -> Plan {
    match accelerated_blocker(info, choice, opts) {
        None => Plan::Accelerated(plan_accelerated(
            info,
            choice,
            opts,
            ctx,
            default_ua,
            delete_inputs,
        )),
        Some(reason) => Plan::YtDlp(plan_ytdlp(
            info,
            choice,
            opts,
            ctx,
            concurrent_fragments,
            reason,
        )),
    }
}

fn plan_accelerated(
    info: &VideoInfo,
    choice: &Choice,
    opts: &OutputOptions,
    ctx: &RequestContext,
    default_ua: Option<&str>,
    delete_inputs: bool,
) -> AcceleratedPlan {
    let stem = temp_stem(info);
    let mut streams = Vec::new();
    let mut inputs = Vec::new();
    let audio_only = choice.kind == ChoiceKind::Audio;
    let section = part(info, opts);
    // Subtitles, chapters and SponsorBlock segments are timed to the whole video: a part gets
    // none (the same as on the yt-dlp path).
    let whole = section.is_none();

    let push_format = |f: &Format,
                       role: StreamRole,
                       streams: &mut Vec<StreamSpec>,
                       inputs: &mut Vec<PostInput>| {
        let url = f.url.clone().unwrap_or_default();
        let file_name = format!(
            "{stem}.f{}.{}",
            f.format_id.replace(['/', '\\', ':'], "_"),
            if f.ext.is_empty() { "bin" } else { &f.ext }
        );
        streams.push(StreamSpec {
            role,
            headers: stream_headers(
                &f.http_headers,
                f.cookies.as_deref(),
                &url,
                info,
                ctx,
                default_ua,
            ),
            url,
            size: f.filesize.or(f.filesize_approx),
            size_exact: f.filesize.is_some(),
            ext: f.ext.clone(),
            file_name: file_name.clone(),
            format_id: Some(f.format_id.clone()),
            chunk_size: f.http_chunk_size,
            language: f.language.clone(),
            optional: false,
        });
        inputs.push(PostInput {
            role,
            file_name,
            path: None,
            language: f.language.clone(),
            name: None,
            codec: f.acodec.clone().or_else(|| f.vcodec.clone()),
            ext: f.ext.clone(),
        });
    };

    let get = |id: &Option<String>| {
        id.as_ref()
            .and_then(|id| info.formats.iter().find(|f| &f.format_id == id))
    };
    let source_codec;
    if audio_only {
        let src = get(&choice.audio_format_id).or_else(|| get(&choice.format_id));
        source_codec = src.and_then(|f| f.acodec.clone());
        if let Some(f) = src {
            let role = if f.kind == crate::model::FormatKind::Audio {
                StreamRole::Audio
            } else {
                StreamRole::Muxed
            };
            push_format(f, role, &mut streams, &mut inputs);
        }
    } else {
        source_codec = None;
        if let Some(f) = get(&choice.format_id) {
            push_format(f, StreamRole::Muxed, &mut streams, &mut inputs);
        }
        if let Some(f) = get(&choice.video_format_id) {
            push_format(f, StreamRole::Video, &mut streams, &mut inputs);
        }
        if let Some(f) = get(&choice.audio_format_id) {
            push_format(f, StreamRole::Audio, &mut streams, &mut inputs);
        }
    }

    // Subtitles (video outputs only).
    if whole && !audio_only && (opts.embed_subtitles || opts.write_subtitles) {
        for t in select_subtitles(&info.subtitles, &opts.sub_langs, opts.auto_subtitles) {
            let pick = ["vtt", "srt", "ass"].iter().find_map(|e| {
                t.formats.iter().find(|f| {
                    f.ext == *e
                        && (f.url.starts_with("http://") || f.url.starts_with("https://"))
                        && !f.protocol.as_deref().is_some_and(|p| p.contains("m3u8"))
                        && !f.url.contains(".m3u8")
                })
            });
            let Some(sf) = pick else { continue };
            let file_name = format!(
                "{stem}.{}{}.{}",
                t.lang.replace(['/', '\\', ':'], "_"),
                if t.auto { ".auto" } else { "" },
                sf.ext
            );
            streams.push(StreamSpec {
                role: StreamRole::Subtitle,
                url: sf.url.clone(),
                headers: stream_headers(&Default::default(), None, &sf.url, info, ctx, default_ua),
                size: None,
                size_exact: false,
                ext: sf.ext.clone(),
                file_name: file_name.clone(),
                format_id: None,
                chunk_size: None,
                language: Some(t.lang.clone()),
                optional: true,
            });
            inputs.push(PostInput {
                role: StreamRole::Subtitle,
                file_name,
                path: None,
                language: Some(t.lang.clone()),
                name: t.name.clone(),
                codec: None,
                ext: sf.ext.clone(),
            });
        }
    }

    // Thumbnail.
    if (opts.embed_thumbnail || opts.write_thumbnail)
        && let Some(url) = &info.thumbnail
    {
        let ext = thumb_ext(url);
        let file_name = format!("{stem}.thumb.{ext}");
        streams.push(StreamSpec {
            role: StreamRole::Thumbnail,
            url: url.clone(),
            headers: stream_headers(&Default::default(), None, url, info, ctx, default_ua),
            size: None,
            size_exact: false,
            ext: ext.clone(),
            file_name: file_name.clone(),
            format_id: None,
            chunk_size: None,
            language: None,
            optional: true,
        });
        inputs.push(PostInput {
            role: StreamRole::Thumbnail,
            file_name,
            path: None,
            language: None,
            name: None,
            codec: None,
            ext,
        });
    }

    let audio_conversion = audio_only.then(|| {
        let fmt = audio_format_of(choice, opts);
        AudioConversion {
            format: fmt,
            bitrate_kbps: match fmt {
                AudioFormat::Mp3 => {
                    Some(choice.audio_bitrate_kbps.unwrap_or(opts.mp3_bitrate_kbps))
                }
                _ => choice.audio_bitrate_kbps,
            },
            source_codec,
        }
    });

    let sponsorblock = (whole
        && opts.sponsorblock.mode == SponsorBlockMode::Mark
        && info.extractor.to_ascii_lowercase().starts_with("youtube")
        && !opts.sponsorblock.categories.is_empty())
    .then(|| SponsorBlockRequest {
        video_id: info.id.clone(),
        categories: opts.sponsorblock.categories.clone(),
    });

    let post = PostProcess {
        inputs,
        output_path: output_path(info, choice, opts),
        container: if audio_only {
            Container::Mp4
        } else {
            choice.container
        },
        audio_conversion,
        metadata: opts.embed_metadata.then(|| metadata_for(info)),
        chapters: if whole && opts.embed_chapters {
            info.chapters.clone()
        } else {
            Vec::new()
        },
        embed_subtitles: whole && opts.embed_subtitles && !audio_only,
        embed_thumbnail: opts.embed_thumbnail,
        embed_chapters: whole && opts.embed_chapters,
        write_subtitles: whole && opts.write_subtitles && !audio_only,
        write_thumbnail: opts.write_thumbnail,
        sponsorblock,
        duration: section.map(|s| s.length()).or(info.duration),
        delete_inputs,
        cut: section.map(|s| Cut {
            start: s.start,
            length: s.length(),
            main_offset: 0.0,
            audio_offset: 0.0,
        }),
    };
    AcceleratedPlan { streams, post }
}

/// `--downloader-args` of a part: ffmpeg writes `-progress` blocks to stdout instead of its
/// stats line.
pub const PART_PROGRESS_ARGS: &str = "ffmpeg:-progress pipe:1 -nostats";

/// yt-dlp `-f` selector for a choice, with graceful fallbacks if format ids changed.
pub fn format_selector(choice: &Choice) -> String {
    let h = choice
        .height
        .map(|h| format!("[height<={h}]"))
        .unwrap_or_default();
    match choice.kind {
        ChoiceKind::Audio => match (&choice.audio_format_id, &choice.format_id) {
            (Some(a), _) => format!("{a}/ba/b"),
            (None, Some(m)) => format!("{m}/ba/b"),
            _ => "ba/b".into(),
        },
        ChoiceKind::Video => match (
            &choice.video_format_id,
            &choice.audio_format_id,
            &choice.format_id,
        ) {
            (Some(v), Some(a), _) => format!("{v}+{a}/bv*{h}+ba/b{h}/b"),
            (Some(v), None, _) => format!("{v}/bv*{h}+ba/b{h}/b"),
            (None, _, Some(m)) => format!("{m}/bv*{h}+ba/b{h}/b"),
            _ => format!("bv*{h}+ba/b{h}/b"),
        },
    }
}

/// Force the yt-dlp path (e.g. after the accelerated download got a 403).
pub fn plan_ytdlp(
    info: &VideoInfo,
    choice: &Choice,
    opts: &OutputOptions,
    ctx: &RequestContext,
    concurrent_fragments: u8,
    reason: String,
) -> YtDlpJob {
    let mut a: Vec<String> = Vec::new();
    let push = |a: &mut Vec<String>, xs: &[&str]| a.extend(xs.iter().map(|x| x.to_string()));
    let audio_only = choice.kind == ChoiceKind::Audio;

    push(&mut a, &["-f", &format_selector(choice)]);
    if audio_only {
        let fmt = audio_format_of(choice, opts);
        push(&mut a, &["-x", "--audio-format", fmt.ext()]);
        match fmt {
            AudioFormat::Mp3 => {
                let kbps = choice.audio_bitrate_kbps.unwrap_or(opts.mp3_bitrate_kbps);
                push(&mut a, &["--audio-quality", &format!("{kbps}K")]);
            }
            AudioFormat::M4a | AudioFormat::Opus if choice.audio_bitrate_kbps.is_some() => {
                push(
                    &mut a,
                    &[
                        "--audio-quality",
                        &format!("{}K", choice.audio_bitrate_kbps.unwrap_or(0)),
                    ],
                );
            }
            _ => push(&mut a, &["--audio-quality", "0"]),
        }
    } else {
        let c = choice.container.ext();
        push(&mut a, &["--merge-output-format", c, "--remux-video", c]);
    }
    push(&mut a, &["-N", &concurrent_fragments.max(1).to_string()]);
    let section = part(info, opts);
    if let Some(s) = section {
        push(&mut a, &["--download-sections", &s.ytdlp_arg()]);
        // The part is copied, not encoded again (#135): re-encoding it to cut on the exact frame
        // kept every core busy for the whole download. A video part now starts on the keyframe
        // at or before its start (Twitch: up to 2 s early); the end and the audio stay exact.
        // (The accelerated path still cuts on the exact frame: see ffmpeg.rs.)
        // ffmpeg fetches the part and yt-dlp reports nothing until it's done: ffmpeg's own
        // progress (stdout) is what the list shows.
        push(&mut a, &["--downloader-args", PART_PROGRESS_ARGS]);
    }
    // Subtitles, chapters and SponsorBlock segments are timed to the whole video, and yt-dlp
    // doesn't shift them to a part (they'd run from the video's start): a part gets none.
    let whole = section.is_none();

    if whole
        && !audio_only
        && (opts.embed_subtitles || opts.write_subtitles)
        && !opts.sub_langs.is_empty()
    {
        push(&mut a, &["--sub-langs", &opts.sub_langs.join(",")]);
        if opts.write_subtitles {
            a.push("--write-subs".into());
        }
        if opts.auto_subtitles {
            a.push("--write-auto-subs".into());
        }
        if opts.embed_subtitles {
            a.push("--embed-subs".into());
        }
    }
    let thumb_embeddable = !audio_only && choice.container != Container::Webm
        || audio_only
            && matches!(
                audio_format_of(choice, opts),
                AudioFormat::Mp3 | AudioFormat::M4a | AudioFormat::Flac | AudioFormat::Opus
            );
    if opts.embed_thumbnail && thumb_embeddable {
        push(
            &mut a,
            &["--embed-thumbnail", "--convert-thumbnails", "jpg"],
        );
    }
    if opts.write_thumbnail {
        a.push("--write-thumbnail".into());
    }
    if opts.embed_metadata {
        a.push("--embed-metadata".into());
    }
    if whole && opts.embed_chapters {
        a.push("--embed-chapters".into());
    }
    let cats = if opts.sponsorblock.categories.is_empty() {
        "default".to_string()
    } else {
        opts.sponsorblock.categories.join(",")
    };
    let sponsorblock = match whole {
        true => opts.sponsorblock.mode,
        false => SponsorBlockMode::Off,
    };
    match sponsorblock {
        SponsorBlockMode::Off => {}
        SponsorBlockMode::Mark => push(&mut a, &["--sponsorblock-mark", &cats]),
        SponsorBlockMode::Remove => push(&mut a, &["--sponsorblock-remove", &cats]),
    }
    push(
        &mut a,
        &[
            "--newline",
            "--progress",
            "--color",
            "never",
            "--progress-delta",
            "0.5",
            "--progress-template",
            DL_TEMPLATE,
            "--progress-template",
            PP_TEMPLATE,
            "--print",
            FILE_PRINT,
            "--no-mtime",
            "--windows-filenames",
            "--no-playlist",
        ],
    );
    push(&mut a, &["-P", &opts.output_dir.to_string_lossy()]);
    let tpl = if opts.file_template.trim().is_empty() {
        crate::model::DEFAULT_TEMPLATE
    } else {
        opts.file_template.as_str()
    };
    push(&mut a, &["-o", tpl]);

    YtDlpJob {
        url: if info.webpage_url.is_empty() {
            info.id.clone()
        } else {
            info.webpage_url.clone()
        },
        args: a,
        output_dir: opts.output_dir.clone(),
        context: ctx.clone(),
        reason,
        section,
    }
}
