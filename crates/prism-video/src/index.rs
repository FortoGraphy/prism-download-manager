//! Where a part of a video sits in its file. A DASH stream (YouTube's video and audio formats)
//! starts with an index of its pieces — an MP4's `sidx` box, a WebM's `Cues` — with the time
//! and the bytes of each, so a part can be downloaded as just the pieces it covers, over many
//! connections, instead of having ffmpeg read it over one.
//!
//! The pieces start on keyframes. Put after the file's header ([`MediaIndex::init`]) they make
//! a playable file that starts a little before the part; ffmpeg then cuts the part out of it.

/// How much of a file is read first: the header and the index are almost always in it.
pub const HEAD: u64 = 64 * 1024;
/// The furthest into a file Prism reads to find the index.
pub const MAX_HEAD: u64 = 4 << 20;

/// One piece of the file: from a keyframe to the next piece.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// Seconds.
    pub start: f64,
    /// Seconds.
    pub end: f64,
    /// Bytes `[from, to)` of the file.
    pub from: u64,
    pub to: u64,
}

/// A file's header and its pieces, in order.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaIndex {
    /// What goes before the pieces to make them a playable file. MP4: the `ftyp` and `moov`
    /// boxes. WebM: the EBML header, the start of the `Segment` (its size made "unknown", as it
    /// now ends where the pieces do), `Info` and `Tracks`.
    pub init: Vec<u8>,
    pub pieces: Vec<Piece>,
}

/// The pieces covering a part.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    /// Bytes `[from, to)` of the file.
    pub from: u64,
    pub to: u64,
    /// Seconds at which the first piece starts.
    pub start: f64,
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum IndexError {
    /// The header or the index goes on past what was read: read the first `n` bytes.
    #[error("the index goes on past the bytes read (needs {0})")]
    NeedMore(u64),
    /// The file has no index at its start (a plain MP4, an index at the end, an odd file).
    #[error("no index at the start of the file: {0}")]
    NoIndex(&'static str),
}

/// Parses the start of a file (`head`, read from byte 0). `file_size` gives the end of a
/// WebM's last piece.
pub fn parse(head: &[u8], file_size: Option<u64>) -> Result<MediaIndex, IndexError> {
    if head.len() >= 8 && &head[4..8] == b"ftyp" {
        parse_mp4(head)
    } else if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        parse_webm(head, file_size)
    } else if head.len() < 8 {
        Err(IndexError::NeedMore(HEAD))
    } else {
        Err(IndexError::NoIndex("neither MP4 nor WebM"))
    }
}

impl MediaIndex {
    /// The pieces covering `start..end` seconds: from the piece the part starts in to the one
    /// it ends in.
    pub fn span(&self, start: f64, end: f64) -> Option<Span> {
        let p = &self.pieces;
        if p.is_empty() || !(start.is_finite() && end.is_finite()) || end <= start {
            return None;
        }
        // A part picked right on a piece's edge starts with that piece (float rounding aside).
        let first = p.iter().rposition(|x| x.start <= start + 1e-6).unwrap_or(0);
        let last = p
            .iter()
            .position(|x| x.end >= end - 1e-6)
            .unwrap_or(p.len() - 1)
            .max(first);
        (p[last].to > p[first].from).then_some(Span {
            from: p[first].from,
            to: p[last].to,
            start: p[first].start,
        })
    }
}

fn be(b: &[u8]) -> u64 {
    b.iter().fold(0u64, |v, x| (v << 8) | u64::from(*x))
}

// ---------------------------------------------------------------------------------------------
// MP4
// ---------------------------------------------------------------------------------------------

