//! Quality-picker computation: turns yt-dlp formats into a short list of
//! user-facing [`Choice`]s (one per height / HDR flag, plus audio-only entries).

use std::cmp::Ordering;
use std::collections::BTreeMap;

use crate::model::{AudioFormat, Choice, ChoiceKind, Container, Format, FormatKind, VideoInfo};

/// Highest resolution offered in the picker (8K).
pub const MAX_HEIGHT: u32 = 4320;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    Hevc,
    Av1,
    Vp9,
    Vp8,
    Other,
    Unknown,
}

impl VideoCodec {
    pub fn from_codec(c: Option<&str>) -> Self {
        let Some(c) = c else {
            return VideoCodec::Unknown;
        };
        let c = c.to_ascii_lowercase();
        if c.starts_with("avc") || c.starts_with("h264") || c == "h.264" {
            VideoCodec::H264
        } else if c.starts_with("hev")
            || c.starts_with("hvc")
            || c.starts_with("h265")
            || c == "hevc"
        {
            VideoCodec::Hevc
        } else if c.starts_with("av01") || c == "av1" {
            VideoCodec::Av1
        } else if c.starts_with("vp09") || c.starts_with("vp9") {
            VideoCodec::Vp9
        } else if c.starts_with("vp8") || c.starts_with("vp08") {
            VideoCodec::Vp8
        } else {
            VideoCodec::Other
        }
    }

