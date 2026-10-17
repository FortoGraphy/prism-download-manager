//! HLS: master/media playlist resolution (variant + audio rendition selection, AES-128 keys,
//! `EXT-X-MAP`, byte ranges) and AES-128-CBC segment decryption.

use std::collections::HashMap;

use aes::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
use m3u8_rs::{
    AlternativeMedia, AlternativeMediaType, KeyMethod, MediaPlaylist, Playlist, VariantStream,
};
use reqwest::header::HeaderMap;
use url::Url;

use super::{KeyRef, MediaPlan, MediaSegment, Track, TrackKind, fetch_text, name_from_manifest};
use crate::error::EngineError;
use crate::request::{MediaKind, VariantSelection};

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

fn hls_err(m: impl Into<String>) -> EngineError {
    EngineError::Hls { message: m.into() }
}

/// Decrypts an AES-128-CBC (PKCS#7) segment.
pub(crate) fn decrypt_aes128(
    key: &[u8; 16],
    iv: &[u8; 16],
    mut data: Vec<u8>,
) -> Result<Vec<u8>, EngineError> {
    let len = Aes128CbcDec::new(key.into(), iv.into())
        .decrypt_padded_mut::<Pkcs7>(&mut data)
        .map_err(|_| hls_err("AES-128 decryption failed (wrong key or corrupt segment)"))?
        .len();
    data.truncate(len);
    Ok(data)
}

/// Parses an `IV=0x…` attribute (up to 32 hex digits, left-padded with zeros).
pub(crate) fn parse_iv(s: &str) -> Option<[u8; 16]> {
    let hex = s
        .trim()
        .strip_prefix("0x")
        .or_else(|| s.trim().strip_prefix("0X"))?;
    if hex.is_empty() || hex.len() > 32 {
        return None;
    }
    u128::from_str_radix(hex, 16).ok().map(u128::to_be_bytes)
}

/// IV derived from the media sequence number (RFC 8216 §5.2).
fn sequence_iv(seq: u64) -> [u8; 16] {
    u128::from(seq).to_be_bytes()
}

/// Picks a variant index according to `sel` (variants in manifest order).
pub(crate) fn choose_variant(
    variants: &[&VariantStream],
    sel: VariantSelection,
) -> Result<usize, EngineError> {
    if variants.is_empty() {
        return Err(hls_err("master playlist has no variants"));
    }
    let best = |it: &mut dyn Iterator<Item = (usize, &&VariantStream)>| {
        it.max_by_key(|(_, v)| v.bandwidth).map(|(i, _)| i)
    };
    match sel {
        VariantSelection::Best => Ok(best(&mut variants.iter().enumerate()).expect("non-empty")),
        VariantSelection::MaxHeight(h) => {
            let fitting = best(&mut variants.iter().enumerate().filter(|(_, v)| {
                v.resolution
                    .as_ref()
                    .is_some_and(|r| r.height <= u64::from(h))
            }));
            Ok(fitting.unwrap_or_else(|| {
                variants
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| v.bandwidth)
                    .map(|(i, _)| i)
                    .expect("non-empty")
            }))
        }
        VariantSelection::Index(i) if i < variants.len() => Ok(i),
        VariantSelection::Index(i) => Err(hls_err(format!(
            "variant {i} does not exist ({} available)",
            variants.len()
        ))),
    }
}

/// Lists every variant a master playlist declares, in manifest order — the position of each
/// entry in the returned `Vec` is exactly what `VariantSelection::Index` expects. A plain
/// media playlist (no variants) returns a single entry standing for the stream itself, so
/// single-rendition streams keep behaving exactly as before.
pub(crate) fn list_variants(text: &str, base_url: &Url) -> Result<Vec<super::Variant>, EngineError> {
    Ok(match parse(text)? {
        Playlist::MediaPlaylist(_) => vec![super::Variant {
            url: base_url.clone(),
            bandwidth: None,
            width: None,
            height: None,
        }],
        Playlist::MasterPlaylist(master) => master
            .variants
            .iter()
            .filter(|v| !v.is_i_frame)
            .map(|v| super::Variant {
                url: base_url.join(&v.uri).unwrap_or_else(|_| base_url.clone()),
                bandwidth: Some(v.bandwidth),
                width: v.resolution.as_ref().map(|r| r.width),
                height: v.resolution.as_ref().map(|r| r.height),
            })
            .collect(),
    })
}

