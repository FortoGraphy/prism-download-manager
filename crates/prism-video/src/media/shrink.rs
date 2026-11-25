//! "Make smaller": fit a file under a size limit (Discord, e-mail, WhatsApp, or a custom one).
//!
//! * Video: the bitrate comes from the size and the length (5 % kept for the container and the
//!   encoder's aim), the sound takes 64–128 kbps of it, and the resolution and frame rate
//!   follow the bitrate (≥ 2.5 Mbps up to 1080p, ≥ 1.2 Mbps 720p, ≥ 0.6 Mbps 480p, else
//!   360p; 30 fps below 1.5 Mbps). H.264 in two passes, then the result is weighed: one more,
//!   smaller second pass if it came out over.
//! * Audio: MP3 (or AAC for an M4A) at the bitrate that fits, 32–320 kbps.
//! * Pictures: JPEG (WebP when the picture has transparency), first lowering the quality, then
//!   the size, until it fits.
//!
//! A limit that can't be met sensibly (two hours of video in 10 MB) is said before anything
//! runs, and nothing is made.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::convert::{FLATTEN_ON_WHITE, fit_long_side, fit_short_side};
use super::{
    MediaInfo, MediaKind, MediaProblem, MediaProgress, ProgressFn, base_args, discard,
    finish_output, path_arg, run_mapped, s, temp_output,
};
use crate::error::{Result, VideoError};

/// Share of the limit the streams aim at (the rest: container overhead, encoder overshoot).
pub const SAFETY: f64 = 0.95;
/// Below this the picture is mush at any resolution: the limit is "too small".
pub const MIN_VIDEO_KBPS: f64 = 150.0;
pub const MIN_AUDIO_KBPS: f64 = 32.0;
pub const MAX_AUDIO_KBPS: u32 = 320;
/// Pictures aren't made smaller than this on their long side.
pub const MIN_IMAGE_SIDE: u32 = 640;

const MP3_RATES: &[u32] = &[
    32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    /// It can be done.
    Ok,
    /// Under the limit already: nothing to do.
    AlreadySmall,
    /// Too much video or sound for the limit.
    TooLong,
    /// The file couldn't be read (no length, no size).
    Unknown,
}

/// How the result will look or sound, in words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Grade {
    Low,
    Ok,
    Good,
}

/// What "Make smaller" will do with one file, shown before it starts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShrinkPlan {
    pub verdict: Verdict,
    pub grade: Option<Grade>,
    /// Extension of the result (`mp4`, `mp3`, `jpg`, …).
    pub ext: String,
    /// About this big (never more than the limit).
    pub est_size: Option<u64>,
    /// The result's picture size (shown, so a portrait stays taller than wide).
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub video_kbps: Option<u32>,
    pub audio_kbps: Option<u32>,
}