    pub fn label(self) -> Option<&'static str> {
        Some(match self {
            VideoCodec::H264 => "H.264",
            VideoCodec::Hevc => "HEVC",
            VideoCodec::Av1 => "AV1",
            VideoCodec::Vp9 => "VP9",
            VideoCodec::Vp8 => "VP8",
            VideoCodec::Other | VideoCodec::Unknown => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioCodec {
    Aac,
    Opus,
    Vorbis,
    Mp3,
    Ac3,
    Eac3,
    Flac,
    Other,
    Unknown,
}

impl AudioCodec {
    pub fn from_codec(c: Option<&str>) -> Self {
        let Some(c) = c else {
            return AudioCodec::Unknown;
        };
        let c = c.to_ascii_lowercase();
        if c.starts_with("mp4a.40") || c == "aac" || c.starts_with("mp4a") {
            AudioCodec::Aac
        } else if c.starts_with("opus") {
            AudioCodec::Opus
        } else if c.starts_with("vorbis") {
            AudioCodec::Vorbis
        } else if c == "mp3" || c.starts_with("mp4a.6b") || c.starts_with("mp4a.69") {
            AudioCodec::Mp3
        } else if c == "ac-3" || c == "ac3" {
            AudioCodec::Ac3
        } else if c == "ec-3" || c == "eac3" {
            AudioCodec::Eac3
        } else if c.starts_with("flac") {
            AudioCodec::Flac
        } else {
            AudioCodec::Other
        }
    }

    pub fn label(self) -> Option<&'static str> {
        Some(match self {
            AudioCodec::Aac => "AAC",
            AudioCodec::Opus => "Opus",
            AudioCodec::Vorbis => "Vorbis",
            AudioCodec::Mp3 => "MP3",
            AudioCodec::Ac3 => "AC-3",
            AudioCodec::Eac3 => "E-AC-3",
            AudioCodec::Flac => "FLAC",
            AudioCodec::Other | AudioCodec::Unknown => return None,
        })
    }
}

/// Lower is better. `None` = not allowed in this container.
fn video_rank(codec: VideoCodec, container: Container, hdr: bool) -> Option<u8> {
    use VideoCodec::*;
    match container {
        Container::Mp4 if hdr => match codec {
            Av1 => Some(0),
            Vp9 => Some(1),
            Hevc => Some(2),
            Unknown => Some(5),
            _ => Some(6),
        },
        Container::Mp4 => match codec {
            H264 => Some(0),
            Av1 => Some(1),
            Vp9 => Some(2),
            Hevc => Some(3),
            Unknown => Some(5),
            Vp8 | Other => None,
        },
        Container::Mkv => match codec {
            Vp9 => Some(0),
            Av1 => Some(1),
            H264 => Some(2),
            Hevc => Some(3),
            Vp8 => Some(4),
            Unknown => Some(5),
            Other => Some(6),
        },
        Container::Webm => match codec {
            Vp9 => Some(0),
            Av1 => Some(1),
            Vp8 => Some(2),
            Unknown => Some(5),
            _ => None,
        },
    }
}

fn audio_rank(codec: AudioCodec, container: Container) -> Option<u8> {
    use AudioCodec::*;
    match container {
        Container::Mp4 => match codec {
            Aac => Some(0),
            Opus => Some(1),
            Mp3 => Some(2),
            Eac3 | Ac3 => Some(3),
            Flac => Some(4),
            Unknown => Some(8),
            Vorbis | Other => None,
        },
        Container::Webm => match codec {
            Opus => Some(0),
            Vorbis => Some(1),
            Unknown => Some(8),
            _ => None,
        },
        Container::Mkv => match codec {
            Opus => Some(0),
            Aac => Some(1),
            Vorbis => Some(2),
            Flac => Some(3),
            Eac3 | Ac3 | Mp3 => Some(4),
            Unknown => Some(8),
            Other => Some(9),
        },
    }
}

fn cmp_f64(a: f64, b: f64) -> Ordering {
    a.partial_cmp(&b).unwrap_or(Ordering::Equal)
}

fn fps_bucket(f: &Format) -> i64 {
    f.fps.map(|x| x.round() as i64).unwrap_or(0)
}

fn bitrate(f: &Format) -> f64 {
    f.abr.or(f.tbr).or(f.vbr).unwrap_or(0.0)
}

/// Sort key for audio sources: original language, non-DRC, codec, accelerable, bitrate.
fn audio_better(a: &Format, b: &Format, rank: impl Fn(&Format) -> u8) -> Ordering {
    let lp = |f: &Format| f.language_preference.unwrap_or(-1);
    lp(b)
        .cmp(&lp(a))
        .then(a.is_drc().cmp(&b.is_drc()))
        .then(rank(a).cmp(&rank(b)))
        .then(b.is_progressive_http().cmp(&a.is_progressive_http()))
        .then(cmp_f64(bitrate(b), bitrate(a)))
}

/// Best audio-only format compatible with `container`.
pub fn best_audio(formats: &[Format], container: Container) -> Option<&Format> {
    formats
        .iter()
        .filter(|f| f.usable && f.kind == FormatKind::Audio)
        .filter(|f| audio_rank(AudioCodec::from_codec(f.acodec.as_deref()), container).is_some())
        .min_by(|a, b| {
            audio_better(a, b, |f| {
                audio_rank(AudioCodec::from_codec(f.acodec.as_deref()), container).unwrap_or(9)
            })
        })
}

/// Best audio-only format of a given codec family.
fn best_audio_of(formats: &[Format], codec: AudioCodec) -> Option<&Format> {
    formats
        .iter()
        .filter(|f| {
            f.usable
                && f.kind == FormatKind::Audio
                && AudioCodec::from_codec(f.acodec.as_deref()) == codec
        })
        .min_by(|a, b| audio_better(a, b, |_| 0))
}

/// Best audio source for conversions (any known codec, highest bitrate);
/// falls back to a muxed format when the site has no audio-only streams.
fn best_audio_any(formats: &[Format]) -> Option<&Format> {
    let known = |f: &Format| match AudioCodec::from_codec(f.acodec.as_deref()) {
        AudioCodec::Unknown | AudioCodec::Other => 1,
        _ => 0,
    };
    formats
        .iter()
        .filter(|f| f.usable && f.kind == FormatKind::Audio)
        .min_by(|a, b| audio_better(a, b, known))
        .or_else(|| {
            formats
                .iter()
                .filter(|f| f.usable && f.kind == FormatKind::Muxed)
                .min_by(|a, b| cmp_f64(bitrate(a), bitrate(b)).reverse())
        })
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1000.0 && i < UNITS.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else if v >= 100.0 {
        format!("{v:.0} {}", UNITS[i])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// Whether `s` looks like a resolution label (`"1080p"`, `"1080p60"`) rather than other text
/// a `format_note` can carry ("Premium", a codec hint, …) that shouldn't leak into the
/// quality picker verbatim.
fn looks_like_resolution_label(s: &str) -> bool {
    let Some(p) = s.find('p') else { return false };
    let (digits, rest) = s.split_at(p);
    !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit())
        && rest[1..].chars().all(|c| c.is_ascii_digit())
}

fn container_label(c: Container) -> &'static str {
    match c {
        Container::Mp4 => "MP4",
        Container::Mkv => "MKV",
        Container::Webm => "WebM",
    }
}

fn audio_ok_in(codec: AudioCodec, container: Container) -> bool {
    audio_rank(codec, container).is_some()
}

/// Compute the quality-picker list for `info` targeting `container`.
pub fn compute_choices(info: &VideoInfo, container: Container) -> Vec<Choice> {
    let formats = &info.formats;
    let duration = info.duration;
    let mut out = Vec::new();
    if info.has_drm {
        return out;
    }

    let audio = best_audio(formats, container).or_else(|| best_audio(formats, Container::Mkv));
    let has_audio_only = formats
        .iter()
        .any(|f| f.usable && f.kind == FormatKind::Audio);

    // Bucket video-carrying formats by (height, hdr).
    let mut buckets: BTreeMap<(u32, bool), Vec<&Format>> = BTreeMap::new();
    for f in formats.iter().filter(|f| f.usable && f.has_video()) {
        let h = f.height.unwrap_or(0);
        if h > MAX_HEIGHT {
            continue;
        }
        buckets.entry((h, f.is_hdr())).or_default().push(f);
    }

    // Highest first; HDR before SDR at the same height.
    let mut keys: Vec<(u32, bool)> = buckets.keys().copied().collect();
    keys.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));