fn parse_mp4(b: &[u8]) -> Result<MediaIndex, IndexError> {
    let mut init = Vec::new();
    let mut moov = false;
    let mut pos = 0usize;
    loop {
        if pos + 8 > b.len() {
            return Err(IndexError::NeedMore(pos as u64 + 16));
        }
        let size32 = be(&b[pos..pos + 4]);
        let kind = &b[pos + 4..pos + 8];
        let (header, size) = match size32 {
            0 => return Err(IndexError::NoIndex("a box runs to the end of the file")),
            1 => {
                if pos + 16 > b.len() {
                    return Err(IndexError::NeedMore(pos as u64 + 16));
                }
                (16u64, be(&b[pos + 8..pos + 16]))
            }
            n => (8, n),
        };
        if size < header {
            return Err(IndexError::NoIndex("a broken box"));
        }
        let end = (pos as u64)
            .checked_add(size)
            .ok_or(IndexError::NoIndex("a broken box"))?;
        let whole = end <= b.len() as u64;
        match kind {
            b"ftyp" | b"moov" => {
                if !whole {
                    return Err(IndexError::NeedMore(end));
                }
                init.extend_from_slice(&b[pos..end as usize]);
                moov |= kind == b"moov";
            }
            b"sidx" => {
                if !whole {
                    return Err(IndexError::NeedMore(end));
                }
                if !moov {
                    return Err(IndexError::NoIndex("the index comes before the header"));
                }
                let pieces = parse_sidx(&b[pos + header as usize..end as usize], end)?;
                return Ok(MediaIndex { init, pieces });
            }
            b"moof" | b"mdat" => return Err(IndexError::NoIndex("no sidx before the media")),
            _ => {}
        }
        if !whole {
            // A box we skip, longer than what was read: the next one starts after it.
            let next = end
                .checked_add(16)
                .ok_or(IndexError::NoIndex("a broken box"))?;
            return Err(IndexError::NeedMore(next));
        }
        pos = end as usize;
    }
}

/// The entries of a `sidx` box (`d`: its contents after the box header; `end`: where the box
/// ends in the file, which the first piece's offset counts from).
fn parse_sidx(d: &[u8], end: u64) -> Result<Vec<Piece>, IndexError> {
    let short = IndexError::NoIndex("a short sidx");
    if d.len() < 12 {
        return Err(short);
    }
    let version = d[0];
    let timescale = be(&d[8..12]);
    let mut p = 12;
    let (ept, first) = if version == 0 {
        if d.len() < p + 8 {
            return Err(short);
        }
        let v = (be(&d[p..p + 4]), be(&d[p + 4..p + 8]));
        p += 8;
        v
    } else {
        if d.len() < p + 16 {
            return Err(short);
        }
        let v = (be(&d[p..p + 8]), be(&d[p + 8..p + 16]));
        p += 16;
        v
    };
    if d.len() < p + 4 {
        return Err(short);
    }
    let count = be(&d[p + 2..p + 4]) as usize;
    p += 4;
    if timescale == 0 || count == 0 || d.len() < p + count * 12 {
        return Err(IndexError::NoIndex("an empty or broken sidx"));
    }
    let ts = timescale as f64;
    let mut offset = end + first;
    let mut t = ept;
    let mut pieces = Vec::with_capacity(count);
    for _ in 0..count {
        let r = be(&d[p..p + 4]);
        let dur = be(&d[p + 4..p + 8]);
        p += 12;
        if r >> 31 == 1 {
            return Err(IndexError::NoIndex("an index of indexes"));
        }
        let size = r & 0x7FFF_FFFF;
        pieces.push(Piece {
            start: t as f64 / ts,
            end: (t + dur) as f64 / ts,
            from: offset,
            to: offset + size,
        });
        offset += size;
        t += dur;
    }
    Ok(pieces)
}

// ---------------------------------------------------------------------------------------------
// WebM (Matroska)
// ---------------------------------------------------------------------------------------------

const EBML: u64 = 0x1A45_DFA3;
const SEGMENT: u64 = 0x1853_8067;
const INFO: u64 = 0x1549_A966;
const TIMECODE_SCALE: u64 = 0x2A_D7B1;
const DURATION: u64 = 0x4489;
const TRACKS: u64 = 0x1654_AE6B;
const CUES: u64 = 0x1C53_BB6B;
const CUE_POINT: u64 = 0xBB;
const CUE_TIME: u64 = 0xB3;
const CUE_TRACK_POSITIONS: u64 = 0xB7;
const CUE_CLUSTER_POSITION: u64 = 0xF1;
const CLUSTER: u64 = 0x1F43_B675;

/// An element's header at `pos`: (id, size or `None` for "unknown", header length), or `None`
/// when the header goes past the end of `b`. An invalid header is an error.
fn element(b: &[u8], pos: usize) -> Result<Option<(u64, Option<u64>, usize)>, IndexError> {
    let bad = IndexError::NoIndex("a broken WebM element");
    let Some(&first) = b.get(pos) else {
        return Ok(None);
    };
    let id_len = first.leading_zeros() as usize + 1;
    if id_len > 4 {
        return Err(bad);
    }
    let Some(&s0) = b.get(pos + id_len) else {
        return Ok(None);
    };
    let size_len = s0.leading_zeros() as usize + 1;
    if size_len > 8 {
        return Err(bad);
    }
    let header = id_len + size_len;
    if pos + header > b.len() {
        return Ok(None);
    }
    let id = be(&b[pos..pos + id_len]);
    let raw = &b[pos + id_len..pos + header];
    let mask = if size_len == 8 { 0 } else { 0xFFu8 >> size_len };
    let value = raw[1..]
        .iter()
        .fold(u64::from(raw[0] & mask), |v, x| (v << 8) | u64::from(*x));
    let all_ones = (1u64 << (7 * size_len)) - 1;
    let size = (value != all_ones).then_some(value);
    Ok(Some((id, size, header)))
}