/// The audio rendition of `group` (default, then autoselect, then first) that has its own URI.
fn choose_audio<'a>(alts: &'a [AlternativeMedia], group: &str) -> Option<&'a AlternativeMedia> {
    let mut candidates: Vec<&AlternativeMedia> = alts
        .iter()
        .filter(|a| {
            a.media_type == AlternativeMediaType::Audio && a.group_id == group && a.uri.is_some()
        })
        .collect();
    candidates.sort_by_key(|a| (!a.default, !a.autoselect));
    candidates.first().copied()
}

fn extension_for(url: &Url, fmp4: bool, kind: TrackKind) -> &'static str {
    let path = url.path().to_ascii_lowercase();
    match () {
        _ if fmp4 => {
            if kind == TrackKind::Audio {
                "m4a"
            } else {
                "mp4"
            }
        }
        _ if path.ends_with(".aac") => "aac",
        _ if path.ends_with(".mp3") => "mp3",
        _ => "ts",
    }
}

/// Converts a media playlist into a track.
pub(crate) fn build_track(
    id: String,
    kind: TrackKind,
    media: &MediaPlaylist,
    base: &Url,
) -> Result<Track, EngineError> {
    if !media.end_list {
        return Err(EngineError::Unsupported {
            message: "live HLS streams (no EXT-X-ENDLIST)".into(),
        });
    }
    let join = |u: &str| {
        base.join(u)
            .map_err(|e| hls_err(format!("bad URI {u:?}: {e}")))
    };
    let mut key: Option<(Url, Option<[u8; 16]>)> = None;
    let mut map: Option<(Url, Option<(u64, u64)>)> = None;
    let mut last_end: HashMap<Url, u64> = HashMap::new();
    let mut segments = Vec::with_capacity(media.segments.len() + 1);
    for (i, s) in media.segments.iter().enumerate() {
        // m3u8-rs rejects `METHOD=NONE` without an IV (inverted check) and keeps such a tag as
        // an unknown tag; honour it here so encryption switches off as the playlist says.
        if s.unknown_tags.iter().any(|t| {
            t.tag == "X-KEY"
                && t.rest
                    .as_deref()
                    .is_some_and(|r| r.to_ascii_uppercase().contains("METHOD=NONE"))
        }) {
            key = None;
        }
        if let Some(k) = &s.key {
            key = match &k.method {
                KeyMethod::None => None,
                KeyMethod::AES128 => {
                    let uri = k
                        .uri
                        .as_deref()
                        .ok_or_else(|| hls_err("AES-128 key without URI"))?;
                    let iv = match &k.iv {
                        Some(v) => {
                            Some(parse_iv(v).ok_or_else(|| hls_err(format!("bad IV {v:?}")))?)
                        }
                        None => None,
                    };
                    Some((join(uri)?, iv))
                }
                KeyMethod::SampleAES => {
                    return Err(EngineError::Unsupported {
                        message: "SAMPLE-AES encrypted HLS (DRM)".into(),
                    });
                }
                KeyMethod::Other(m) => {
                    return Err(EngineError::Unsupported {
                        message: format!("HLS encryption method {m}"),
                    });
                }
            };
        }
        if let Some(m) = &s.map {
            let url = join(&m.uri)?;
            let range = m.byte_range.as_ref().map(|r| {
                let start = r.offset.unwrap_or(0);
                (start, start + r.length)
            });
            if map.as_ref() != Some(&(url.clone(), range)) {
                map = Some((url.clone(), range));
                // Initialization sections are stored unencrypted (see the crate README).
                segments.push(MediaSegment {
                    url,
                    range,
                    key: None,
                });
            }
        }
        let url = join(&s.uri)?;
        let range = s.byte_range.as_ref().map(|r| {
            let start = r
                .offset
                .unwrap_or_else(|| last_end.get(&url).copied().unwrap_or(0));
            (start, start + r.length)
        });
        if let Some((_, end)) = range {
            last_end.insert(url.clone(), end);
        }
        let key = key.as_ref().map(|(uri, iv)| KeyRef {
            uri: uri.clone(),
            iv: iv.unwrap_or_else(|| sequence_iv(media.media_sequence + i as u64)),
        });
        segments.push(MediaSegment { url, range, key });
    }
    let first = segments
        .first()
        .map(|s| s.url.clone())
        .unwrap_or_else(|| base.clone());
    Ok(Track {
        id,
        ext: extension_for(&first, map.is_some(), kind),
        segments,
    })
}

