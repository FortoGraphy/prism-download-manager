//! Where torrents put their files: a torrent with several files gets its own folder (named
//! after the torrent) inside the chosen download folder; a single file goes straight in.
//!
//! Each test builds a small `.torrent` in memory and places its complete data where Prism
//! should look for it. A session that never touches the network then reports the torrent
//! as finished only if it found the data there.

use prism_torrent::{TorrentEngine, TorrentMeta, TorrentSource, content_path};
use sha1::{Digest, Sha1};
use std::path::Path;
use std::time::{Duration, Instant};

const PIECE: usize = 16 * 1024;

fn bstr(out: &mut Vec<u8>, s: &[u8]) {
    out.extend_from_slice(format!("{}:", s.len()).as_bytes());
    out.extend_from_slice(s);
}

fn bint(out: &mut Vec<u8>, n: usize) {
    out.extend_from_slice(format!("i{n}e").as_bytes());
}

/// A bencoded `.torrent` for `files` (relative path parts, contents); one entry = a
/// single-file torrent named `name`.
fn torrent(name: &str, files: &[(&[&str], Vec<u8>)]) -> Vec<u8> {
    let data: Vec<u8> = files.iter().flat_map(|(_, d)| d.clone()).collect();
    let pieces: Vec<u8> = data
        .chunks(PIECE)
        .flat_map(|c| Sha1::digest(c).to_vec())
        .collect();
    // Keys of a bencoded dictionary must be sorted.
    let mut info = b"d".to_vec();
    if files.len() == 1 {
        bstr(&mut info, b"length");
        bint(&mut info, files[0].1.len());
    } else {
        bstr(&mut info, b"files");
        info.push(b'l');
        for (path, contents) in files {
            info.push(b'd');
            bstr(&mut info, b"length");
            bint(&mut info, contents.len());
            bstr(&mut info, b"path");
            info.push(b'l');
            for part in *path {
                bstr(&mut info, part.as_bytes());
            }
            info.extend_from_slice(b"ee");
        }
        info.push(b'e');
    }
    bstr(&mut info, b"name");
    bstr(&mut info, name.as_bytes());
    bstr(&mut info, b"piece length");
    bint(&mut info, PIECE);
    bstr(&mut info, b"pieces");
    bstr(&mut info, &pieces);
    info.push(b'e');
    let mut t = b"d".to_vec();
    bstr(&mut t, b"info");
    t.extend_from_slice(&info);
    t.push(b'e');
    t
}

fn contents(seed: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31) ^ seed)
        .collect()
}

fn write(path: &Path, data: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, data).unwrap();
}

/// Adds the torrent and waits until the session has checked the data on disk.
async fn finished(engine: &TorrentEngine, source: &TorrentSource, out: &Path) -> bool {
    added(engine, source, out).await.0
}