/// The children of an element's contents `d`: (id, contents).
fn children(d: &[u8]) -> Result<Vec<(u64, &[u8])>, IndexError> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < d.len() {
        let Some((id, Some(size), header)) = element(d, pos)? else {
            return Err(IndexError::NoIndex("a broken WebM element"));
        };
        let start = pos + header;
        let end = start
            .checked_add(size as usize)
            .filter(|e| *e <= d.len())
            .ok_or(IndexError::NoIndex("a broken WebM element"))?;
        out.push((id, &d[start..end]));
        pos = end;
    }
    Ok(out)
}

fn float(d: &[u8]) -> Option<f64> {
    match d.len() {
        4 => Some(f32::from_be_bytes(d.try_into().ok()?) as f64),
        8 => Some(f64::from_be_bytes(d.try_into().ok()?)),
        _ => None,
    }
}

fn parse_webm(b: &[u8], file_size: Option<u64>) -> Result<MediaIndex, IndexError> {
    let need = |n: usize| IndexError::NeedMore(n as u64 + 16);
    // EBML header.
    let (id, size, header) = element(b, 0)?.ok_or(need(0))?;
    let ebml_end = header + size.ok_or(IndexError::NoIndex("a broken WebM header"))? as usize;
    if id != EBML {
        return Err(IndexError::NoIndex("not a WebM file"));
    }
    if ebml_end > b.len() {
        return Err(need(ebml_end));
    }
    // Segment.
    let (id, seg_size, header) = element(b, ebml_end)?.ok_or(need(ebml_end))?;
    if id != SEGMENT {
        return Err(IndexError::NoIndex("no WebM segment"));
    }
    let data = ebml_end + header;
    let seg_end = seg_size.map(|s| data as u64 + s);
    let mut init = b[..ebml_end].to_vec();
    init.extend_from_slice(&b[ebml_end..ebml_end + 4]);
    // The new file's segment ends where its pieces do: its size is unknown.
    init.extend_from_slice(&[0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);

    let mut scale = 1_000_000u64;
    let mut duration = None;
    let (mut info, mut tracks, mut cues) = (false, false, None);
    let mut pos = data;
    loop {
        let Some((id, size, header)) = element(b, pos)? else {
            return Err(need(pos));
        };
        if id == CLUSTER {
            break;
        }
        let Some(size) = size else {
            return Err(IndexError::NoIndex(
                "an element of unknown size before the media",
            ));
        };
        let end = pos + header + size as usize;
        if end > b.len() {
            if matches!(id, INFO | TRACKS | CUES) || cues.is_none() {
                return Err(need(end));
            }
            break;
        }
        let body = &b[pos + header..end];
        match id {
            INFO => {
                init.extend_from_slice(&b[pos..end]);
                info = true;
                for (cid, v) in children(body)? {
                    match cid {
                        TIMECODE_SCALE if !v.is_empty() => scale = be(v).max(1),
                        DURATION => duration = float(v),
                        _ => {}
                    }
                }
            }
            TRACKS => {
                init.extend_from_slice(&b[pos..end]);
                tracks = true;
            }
            CUES => cues = Some(body),
            _ => {}
        }
        if info && tracks && cues.is_some() {
            break;
        }
        pos = end;
    }
    if !(info && tracks) {
        return Err(IndexError::NoIndex("no WebM header"));
    }
    let cues = cues.ok_or(IndexError::NoIndex("the WebM index is not at the start"))?;

    let secs = |t: u64| t as f64 * scale as f64 / 1e9;
    let mut points: Vec<(u64, u64)> = Vec::new(); // (cluster position, time)
    for (id, cp) in children(cues)? {
        if id != CUE_POINT {
            continue;
        }
        let mut time = None;
        let mut cluster = None;
        for (cid, v) in children(cp)? {
            match cid {
                CUE_TIME => time = Some(be(v)),
                CUE_TRACK_POSITIONS if cluster.is_none() => {
                    cluster = children(v)?
                        .into_iter()
                        .find(|(i, _)| *i == CUE_CLUSTER_POSITION)
                        .map(|(_, x)| be(x));
                }
                _ => {}
            }
        }
        if let (Some(t), Some(c)) = (time, cluster) {
            points.push((c, t));
        }
    }
    points.sort_unstable();
    points.dedup_by_key(|p| p.0);
    if points.is_empty() {
        return Err(IndexError::NoIndex("an empty WebM index"));
    }
    let last_end = seg_end
        .or(file_size)
        .ok_or(IndexError::NoIndex("the WebM file's size is unknown"))?;
    let total = duration
        .map(|d| d * scale as f64 / 1e9)
        .unwrap_or(f64::INFINITY);
    let pieces = points
        .iter()
        .enumerate()
        .map(|(i, &(c, t))| {
            let next = points.get(i + 1);
            Piece {
                start: secs(t),
                end: next.map(|n| secs(n.1)).unwrap_or(total),
                from: data as u64 + c,
                to: next.map(|n| data as u64 + n.0).unwrap_or(last_end),
            }
        })
        .collect();
    Ok(MediaIndex { init, pieces })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(body);
        v
    }

    /// `sidx` v0, timescale 1000, pieces of (size, duration ms).
    fn sidx(first_offset: u32, ept: u32, refs: &[(u32, u32)]) -> Vec<u8> {
        let mut d = vec![0, 0, 0, 0];
        d.extend_from_slice(&1u32.to_be_bytes());
        d.extend_from_slice(&1000u32.to_be_bytes());
        d.extend_from_slice(&ept.to_be_bytes());
        d.extend_from_slice(&first_offset.to_be_bytes());
        d.extend_from_slice(&[0, 0]);
        d.extend_from_slice(&(refs.len() as u16).to_be_bytes());
        for (size, dur) in refs {
            d.extend_from_slice(&size.to_be_bytes());
            d.extend_from_slice(&dur.to_be_bytes());
            d.extend_from_slice(&0x9000_0000u32.to_be_bytes());
        }
        mp4_box(b"sidx", &d)
    }

    fn mp4() -> (Vec<u8>, usize) {
        let mut f = mp4_box(b"ftyp", b"dash\0\0\0\0iso6");
        f.extend(mp4_box(b"moov", &[7u8; 40]));
        let init_len = f.len();
        f.extend(sidx(0, 0, &[(1000, 2000), (3000, 2000), (500, 1500)]));
        f.extend(mp4_box(b"moof", &[0; 8]));
        (f, init_len)
    }

    #[test]
    fn mp4_pieces_and_header() {
        let (f, init_len) = mp4();
        let sidx_end = (init_len + 32 + 3 * 12) as u64;
        let ix = parse(&f, None).unwrap();
        assert_eq!(ix.init, f[..init_len]);
        assert_eq!(
            ix.pieces,
            vec![
                Piece {
                    start: 0.0,
                    end: 2.0,
                    from: sidx_end,
                    to: sidx_end + 1000
                },
                Piece {
                    start: 2.0,
                    end: 4.0,
                    from: sidx_end + 1000,
                    to: sidx_end + 4000
                },
                Piece {
                    start: 4.0,
                    end: 5.5,
                    from: sidx_end + 4000,
                    to: sidx_end + 4500
                },
            ]
        );
        // 2.5–3.9 s lies in the second piece; 1.0–4.2 s needs all three.
        assert_eq!(
            ix.span(2.5, 3.9),
            Some(Span {
                from: sidx_end + 1000,
                to: sidx_end + 4000,
                start: 2.0
            })
        );
        assert_eq!(
            ix.span(1.0, 4.2),
            Some(Span {
                from: sidx_end,
                to: sidx_end + 4500,
                start: 0.0
            })
        );
        // A part starting right on a piece's edge starts with that piece; past the end, the last.
        assert_eq!(ix.span(2.0, 3.0).map(|s| s.start), Some(2.0));
        assert_eq!(
            ix.span(5.0, 99.0).map(|s| (s.start, s.to)),
            Some((4.0, sidx_end + 4500))
        );
        assert_eq!(ix.span(3.0, 3.0), None);
    }

    #[test]
    fn mp4_needs_more_when_cut_short() {
        let (f, init_len) = mp4();
        // Cut inside the moov box, then inside the sidx: both ask for the box's end.
        assert_eq!(
            parse(&f[..30], None),
            Err(IndexError::NeedMore(init_len as u64))
        );
        assert_eq!(parse(&f[..20], None), Err(IndexError::NeedMore(36)));
        let total_sidx = (init_len + 32 + 36) as u64;
        assert_eq!(
            parse(&f[..init_len + 10], None),
            Err(IndexError::NeedMore(total_sidx))
        );
    }

    #[test]
    fn mp4_without_index_is_not_indexed() {
        let mut f = mp4_box(b"ftyp", b"isom\0\0\0\0");
        f.extend(mp4_box(b"moov", &[1; 16]));
        f.extend(mp4_box(b"mdat", &[0; 64]));
        assert!(matches!(parse(&f, None), Err(IndexError::NoIndex(_))));
        assert!(matches!(
            parse(b"<!DOCTYPE html><html>", None),
            Err(IndexError::NoIndex(_))
        ));
    }

    #[test]
    fn mp4_large_box_size_does_not_overflow() {
        // 24-byte ftyp followed by a 64-bit box whose size wraps pos + size to 0 in release mode.
        let mut ftyp = Vec::new();
        ftyp.extend_from_slice(&24u32.to_be_bytes());
        ftyp.extend_from_slice(b"ftyp");
        ftyp.extend_from_slice(&[0u8; 16]);
        let wrap_size = u64::MAX - (ftyp.len() as u64) + 1;

        for kind in [b"free", b"moov"] {
            let mut buf = ftyp.clone();
            buf.extend_from_slice(&1u32.to_be_bytes());
            buf.extend_from_slice(kind);
            buf.extend_from_slice(&wrap_size.to_be_bytes());
            assert!(matches!(parse(&buf, None), Err(IndexError::NoIndex(_))));
        }
    }

    // WebM builders: element = id bytes + 8-byte size + body.
    fn el(id: &[u8], body: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        let mut size = (body.len() as u64).to_be_bytes();
        size[0] = 0x01;
        v.extend_from_slice(&size);
        v.extend_from_slice(body);
        v
    }
    fn uint(id: &[u8], n: u64) -> Vec<u8> {
        el(id, &n.to_be_bytes())
    }
    fn cue(time: u64, cluster: u64) -> Vec<u8> {
        let pos = [uint(&[0xF7], 1), uint(&[0xF1], cluster)].concat();
        el(&[0xBB], &[uint(&[0xB3], time), el(&[0xB7], &pos)].concat())
    }

    #[test]
    fn webm_pieces_and_header() {
        let ebml = el(&[0x1A, 0x45, 0xDF, 0xA3], &uint(&[0x42, 0x82], 1));
        let info = el(
            &[0x15, 0x49, 0xA9, 0x66],
            &[
                uint(&[0x2A, 0xD7, 0xB1], 1_000_000),
                el(&[0x44, 0x89], &9000f64.to_be_bytes()),
            ]
            .concat(),
        );
        let tracks = el(&[0x16, 0x54, 0xAE, 0x6B], &[5u8; 20]);
        let seekhead = el(&[0x11, 0x4D, 0x9B, 0x74], &[0u8; 30]);
        let cues = el(
            &[0x1C, 0x53, 0xBB, 0x6B],
            &[cue(0, 1000), cue(4000, 3000), cue(8000, 6000)].concat(),
        );
        let body = [
            seekhead.clone(),
            info.clone(),
            tracks.clone(),
            cues,
            el(&[0x1F, 0x43, 0xB6, 0x75], &[]),
        ]
        .concat();
        // The start of a long file: the segment is declared 10 000 bytes long.
        let mut f = ebml.clone();
        f.extend([0x18, 0x53, 0x80, 0x67, 0x01, 0, 0, 0, 0, 0, 0x27, 0x10]);
        f.extend(&body);
        let data = (ebml.len() + 12) as u64;

        let ix = parse(&f, Some(99_999)).unwrap();
        let mut init = ebml.clone();
        init.extend([
            0x18, 0x53, 0x80, 0x67, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        ]);
        init.extend(&info);
        init.extend(&tracks);
        assert_eq!(ix.init, init);
        let seg_end = data + 10_000;
        assert_eq!(
            ix.pieces,
            vec![
                Piece {
                    start: 0.0,
                    end: 4.0,
                    from: data + 1000,
                    to: data + 3000
                },
                Piece {
                    start: 4.0,
                    end: 8.0,
                    from: data + 3000,
                    to: data + 6000
                },
                Piece {
                    start: 8.0,
                    end: 9.0,
                    from: data + 6000,
                    to: seg_end
                },
            ]
        );
        assert_eq!(
            ix.span(5.0, 8.5),
            Some(Span {
                from: data + 3000,
                to: seg_end,
                start: 4.0
            })
        );
        // The header cut short: read on to the end of what's missing.
        assert!(matches!(
            parse(&f[..ebml.len() + 30], None),
            Err(IndexError::NeedMore(_))
        ));
    }
}