impl ShrinkPlan {
    fn new(ext: &str, verdict: Verdict) -> ShrinkPlan {
        ShrinkPlan {
            verdict,
            grade: None,
            ext: ext.into(),
            est_size: None,
            width: None,
            height: None,
            fps: None,
            video_kbps: None,
            audio_kbps: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShrinkOutcome {
    pub path: PathBuf,
    pub size: u64,
}

/// The plan for fitting `info` under `target` bytes. Pure.
pub fn plan(info: &MediaInfo, target: u64) -> ShrinkPlan {
    match info.kind {
        MediaKind::Video => plan_video(info, target),
        MediaKind::Audio => plan_audio(info, target),
        MediaKind::Image => plan_image(info, target),
        MediaKind::Other => ShrinkPlan::new(&info.ext, Verdict::Unknown),
    }
}

/// The plan's verdict as an error, when there's nothing to run.
pub fn verdict_problem(plan: &ShrinkPlan) -> Option<MediaProblem> {
    match plan.verdict {
        Verdict::Ok => None,
        Verdict::AlreadySmall => Some(MediaProblem::AlreadySmall),
        Verdict::TooLong => Some(MediaProblem::TooLong),
        Verdict::Unknown => Some(MediaProblem::Unreadable),
    }
}

/// `(w, h)` scaled so the short side is `short` (even numbers, aspect kept).
fn scaled_to_short(w: u32, h: u32, short: u32) -> (u32, u32) {
    let even = |x: f64| ((x / 2.0).round() as u32 * 2).max(2);
    if w >= h {
        (even(w as f64 * short as f64 / h as f64), short)
    } else {
        (short, even(h as f64 * short as f64 / w as f64))
    }
}

/// `(w, h)` scaled so the long side is `long`.
fn scaled_to_long(w: u32, h: u32, long: u32) -> (u32, u32) {
    let r = |x: u32, of: u32| ((x as f64 * long as f64 / of as f64).round() as u32).max(1);
    if w >= h {
        (long, r(h, w))
    } else {
        (r(w, h), long)
    }
}

/// Resolution for a video bitrate (kbps): the short side.
pub fn rung_for(video_kbps: f64) -> u32 {
    if video_kbps >= 2500.0 {
        1080
    } else if video_kbps >= 1200.0 {
        720
    } else if video_kbps >= 600.0 {
        480
    } else {
        360
    }
}

/// Sound for a total budget (kbps): more when there's room.
pub fn audio_for(total_kbps: f64) -> u32 {
    if total_kbps >= 1500.0 {
        128
    } else if total_kbps >= 600.0 {
        96
    } else {
        64
    }
}

fn plan_video(info: &MediaInfo, target: u64) -> ShrinkPlan {
    let mut p = ShrinkPlan::new("mp4", Verdict::Ok);
    let Some(d) = info.duration.filter(|d| *d > 0.0) else {
        p.verdict = Verdict::Unknown;
        return p;
    };
    if info.size <= target {
        p.verdict = Verdict::AlreadySmall;
        p.est_size = Some(info.size);
        return p;
    }
    let total = target as f64 * 8.0 * SAFETY / d / 1000.0;
    let mut audio = if info.has_audio { audio_for(total) } else { 0 };
    if let Some(src) = info.audio_kbps.filter(|k| *k > 0 && *k < audio) {
        audio = src.max(32);
    }
    let mut video = total - audio as f64;
    if video < MIN_VIDEO_KBPS {
        p.verdict = Verdict::TooLong;
        p.video_kbps = Some(video.max(0.0) as u32);
        p.audio_kbps = info.has_audio.then_some(audio);
        return p;
    }
    // Never more than the original had: that only makes the file bigger, not better.
    if let Some(src) = info.video_kbps.filter(|k| *k > 0) {
        video = video.min(src as f64);
    }
    let fps_in = info.fps.unwrap_or(30.0);
    let fps = if video < 1500.0 && fps_in > 30.5 {
        30.0
    } else {
        fps_in.min(60.0)
    };
    if let (Some(w), Some(h), Some(short)) = (info.width, info.height, info.short_side()) {
        let to = short.min(rung_for(video));
        let (tw, th) = if to < short {
            scaled_to_short(w, h, to)
        } else {
            (w, h)
        };
        p.width = Some(tw);
        p.height = Some(th);
        let bpp = video * 1000.0 / (tw as f64 * th as f64 * fps.max(1.0));
        p.grade = Some(if bpp >= 0.06 {
            Grade::Good
        } else if bpp >= 0.03 {
            Grade::Ok
        } else {
            Grade::Low
        });
    } else {
        p.grade = Some(if video >= 1200.0 {
            Grade::Ok
        } else {
            Grade::Low
        });
    }
    p.fps = Some(fps);
    p.video_kbps = Some(video.round() as u32);
    p.audio_kbps = info.has_audio.then_some(audio);
    let bytes = (video + audio as f64) * 1000.0 / 8.0 * d * 1.01;
    p.est_size = Some((bytes as u64).min(target));
    p
}

fn audio_ext(info: &MediaInfo) -> &'static str {
    if matches!(info.ext.as_str(), "m4a" | "m4b" | "aac") {
        "m4a"
    } else {
        "mp3"
    }
}

/// The bitrate (kbps) for an audio budget: an MP3 one at or below it, at most 320.
fn audio_rate(ext: &str, kbps: f64) -> u32 {
    if ext == "mp3" {
        MP3_RATES
            .iter()
            .rev()
            .copied()
            .find(|r| *r as f64 <= kbps)
            .unwrap_or(32)
    } else {
        (kbps.floor() as u32).clamp(32, MAX_AUDIO_KBPS)
    }
}

fn plan_audio(info: &MediaInfo, target: u64) -> ShrinkPlan {
    let ext = audio_ext(info);
    let mut p = ShrinkPlan::new(ext, Verdict::Ok);
    let Some(d) = info.duration.filter(|d| *d > 0.0) else {
        p.verdict = Verdict::Unknown;
        return p;
    };
    if info.size <= target {
        p.verdict = Verdict::AlreadySmall;
        p.est_size = Some(info.size);
        return p;
    }
    let budget = target as f64 * 8.0 * SAFETY / d / 1000.0;
    if budget < MIN_AUDIO_KBPS {
        p.verdict = Verdict::TooLong;
        return p;
    }
    let mut k = audio_rate(ext, budget.min(MAX_AUDIO_KBPS as f64));
    if let Some(src) = info.audio_kbps.filter(|s| *s >= 32) {
        k = k.min(audio_rate(ext, src as f64));
    }
    p.audio_kbps = Some(k);
    p.grade = Some(if k >= 128 {
        Grade::Good
    } else if k >= 64 {
        Grade::Ok
    } else {
        Grade::Low
    });
    p.est_size = Some(((k as f64 * 1000.0 / 8.0 * d) as u64).min(target));
    p
}

fn image_ext(info: &MediaInfo) -> &'static str {
    if info.has_alpha && matches!(info.ext.as_str(), "png" | "webp" | "gif" | "tif" | "tiff") {
        "webp"
    } else {
        "jpg"
    }
}

/// About how many bytes per pixel a JPEG of this picture takes at good quality.
fn jpeg_bytes_per_pixel(info: &MediaInfo, pixels: f64) -> f64 {
    let own = info.size as f64 / pixels.max(1.0);
    match info.ext.as_str() {
        "jpg" | "jpeg" | "jfif" => own.clamp(0.08, 0.5),
        // HEIC / AVIF / WebP pack a photo about 1.5× tighter than JPEG.
        "heic" | "heif" | "avif" | "webp" => (own * 1.6).clamp(0.1, 0.5),
        _ => 0.3,
    }
}

fn plan_image(info: &MediaInfo, target: u64) -> ShrinkPlan {
    let mut p = ShrinkPlan::new(image_ext(info), Verdict::Ok);
    let (Some(w), Some(h)) = (info.width, info.height) else {
        p.verdict = Verdict::Unknown;
        return p;
    };
    if info.size <= target {
        p.verdict = Verdict::AlreadySmall;
        p.est_size = Some(info.size);
        return p;
    }
    let pixels = w as f64 * h as f64;
    let full = pixels * jpeg_bytes_per_pixel(info, pixels);
    let f = (target as f64 * 0.9 / full).sqrt().min(1.0);
    let long = w.max(h);
    let to_long = ((long as f64 * f).round() as u32).min(long);
    if to_long < MIN_IMAGE_SIDE.min(long) {
        p.verdict = Verdict::TooLong;
        return p;
    }
    let (tw, th) = if to_long < long {
        scaled_to_long(w, h, to_long)
    } else {
        (w, h)
    };
    p.width = Some(tw);
    p.height = Some(th);
    p.grade = Some(if to_long >= long.min(1600) {
        Grade::Good
    } else if to_long >= 1000 {
        Grade::Ok
    } else {
        Grade::Low
    });
    p.est_size = Some(((full * f * f) as u64).min(target));
    p
}

// ---------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------

fn video_filters(info: &MediaInfo, plan: &ShrinkPlan) -> Vec<String> {
    let mut f = Vec::new();
    if let (Some(short), Some(tw), Some(th)) = (info.short_side(), plan.width, plan.height)
        && tw.min(th) < short
    {
        f.push(fit_short_side(tw.min(th)));
    } else {
        f.push(s("scale=trunc(iw/2)*2:trunc(ih/2)*2"));
    }
    if let (Some(out), Some(src)) = (plan.fps, info.fps)
        && out + 0.5 < src
    {
        f.push(format!("fps={out}"));
    }
    f
}

/// Both passes of the H.264 encode (`pass` 1 or 2) at `video_kbps`. Pure.
pub fn video_args(
    info: &MediaInfo,
    plan: &ShrinkPlan,
    pass: u8,
    video_kbps: u32,
    passlog: &Path,
    out: &Path,
) -> Vec<String> {
    let mut a = base_args();
    a.extend([s("-i"), path_arg(&info.path), s("-map"), s("0:v:0")]);
    if pass == 2 && info.has_audio {
        a.extend([s("-map"), s("0:a:0")]);
    }
    a.extend([s("-vf"), video_filters(info, plan).join(",")]);
    a.extend([
        s("-c:v"),
        s("libx264"),
        s("-preset"),
        s("medium"),
        s("-b:v"),
        format!("{video_kbps}k"),
        s("-pass:v"),
        pass.to_string(),
        s("-passlogfile:v"),
        path_arg(passlog),
        s("-pix_fmt"),
        s("yuv420p"),
    ]);
    if pass == 1 {
        a.extend([s("-an"), s("-sn"), s("-dn"), s("-f"), s("null"), s("-")]);
        return a;
    }
    if info.has_audio {
        let k = plan.audio_kbps.unwrap_or(96);
        a.extend([
            s("-c:a"),
            s("aac"),
            s("-b:a"),
            format!("{k}k"),
            s("-ac"),
            s("2"),
        ]);
    }
    a.extend([s("-sn"), s("-dn"), s("-map_chapters"), s("-1")]);
    a.extend([
        s("-movflags"),
        s("+faststart"),
        s("-f"),
        s("mp4"),
        path_arg(out),
    ]);
    a
}

/// The sound at `kbps` (mono below 48 kbps, where stereo costs too much). Pure.
pub fn audio_args(info: &MediaInfo, ext: &str, kbps: u32, out: &Path) -> Vec<String> {
    let mut a = base_args();
    a.extend([s("-i"), path_arg(&info.path), s("-map"), s("0:a:0")]);
    a.extend([s("-vn"), s("-sn"), s("-dn")]);
    if ext == "m4a" {
        a.extend([s("-c:a"), s("aac"), s("-b:a"), format!("{kbps}k")]);
    } else {
        a.extend([s("-c:a"), s("libmp3lame"), s("-b:a"), format!("{kbps}k")]);
    }
    if kbps < 48 {
        a.extend([s("-ac"), s("1")]);
    }
    if ext == "m4a" {
        a.extend([s("-movflags"), s("+faststart"), s("-f"), s("ipod")]);
    } else {
        a.extend([s("-id3v2_version"), s("3"), s("-f"), s("mp3")]);
    }
    a.push(path_arg(out));
    a
}

/// JPEG `-q:v` (lower is better) / WebP quality, from best to the least we go.
const JPEG_Q: [u32; 3] = [3, 5, 8];
const WEBP_Q: [u32; 3] = [85, 75, 60];

/// One picture at long side `long` (`None`: as it is) and quality step `step`. Pure.
pub fn image_args(
    info: &MediaInfo,
    ext: &str,
    long: Option<u32>,
    step: usize,
    out: &Path,
) -> Vec<String> {
    let mut a = base_args();
    a.extend([s("-i"), path_arg(&info.path)]);
    let mut f = Vec::new();
    if let Some(l) = long {
        f.push(format!("{}:flags=lanczos", fit_long_side(l)));
    }
    let step = step.min(2);
    if ext == "webp" {
        if !f.is_empty() {
            a.extend([s("-vf"), f.join(",")]);
        }
        a.extend([s("-frames:v"), s("1"), s("-c:v"), s("libwebp")]);
        a.extend([s("-quality"), WEBP_Q[step].to_string(), s("-f"), s("webp")]);
    } else {
        if info.has_alpha {
            f.push(s(FLATTEN_ON_WHITE));
        }
        f.push(s("format=yuvj420p"));
        a.extend([s("-vf"), f.join(",")]);
        a.extend([
            s("-frames:v"),
            s("1"),
            s("-update"),
            s("1"),
            s("-c:v"),
            s("mjpeg"),
        ]);
        a.extend([s("-q:v"), JPEG_Q[step].to_string(), s("-f"), s("image2")]);
    }
    a.extend([s("-map_metadata"), s("-1"), path_arg(out)]);
    a
}

// ---------------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------------

fn file_size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(u64::MAX)
}

