//! BitTorrent and magnet-link downloads for Prism, built on `librqbit`.
//!
//! The session persists its state (with fast-resume) in a folder owned by Prism, so
//! torrents survive restarts without re-hashing. Torrents are addressed by their
//! info-hash (hex), which the app core stores with the download item.

use anyhow::{Context, Result};
use librqbit::api::TorrentIdOrHash;
use librqbit::limits::LimitsConfig;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ByteBufOwned, ManagedTorrent, Session,
    SessionOptions, SessionPersistenceConfig, TorrentStatsState, ValidatedTorrentMetaV1Info,
};
use std::str::FromStr;

type ManagedTorrentHandle = Arc<ManagedTorrent>;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Where a torrent comes from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum TorrentSource {
    /// `magnet:?xt=...` link or an `http(s)://.../x.torrent` URL.
    Url(String),
    /// Raw `.torrent` file contents.
    #[serde(with = "serde_bytes_b64")]
    File(Vec<u8>),
}

impl TorrentSource {
    /// What librqbit is given, with `extra` trackers next to the torrent's own: written into a
    /// magnet link (librqbit reads custom trackers only for torrent files), else returned for
    /// [`AddTorrentOptions::trackers`].
    fn to_add(&self, extra: &[String]) -> (AddTorrent<'static>, Option<Vec<String>>) {
        let extra = (!extra.is_empty()).then(|| extra.to_vec());
        match self {
            TorrentSource::Url(u) if is_magnet(u) => (
                AddTorrent::from_url(magnet_with_trackers(u, extra.as_deref().unwrap_or(&[]))),
                None,
            ),
            TorrentSource::Url(u) => (AddTorrent::from_url(u.clone()), extra),
            TorrentSource::File(b) => (AddTorrent::from_bytes(b.clone()), extra),
        }
    }
}

fn is_magnet(url: &str) -> bool {
    url.trim_start()
        .get(..7)
        .is_some_and(|s| s.eq_ignore_ascii_case("magnet:"))
}

/// The magnet link with a `tr` parameter for each tracker of `extra` it doesn't name yet.
fn magnet_with_trackers(magnet: &str, extra: &[String]) -> String {
    let mut out = magnet.trim().to_string();
    for tracker in extra {
        let tr = format!("tr={}", encode_component(tracker));
        if out.contains(&tr) || out.contains(&format!("tr={tracker}")) {
            continue;
        }
        out.push(if out.contains('?') { '&' } else { '?' });
        out.push_str(&tr);
    }
    out
}