    for key in keys {
        let (height, hdr) = key;
        let candidates = &buckets[&key];
        let rank = |f: &Format| {
            video_rank(VideoCodec::from_codec(f.vcodec.as_deref()), container, hdr).unwrap_or(20)
        };
        let Some(best) = candidates.iter().copied().min_by(|a, b| {
            fps_bucket(b)
                .cmp(&fps_bucket(a))
                .then(rank(a).cmp(&rank(b)))
                // Prefer separate video + best audio over a pre-muxed (usually low-bitrate audio) file.
                .then(
                    ((a.kind == FormatKind::Muxed) && has_audio_only)
                        .cmp(&((b.kind == FormatKind::Muxed) && has_audio_only)),
                )
                .then(b.is_progressive_http().cmp(&a.is_progressive_http()))
                .then(cmp_f64(b.tbr.unwrap_or(0.0), a.tbr.unwrap_or(0.0)))
        }) else {
            continue;
        };

        let vcodec = VideoCodec::from_codec(best.vcodec.as_deref());
        let muxed = best.kind == FormatKind::Muxed;
        let pair_audio = if muxed { None } else { audio };
        let acodec = if muxed {
            AudioCodec::from_codec(best.acodec.as_deref())
        } else {
            pair_audio
                .map(|a| AudioCodec::from_codec(a.acodec.as_deref()))
                .unwrap_or(AudioCodec::Unknown)
        };

        // Fall back to MKV when the requested container cannot hold these codecs.
        let video_ok = video_rank(vcodec, container, hdr).is_some();
        let audio_ok = pair_audio.is_none() && !muxed || audio_ok_in(acodec, container);
        let mut target = if video_ok && audio_ok {
            container
        } else {
            Container::Mkv
        };
        if muxed && vcodec == VideoCodec::Unknown {
            // Unknown codecs (direct files): keep the file's own container when it is one of ours.
            target = match best.ext.as_str() {
                "mp4" | "m4v" | "mov" => Container::Mp4,
                "webm" => Container::Webm,
                "mkv" => Container::Mkv,
                _ => container,
            };
        }

        let est_size = match (
            best.size_estimate(duration),
            pair_audio.map(|a| a.size_estimate(duration)),
        ) {
            (Some(v), Some(Some(a))) => Some(v + a),
            (Some(v), None) => Some(v),
            (Some(v), Some(None)) => Some(v),
            _ => None,
        };

        let fps = best.fps;
        // Prefer the source's own quality label ("1080p", "1080p60") when it looks like one:
        // some videos (unusual aspect ratios) encode a tier at a raw pixel height that doesn't
        // match the round number the label advertises, e.g. a "1080p" format at height=1608.
        let mut label = match best
            .format_note
            .as_deref()
            .filter(|n| looks_like_resolution_label(n))
        {
            Some(n) => n.to_string(),
            None if height > 0 => {
                let mut s = format!("{height}p");
                if let Some(f) = fps.filter(|f| *f > 30.5) {
                    s.push_str(&format!("{}", f.round() as i64));
                }
                s
            }
            None => match &best.format_note {
                Some(n) if !n.is_empty() => n.clone(),
                _ => "Original quality".to_string(),
            },
        };
        if hdr {
            label.push_str(" · HDR");
        }
        if let Some(c) = vcodec.label() {
            label.push_str(" · ");
            label.push_str(c);
        }
        let mut sub_label = container_label(target).to_string();
        if let Some(s) = est_size {
            sub_label.push_str(&format!(" · ~{}", human_size(s)));
        }

        let accelerable =
            best.is_progressive_http() && pair_audio.is_none_or(|a| a.is_progressive_http());
        let id = format!(
            "v{}{}{}",
            if height > 0 {
                format!("{height}p")
            } else {
                "best".into()
            },
            fps.filter(|f| *f > 30.5)
                .map(|f| (f.round() as i64).to_string())
                .unwrap_or_default(),
            if hdr { "-hdr" } else { "" }
        );

        out.push(Choice {
            id,
            kind: ChoiceKind::Video,
            label,
            sub_label,
            height: best.height,
            width: best.width,
            fps,
            hdr,
            dynamic_range: best.dynamic_range.clone(),
            vcodec: vcodec.label().map(str::to_string),
            acodec: acodec.label().map(str::to_string),
            est_size,
            video_format_id: (!muxed).then(|| best.format_id.clone()),
            audio_format_id: pair_audio.map(|a| a.format_id.clone()),
            format_id: muxed.then(|| best.format_id.clone()),
            container: target,
            audio_format: None,
            audio_bitrate_kbps: None,
            accelerable,
        });
    }