/// [`finished`], with the torrent's hash.
async fn added(engine: &TorrentEngine, source: &TorrentSource, out: &Path) -> (bool, String) {
    let hash = engine.add(source, out, None, false).await.unwrap();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(30) {
        if let Some(p) = engine.progress(&hash)
            && p.state != prism_torrent::TorrentState::Initializing
        {
            return (p.finished, hash);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the torrent was still being checked after 30 s");
}

#[tokio::test]
async fn several_files_go_into_their_own_folder() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("Downloads");
    // A name Windows can't use as is: the folder gets a safe version of it.
    let name = "My Show: Season 1?";
    let a = contents(1, 3 * PIECE + 100);
    let b = contents(2, PIECE / 2);
    let bytes = torrent(
        name,
        &[(&["a.bin"], a.clone()), (&["sub", "b.bin"], b.clone())],
    );
    let folder = out.join("My Show_ Season 1_");
    write(&folder.join("a.bin"), &a);
    write(&folder.join("sub").join("b.bin"), &b);

    let engine = TorrentEngine::start_offline(dir.path().join("default"))
        .await
        .unwrap();
    let source = TorrentSource::File(bytes);
    assert!(
        finished(&engine, &source, &out).await,
        "Prism didn't find the data in the torrent's own folder"
    );
    let meta = engine.inspect(&source).await.unwrap();
    assert_eq!(content_path(&out, &meta), folder);
    assert!(
        !out.join("a.bin").exists(),
        "nothing may land straight in the download folder"
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn a_single_file_goes_straight_into_the_download_folder() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("Downloads");
    let data = contents(3, 2 * PIECE + 7);
    let bytes = torrent("movie.mkv", &[(&["movie.mkv"], data.clone())]);
    write(&out.join("movie.mkv"), &data);

    let engine = TorrentEngine::start_offline(dir.path().join("default"))
        .await
        .unwrap();
    let source = TorrentSource::File(bytes);
    assert!(finished(&engine, &source, &out).await);
    let meta = engine.inspect(&source).await.unwrap();
    assert_eq!(content_path(&out, &meta), out.join("movie.mkv"));
    engine.shutdown().await;
}

#[test]
fn folder_names_are_safe_everywhere() {
    let meta = |name: &str| TorrentMeta {
        info_hash: String::new(),
        name: name.into(),
        total_size: 2,
        files: vec![
            prism_torrent::TorrentFile {
                index: 0,
                path: "a".into(),
                size: 1,
            },
            prism_torrent::TorrentFile {
                index: 1,
                path: "b".into(),
                size: 1,
            },
        ],
    };
    let out = Path::new("dl");
    for (name, folder) in [
        ("Album (2024)", "Album (2024)"),
        ("a/b\\c", "a_b_c"),
        ("CON", "CON_"),
        ("com1.txt", "com1.txt_"),
        ("trailing. ", "trailing"),
        ("..", "torrent"),
        ("", "torrent"),
    ] {
        assert_eq!(content_path(out, &meta(name)), out.join(folder), "{name:?}");
    }
}

/// Waits up to 10 s for the session to know (or not know) a torrent.
async fn managed(engine: &TorrentEngine, hash: &str, want: bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if engine.is_managed(hash) == want {
            return want;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    !want
}

/// #82: a finished torrent leaves the session with its file closed and kept, and a saved
/// finished torrent isn't loaded again at the next start, so a deleted file stays deleted.
#[tokio::test]
async fn a_finished_torrent_lets_go_of_its_files() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("Downloads");
    let state = dir.path().join("state");
    let data = contents(4, 3 * PIECE + 11);
    let bytes = torrent("song.flac", &[(&["song.flac"], data.clone())]);
    let file = out.join("song.flac");
    write(&file, &data);
    let source = TorrentSource::File(bytes);
    let keep_all = |_: &str| true;

    // Finished and removed without deleting: the data stays and nothing holds the file.
    let engine = TorrentEngine::start_offline_saved(&state, dir.path().join("d"), &keep_all)
        .await
        .unwrap();
    let (done, hash) = added(&engine, &source, &out).await;
    assert!(done);
    engine.remove(&hash, false).await.unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), data, "the file stays");
    let moved = out.join("moved.flac");
    std::fs::rename(&file, &moved).expect("nothing holds the file open");
    std::fs::rename(&moved, &file).unwrap();
    engine.shutdown().await;
    let engine = TorrentEngine::start_offline_saved(&state, dir.path().join("d"), &keep_all)
        .await
        .unwrap();
    assert!(
        !managed(&engine, &hash, false).await,
        "a removed torrent stays removed"
    );
    engine.shutdown().await;

    // A finished torrent saved by an older Prism (only paused): loaded again while kept...
    let engine = TorrentEngine::start_offline_saved(&state, dir.path().join("d"), &keep_all)
        .await
        .unwrap();
    assert!(finished(&engine, &source, &out).await);
    engine.pause(&hash).await.unwrap();
    engine.shutdown().await;
    let engine = TorrentEngine::start_offline_saved(&state, dir.path().join("d"), &keep_all)
        .await
        .unwrap();
    assert!(
        managed(&engine, &hash, true).await,
        "the saved state was read"
    );
    engine.shutdown().await;

    // ...and forgotten when it isn't: its deleted file doesn't come back.
    std::fs::remove_file(&file).unwrap();
    let engine =
        TorrentEngine::start_offline_saved(&state, dir.path().join("d"), &|h: &str| h != hash)
            .await
            .unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        !engine.is_managed(&hash),
        "the finished torrent was forgotten"
    );
    assert!(!file.exists(), "the deleted file stays deleted");
    assert!(!state.join(format!("{hash}.torrent")).exists());
    engine.shutdown().await;
}