fn parse(text: &str) -> Result<Playlist, EngineError> {
    m3u8_rs::parse_playlist_res(text.as_bytes()).map_err(|_| hls_err("invalid playlist"))
}

fn media_of(text: &str) -> Result<MediaPlaylist, EngineError> {
    match parse(text)? {
        Playlist::MediaPlaylist(m) => Ok(m),
        Playlist::MasterPlaylist(_) => Err(hls_err("nested master playlist")),
    }
}

/// Resolves an HLS URL (master or media playlist) into a plan.
pub(crate) async fn plan(
    client: &reqwest::Client,
    headers: &HeaderMap,
    url: &Url,
    text: &str,
    sel: VariantSelection,
) -> Result<MediaPlan, EngineError> {
    let tracks = match parse(text)? {
        Playlist::MediaPlaylist(m) => vec![build_track(url.to_string(), TrackKind::Main, &m, url)?],
        Playlist::MasterPlaylist(master) => {
            let variants: Vec<&VariantStream> =
                master.variants.iter().filter(|v| !v.is_i_frame).collect();
            let v = variants[choose_variant(&variants, sel)?];
            let vurl = url
                .join(&v.uri)
                .map_err(|e| hls_err(format!("bad variant URI: {e}")))?;
            let (vtext, vfinal) = fetch_text(client, headers, &vurl).await?;
            let mut tracks = vec![build_track(
                vurl.to_string(),
                TrackKind::Main,
                &media_of(&vtext)?,
                &vfinal,
            )?];
            if let Some(alt) = v
                .audio
                .as_deref()
                .and_then(|g| choose_audio(&master.alternatives, g))
            {
                let aurl = url
                    .join(alt.uri.as_deref().expect("filtered"))
                    .map_err(|e| hls_err(format!("bad audio URI: {e}")))?;
                let (atext, afinal) = fetch_text(client, headers, &aurl).await?;
                tracks.push(build_track(
                    aurl.to_string(),
                    TrackKind::Audio,
                    &media_of(&atext)?,
                    &afinal,
                )?);
            }
            tracks
        }
    };
    Ok(MediaPlan {
        kind: MediaKind::Hls,
        tracks,
        name: name_from_manifest(url),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockEncryptMut;

    #[test]
    fn iv_and_decryption() {
        assert_eq!(parse_iv("0x1").unwrap()[15], 1);
        assert_eq!(
            parse_iv("0X000102030405060708090a0b0c0d0e0f").unwrap()[1],
            1
        );
        assert!(parse_iv("123").is_none());
        assert!(parse_iv("0x").is_none());
        let key = [7u8; 16];
        let iv = sequence_iv(5);
        let plain = b"hello segment data that spans blocks".to_vec();
        let ct = cbc::Encryptor::<aes::Aes128>::new(&key.into(), &iv.into())
            .encrypt_padded_vec_mut::<Pkcs7>(&plain);
        assert_eq!(decrypt_aes128(&key, &iv, ct.clone()).unwrap(), plain);
        assert!(decrypt_aes128(&[0u8; 16], &iv, ct).is_err());
    }

    #[test]
    fn media_playlist_resolution() {
        let text = "#EXTM3U\n#EXT-X-VERSION:4\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:10\n\
            #EXT-X-MAP:URI=\"init.mp4\"\n\
            #EXT-X-KEY:METHOD=AES-128,URI=\"k.bin\"\n\
            #EXTINF:2,\n#EXT-X-BYTERANGE:100@0\nall.m4s\n\
            #EXTINF:2,\n#EXT-X-BYTERANGE:50\nall.m4s\n\
            #EXT-X-KEY:METHOD=NONE\n\
            #EXTINF:2,\nplain.m4s\n#EXT-X-ENDLIST\n";
        let base = Url::parse("http://h/v/media.m3u8").unwrap();
        let t = build_track(
            "id".into(),
            TrackKind::Main,
            &media_of(text).unwrap(),
            &base,
        )
        .unwrap();
        assert_eq!(t.ext, "mp4");
        assert_eq!(t.segments.len(), 4);
        assert_eq!(t.segments[0].url.as_str(), "http://h/v/init.mp4");
        assert!(t.segments[0].key.is_none());
        assert_eq!(t.segments[1].range, Some((0, 100)));
        assert_eq!(t.segments[2].range, Some((100, 150)));
        let k1 = t.segments[1].key.as_ref().unwrap();
        assert_eq!(k1.uri.as_str(), "http://h/v/k.bin");
        assert_eq!(k1.iv, sequence_iv(10));
        assert_eq!(t.segments[2].key.as_ref().unwrap().iv, sequence_iv(11));
        assert!(t.segments[3].key.is_none());

        let live = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\na.ts\n";
        assert!(matches!(
            build_track("l".into(), TrackKind::Main, &media_of(live).unwrap(), &base),
            Err(EngineError::Unsupported { .. })
        ));
        let sample = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"k\"\n#EXTINF:2,\na.ts\n#EXT-X-ENDLIST\n";
        assert!(matches!(
            build_track(
                "s".into(),
                TrackKind::Main,
                &media_of(sample).unwrap(),
                &base
            ),
            Err(EngineError::Unsupported { .. })
        ));
    }

    #[test]
    fn variant_and_audio_selection() {
        let text = "#EXTM3U\n\
            #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"de\",URI=\"de.m3u8\"\n\
            #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"en\",DEFAULT=YES,URI=\"en.m3u8\"\n\
            #EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360,AUDIO=\"a\"\nlow.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080,AUDIO=\"a\"\nhigh.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=2000000,RESOLUTION=1280x720,AUDIO=\"a\"\nmid.m3u8\n";
        let Playlist::MasterPlaylist(m) = parse(text).unwrap() else {
            panic!()
        };
        let v: Vec<&VariantStream> = m.variants.iter().collect();
        assert_eq!(choose_variant(&v, VariantSelection::Best).unwrap(), 1);
        assert_eq!(
            choose_variant(&v, VariantSelection::MaxHeight(720)).unwrap(),
            2
        );
        assert_eq!(
            choose_variant(&v, VariantSelection::MaxHeight(100)).unwrap(),
            0
        );
        assert_eq!(choose_variant(&v, VariantSelection::Index(0)).unwrap(), 0);
        assert!(choose_variant(&v, VariantSelection::Index(3)).is_err());
        assert_eq!(choose_audio(&m.alternatives, "a").unwrap().name, "en");
        assert!(choose_audio(&m.alternatives, "zzz").is_none());
    }

    #[test]
    fn list_variants_reports_every_rendition_in_manifest_order() {
        let text = "#EXTM3U\n\
            #EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\nlow.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080\nhigh.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=2000000,RESOLUTION=1280x720\nmid.m3u8\n";
        let base = Url::parse("https://example.com/master.m3u8").unwrap();
        let variants = list_variants(text, &base).unwrap();
        // Manifest order preserved (not sorted by quality) — index must match `choose_variant`.
        assert_eq!(variants.len(), 3);
        assert_eq!(variants[0].bandwidth, Some(800_000));
        assert_eq!(variants[0].width, Some(640));
        assert_eq!(variants[0].height, Some(360));
        assert_eq!(variants[0].url.as_str(), "https://example.com/low.m3u8");
        assert_eq!(variants[1].bandwidth, Some(5_000_000));
        assert_eq!(variants[1].height, Some(1080));
        assert_eq!(variants[2].height, Some(720));
    }

    #[test]
    fn list_variants_on_a_media_playlist_returns_one_entry_for_the_stream_itself() {
        let text = "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6,\nseg0.ts\n#EXT-X-ENDLIST\n";
        let base = Url::parse("https://example.com/stream.m3u8").unwrap();
        let variants = list_variants(text, &base).unwrap();
        assert_eq!(variants.len(), 1);
        assert_eq!(variants[0].url, base);
        assert_eq!(variants[0].bandwidth, None);
    }
}