/// Percent-encodes everything but the unreserved characters (RFC 3986), for a query value.
fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TorrentFile {
    pub index: usize,
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TorrentMeta {
    pub info_hash: String,
    pub name: String,
    pub total_size: u64,
    pub files: Vec<TorrentFile>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TorrentState {
    Initializing,
    Downloading,
    Seeding,
    Paused,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TorrentProgress {
    pub state: TorrentState,
    pub downloaded: u64,
    pub uploaded: u64,
    pub total: u64,
    /// Bytes per second.
    pub download_speed: u64,
    pub upload_speed: u64,
    pub peers: u32,
    pub eta_secs: Option<u64>,
    pub finished: bool,
    pub error: Option<String>,
    pub name: Option<String>,
}

/// Which pieces of a torrent are downloaded, for the file map (#176).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TorrentPieces {
    pub total: u32,
    pub done: u32,
    /// The pieces in equal runs, in order: the share (0..=1) of each run already downloaded.
    pub map: Vec<f32>,
}

/// A peer the torrent is connected to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TorrentPeer {
    /// `ip:port`.
    pub address: String,
    /// The client it announced (`qBittorrent 5.1.2`), when it did.
    pub client: Option<String>,
    /// Bytes received from it so far.
    pub downloaded: u64,
}

/// A running BitTorrent session.
pub struct TorrentEngine {
    session: Arc<Session>,
    /// Metadata `inspect` resolved for URL sources (magnets, .torrent links), so `add` knows
    /// the torrent's layout without resolving it a second time.
    inspected: Mutex<HashMap<String, TorrentMeta>>,
    /// Trackers added to every torrent next to its own (the user's list, #175).
    extra_trackers: Mutex<Vec<String>>,
}

impl TorrentEngine {
    /// Start the session. `state_dir` holds persistence; `default_output` is used when
    /// a torrent is added without an explicit folder. Saved torrents for which `keep` says no
    /// (finished or forgotten downloads) are dropped before the session loads them: loading one
    /// opens its files and creates the ones that were deleted, full size (#82).
    pub async fn start(
        state_dir: &Path,
        default_output: PathBuf,
        keep: &(dyn Fn(&str) -> bool + Send + Sync),
    ) -> Result<Self> {
        std::fs::create_dir_all(state_dir).context("create torrent state dir")?;
        forget_saved(state_dir, keep);
        let opts = SessionOptions {
            fastresume: true,
            persistence: Some(SessionPersistenceConfig::Json {
                folder: Some(state_dir.to_path_buf()),
            }),
            client_name_and_version: Some(format!("Prism/{}", env!("CARGO_PKG_VERSION"))),
            ..Default::default()
        };
        Self::with_options(default_output, opts).await
    }

    /// A session that never touches the network (no DHT, trackers, listening or local
    /// discovery) and keeps no state: for tests.
    #[doc(hidden)]
    pub async fn start_offline(default_output: PathBuf) -> Result<Self> {
        let opts = SessionOptions {
            dht: None,
            disable_trackers: true,
            disable_local_service_discovery: true,
            listen: None,
            persistence: None,
            ..Default::default()
        };
        Self::with_options(default_output, opts).await
    }

    /// [`Self::start_offline`] with the state kept in `state_dir`, as [`Self::start`] does: for
    /// tests of what survives a restart.
    #[doc(hidden)]
    pub async fn start_offline_saved(
        state_dir: &Path,
        default_output: PathBuf,
        keep: &(dyn Fn(&str) -> bool + Send + Sync),
    ) -> Result<Self> {
        std::fs::create_dir_all(state_dir).context("create torrent state dir")?;
        forget_saved(state_dir, keep);
        let opts = SessionOptions {
            dht: None,
            disable_trackers: true,
            disable_local_service_discovery: true,
            listen: None,
            fastresume: true,
            persistence: Some(SessionPersistenceConfig::Json {
                folder: Some(state_dir.to_path_buf()),
            }),
            ..Default::default()
        };
        Self::with_options(default_output, opts).await
    }

    async fn with_options(default_output: PathBuf, opts: SessionOptions) -> Result<Self> {
        let session = Session::new_with_opts(default_output, opts)
            .await
            .context("start torrent session")?;
        Ok(Self {
            session,
            inspected: Mutex::new(HashMap::new()),
            extra_trackers: Mutex::new(Vec::new()),
        })
    }

    /// Trackers to add to the torrents added from now on, next to their own. A private torrent
    /// keeps only its own tracker: librqbit drops the others for it.
    pub fn set_extra_trackers(&self, trackers: Vec<String>) {
        *self
            .extra_trackers
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = trackers;
    }

    fn extra_trackers(&self) -> Vec<String> {
        self.extra_trackers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Read a torrent's metadata (resolving a magnet via DHT/trackers) without downloading.
    pub async fn inspect(&self, source: &TorrentSource) -> Result<TorrentMeta> {
        let (add, trackers) = source.to_add(&self.extra_trackers());
        let opts = AddTorrentOptions {
            list_only: true,
            trackers,
            ..Default::default()
        };
        let resp = self.session.add_torrent(add, Some(opts)).await?;
        let meta = match resp {
            AddTorrentResponse::ListOnly(list) => {
                meta_from_info(list.info_hash.as_string(), &list.info)
            }
            AddTorrentResponse::AlreadyManaged(_, handle)
            | AddTorrentResponse::Added(_, handle) => {
                meta_from_handle(&handle).context("torrent metadata not resolved yet")?
            }
        };
        if let TorrentSource::Url(url) = source {
            let mut seen = self.inspected.lock().unwrap_or_else(|e| e.into_inner());
            if seen.len() >= 64 {
                seen.clear();
            }
            seen.insert(url.clone(), meta.clone());
        }
        Ok(meta)
    }

    /// The torrent's file layout: from `inspect`'s cache for URL sources, else resolved now
    /// (instant for `.torrent` bytes).
    async fn layout(&self, source: &TorrentSource) -> Result<TorrentMeta> {
        if let TorrentSource::Url(url) = source {
            let seen = self.inspected.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(meta) = seen.get(url) {
                return Ok(meta.clone());
            }
        }
        self.inspect(source).await
    }

    /// Add (or re-attach to) a torrent and return its info-hash. A torrent with several
    /// files goes into its own folder inside `output_folder` (see [`content_path`]), like in
    /// other clients; a single file goes straight into `output_folder`.
    pub async fn add(
        &self,
        source: &TorrentSource,
        output_folder: &Path,
        only_files: Option<Vec<usize>>,
        paused: bool,
    ) -> Result<String> {
        let meta = self.layout(source).await?;
        let folder = if meta.files.len() >= 2 {
            content_path(output_folder, &meta)
        } else {
            output_folder.to_path_buf()
        };
        let (add, trackers) = source.to_add(&self.extra_trackers());
        let opts = AddTorrentOptions {
            paused,
            only_files,
            output_folder: Some(folder.to_string_lossy().into_owned()),
            overwrite: true,
            trackers,
            ..Default::default()
        };
        let resp = self.session.add_torrent(add, Some(opts)).await?;
        let handle = resp.into_handle().context("torrent was not added")?;
        Ok(handle.info_hash().as_string())
    }

    fn handle(&self, info_hash: &str) -> Option<ManagedTorrentHandle> {
        let id = librqbit::dht::Id20::from_str(info_hash).ok()?;
        self.session.get(TorrentIdOrHash::Hash(id))
    }

    pub fn is_managed(&self, info_hash: &str) -> bool {
        self.handle(info_hash).is_some()
    }

    pub fn meta(&self, info_hash: &str) -> Option<TorrentMeta> {
        self.handle(info_hash).and_then(|h| meta_from_handle(&h))
    }

    pub fn progress(&self, info_hash: &str) -> Option<TorrentProgress> {
        let handle = self.handle(info_hash)?;
        let stats = handle.stats();
        let live = stats.live.as_ref();
        let mib = |mbps: f64| (mbps * 1024.0 * 1024.0).max(0.0) as u64;
        let state = match stats.state {
            TorrentStatsState::Initializing { .. } => TorrentState::Initializing,
            TorrentStatsState::Paused => TorrentState::Paused,
            TorrentStatsState::Error => TorrentState::Error,
            TorrentStatsState::Live if stats.finished => TorrentState::Seeding,
            TorrentStatsState::Live => TorrentState::Downloading,
        };
        let download_speed = live.map(|l| mib(l.download_speed.mbps)).unwrap_or(0);
        let remaining = stats.total_bytes.saturating_sub(stats.progress_bytes);
        Some(TorrentProgress {
            state,
            downloaded: stats.progress_bytes,
            uploaded: stats.uploaded_bytes,
            total: stats.total_bytes,
            download_speed,
            upload_speed: live.map(|l| mib(l.upload_speed.mbps)).unwrap_or(0),
            peers: live.map(|l| l.snapshot.peer_stats.live).unwrap_or(0),
            eta_secs: (download_speed > 0 && !stats.finished).then(|| remaining / download_speed),
            finished: stats.finished,
            error: stats.error.clone(),
            name: handle.name(),
        })
    }

    /// Which pieces are downloaded, in at most `runs` runs. None while the torrent is starting
    /// (its metadata or the check of existing data isn't done yet).
    pub fn pieces(&self, info_hash: &str, runs: usize) -> Option<TorrentPieces> {
        let id = librqbit::dht::Id20::from_str(info_hash).ok()?;
        let api = librqbit::api::Api::new(self.session.clone(), None);
        let (have, total) = api.api_dump_haves(TorrentIdOrHash::Hash(id)).ok()?;
        let bytes = have.as_raw_slice();
        // Most significant bit first: piece 0 is the top bit of the first byte.
        let has = |i: u32| {
            bytes
                .get((i / 8) as usize)
                .is_some_and(|b| (b >> (7 - i % 8)) & 1 == 1)
        };
        Some(piece_map(total, runs, has))
    }

    /// The peers the torrent is connected to, the ones that sent the most first.
    pub fn peers(&self, info_hash: &str) -> Vec<TorrentPeer> {
        let Some(live) = self.handle(info_hash).and_then(|h| h.live()) else {
            return Vec::new();
        };
        let mut peers: Vec<TorrentPeer> = live
            .per_peer_stats_snapshot(Default::default())
            .peers
            .into_iter()
            .map(|(address, p)| TorrentPeer {
                address,
                client: p.client_name.filter(|c| !c.trim().is_empty()),
                downloaded: p.counters.fetched_bytes,
            })
            .collect();
        peers.sort_by(|a, b| {
            b.downloaded
                .cmp(&a.downloaded)
                .then_with(|| a.address.cmp(&b.address))
        });
        peers
    }

    pub async fn pause(&self, info_hash: &str) -> Result<()> {
        let handle = self.handle(info_hash).context("unknown torrent")?;
        self.session.pause(&handle).await
    }

    pub async fn resume(&self, info_hash: &str) -> Result<()> {
        let handle = self.handle(info_hash).context("unknown torrent")?;
        self.session.unpause(&handle).await
    }

    /// Stop managing a torrent, optionally deleting its files. Without deleting, its files are
    /// closed and stay where they are, and the session no longer remembers it.
    pub async fn remove(&self, info_hash: &str, delete_files: bool) -> Result<()> {
        let id = librqbit::dht::Id20::from_str(info_hash)?;
        self.session
            .delete(TorrentIdOrHash::Hash(id), delete_files)
            .await
    }

    /// Global download limit in bytes/s (0 = unlimited). Upload is limited to the same
    /// value to keep the link usable while seeding.
    pub fn set_limits(&self, download_bps: u64, upload_bps: u64) {
        let nz = |v: u64| NonZeroU32::new(v.min(u32::MAX as u64) as u32);
        self.session.ratelimits.set_download_bps(nz(download_bps));
        self.session.ratelimits.set_upload_bps(nz(upload_bps));
    }

    pub async fn shutdown(&self) {
        self.session.stop().await;
    }

    /// Default limits used by tests and as a template for the app.
    pub fn default_limits() -> LimitsConfig {
        LimitsConfig::default()
    }
}

/// `total` pieces in `runs` (at most one per piece) runs of nearly equal length, each the share
/// of its pieces that `has` says are downloaded.
fn piece_map(total: u32, runs: usize, has: impl Fn(u32) -> bool) -> TorrentPieces {
    let n = runs.clamp(1, total.max(1) as usize) as u64;
    let mut map = Vec::with_capacity(n as usize);
    let mut done = 0;
    for r in 0..n {
        let from = (r * total as u64 / n) as u32;
        let to = ((r + 1) * total as u64 / n) as u32;
        let got = (from..to).filter(|&i| has(i)).count() as u32;
        done += got;
        map.push(if to > from {
            got as f32 / (to - from) as f32
        } else {
            0.0
        });
    }
    TorrentPieces { total, done, map }
}

/// Drops the torrents `keep` says no to from the session's saved state (librqbit's
/// `session.json` and its `<hash>.torrent` / `<hash>.bitv` files). Best effort: a state it can't
/// read is left as it is.
fn forget_saved(state_dir: &Path, keep: &dyn Fn(&str) -> bool) {
    let db = state_dir.join("session.json");
    let Ok(text) = std::fs::read_to_string(&db) else {
        return;
    };
    let Ok(mut doc) = serde_json::from_str::<serde_json::Value>(&text) else {
        tracing::warn!("unreadable torrent session state; left as it is");
        return;
    };
    let Some(torrents) = doc.get_mut("torrents").and_then(|t| t.as_object_mut()) else {
        return;
    };
    let gone: Vec<(String, String)> = torrents
        .iter()
        .filter_map(|(id, t)| {
            let hash = t.get("info_hash")?.as_str()?.to_ascii_lowercase();
            (!keep(&hash)).then(|| (id.clone(), hash))
        })
        .collect();
    if gone.is_empty() {
        return;
    }
    for (id, _) in &gone {
        torrents.remove(id);
    }
    let tmp = state_dir.join("session.json.prism-tmp");
    let written = serde_json::to_vec(&doc)
        .map_err(std::io::Error::other)
        .and_then(|bytes| std::fs::write(&tmp, bytes))
        .and_then(|()| std::fs::rename(&tmp, &db));
    if let Err(e) = written {
        tracing::warn!("could not update the torrent session state: {e}");
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    for (_, hash) in &gone {
        for ext in ["torrent", "bitv"] {
            let _ = std::fs::remove_file(state_dir.join(format!("{hash}.{ext}")));
        }
    }
    tracing::info!("forgot {} finished torrents", gone.len());
}

/// Where a torrent's content ends up inside `output_folder`: its own folder (named after the
/// torrent) when it has several files, else the single file.
pub fn content_path(output_folder: &Path, meta: &TorrentMeta) -> PathBuf {
    match meta.files.as_slice() {
        [single] => output_folder.join(&single.path),
        _ => output_folder.join(folder_name(&meta.name)),
    }
}

/// The torrent's name as a single folder name that works on every system: no path
/// separators or characters Windows refuses, no trailing dots or spaces, no device names.
fn folder_name(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    while s.ends_with(['.', ' ']) {
        s.pop();
    }
    let s = s.trim_start().to_string();
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    match s.as_str() {
        "" | "." | ".." => "torrent".to_string(),
        _ if reserved => format!("{s}_"),
        _ => s,
    }
}

fn meta_from_handle(handle: &ManagedTorrentHandle) -> Option<TorrentMeta> {
    let hash = handle.info_hash().as_string();
    handle
        .with_metadata(|m| meta_from_info(hash.clone(), &m.info))
        .ok()
}

fn meta_from_info(
    info_hash: String,
    info: &ValidatedTorrentMetaV1Info<ByteBufOwned>,
) -> TorrentMeta {
    let files = info
        .iter_file_details()
        .enumerate()
        .map(|(index, fd)| TorrentFile {
            index,
            path: fd.filename.to_pathbuf().to_string_lossy().into_owned(),
            size: fd.len,
        })
        .collect::<Vec<_>>();
    TorrentMeta {
        info_hash,
        name: info
            .name()
            .map(|n| n.to_string())
            .unwrap_or_else(|| "torrent".into()),
        total_size: files.iter().map(|f| f.size).sum(),
        files,
    }
}

/// Serialize `.torrent` bytes as base64 so sources fit in JSON columns.
mod serde_bytes_b64 {
    use serde::{Deserialize, Deserializer, Serializer};

    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let b = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        s.serialize_str(&out)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        let mut out = Vec::with_capacity(s.len() / 4 * 3);
        let mut buf = 0u32;
        let mut bits = 0;
        for c in s.bytes().filter(|c| *c != b'=') {
            let v = ALPHABET
                .iter()
                .position(|a| *a == c)
                .ok_or_else(|| serde::de::Error::custom("invalid base64"))?
                as u32;
            buf = buf << 6 | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((buf >> bits) as u8);
                buf &= (1 << bits) - 1;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piece_map_folds_pieces_into_runs() {
        let have = [0u32, 1, 2, 3, 9];
        let m = piece_map(10, 5, |i| have.contains(&i));
        assert_eq!((m.total, m.done), (10, 5));
        assert_eq!(m.map, vec![1.0, 1.0, 0.0, 0.0, 0.5]);
        // Fewer pieces than runs: one run per piece.
        let m = piece_map(3, 100, |i| i == 1);
        assert_eq!(m.map, vec![0.0, 1.0, 0.0]);
        // Uneven runs still cover every piece once.
        let m = piece_map(7, 3, |_| true);
        assert_eq!((m.done, m.map.len()), (7, 3));
        assert!(m.map.iter().all(|f| *f == 1.0));
        assert_eq!(piece_map(0, 10, |_| true).done, 0);
    }

    #[test]
    fn extra_trackers_go_into_a_magnet_once() {
        let hash = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=x";
        let own = format!("{hash}&tr=udp%3A%2F%2Fa.example%3A80%2Fannounce");
        let extra = vec![
            "udp://a.example:80/announce".to_string(),
            "https://b.example/announce?k=1&x=2".to_string(),
        ];
        let out = magnet_with_trackers(&own, &extra);
        let m = librqbit::Magnet::parse(&out).unwrap();
        assert_eq!(
            m.trackers,
            vec![
                "udp://a.example:80/announce".to_string(),
                "https://b.example/announce?k=1&x=2".to_string(),
            ]
        );
        assert_eq!(magnet_with_trackers(hash, &[]), hash);
        assert!(is_magnet("  MAGNET:?xt=urn:btih:00"));
        assert!(!is_magnet("https://example.com/a.torrent"));
        // Other sources hand the list to librqbit as it is.
        let (_, t) = TorrentSource::Url("https://example.com/a.torrent".into()).to_add(&extra);
        assert_eq!(t, Some(extra.clone()));
        let (_, t) = TorrentSource::Url(own).to_add(&extra);
        assert_eq!(t, None);
        let (_, t) = TorrentSource::File(vec![]).to_add(&[]);
        assert_eq!(t, None);
    }

    #[test]
    fn source_roundtrip_base64() {
        for data in [
            vec![],
            vec![1u8],
            vec![1, 2],
            vec![1, 2, 3],
            (0..=255u8).collect(),
        ] {
            let src = TorrentSource::File(data.clone());
            let json = serde_json::to_string(&src).unwrap();
            assert_eq!(serde_json::from_str::<TorrentSource>(&json).unwrap(), src);
        }
    }
}
