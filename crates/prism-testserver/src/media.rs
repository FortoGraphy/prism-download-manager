//! HLS and DASH fixtures: playlists/manifests plus segment payloads served from memory.

use aes::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use bytes::Bytes;

use crate::content;

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

/// One HLS variant stream (rendition of the video at a given bitrate).
#[derive(Debug, Clone)]
pub struct HlsVariant {
    /// `BANDWIDTH` attribute.
    pub bandwidth: u64,
    /// `RESOLUTION` attribute (width, height).
    pub resolution: Option<(u32, u32)>,
    /// Plaintext segment payloads.
    pub segments: Vec<Bytes>,
    /// Optional `EXT-X-MAP` initialization section (plaintext; see `HlsFixture::encrypt_init`).
    pub init: Option<Bytes>,
}

/// An HLS fixture served under `/hls/`:
///
/// * `/hls/master.m3u8` — master playlist (variants + optional `AUDIO` group `aud`)
/// * `/hls/v{i}/media.m3u8`, `/hls/v{i}/seg{n}.ts`, `/hls/v{i}/all.ts` (byte-range mode),
///   `/hls/v{i}/init.mp4`
/// * `/hls/audio/media.m3u8`, `/hls/audio/seg{n}.aac`
/// * `/hls/key.bin` — the AES-128 key
#[derive(Debug, Clone)]
pub struct HlsFixture {
    /// Variant streams.
    pub variants: Vec<HlsVariant>,
    /// Optional separate audio rendition (plaintext segments), referenced as `AUDIO="aud"`.
    pub audio: Option<Vec<Bytes>>,
    /// `EXTINF` duration of each segment.
    pub segment_duration: f32,
    /// `EXT-X-MEDIA-SEQUENCE`.
    pub media_sequence: u64,
    /// AES-128 key; `None` = unencrypted.
    pub key: Option<[u8; 16]>,
    /// Explicit IV; `None` = IV derived from the media sequence number.
    pub explicit_iv: Option<[u8; 16]>,
    /// Serve segments as `EXT-X-BYTERANGE` slices of a single `all.ts` resource.
    pub byte_ranges: bool,
    /// With a key: put `EXT-X-KEY` before `EXT-X-MAP` and serve the init section encrypted
    /// (IV as for the first segment). Otherwise the MAP comes first and stays plain.
    pub encrypt_init: bool,
}

impl HlsFixture {
    /// Builds a fixture with `variants` variants of `segments` synthetic segments each.
    /// Variant `i` has bandwidth `(i + 1) * 1_000_000` and height `(i + 1) * 360`.
    pub fn synthetic(variants: usize, segments: usize, segment_size: usize, seed: u64) -> Self {
        let variants = (0..variants)
            .map(|v| HlsVariant {
                bandwidth: (v as u64 + 1) * 1_000_000,
                resolution: Some((((v as u32) + 1) * 640, ((v as u32) + 1) * 360)),
                segments: (0..segments)
                    .map(|n| synthetic_segment(seed, v as u64 * 10_000 + n as u64, segment_size))
                    .collect(),
                init: None,
            })
            .collect();
        Self {
            variants,
            audio: None,
            segment_duration: 2.0,
            media_sequence: 0,
            key: None,
            explicit_iv: None,
            byte_ranges: false,
            encrypt_init: false,
        }
    }

    /// The plaintext bytes a client should produce for variant `index` (init + segments).
    pub fn expected_variant_bytes(&self, index: usize) -> Vec<u8> {
        let v = &self.variants[index];
        let mut out = Vec::new();
        if let Some(init) = &v.init {
            out.extend_from_slice(init);
        }
        for s in &v.segments {
            out.extend_from_slice(s);
        }
        out
    }

    /// The plaintext bytes of the audio rendition.
    pub fn expected_audio_bytes(&self) -> Vec<u8> {
        self.audio
            .iter()
            .flatten()
            .flat_map(|b| b.iter().copied())
            .collect()
    }

    fn iv_for(&self, n: usize) -> [u8; 16] {
        self.explicit_iv
            .unwrap_or_else(|| ((self.media_sequence + n as u64) as u128).to_be_bytes())
    }

