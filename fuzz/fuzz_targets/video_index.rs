//! The start of a video file, as a server sends it, through the MP4 / WebM index parser.
#![no_main]

use libfuzzer_sys::fuzz_target;
use prism_video::index;

fuzz_target!(|head: &[u8]| {
    // The file size only matters for WebM: unknown, exactly what was read, or absurd.
    for size in [None, Some(head.len() as u64), Some(u64::MAX)] {
        let Ok(ix) = index::parse(head, size) else {
            continue;
        };
        // Parts picked by the user: inside, around and past the pieces.
        let (first, last) = match (ix.pieces.first(), ix.pieces.last()) {
            (Some(f), Some(l)) => (f.start, l.end),
            _ => continue,
        };
        for (start, end) in [(first, last), (0.0, f64::MAX), ((first + last) / 2.0, last)] {
            if let Some(span) = ix.span(start, end) {
                assert!(span.from < span.to, "empty span {span:?}");
            }
        }
    }
});