static PASS_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Removes x264's two-pass statistics (`<prefix>-0.log`, `.mbtree`, `.temp`) when dropped.
struct PassLog(PathBuf);

impl PassLog {
    fn new() -> PassLog {
        let n = PASS_COUNTER.fetch_add(1, Ordering::Relaxed);
        PassLog(std::env::temp_dir().join(format!("prism-pass-{}-{n}", std::process::id())))
    }
}

impl Drop for PassLog {
    fn drop(&mut self) {
        let (Some(dir), Some(prefix)) = (self.0.parent(), self.0.file_name()) else {
            return;
        };
        let prefix = prefix.to_string_lossy().into_owned();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().starts_with(&prefix) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }
}

/// Make `info` smaller than `target` bytes into `out` (whose extension is the plan's).
pub(crate) async fn shrink(
    ffmpeg: &Path,
    info: &MediaInfo,
    target: u64,
    out: &Path,
    progress: ProgressFn<'_>,
    cancel: &CancellationToken,
) -> Result<ShrinkOutcome> {
    let plan = plan(info, target);
    if let Some(problem) = verdict_problem(&plan) {
        return Err(VideoError::Media(problem));
    }
    let tmp = temp_output(out);
    let res = match info.kind {
        MediaKind::Video => shrink_video(ffmpeg, info, &plan, target, &tmp, progress, cancel).await,
        MediaKind::Audio => shrink_audio(ffmpeg, info, &plan, target, &tmp, progress, cancel).await,
        MediaKind::Image => shrink_image(ffmpeg, info, &plan, target, &tmp, progress, cancel).await,
        MediaKind::Other => Err(VideoError::Media(MediaProblem::Unreadable)),
    };
    match res {
        Ok(size) => {
            let path = finish_output(&tmp, out)?;
            Ok(ShrinkOutcome { path, size })
        }
        Err(e) => {
            discard(&tmp).await;
            Err(e)
        }
    }
}