    /// Segment payload as served (encrypted when a key is configured).
    pub(crate) fn served_segment(&self, payload: &[u8], n: usize) -> Bytes {
        match self.key {
            Some(key) => {
                let iv = self.iv_for(n);
                let ct = Aes128CbcEnc::new(&key.into(), &iv.into())
                    .encrypt_padded_vec_mut::<Pkcs7>(payload);
                Bytes::from(ct)
            }
            None => Bytes::copy_from_slice(payload),
        }
    }

    /// Init section as served (encrypted when a key and `encrypt_init` are set).
    pub(crate) fn served_init(&self, init: &Bytes) -> Bytes {
        if self.encrypt_init {
            self.served_segment(init, 0)
        } else {
            init.clone()
        }
    }

    /// Master playlist text.
    pub(crate) fn master_playlist(&self) -> String {
        let mut s = String::from("#EXTM3U\n#EXT-X-VERSION:4\n");
        if self.audio.is_some() {
            s.push_str(
                "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,AUTOSELECT=YES,URI=\"audio/media.m3u8\"\n",
            );
        }
        for (i, v) in self.variants.iter().enumerate() {
            s.push_str(&format!("#EXT-X-STREAM-INF:BANDWIDTH={}", v.bandwidth));
            if let Some((w, h)) = v.resolution {
                s.push_str(&format!(",RESOLUTION={w}x{h}"));
            }
            if self.audio.is_some() {
                s.push_str(",AUDIO=\"aud\"");
            }
            s.push_str(&format!("\nv{i}/media.m3u8\n"));
        }
        s
    }

    /// Media playlist for a list of plaintext segments.
    pub(crate) fn media_playlist(&self, segments: &[Bytes], init: bool, ext: &str) -> String {
        let mut s = format!(
            "#EXTM3U\n#EXT-X-VERSION:4\n#EXT-X-TARGETDURATION:{}\n#EXT-X-MEDIA-SEQUENCE:{}\n#EXT-X-PLAYLIST-TYPE:VOD\n",
            self.segment_duration.ceil() as u64,
            self.media_sequence
        );
        const MAP: &str = "#EXT-X-MAP:URI=\"init.mp4\"\n";
        if init && !self.encrypt_init {
            s.push_str(MAP);
        }
        if self.key.is_some() {
            s.push_str("#EXT-X-KEY:METHOD=AES-128,URI=\"../key.bin\"");
            if let Some(iv) = self.explicit_iv {
                s.push_str(&format!(",IV=0x{}", content::hex(&iv)));
            }
            s.push('\n');
        }
        if init && self.encrypt_init {
            s.push_str(MAP);
        }
        let mut offset = 0u64;
        for (n, seg) in segments.iter().enumerate() {
            s.push_str(&format!("#EXTINF:{:.3},\n", self.segment_duration));
            if self.byte_ranges {
                let len = self.served_segment(seg, n).len() as u64;
                s.push_str(&format!("#EXT-X-BYTERANGE:{len}@{offset}\nall.{ext}\n"));
                offset += len;
            } else {
                s.push_str(&format!("seg{n}.{ext}\n"));
            }
        }
        s.push_str("#EXT-X-ENDLIST\n");
        s
    }

    /// Concatenation of all served (possibly encrypted) segments, for byte-range mode.
    pub(crate) fn served_all(&self, segments: &[Bytes]) -> Bytes {
        let mut out = Vec::new();
        for (n, seg) in segments.iter().enumerate() {
            out.extend_from_slice(&self.served_segment(seg, n));
        }
        Bytes::from(out)
    }
}

/// One DASH representation.
#[derive(Debug, Clone)]
pub struct DashRepresentation {
    /// `id` attribute (used in `$RepresentationID$`).
    pub id: String,
    /// `bandwidth` attribute.
    pub bandwidth: u64,
    /// Video width.
    pub width: Option<u32>,
    /// Video height.
    pub height: Option<u32>,
    /// Initialization segment payload.
    pub init: Bytes,
    /// Media segment payloads.
    pub segments: Vec<Bytes>,
}

/// A static (VOD) DASH fixture served under `/dash/`:
/// `/dash/manifest.mpd`, `/dash/{rep}/init.mp4`, `/dash/{rep}/seg-{n:05}.m4s` (numbers start at 1).
#[derive(Debug, Clone)]
pub struct DashFixture {
    /// Video representations (one adaptation set).
    pub video: Vec<DashRepresentation>,
    /// Audio representations (one adaptation set).
    pub audio: Vec<DashRepresentation>,
    /// Segment duration in seconds.
    pub segment_duration: u32,
    /// Use `SegmentTimeline` instead of `@duration` addressing.
    pub timeline: bool,
}

