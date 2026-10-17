//! DASH: MPD parsing (`dash-mpd-core`) and segment resolution for the common static cases:
//! `SegmentTemplate` (`$Number$` with `@duration`, or `SegmentTimeline` with `$Time$`/`$Number$`),
//! `SegmentList`, and single-file representations (`SegmentBase` / bare `BaseURL`).
//! Live (`type="dynamic"`), multi-period and DRM-protected manifests are reported as
//! [`EngineError::Unsupported`] so the application can fall back to another downloader.

use dash_mpd_core::{AdaptationSet, BaseURL, MPD, Period, Representation, SegmentTemplate};
use reqwest::header::HeaderMap;
use url::Url;

use super::{MediaPlan, MediaSegment, Track, name_from_manifest};
use crate::error::EngineError;
use crate::request::{MediaKind, VariantSelection};

fn dash_err(m: impl Into<String>) -> EngineError {
    EngineError::Dash { message: m.into() }
}

fn unsupported(m: impl Into<String>) -> EngineError {
    EngineError::Unsupported { message: m.into() }
}

/// Upper bound of segments per representation (protects against absurd manifests).
const MAX_SEGMENTS: u64 = 200_000;

/// Expands a `SegmentTemplate` string (`$RepresentationID$`, `$Number%05d$`, `$Time$`,
/// `$Bandwidth$`, `$$`).
pub(crate) fn fill_template(
    tpl: &str,
    rep_id: &str,
    number: u64,
    time: u64,
    bandwidth: u64,
) -> Result<String, EngineError> {
    let mut out = String::with_capacity(tpl.len() + 16);
    let mut rest = tpl;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        rest = &rest[i + 1..];
        let j = rest
            .find('$')
            .ok_or_else(|| dash_err(format!("unterminated identifier in template {tpl:?}")))?;
        let ident = &rest[..j];
        rest = &rest[j + 1..];
        if ident.is_empty() {
            out.push('$');
            continue;
        }
        let (name, fmt) = match ident.split_once('%') {
            Some((n, f)) => (n, Some(f)),
            None => (ident, None),
        };
        let value = match name {
            "RepresentationID" => {
                out.push_str(rep_id);
                continue;
            }
            "Number" => number,
            "Time" => time,
            "Bandwidth" => bandwidth,
            other => return Err(unsupported(format!("DASH template identifier ${other}$"))),
        };
        match fmt {
            None => out.push_str(&value.to_string()),
            Some(f) => {
                let width: usize = f
                    .strip_suffix('d')
                    .map(|w| w.trim_start_matches('0'))
                    .and_then(|w| {
                        if w.is_empty() {
                            Some(0)
                        } else {
                            w.parse().ok()
                        }
                    })
                    .ok_or_else(|| dash_err(format!("bad format {f:?} in template")))?;
                out.push_str(&format!("{value:0width$}"));
            }
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// Merges templates from the general (period) to the specific (representation) level.
fn merge_templates(levels: [Option<&SegmentTemplate>; 3]) -> Option<SegmentTemplate> {
    let mut acc: Option<SegmentTemplate> = None;
    for t in levels.into_iter().flatten() {
        acc = Some(match acc {
            None => t.clone(),
            Some(mut a) => {
                macro_rules! take {
                    ($($f:ident),*) => { $( if t.$f.is_some() { a.$f = t.$f.clone(); } )* };
                }
                take!(
                    media,
                    initialization,
                    startNumber,
                    endNumber,
                    duration,
                    timescale,
                    SegmentTimeline,
                    Initialization,
                    presentationTimeOffset
                );
                a
            }
        });
    }
    acc
}

fn join(base: &Url, b: Option<&BaseURL>) -> Result<Url, EngineError> {
    match b {
        Some(b) if !b.base.trim().is_empty() => base
            .join(b.base.trim())
            .map_err(|e| dash_err(format!("bad BaseURL: {e}"))),
        _ => Ok(base.clone()),
    }
}

/// Parses `a-b` (inclusive) into `[a, b+1)`.
fn parse_byte_range(s: &str) -> Option<(u64, u64)> {
    let (a, b) = s.trim().split_once('-')?;
    let (a, b): (u64, u64) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
    (b >= a).then_some((a, b + 1))
}

/// Resolves the segment list of one representation.
fn representation_segments(
    period: &Period,
    adapt: &AdaptationSet,
    rep: &Representation,
    base: &Url,
    period_secs: Option<f64>,
) -> Result<Vec<MediaSegment>, EngineError> {
    let base = join(&join(base, adapt.BaseURL.first())?, rep.BaseURL.first())?;
    let seg = |u: &str, range: Option<(u64, u64)>| -> Result<MediaSegment, EngineError> {
        Ok(MediaSegment {
            url: base
                .join(u)
                .map_err(|e| dash_err(format!("bad segment URL {u:?}: {e}")))?,
            range,
            key: None,
        })
    };
    let rid = rep.id.clone().unwrap_or_default();
    let bw = rep.bandwidth.unwrap_or(0);
    let mut out = Vec::new();

    if let Some(t) = merge_templates([
        period.SegmentTemplate.as_ref(),
        adapt.SegmentTemplate.as_ref(),
        rep.SegmentTemplate.as_ref(),
    ])
    .filter(|t| t.media.is_some())
    {
        let media = t.media.clone().expect("filtered");
        let timescale = t.timescale.unwrap_or(1).max(1);
        let start_number = t.startNumber.unwrap_or(1);
        let init = t
            .initialization
            .clone()
            .or_else(|| t.Initialization.as_ref().and_then(|i| i.sourceURL.clone()));
        if let Some(init) = init {
            out.push(seg(
                &fill_template(&init, &rid, start_number, 0, bw)?,
                None,
            )?);
        }
        if let Some(tl) = &t.SegmentTimeline {
            let end_time = period_secs.map(|s| (s * timescale as f64).round() as u64);
            let mut time = 0u64;
            let mut number = start_number;
            for s in &tl.segments {
                if let Some(st) = s.t {
                    time = st;
                }
                if s.d == 0 {
                    return Err(dash_err("SegmentTimeline entry with zero duration"));
                }
                let repeat: u64 = match s.r {
                    Some(r) if r >= 0 => r as u64,
                    // r = -1: repeat until the end of the period.
                    Some(_) => {
                        let end = end_time.ok_or_else(|| {
                            dash_err("open-ended SegmentTimeline without a duration")
                        })?;
                        end.saturating_sub(time).div_ceil(s.d).saturating_sub(1)
                    }
                    None => 0,
                };
                for _ in 0..=repeat {
                    out.push(seg(&fill_template(&media, &rid, number, time, bw)?, None)?);
                    time += s.d;
                    number += 1;
                    if out.len() as u64 > MAX_SEGMENTS {
                        return Err(unsupported("more than 200000 segments"));
                    }
                }
            }
        } else if let Some(d) = t.duration.filter(|d| *d > 0.0) {
            let count = match t.endNumber {
                Some(end) if end >= start_number => end - start_number + 1,
                _ => {
                    let secs = period_secs.ok_or_else(|| {
                        dash_err("SegmentTemplate@duration without a period duration")
                    })?;
                    (secs * timescale as f64 / d).ceil() as u64
                }
            };
            if count > MAX_SEGMENTS {
                return Err(unsupported("more than 200000 segments"));
            }
            for n in 0..count {
                let time = (n as f64 * d) as u64;
                out.push(seg(
                    &fill_template(&media, &rid, start_number + n, time, bw)?,
                    None,
                )?);
            }
        } else {
            return Err(unsupported(
                "SegmentTemplate without @duration or SegmentTimeline",
            ));
        }
        return Ok(out);
    }

    if let Some(list) = rep
        .SegmentList
        .as_ref()
        .or(adapt.SegmentList.as_ref())
        .or(period.SegmentList.as_ref())
    {
        if let Some(init) = &list.Initialization {
            let url = init.sourceURL.as_deref().unwrap_or("");
            out.push(seg(url, init.range.as_deref().and_then(parse_byte_range))?);
        }
        for su in &list.segment_urls {
            out.push(seg(
                su.media.as_deref().unwrap_or(""),
                su.mediaRange.as_deref().and_then(parse_byte_range),
            )?);
        }
        if out.is_empty() {
            return Err(dash_err("empty SegmentList"));
        }
        return Ok(out);
    }

    // SegmentBase or a bare BaseURL: the representation is a single file.
    Ok(vec![MediaSegment {
        url: base,
        range: None,
        key: None,
    }])
}

fn is_protected(a: &AdaptationSet, r: &Representation) -> bool {
    !a.ContentProtection.is_empty() || !r.ContentProtection.is_empty()
}

fn select_video<'a>(
    reps: &[(&'a AdaptationSet, &'a Representation)],
    sel: VariantSelection,
) -> Result<usize, EngineError> {
    let bw = |i: &usize| reps[*i].1.bandwidth.unwrap_or(0);
    let height = |i: usize| reps[i].1.height.or(reps[i].0.height);
    let idx: Vec<usize> = (0..reps.len()).collect();
    match sel {
        VariantSelection::Best => idx
            .iter()
            .copied()
            .max_by_key(bw)
            .ok_or_else(|| dash_err("no video representation")),
        VariantSelection::MaxHeight(h) => idx
            .iter()
            .copied()
            .filter(|i| height(*i).is_some_and(|x| x <= u64::from(h)))
            .max_by_key(bw)
            .or_else(|| idx.iter().copied().min_by_key(bw))
            .ok_or_else(|| dash_err("no video representation")),
        VariantSelection::Index(i) if i < reps.len() => Ok(i),
        VariantSelection::Index(i) => Err(dash_err(format!(
            "representation {i} does not exist ({} available)",
            reps.len()
        ))),
    }
}

/// Lists every video representation an MPD declares, in the same order `select_video`/
/// `VariantSelection::Index` uses. The URL is the manifest URL itself for every entry
/// (DASH representations are selected by index within the one manifest, not a separate
/// per-quality URL the way HLS variants are) — display/debug only, not needed for download.
pub(crate) fn list_variants(text: &str, url: &Url) -> Result<Vec<super::Variant>, EngineError> {
    let mpd = dash_mpd_core::parse(text).map_err(|e| dash_err(format!("invalid MPD: {e}")))?;
    if mpd.mpdtype.as_deref() == Some("dynamic") {
        return Err(unsupported("live DASH streams"));
    }
    let period = match mpd.periods.as_slice() {
        [p] => p,
        [] => return Err(dash_err("manifest has no period")),
        _ => return Err(unsupported("multi-period DASH manifests")),
    };
    let video: Vec<(&AdaptationSet, &Representation)> = period
        .adaptations
        .iter()
        .filter(dash_mpd_core::is_video_adaptation)
        .flat_map(|a| a.representations.iter().map(move |r| (a, r)))
        .collect();
    if video.is_empty() {
        return Err(unsupported("DASH manifest without video representations"));
    }
    Ok(video
        .iter()
        .map(|(a, r)| super::Variant {
            url: url.clone(),
            bandwidth: r.bandwidth,
            width: r.width.or(a.width),
            height: r.height.or(a.height),
        })
        .collect())
}

/// Resolves a parsed MPD into a plan (exposed for unit tests).
pub(crate) fn plan_from_mpd(
    mpd: &MPD,
    url: &Url,
    sel: VariantSelection,
) -> Result<MediaPlan, EngineError> {
    if mpd.mpdtype.as_deref() == Some("dynamic") {
        return Err(unsupported("live DASH streams"));
    }
    let period = match mpd.periods.as_slice() {
        [p] => p,
        [] => return Err(dash_err("manifest has no period")),
        _ => return Err(unsupported("multi-period DASH manifests")),
    };
    let period_secs = period
        .duration
        .or(mpd.mediaPresentationDuration)
        .map(|d| d.as_secs_f64());
    let base = join(&join(url, mpd.base_url.first())?, period.BaseURL.first())?;
    let video: Vec<(&AdaptationSet, &Representation)> = period
        .adaptations
        .iter()
        .filter(dash_mpd_core::is_video_adaptation)
        .flat_map(|a| a.representations.iter().map(move |r| (a, r)))
        .collect();
    let audio: Vec<(&AdaptationSet, &Representation)> = period
        .adaptations
        .iter()
        .filter(dash_mpd_core::is_audio_adaptation)
        .flat_map(|a| a.representations.iter().map(move |r| (a, r)))
        .collect();
    let mut tracks = Vec::new();
    if !video.is_empty() {
        let (a, r) = video[select_video(&video, sel)?];
        if is_protected(a, r) {
            return Err(unsupported("DRM-protected DASH (ContentProtection)"));
        }
        tracks.push(Track {
            id: r.id.clone().unwrap_or_else(|| "video".into()),
            ext: "mp4",
            segments: representation_segments(period, a, r, &base, period_secs)?,
        });
    }
    // Best audio: highest bandwidth (first adaptation set wins ties).
    if let Some(&(a, r)) = audio
        .iter()
        .rev()
        .max_by_key(|(_, r)| r.bandwidth.unwrap_or(0))
    {
        if is_protected(a, r) {
            return Err(unsupported("DRM-protected DASH (ContentProtection)"));
        }
        tracks.push(Track {
            id: r.id.clone().unwrap_or_else(|| "audio".into()),
            ext: "m4a",
            segments: representation_segments(period, a, r, &base, period_secs)?,
        });
    }
    if tracks.is_empty() {
        return Err(unsupported(
            "DASH manifest without audio or video representations",
        ));
    }
    Ok(MediaPlan {
        kind: MediaKind::Dash,
        tracks,
        name: name_from_manifest(url),
    })
}

/// Resolves a DASH manifest into a plan.
pub(crate) async fn plan(
    _client: &reqwest::Client,
    _headers: &HeaderMap,
    url: &Url,
    text: &str,
    sel: VariantSelection,
) -> Result<MediaPlan, EngineError> {
    let mpd = dash_mpd_core::parse(text).map_err(|e| dash_err(format!("invalid MPD: {e}")))?;
    plan_from_mpd(&mpd, url, sel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates() {
        assert_eq!(
            fill_template("$RepresentationID$/seg-$Number%05d$.m4s", "v1", 7, 0, 0).unwrap(),
            "v1/seg-00007.m4s"
        );
        assert_eq!(
            fill_template("t$Time$-b$Bandwidth$-$$", "x", 1, 9000, 500).unwrap(),
            "t9000-b500-$"
        );
        assert_eq!(fill_template("n$Number%d$", "x", 12, 0, 0).unwrap(), "n12");
        assert!(fill_template("$Number", "x", 1, 0, 0).is_err());
        assert!(matches!(
            fill_template("$SubNumber$", "x", 1, 0, 0),
            Err(EngineError::Unsupported { .. })
        ));
        assert_eq!(parse_byte_range("0-99"), Some((0, 100)));
        assert_eq!(parse_byte_range("9-1"), None);
    }

    const MPD_TEMPLATE: &str = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT9S" minBufferTime="PT2S">
 <BaseURL>media/</BaseURL>
 <Period id="0">
  <AdaptationSet contentType="video" mimeType="video/mp4">
   <SegmentTemplate timescale="1000" duration="2000" initialization="$RepresentationID$/init.mp4" media="$RepresentationID$/$Number%03d$.m4s" startNumber="1"/>
   <Representation id="lo" bandwidth="100000" width="640" height="360"/>
   <Representation id="hi" bandwidth="900000" width="1920" height="1080"/>
  </AdaptationSet>
  <AdaptationSet contentType="audio" mimeType="audio/mp4">
   <SegmentTemplate timescale="48000" initialization="a/init.mp4" media="a/$Time$.m4s">
    <SegmentTimeline><S t="0" d="96000" r="2"/><S d="48000"/></SegmentTimeline>
   </SegmentTemplate>
   <Representation id="aud" bandwidth="128000"/>
  </AdaptationSet>
 </Period>
</MPD>"#;

    #[test]
    fn template_manifest() {
        let mpd = dash_mpd_core::parse(MPD_TEMPLATE).unwrap();
        let url = Url::parse("http://h/x/manifest.mpd").unwrap();
        let plan = plan_from_mpd(&mpd, &url, VariantSelection::Best).unwrap();
        assert_eq!(plan.tracks.len(), 2);
        let v = &plan.tracks[0];
        assert_eq!(v.id, "hi");
        // init + ceil(9 / 2) = 5 segments
        assert_eq!(v.segments.len(), 6);
        assert_eq!(v.segments[0].url.as_str(), "http://h/x/media/hi/init.mp4");
        assert_eq!(v.segments[1].url.as_str(), "http://h/x/media/hi/001.m4s");
        assert_eq!(v.segments[5].url.as_str(), "http://h/x/media/hi/005.m4s");
        let a = &plan.tracks[1];
        assert_eq!(a.ext, "m4a");
        let urls: Vec<&str> = a.segments.iter().map(|s| s.url.as_str()).collect();
        assert_eq!(
            urls,
            vec![
                "http://h/x/media/a/init.mp4",
                "http://h/x/media/a/0.m4s",
                "http://h/x/media/a/96000.m4s",
                "http://h/x/media/a/192000.m4s",
                "http://h/x/media/a/288000.m4s"
            ]
        );
        let low = plan_from_mpd(&mpd, &url, VariantSelection::MaxHeight(480)).unwrap();
        assert_eq!(low.tracks[0].id, "lo");
    }

    #[test]
    fn list_variants_reports_every_video_representation() {
        let url = Url::parse("http://h/x/manifest.mpd").unwrap();
        let variants = list_variants(MPD_TEMPLATE, &url).unwrap();
        // Video representations only (audio is excluded, same as `select_video`), in
        // manifest order ("lo" before "hi") — the order `VariantSelection::Index` relies on.
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0].bandwidth, Some(100_000));
        assert_eq!(variants[0].width, Some(640));
        assert_eq!(variants[0].height, Some(360));
        assert_eq!(variants[1].bandwidth, Some(900_000));
        assert_eq!(variants[1].height, Some(1080));
    }

    #[test]
    fn segment_list_base_and_unsupported() {
        let list = r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT4S">
 <Period><AdaptationSet mimeType="video/mp4">
  <Representation id="v" bandwidth="1">
   <SegmentList><Initialization sourceURL="v.mp4" range="0-99"/><SegmentURL media="v.mp4" mediaRange="100-199"/><SegmentURL media="v.mp4" mediaRange="200-299"/></SegmentList>
  </Representation></AdaptationSet>
  <AdaptationSet mimeType="audio/mp4"><Representation id="a" bandwidth="1"><BaseURL>audio.m4a</BaseURL><SegmentBase indexRange="0-10"/></Representation></AdaptationSet>
 </Period></MPD>"#;
        let mpd = dash_mpd_core::parse(list).unwrap();
        let url = Url::parse("http://h/m.mpd").unwrap();
        let plan = plan_from_mpd(&mpd, &url, VariantSelection::Best).unwrap();
        let v = &plan.tracks[0];
        assert_eq!(
            v.segments.iter().map(|s| s.range).collect::<Vec<_>>(),
            vec![Some((0, 100)), Some((100, 200)), Some((200, 300))]
        );
        assert_eq!(plan.tracks[1].segments.len(), 1);
        assert_eq!(
            plan.tracks[1].segments[0].url.as_str(),
            "http://h/audio.m4a"
        );

        let live = MPD_TEMPLATE.replace("type=\"static\"", "type=\"dynamic\"");
        assert!(matches!(
            plan_from_mpd(
                &dash_mpd_core::parse(&live).unwrap(),
                &url,
                VariantSelection::Best
            ),
            Err(EngineError::Unsupported { .. })
        ));
        let drm = MPD_TEMPLATE.replace(
            "<Representation id=\"hi\"",
            "<ContentProtection schemeIdUri=\"urn:mpeg:dash:mp4protection:2011\" value=\"cenc\"/><Representation id=\"hi\"",
        );
        assert!(matches!(
            plan_from_mpd(
                &dash_mpd_core::parse(&drm).unwrap(),
                &url,
                VariantSelection::Best
            ),
            Err(EngineError::Unsupported { .. })
        ));
    }
}