/// The next smaller resolution step below `short` (none below 360p).
fn rung_below(short: u32) -> Option<u32> {
    [720, 480, 360].into_iter().find(|r| *r < short)
}

async fn shrink_video(
    ffmpeg: &Path,
    info: &MediaInfo,
    plan: &ShrinkPlan,
    target: u64,
    tmp: &Path,
    progress: ProgressFn<'_>,
    cancel: &CancellationToken,
) -> Result<u64> {
    let log = PassLog::new();
    let d = info.duration.unwrap_or(1.0).max(0.1);
    let mut plan = plan.clone();
    let mut video = plan.video_kbps.unwrap_or(500);
    let audio = plan.audio_kbps.unwrap_or(0) as f64;
    let budget = target as f64 * 8.0 * SAFETY / d / 1000.0 - audio;
    let pass1 = video_args(info, &plan, 1, video, &log.0, tmp);
    run_mapped(ffmpeg, &pass1, Some(d), (0.0, 0.3), false, progress, cancel).await?;
    for attempt in 0..3 {
        let pass2 = video_args(info, &plan, 2, video, &log.0, tmp);
        run_mapped(
            ffmpeg,
            &pass2,
            Some(d),
            (0.3, 1.0),
            attempt > 0,
            progress,
            cancel,
        )
        .await?;
        let size = file_size(tmp);
        if size <= target {
            return Ok(size);
        }
        // Over: aim lower by as much as the picture overshot. Very busy footage (noise, rain,
        // confetti) can't go lower at the same size, so the last try also steps the
        // resolution down (which needs a new first pass).
        let got = (size as f64 * 8.0 / d / 1000.0 - audio).max(1.0);
        let next = (video as f64 * (budget / got) * 0.97).floor();
        tracing::info!(
            size,
            target,
            video,
            next,
            "the video came out over the limit; trying smaller"
        );
        if next < MIN_VIDEO_KBPS {
            break;
        }
        video = next as u32;
        if attempt == 1
            && let (Some(w), Some(h)) = (plan.width, plan.height)
            && let Some(lower) = rung_below(w.min(h))
        {
            let (lw, lh) = scaled_to_short(w, h, lower);
            (plan.width, plan.height) = (Some(lw), Some(lh));
            let pass1 = video_args(info, &plan, 1, video, &log.0, tmp);
            run_mapped(ffmpeg, &pass1, Some(d), (0.0, 0.3), true, progress, cancel).await?;
        }
    }
    Err(VideoError::Media(MediaProblem::OverTarget))
}