impl DashFixture {
    /// Synthetic fixture: `video` video representations and one audio representation.
    pub fn synthetic(video: usize, segments: usize, segment_size: usize, seed: u64) -> Self {
        let rep = |id: String, bw: u64, h: Option<u32>, salt: u64| DashRepresentation {
            id,
            bandwidth: bw,
            width: h.map(|h| h * 16 / 9),
            height: h,
            init: synthetic_segment(seed, salt * 100_000 + 99_999, 512),
            segments: (0..segments)
                .map(|n| synthetic_segment(seed, salt * 100_000 + n as u64, segment_size))
                .collect(),
        };
        Self {
            video: (0..video)
                .map(|i| {
                    rep(
                        format!("v{i}"),
                        (i as u64 + 1) * 1_000_000,
                        Some((i as u32 + 1) * 360),
                        i as u64 + 1,
                    )
                })
                .collect(),
            audio: vec![rep("a0".into(), 128_000, None, 50)],
            segment_duration: 2,
            timeline: false,
        }
    }

    /// Plaintext bytes (init + segments) of a representation.
    pub fn expected_bytes(rep: &DashRepresentation) -> Vec<u8> {
        let mut out = rep.init.to_vec();
        for s in &rep.segments {
            out.extend_from_slice(s);
        }
        out
    }

    pub(crate) fn find(&self, id: &str) -> Option<&DashRepresentation> {
        self.video
            .iter()
            .chain(self.audio.iter())
            .find(|r| r.id == id)
    }

    /// The MPD document.
    pub(crate) fn manifest(&self) -> String {
        let count = self
            .video
            .first()
            .or(self.audio.first())
            .map_or(0, |r| r.segments.len());
        let total = count as u32 * self.segment_duration;
        let template = |count: usize| {
            if self.timeline {
                format!(
                    "<SegmentTemplate timescale=\"1000\" initialization=\"$RepresentationID$/init.mp4\" media=\"$RepresentationID$/seg-$Number%05d$.m4s\" startNumber=\"1\"><SegmentTimeline><S t=\"0\" d=\"{}\" r=\"{}\"/></SegmentTimeline></SegmentTemplate>",
                    self.segment_duration * 1000,
                    count.saturating_sub(1)
                )
            } else {
                format!(
                    "<SegmentTemplate timescale=\"1000\" duration=\"{}\" initialization=\"$RepresentationID$/init.mp4\" media=\"$RepresentationID$/seg-$Number%05d$.m4s\" startNumber=\"1\"/>",
                    self.segment_duration * 1000
                )
            }
        };
        let mut s = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" profiles=\"urn:mpeg:dash:profile:isoff-live:2011\" mediaPresentationDuration=\"PT{total}S\" minBufferTime=\"PT2S\">\n<Period id=\"0\" start=\"PT0S\">\n"
        );
        if !self.video.is_empty() {
            s.push_str("<AdaptationSet contentType=\"video\" mimeType=\"video/mp4\" segmentAlignment=\"true\">\n");
            s.push_str(&template(count));
            for r in &self.video {
                s.push_str(&format!(
                    "<Representation id=\"{}\" bandwidth=\"{}\" codecs=\"avc1.64001f\" width=\"{}\" height=\"{}\"/>\n",
                    r.id,
                    r.bandwidth,
                    r.width.unwrap_or(0),
                    r.height.unwrap_or(0)
                ));
            }
            s.push_str("</AdaptationSet>\n");
        }
        if !self.audio.is_empty() {
            s.push_str(
                "<AdaptationSet contentType=\"audio\" mimeType=\"audio/mp4\" lang=\"en\">\n",
            );
            s.push_str(&template(count));
            for r in &self.audio {
                s.push_str(&format!(
                    "<Representation id=\"{}\" bandwidth=\"{}\" codecs=\"mp4a.40.2\" audioSamplingRate=\"48000\"/>\n",
                    r.id, r.bandwidth
                ));
            }
            s.push_str("</AdaptationSet>\n");
        }
        s.push_str("</Period>\n</MPD>\n");
        s
    }
}

fn synthetic_segment(seed: u64, salt: u64, size: usize) -> Bytes {
    let mut v = vec![0u8; size];
    content::fill(content::versioned_seed(seed, salt + 1), 0, &mut v);
    Bytes::from(v)
}