    out.extend(audio_choices(formats, duration));
    out
}

fn audio_choices(formats: &[Format], duration: Option<f64>) -> Vec<Choice> {
    let mut out = Vec::new();
    let Some(any) = best_audio_any(formats) else {
        return out;
    };
    let aac = best_audio_of(formats, AudioCodec::Aac);
    let opus = best_audio_of(formats, AudioCodec::Opus);

    let make = |id: &str,
                label: String,
                src: &Format,
                fmt: AudioFormat,
                kbps: Option<u32>,
                est: Option<u64>| {
        let mut sub = fmt.ext().to_uppercase();
        if let Some(s) = est {
            sub.push_str(&format!(" · ~{}", human_size(s)));
        }
        Choice {
            id: id.to_string(),
            kind: ChoiceKind::Audio,
            label,
            sub_label: sub,
            acodec: AudioCodec::from_codec(src.acodec.as_deref())
                .label()
                .map(str::to_string),
            est_size: est,
            audio_format_id: (src.kind == FormatKind::Audio).then(|| src.format_id.clone()),
            format_id: (src.kind != FormatKind::Audio).then(|| src.format_id.clone()),
            container: Container::Mp4,
            audio_format: Some(fmt),
            audio_bitrate_kbps: kbps,
            accelerable: src.is_progressive_http(),
            ..Default::default()
        }
    };
    let by_rate = |kbps: f64| duration.map(|d| (kbps * 1000.0 / 8.0 * d) as u64);

    out.push(make(
        "a-mp3",
        "Audio · MP3 320 kbps".into(),
        any,
        AudioFormat::Mp3,
        Some(320),
        by_rate(320.0),
    ));
    match aac {
        Some(a) => out.push(make(
            "a-m4a",
            "Audio · M4A (original AAC)".into(),
            a,
            AudioFormat::M4a,
            None,
            a.size_estimate(duration),
        )),
        None => out.push(make(
            "a-m4a",
            "Audio · M4A (AAC 256 kbps)".into(),
            any,
            AudioFormat::M4a,
            Some(256),
            by_rate(256.0),
        )),
    }
    match opus {
        Some(o) => out.push(make(
            "a-opus",
            "Audio · Opus (original)".into(),
            o,
            AudioFormat::Opus,
            None,
            o.size_estimate(duration),
        )),
        None => out.push(make(
            "a-opus",
            "Audio · Opus (160 kbps)".into(),
            any,
            AudioFormat::Opus,
            Some(160),
            by_rate(160.0),
        )),
    }
    out.push(make(
        "a-flac",
        "Audio · FLAC (lossless container)".into(),
        any,
        AudioFormat::Flac,
        None,
        None,
    ));
    out
}