async fn shrink_audio(
    ffmpeg: &Path,
    info: &MediaInfo,
    plan: &ShrinkPlan,
    target: u64,
    tmp: &Path,
    progress: ProgressFn<'_>,
    cancel: &CancellationToken,
) -> Result<u64> {
    let mut k = plan.audio_kbps.unwrap_or(64);
    for attempt in 0..2 {
        let args = audio_args(info, &plan.ext, k, tmp);
        run_mapped(
            ffmpeg,
            &args,
            info.duration,
            (0.0, 1.0),
            attempt > 0,
            progress,
            cancel,
        )
        .await?;
        let size = file_size(tmp);
        if size <= target {
            return Ok(size);
        }
        let next = audio_rate(&plan.ext, k as f64 * target as f64 * SAFETY / size as f64);
        if next >= k || (next as f64) < MIN_AUDIO_KBPS {
            break;
        }
        k = next;
    }
    Err(VideoError::Media(MediaProblem::OverTarget))
}

async fn shrink_image(
    ffmpeg: &Path,
    info: &MediaInfo,
    plan: &ShrinkPlan,
    target: u64,
    tmp: &Path,
    progress: ProgressFn<'_>,
    cancel: &CancellationToken,
) -> Result<u64> {
    let full = info.width.unwrap_or(0).max(info.height.unwrap_or(0));
    let start = plan.width.unwrap_or(0).max(plan.height.unwrap_or(0));
    let mut long = if start > 0 && start < full {
        start
    } else {
        full
    };
    let mut tries = 0u32;
    let mut encode = async |long: u32, step: usize| -> Result<u64> {
        tries += 1;
        progress(MediaProgress {
            fraction: Some((tries as f64 / 6.0).min(0.9)),
            retrying: false,
        });
        let scale = (long < full).then_some(long);
        let args = image_args(info, &plan.ext, scale, step, tmp);
        run_mapped(ffmpeg, &args, None, (0.0, 1.0), false, &|_| {}, cancel).await?;
        Ok(file_size(tmp))
    };
    // The best quality at the planned size; a lot of room left over means the estimate was
    // careful, so it goes back up towards the full size once.
    let mut size = encode(long, 0).await?;
    if size <= target {
        if long < full && (size as f64) < target as f64 * 0.6 {
            let bigger =
                ((long as f64 * (target as f64 * 0.85 / size as f64).sqrt()) as u32).min(full);
            let again = encode(bigger, 0).await?;
            if again <= target {
                return Ok(again);
            }
            return encode(long, 0).await;
        }
        return Ok(size);
    }
    size = encode(long, 1).await?;
    for _ in 0..6 {
        if size <= target {
            return Ok(size);
        }
        let f = (target as f64 * 0.92 / size as f64).sqrt().clamp(0.5, 0.95);
        let next = (long as f64 * f) as u32;
        if next < MIN_IMAGE_SIDE.min(full) {
            // The smallest it goes, at the lowest quality.
            size = encode(MIN_IMAGE_SIDE.min(full), 2).await?;
            return if size <= target {
                Ok(size)
            } else {
                Err(VideoError::Media(MediaProblem::OverTarget))
            };
        }
        long = next;
        size = encode(long, 1).await?;
    }
    if size <= target {
        Ok(size)
    } else {
        Err(VideoError::Media(MediaProblem::OverTarget))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1_000_000;

    fn video(secs: f64, w: u32, h: u32, size: u64) -> MediaInfo {
        MediaInfo {
            path: PathBuf::from("/d/clip.mov"),
            ext: "mov".into(),
            kind: MediaKind::Video,
            size,
            width: Some(w),
            height: Some(h),
            duration: Some(secs),
            fps: Some(60.0),
            has_audio: true,
            audio_kbps: Some(192),
            video_kbps: Some(16_000),
            video_codec: Some("hevc".into()),
            audio_codec: Some("aac".into()),
            ..Default::default()
        }
    }

    #[test]
    fn steps_down() {
        assert_eq!(rung_below(1080), Some(720));
        assert_eq!(rung_below(720), Some(480));
        assert_eq!(rung_below(500), Some(480));
        assert_eq!(rung_below(360), None);
    }

    #[test]
    fn ladder_and_sound() {
        assert_eq!(rung_for(4000.0), 1080);
        assert_eq!(rung_for(2500.0), 1080);
        assert_eq!(rung_for(2499.0), 720);
        assert_eq!(rung_for(1200.0), 720);
        assert_eq!(rung_for(800.0), 480);
        assert_eq!(rung_for(300.0), 360);
        assert_eq!(audio_for(3000.0), 128);
        assert_eq!(audio_for(1000.0), 96);
        assert_eq!(audio_for(400.0), 64);
    }

    #[test]
    fn a_minute_of_4k_for_discord() {
        // 60 s of 4K60 (120 MB) into 20 MB: 2.53 Mbps total, 128 kbps sound, 2.4 Mbps picture.
        let p = plan(&video(60.0, 3840, 2160, 120 * MB), 20 * MB);
        assert_eq!(p.verdict, Verdict::Ok);
        assert_eq!(p.ext, "mp4");
        assert_eq!(p.audio_kbps, Some(128));
        assert_eq!(p.video_kbps, Some(2405));
        assert_eq!((p.width, p.height), (Some(1280), Some(720)));
        assert_eq!(p.fps, Some(60.0));
        assert_eq!(p.grade, Some(Grade::Ok));
        let est = p.est_size.unwrap();
        assert!(est <= 20 * MB && est > 18 * MB, "{est}");
    }

    #[test]
    fn portrait_and_frame_rate() {
        // 3 minutes of a phone video into 25 MB: ~1.06 Mbps → 480p, 30 fps, 96 kbps sound.
        let p = plan(&video(180.0, 1080, 1920, 300 * MB), 25 * MB);
        assert_eq!(p.audio_kbps, Some(96));
        assert_eq!((p.width, p.height), (Some(480), Some(854)));
        assert_eq!(p.fps, Some(30.0));
        assert_eq!(p.grade, Some(Grade::Good));
        let args = video_args(
            &video(180.0, 1080, 1920, 300 * MB),
            &p,
            2,
            p.video_kbps.unwrap(),
            Path::new("/t/log"),
            Path::new("/d/o.mp4"),
        );
        let vf = &args[args.iter().position(|a| a == "-vf").unwrap() + 1];
        assert_eq!(vf, &format!("{},fps=30", fit_short_side(480)));
        assert!(args.windows(2).any(|w| w == ["-b:a", "96k"]));
        assert!(args.windows(2).any(|w| w == ["-pass:v", "2"]));
        assert!(args.windows(2).any(|w| w == ["-passlogfile:v", "/t/log"]));
    }

    #[test]
    fn too_long_and_already_small() {
        // Ten minutes into 20 MB is low quality but possible…
        let p = plan(&video(600.0, 1920, 1080, 900 * MB), 20 * MB);
        assert_eq!(p.verdict, Verdict::Ok);
        assert_eq!(p.grade, Some(Grade::Low));
        assert_eq!((p.width, p.height), (Some(640), Some(360)));
        assert_eq!((p.audio_kbps, p.fps), (Some(64), Some(30.0)));
        // …into 10 MB it isn't (60 kbps of picture), nor two hours.
        assert_eq!(
            plan(&video(600.0, 1920, 1080, 900 * MB), 10 * MB).verdict,
            Verdict::TooLong
        );
        let p = plan(&video(7200.0, 1920, 1080, 4000 * MB), 10 * MB);
        assert_eq!(p.verdict, Verdict::TooLong);
        assert_eq!(verdict_problem(&p), Some(MediaProblem::TooLong));
        let p = plan(&video(10.0, 1920, 1080, 8 * MB), 10 * MB);
        assert_eq!(p.verdict, Verdict::AlreadySmall);
        let mut unknown = video(10.0, 1920, 1080, 80 * MB);
        unknown.duration = None;
        assert_eq!(plan(&unknown, 10 * MB).verdict, Verdict::Unknown);
    }

    #[test]
    fn small_sources_keep_their_size() {
        // A 480p clip stays 480p (never scaled up) and good.
        let mut v = video(30.0, 854, 480, 30 * MB);
        v.fps = Some(30.0);
        let p = plan(&v, 25 * MB);
        assert_eq!((p.width, p.height), (Some(854), Some(480)));
        assert_eq!(p.grade, Some(Grade::Good));
        let args = video_args(&v, &p, 1, 1000, Path::new("/t/log"), Path::new("/d/o.mp4"));
        assert!(args.windows(2).any(|w| w == ["-pass:v", "1"]));
        assert!(args.windows(2).any(|w| w == ["-passlogfile:v", "/t/log"]));
        assert!(args.ends_with(&[s("-an"), s("-sn"), s("-dn"), s("-f"), s("null"), s("-")]));
        assert!(args.contains(&s("scale=trunc(iw/2)*2:trunc(ih/2)*2")));
    }

    #[test]
    fn audio_plans() {
        let song = MediaInfo {
            path: PathBuf::from("/d/set.flac"),
            ext: "flac".into(),
            kind: MediaKind::Audio,
            size: 400 * MB,
            duration: Some(3600.0),
            has_audio: true,
            ..Default::default()
        };
        // An hour into 25 MB: 52.8 kbps → 48 kbps MP3, mono.
        let p = plan(&song, 25 * MB);
        assert_eq!(
            (p.ext.as_str(), p.audio_kbps, p.grade),
            ("mp3", Some(48), Some(Grade::Low))
        );
        let a = audio_args(&song, "mp3", 40, Path::new("o.mp3"));
        assert!(a.windows(2).any(|w| w == ["-ac", "1"]));
        // Ten minutes into 20 MB: 253 kbps → 224 kbps MP3, good.
        let mut short = song.clone();
        short.duration = Some(600.0);
        short.size = 100 * MB;
        let p = plan(&short, 20 * MB);
        assert_eq!((p.audio_kbps, p.grade), (Some(224), Some(Grade::Good)));
        // Three hours into 10 MB can't be done.
        let mut long = song.clone();
        long.duration = Some(3.0 * 3600.0);
        assert_eq!(plan(&long, 10 * MB).verdict, Verdict::TooLong);
        let mut m4a = short.clone();
        m4a.ext = "m4a".into();
        assert_eq!(plan(&m4a, 20 * MB).ext, "m4a");
    }

    #[test]
    fn image_plans() {
        let photo = MediaInfo {
            path: PathBuf::from("/d/IMG.HEIC"),
            ext: "heic".into(),
            kind: MediaKind::Image,
            size: 3_400_000,
            width: Some(3024),
            height: Some(4032),
            ..Default::default()
        };
        // A 12 MP HEIC photo into 1 MB: smaller, still portrait.
        let p = plan(&photo, MB);
        assert_eq!(p.verdict, Verdict::Ok);
        assert_eq!(p.ext, "jpg");
        let (w, h) = (p.width.unwrap(), p.height.unwrap());
        assert!(h > w && (1600..4032).contains(&h), "{w}x{h}");
        assert_eq!(p.grade, Some(Grade::Good));
        assert_eq!(plan(&photo, 10 * MB).verdict, Verdict::AlreadySmall);
        // A transparent PNG becomes a WebP.
        let logo = MediaInfo {
            ext: "png".into(),
            has_alpha: true,
            size: 30 * MB,
            ..photo.clone()
        };
        assert_eq!(plan(&logo, 10 * MB).ext, "webp");
        // A huge scan into 50 kB would be a thumbnail: no.
        let scan = MediaInfo {
            ext: "png".into(),
            size: 60 * MB,
            width: Some(10_000),
            height: Some(7000),
            ..photo.clone()
        };
        assert_eq!(plan(&scan, 50_000).verdict, Verdict::TooLong);
        let a = image_args(&logo, "jpg", Some(2048), 1, Path::new("o.jpg"));
        let vf = &a[a.iter().position(|x| x == "-vf").unwrap() + 1];
        assert!(
            vf.starts_with("scale=w='if(gte(iw,ih),min(2048,iw),-2)'")
                && vf.contains(FLATTEN_ON_WHITE),
            "{vf}"
        );
        assert!(a.windows(2).any(|w| w == ["-q:v", "5"]));
        let a = image_args(&logo, "webp", None, 0, Path::new("o.webp"));
        assert!(!a.contains(&s("-vf")));
        assert!(a.windows(2).any(|w| w == ["-quality", "85"]));
    }
}
