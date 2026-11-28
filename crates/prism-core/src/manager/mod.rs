//! The download manager: owns every item, runs jobs, enforces queues and schedules,
//! persists state and publishes [`CoreEvent`]s.
//!
//! Concurrency model: one `parking_lot::Mutex<State>` protects all mutable state and
//! is never held across an `.await`. Jobs run as independent tokio tasks and report
//! back through an outcome channel; a few periodic tasks drive progress events
//! (250 ms), resume checkpoints (2 s) and queue/schedule evaluation (1 s).

mod capture;
mod grab;
mod jobs;
mod peek;
mod queues;

pub use capture::{
    BatchDecision, CaptureDecision, NewItem, ONE_AT_A_TIME_QUEUE, StartMode, VideoDecision,
    VideoEntry,
};
pub use jobs::ItemDetails;
pub(crate) use jobs::{engine_config, http_preference, proxy_config};
pub use peek::{PeekProblem, ZipFile};

use crate::categories;
use crate::db::Db;
use crate::events::*;
use crate::model::*;
use crate::paths;
use crate::settings::Settings;
use anyhow::{Context, Result, anyhow};
use jobs::{Job, JobOutcome};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

/// Number of speed samples kept per item for the details graph (1 sample / 500 ms).
const SPEED_HISTORY: usize = 240;

pub struct CoreOptions {
    pub data_dir: PathBuf,
    /// Start the browser IPC server (disabled in tests).
    pub ipc: bool,
    pub clipboard: bool,
}

impl Default for CoreOptions {
    fn default() -> Self {
        Self {
            data_dir: paths::data_dir(),
            ipc: true,
            clipboard: true,
        }
    }
}

/// Everything the UI needs on startup.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub version: String,
    pub items: Vec<DownloadItem>,
    pub categories: Vec<Category>,
    pub queues: Vec<Queue>,
    pub settings: Settings,
    pub grabber_projects: Vec<GrabberProject>,
    pub data_dir: PathBuf,
    pub captures: Vec<CaptureInfo>,
}

/// Fields the user may edit on an existing item (Properties dialog).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ItemPatch {
    pub file_name: Option<String>,
    pub save_dir: Option<PathBuf>,
    pub description: Option<String>,
    pub category: Option<String>,
    /// `Some(None)` removes the item from its queue.
    pub queue_id: Option<Option<String>>,
    /// New address ("Refresh download address").
    pub url: Option<String>,
    pub connections: Option<u8>,
    pub speed_limit: Option<u64>,
    pub checksum: Option<Option<String>>,
    pub username: Option<String>,
    pub password: Option<String>,
}

pub(crate) struct State {
    pub settings: Settings,
    pub items: HashMap<String, DownloadItem>,
    pub categories: Vec<Category>,
    pub queues: Vec<Queue>,
    pub jobs: HashMap<String, Job>,
    pub captures: HashMap<String, capture::PendingCapture>,
    pub batches: HashMap<String, BatchInfo>,
    pub videos: HashMap<String, VideoCapture>,
    /// Recent yt-dlp probes by page URL (the in-page menu probes, then picks a format).
    pub video_probes: HashMap<String, (std::time::Instant, serde_json::Value)>,
    pub grabbers: HashMap<String, crate::grabber::CancelFlag>,
    pub speed_history: HashMap<String, VecDeque<u64>>,
    /// Items whose row must be persisted at the next checkpoint.
    pub dirty: HashSet<String>,
    /// Queues that were running in the previous tick (to detect "queue finished").
    pub queue_was_busy: HashSet<String>,
    pub last_schedule_tick: chrono::NaiveDateTime,
}

pub(crate) struct Inner {
    pub db: Db,
    pub state: Mutex<State>,
    pub events: broadcast::Sender<CoreEvent>,
    pub engine: prism_engine::Engine,
    pub video: prism_video::VideoService,
    pub torrent: tokio::sync::OnceCell<prism_torrent::TorrentEngine>,
    pub queue_limiters: Mutex<HashMap<String, Arc<prism_engine::RateLimiter>>>,
    /// Held while queue admission counts free slots and starts items (see `admit`).
    pub admission: Mutex<()>,
    pub outcomes: mpsc::UnboundedSender<JobOutcome>,
    pub data_dir: PathBuf,
    pub ipc: Mutex<Option<crate::ipc::IpcServer>>,
    pub clipboard: Mutex<Option<crate::clipboard::ClipboardMonitor>>,
    pub post_action_cancel: Mutex<Option<tokio_util::sync::CancellationToken>>,
    /// Last time the browser extension talked to us (ms since epoch, 0 = never).
    pub ext_seen: std::sync::atomic::AtomicI64,
    /// ffmpeg resolved by the video service (managed or system), for the engine's remuxer.
    pub ffmpeg_path: Mutex<Option<PathBuf>>,
    /// One `probe_video` per page at a time: a second request waits for the first's result.
    pub probe_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Version the browser extension reported in its last hello, and how the browser installed
    /// it (`management.getSelf().installType`, sent since 1.0.7); kept across restarts in
    /// `extension-version` (version, then install type on a second line).
    pub ext_version: Mutex<Option<String>>,
    pub ext_install_type: Mutex<Option<String>>,
    /// The Tools' jobs (convert, make smaller, ZIP).
    pub media: crate::media::MediaJobs,
}

/// Cheap, cloneable handle to the application core.
#[derive(Clone)]
pub struct Core(pub(crate) Arc<Inner>);

impl Core {
    /// Open storage, restore state and start background services.
    pub async fn start(opts: CoreOptions) -> Result<Self> {
        crate::rt::init();
        std::fs::create_dir_all(&opts.data_dir).context("create data dir")?;
        std::fs::create_dir_all(opts.data_dir.join("logs")).ok();
        let db = Db::open(&opts.data_dir.join("prism.db")).context("open database")?;
        restrict_permissions(&opts.data_dir);

        let settings: Settings = db
            .get_setting("settings")?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        let mut categories = db.categories()?;
        if categories.is_empty() {
            for c in categories::builtin_categories() {
                db.upsert_category(&c)?;
            }
            categories = db.categories()?;
        }
        let mut queues = db.queues()?;
        if queues.is_empty() {
            let main = Queue {
                id: "main".into(),
                name: "Main queue".into(),
                max_concurrent: 3,
                builtin: true,
                ..Default::default()
            };
            db.upsert_queue(&main)?;
            queues = vec![main];
        }

        let mut items: HashMap<String, DownloadItem> = HashMap::new();
        for mut item in db.downloads()? {
            // Anything that was running when the app died is resumable from its checkpoint.
            if item.status.is_active() || item.status == Status::Queued {
                item.status = Status::Paused;
                db.upsert_download(&item)?;
            }
            items.insert(item.id.clone(), item);
        }

        let engine = prism_engine::Engine::new(jobs::engine_config(&settings));
        let video = prism_video::VideoService::new(
            opts.data_dir.join("tools"),
            jobs::video_options(&settings),
        );
        let (outcome_tx, outcome_rx) = mpsc::unbounded_channel();
        let (events, _) = broadcast::channel(1024);
        let ext_seen =
            std::fs::read_to_string(opts.data_dir.join("extension-version")).unwrap_or_default();
        let mut ext_seen = ext_seen
            .lines()
            .map(|l| Some(l.trim().to_string()).filter(|l| !l.is_empty()));
        let (ext_version, ext_install_type) =
            (ext_seen.next().flatten(), ext_seen.next().flatten());

        let state = State {
            settings,
            items,
            categories,
            queues,
            jobs: HashMap::new(),
            captures: HashMap::new(),
            batches: HashMap::new(),
            videos: HashMap::new(),
            video_probes: HashMap::new(),
            grabbers: HashMap::new(),
            speed_history: HashMap::new(),
            dirty: HashSet::new(),
            queue_was_busy: HashSet::new(),
            last_schedule_tick: chrono::Local::now().naive_local(),
        };
        let core = Core(Arc::new(Inner {
            db,
            state: Mutex::new(state),
            events,
            engine,
            video,
            torrent: tokio::sync::OnceCell::new(),
            queue_limiters: Mutex::new(HashMap::new()),
            admission: Mutex::new(()),
            outcomes: outcome_tx,
            data_dir: opts.data_dir.clone(),
            ipc: Mutex::new(None),
            clipboard: Mutex::new(None),
            post_action_cancel: Mutex::new(None),
            ext_seen: std::sync::atomic::AtomicI64::new(
                std::fs::read_to_string(opts.data_dir.join("extension-seen"))
                    .ok()
                    .and_then(|t| t.trim().parse().ok())
                    .unwrap_or(0),
            ),
            ffmpeg_path: Mutex::new(None),
            probe_locks: Mutex::new(HashMap::new()),
            ext_version: Mutex::new(ext_version),
            ext_install_type: Mutex::new(ext_install_type),
            media: Default::default(),
        }));
        core.apply_runtime_settings();
        core.spawn_background(outcome_rx);

        if opts.ipc {
            let handler: Arc<dyn crate::ipc::IpcHandler> = Arc::new(core.clone());
            match crate::ipc::IpcServer::start(
                handler,
                &opts.data_dir.join("ipc.json"),
                crate::APP_VERSION,
            )
            .await
            {
                Ok(server) => *core.0.ipc.lock() = Some(server),
                Err(e) => tracing::error!("browser IPC unavailable: {e}"),
            }
        }
        if opts.clipboard {
            core.start_clipboard_monitor();
        }
        // Torrents that were active need the session; start it lazily in the background.
        if core
            .0
            .state
            .lock()
            .items
            .values()
            .any(|i| i.kind == ItemKind::Torrent && !i.trashed)
        {
            let c = core.clone();
            tokio::spawn(async move {
                if let Err(e) = c.torrent_engine().await {
                    tracing::warn!("torrent session failed to start: {e:#}");
                }
            });
        }
        // After an in-app update: continue what was running before the restart.
        core.resume_after_restart();
        // HLS/DASH need ffmpeg to join video and audio: find it now, not only after an update.
        let c = core.clone();
        tokio::spawn(async move { c.sync_ffmpeg_path().await });
        let c = core.clone();
        tokio::spawn(async move { c.remove_deleted_files().await });
        Ok(core)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CoreEvent> {
        self.0.events.subscribe()
    }

    pub(crate) fn emit(&self, event: CoreEvent) {
        let _ = self.0.events.send(event);
    }

    pub fn data_dir(&self) -> &Path {
        &self.0.data_dir
    }

    /// When the browser extension last contacted the app (ms since epoch).
    pub fn extension_last_seen(&self) -> Option<i64> {
        let v = self.0.ext_seen.load(std::sync::atomic::Ordering::Relaxed);
        (v > 0).then_some(v)
    }

    /// Version of the browser extension that last said hello.
    pub fn extension_version(&self) -> Option<String> {
        self.0.ext_version.lock().clone()
    }

    /// How the browser installed the extension that last said hello: "development" (loaded
    /// unpacked), "normal" (from a store), "admin" (by policy)... `None` before 1.0.7.
    pub fn extension_install_type(&self) -> Option<String> {
        self.0.ext_install_type.lock().clone()
    }

    /// The browser runs an extension older than the one this app ships: browsers don't reload
    /// an unpacked extension's files by themselves after an update. One installed from a store
    /// or by policy is updated by the browser (and may well lag behind the app for a while).
    pub fn extension_outdated(&self) -> bool {
        let managed = matches!(
            self.extension_install_type().as_deref(),
            Some("normal" | "admin")
        );
        !managed
            && self
                .extension_version()
                .is_some_and(|v| capture::version_older(&v, crate::APP_VERSION))
    }

    pub fn ipc_port(&self) -> Option<u16> {
        self.0.ipc.lock().as_ref().map(|s| s.info.port)
    }

    pub fn bootstrap(&self) -> Bootstrap {
        let st = self.0.state.lock();
        let mut items: Vec<DownloadItem> = st.items.values().cloned().collect();
        items.sort_by_key(|i| i.added_at);
        Bootstrap {
            version: crate::APP_VERSION.into(),
            items,
            categories: st.categories.clone(),
            queues: st.queues.clone(),
            settings: st.settings.clone(),
            grabber_projects: self.0.db.grabber_projects().unwrap_or_default(),
            data_dir: self.0.data_dir.clone(),
            captures: st.captures.values().map(|c| c.info.clone()).collect(),
        }
    }

    pub fn settings(&self) -> Settings {
        self.0.state.lock().settings.clone()
    }

    /// Number of running jobs.
    pub fn active_count(&self) -> usize {
        self.0.state.lock().jobs.len()
    }

    pub fn item(&self, id: &str) -> Option<DownloadItem> {
        self.0.state.lock().items.get(id).cloned()
    }

    // ----- settings -----------------------------------------------------------------------

    pub fn update_settings(&self, patch: &serde_json::Value) -> Result<Settings> {
        let settings = {
            let mut st = self.0.state.lock();
            let mut s = st.settings.clone();
            s.apply_patch(patch).context("invalid settings")?;
            st.settings = s.clone();
            s
        };
        self.0
            .db
            .set_setting("settings", &serde_json::to_string(&settings)?)?;
        self.apply_runtime_settings();
        self.emit(CoreEvent::SettingsChanged(Box::new(settings.clone())));
        Ok(settings)
    }

    pub fn replace_settings(&self, settings: Settings) -> Result<Settings> {
        let value = serde_json::to_value(&settings)?;
        self.update_settings(&value)
    }

    /// Push settings that affect running components (limiters, engine, clipboard, video).
    fn apply_runtime_settings(&self) {
        let s = self.settings();
        let global = if s.downloads.speed_limiter_enabled {
            s.downloads.speed_limit
        } else {
            0
        };
        self.0.engine.global_limiter().set_rate(global);
        self.apply_engine_config();
        self.0.video.set_options(jobs::video_options(&s));
        if let Some(t) = self.0.torrent.get() {
            t.set_limits(global, global);
        }
    }

    pub fn toggle_speed_limiter(&self) -> Result<bool> {
        let on = !self.settings().downloads.speed_limiter_enabled;
        self.update_settings(&serde_json::json!({"downloads": {"speedLimiterEnabled": on}}))?;
        Ok(on)
    }

    // ----- item mutation helpers -------------------------------------------------------------

    /// Mutate an item, persist it and broadcast the change.
    pub(crate) fn with_item<R>(
        &self,
        id: &str,
        f: impl FnOnce(&mut DownloadItem) -> R,
    ) -> Option<R> {
        let (r, item) = {
            let mut st = self.0.state.lock();
            let item = st.items.get_mut(id)?;
            let r = f(item);
            (r, item.clone())
        };
        if let Err(e) = self.0.db.upsert_download(&item) {
            tracing::error!("persist {id}: {e}");
        }
        self.emit(CoreEvent::ItemUpdated(item));
        Some(r)
    }

    pub(crate) fn insert_items(&self, new_items: Vec<DownloadItem>) -> Result<()> {
        if new_items.is_empty() {
            return Ok(());
        }
        self.0.db.upsert_downloads(&new_items)?;
        {
            let mut st = self.0.state.lock();
            for item in &new_items {
                st.items.insert(item.id.clone(), item.clone());
            }
        }
        self.emit(CoreEvent::ItemsAdded(new_items));
        Ok(())
    }

    /// Next free position at the end of a queue.
    pub(crate) fn next_position(&self, queue_id: Option<&str>) -> i64 {
        let st = self.0.state.lock();
        st.items
            .values()
            .filter(|i| i.queue_id.as_deref() == queue_id)
            .map(|i| i.position)
            .max()
            .unwrap_or(0)
            + 1
    }

    pub fn update_item(&self, id: &str, patch: ItemPatch) -> Result<DownloadItem> {
        let _rt = crate::rt::enter();
        let running = self.0.state.lock().jobs.contains_key(id);
        if running && (patch.file_name.is_some() || patch.save_dir.is_some()) {
            return Err(anyhow!("Pause the download before renaming or moving it"));
        }
        let current = self.item(id).ok_or_else(|| anyhow!("unknown item"))?;
        // Moving/renaming a finished file moves it on disk too.
        if current.status == Status::Completed
            && (patch.file_name.is_some() || patch.save_dir.is_some())
        {
            let from = current.path();
            let to_dir = patch.save_dir.clone().unwrap_or(current.save_dir.clone());
            let to_name = patch.file_name.clone().unwrap_or(current.file_name.clone());
            let to = to_dir.join(sanitize_name(&to_name));
            if from.exists() && from != to {
                std::fs::create_dir_all(&to_dir)?;
                move_file(&from, &to)?;
            }
        }
        let limit = patch.speed_limit;
        let connections = patch.connections;
        let updated = self
            .with_item(id, |item| {
                if let Some(v) = patch.file_name {
                    item.file_name = sanitize_name(&v);
                }
                if let Some(v) = patch.save_dir {
                    item.save_dir = v;
                }
                if let Some(v) = patch.description {
                    item.description = v;
                }
                if let Some(v) = patch.category {
                    item.category = v;
                }
                if let Some(v) = patch.queue_id {
                    item.queue_id = v;
                }
                if let Some(v) = patch.connections {
                    item.connections = v.clamp(1, 32);
                }
                if let Some(v) = patch.speed_limit {
                    item.speed_limit = v;
                }
                if let Some(v) = patch.checksum {
                    item.checksum = v.filter(|s| !s.trim().is_empty());
                }
                if let Some(url) = patch.url {
                    item.url = url.clone();
                    item.final_url = None;
                    jobs::set_request_url(item, &url);
                }
                if patch.username.is_some() || patch.password.is_some() {
                    jobs::set_request_auth(
                        item,
                        patch.username.unwrap_or_default(),
                        patch.password.unwrap_or_default(),
                    );
                }
                item.clone()
            })
            .ok_or_else(|| anyhow!("unknown item"))?;
        // Live adjustments of a running job.
        let st = self.0.state.lock();
        if let Some(job) = st.jobs.get(id) {
            if let Some(l) = limit {
                job.set_speed_limit(l);
            }
            if let Some(c) = connections {
                job.set_connections(c);
            }
            if let Some(url) = updated
                .url
                .parse()
                .ok()
                .filter(|_| updated.final_url.is_none())
            {
                job.change_url(url);
            }
        }
        Ok(updated)
    }

    // ----- start / pause / delete --------------------------------------------------------------

    /// Start items immediately (bypassing queue limits), like IDM's "Resume".
    pub fn start_items(&self, ids: &[String]) {
        let _rt = crate::rt::enter();
        for id in ids {
            if let Err(e) = self.start_item(id) {
                tracing::warn!("start {id}: {e:#}");
                self.with_item(id, |i| {
                    i.status = Status::Failed;
                    i.error = Some(format!("{e:#}"));
                });
            }
        }
    }

    pub(crate) fn start_item(&self, id: &str) -> Result<()> {
        let item = {
            let st = self.0.state.lock();
            if st.jobs.contains_key(id) {
                return Ok(());
            }
            st.items
                .get(id)
                .cloned()
                .ok_or_else(|| anyhow!("unknown item"))?
        };
        if item.trashed || item.status == Status::Completed {
            return Ok(());
        }
        self.with_item(id, |i| {
            i.status = Status::Connecting;
            i.error = None;
            i.last_try_at = Some(now_ms());
        });
        let job = jobs::spawn(self, &item)?;
        self.0.state.lock().jobs.insert(id.to_string(), job);
        platform_keep_awake(self);
        Ok(())
    }

    pub fn pause_items(&self, ids: &[String]) {
        for id in ids {
            let job_exists = {
                let st = self.0.state.lock();
                if let Some(job) = st.jobs.get(id) {
                    job.pause();
                    true
                } else {
                    false
                }
            };
            if !job_exists {
                // Queued/idle items simply leave the queue's waiting list.
                self.with_item(id, |i| {
                    if i.status == Status::Queued {
                        i.status = Status::Paused;
                    }
                });
            }
        }
    }

    pub fn pause_all(&self) {
        let ids: Vec<String> = {
            let st = self.0.state.lock();
            st.items
                .values()
                .filter(|i| i.status.is_active() || i.status == Status::Queued)
                .map(|i| i.id.clone())
                .collect()
        };
        self.pause_items(&ids);
    }

    pub fn resume_all(&self) {
        let _rt = crate::rt::enter();
        let ids: Vec<String> = {
            let st = self.0.state.lock();
            st.items
                .values()
                .filter(|i| {
                    !i.trashed && matches!(i.status, Status::Paused | Status::Failed | Status::Idle)
                })
                .map(|i| i.id.clone())
                .collect()
        };
        self.start_items(&ids);
    }

    /// "Remove downloads whose files were deleted" (Settings › General): finished downloads whose
    /// file is gone from a folder that's still there leave the list. A missing folder (an
    /// unplugged drive) keeps them, and so do torrents, whose folder can be named differently.
    /// Returns how many were removed.
    pub async fn remove_deleted_files(&self) -> usize {
        if !self.settings().general.remove_deleted_files {
            return 0;
        }
        let finished: Vec<(String, PathBuf, PathBuf)> = {
            let st = self.0.state.lock();
            st.items
                .values()
                .filter(|i| {
                    i.status == Status::Completed
                        && !i.trashed
                        && i.kind != ItemKind::Torrent
                        && !i.file_name.is_empty()
                })
                .map(|i| (i.id.clone(), i.save_dir.clone(), i.path()))
                .collect()
        };
        let gone: Vec<String> = finished
            .into_iter()
            .filter(|(_, dir, path)| dir.is_dir() && !path.exists())
            .map(|(id, _, _)| id)
            .collect();
        if gone.is_empty() {
            return 0;
        }
        match self.delete_items(&gone, false, true).await {
            Ok(()) => {
                tracing::info!("removed {} downloads whose files were deleted", gone.len());
                gone.len()
            }
            Err(e) => {
                tracing::warn!("could not remove downloads whose files were deleted: {e:#}");
                0
            }
        }
    }

    /// Delete items. With `use_trash` (and not `permanent`) they move to Trash and can be
    /// restored; otherwise they are forgotten. `delete_files` also removes data on disk.
    pub async fn delete_items(
        &self,
        ids: &[String],
        delete_files: bool,
        permanent: bool,
    ) -> Result<()> {
        let use_trash = self.settings().general.use_trash && !permanent;
        let mut removed = Vec::new();
        for id in ids {
            let Some(item) = self.item(id) else { continue };
            let leaving_list = !(use_trash && !item.trashed);
            if leaving_list || delete_files {
                let job = self.0.state.lock().jobs.remove(id);
                if let Some(job) = job {
                    job.cancel(delete_files);
                }
                if item.kind == ItemKind::Torrent
                    && let (Some(t), Some(hash)) = (self.0.torrent.get(), jobs::torrent_hash(&item))
                {
                    let _ = t.remove(&hash, delete_files).await;
                }
            } else if let Some(job) = self.0.state.lock().jobs.get(id) {
                // Into the trash: pause (not cancel) so the resume point is saved and the
                // download continues where it stopped once it is restored.
                job.pause();
            }
            if delete_files {
                remove_item_files(&item);
            } else if leaving_list && item.status != Status::Completed {
                // A partial file can't be resumed once its item is gone: don't leave it behind.
                remove_partial_files(&item);
            }
            if !leaving_list {
                self.with_item(id, |i| {
                    i.trashed = true;
                    if i.status.is_active() || i.status == Status::Queued {
                        i.status = Status::Paused;
                    }
                    if delete_files {
                        i.downloaded = 0;
                        i.resume = None;
                        if i.status == Status::Completed {
                            i.status = Status::Idle;
                        }
                    }
                });
            } else {
                self.0.state.lock().items.remove(id);
                self.0.db.delete_download(id)?;
                removed.push(id.clone());
            }
        }
        if !removed.is_empty() {
            self.emit(CoreEvent::ItemsRemoved(removed));
        }
        Ok(())
    }

    pub fn restore_items(&self, ids: &[String]) {
        for id in ids {
            self.with_item(id, |i| i.trashed = false);
        }
    }

    pub async fn empty_trash(&self) -> Result<()> {
        let ids: Vec<String> = self
            .0
            .state
            .lock()
            .items
            .values()
            .filter(|i| i.trashed)
            .map(|i| i.id.clone())
            .collect();
        self.delete_items(&ids, false, true).await
    }

    /// Remove finished items from the list (files stay on disk).
    pub async fn remove_completed(&self) -> Result<()> {
        let ids: Vec<String> = self
            .0
            .state
            .lock()
            .items
            .values()
            .filter(|i| i.status == Status::Completed && !i.trashed)
            .map(|i| i.id.clone())
            .collect();
        self.delete_items(&ids, false, false).await
    }

    /// Download again from scratch (keeps settings, discards data).
    pub fn redownload(&self, id: &str) -> Result<()> {
        let _rt = crate::rt::enter();
        if let Some(job) = self.0.state.lock().jobs.remove(id) {
            job.cancel(true);
        }
        let item = self.item(id).ok_or_else(|| anyhow!("unknown item"))?;
        remove_item_files(&item);
        self.with_item(id, |i| {
            i.downloaded = 0;
            i.resume = None;
            i.status = Status::Idle;
            i.error = None;
            i.finished_at = None;
            i.trashed = false;
        });
        self.start_item(id)
    }

    pub fn item_details(&self, id: &str) -> Option<ItemDetails> {
        let st = self.0.state.lock();
        let item = st.items.get(id)?.clone();
        let job = st.jobs.get(id);
        Some(ItemDetails {
            segments: job
                .map(|j| j.segments())
                .unwrap_or_else(|| jobs::stored_segments(&item)),
            progress: job.map(|j| j.progress(&item)),
            speed_history: st
                .speed_history
                .get(id)
                .map(|h| h.iter().copied().collect())
                .unwrap_or_default(),
            item,
        })
    }

    // ----- background loops -------------------------------------------------------------------

    fn spawn_background(&self, mut outcomes: mpsc::UnboundedReceiver<JobOutcome>) {
        // Job outcomes.
        let core = self.clone();
        tokio::spawn(async move {
            while let Some(outcome) = outcomes.recv().await {
                core.handle_outcome(outcome).await;
            }
        });
        // Live progress at 4 Hz, speed history at 2 Hz.
        let core = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(250));
            let mut n: u64 = 0;
            loop {
                tick.tick().await;
                n += 1;
                core.progress_tick(n.is_multiple_of(2));
            }
        });
        // Checkpoints every 2 s.
        let core = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(2));
            loop {
                tick.tick().await;
                core.checkpoint().await;
            }
        });
        // Queue admission and schedules every second.
        let core = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            loop {
                tick.tick().await;
                core.queue_tick();
            }
        });
    }

    fn progress_tick(&self, sample_history: bool) {
        // Sent while the state is locked: a job that finishes meanwhile (handle_outcome takes the
        // same lock to remove it) announces its end only after this progress, never before it
        // (#41: late progress kept a finished row "downloading").
        let mut st = self.0.state.lock();
        let progress: Vec<Progress> = {
            let mut out = Vec::new();
            let ids: Vec<String> = st.jobs.keys().cloned().collect();
            for id in ids {
                let Some(item) = st.items.get(&id).cloned() else {
                    continue;
                };
                let p = st.jobs[&id].progress(&item);
                if sample_history {
                    let h = st.speed_history.entry(id.clone()).or_default();
                    h.push_back(p.speed);
                    while h.len() > SPEED_HISTORY {
                        h.pop_front();
                    }
                }
                if let Some(item) = st.items.get_mut(&id) {
                    item.downloaded = p.downloaded;
                    if p.size.is_some() {
                        item.size = p.size;
                    }
                    if item.status != p.status && p.status.is_active() {
                        item.status = p.status;
                    }
                }
                st.dirty.insert(id.clone());
                out.push(p);
            }
            out
        };
        if !progress.is_empty() {
            self.emit(CoreEvent::Progress(progress));
        }
        drop(st);
    }

    /// Persist resume state of running jobs. Each job's checkpoint flushes and fsyncs its
    /// data first, so the stored state never claims bytes that are not on disk.
    pub async fn checkpoint(&self) {
        let jobs: Vec<(String, Job)> = {
            let st = self.0.state.lock();
            st.jobs
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        };
        for (id, job) in jobs {
            if let Some(resume) = job.checkpoint().await {
                let mut st = self.0.state.lock();
                if let Some(item) = st.items.get_mut(&id) {
                    item.resume = Some(resume);
                }
            }
        }
        let dirty: Vec<DownloadItem> = {
            let mut st = self.0.state.lock();
            let ids: Vec<String> = st.dirty.drain().collect();
            ids.iter()
                .filter_map(|id| st.items.get(id).cloned())
                .collect()
        };
        if !dirty.is_empty()
            && let Err(e) = self.0.db.upsert_downloads(&dirty)
        {
            tracing::error!("checkpoint failed: {e}");
        }
    }

    async fn handle_outcome(&self, outcome: JobOutcome) {
        let id = outcome.id.clone();
        self.0.state.lock().jobs.remove(&id);
        platform_keep_awake(self);
        match outcome.result {
            jobs::JobResult::Completed { path, size } => {
                let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                self.with_item(&id, |i| {
                    i.status = Status::Processing;
                    if let Some(name) = file_name {
                        i.file_name = name;
                    }
                    if let Some(dir) = path.parent() {
                        i.save_dir = dir.to_path_buf();
                    }
                    i.size = Some(size);
                    i.downloaded = size;
                    i.resume = None;
                });
                let av = self.settings().antivirus;
                let scan = crate::antivirus::scan(&av, &path).await;
                let error = match &scan {
                    crate::antivirus::ScanResult::Infected { detail } => {
                        Some(format!("Antivirus: threat detected\n{detail}"))
                    }
                    _ => None,
                };
                let item = self.with_item(&id, |i| {
                    i.finished_at = Some(now_ms());
                    if let Some(e) = &error {
                        i.status = Status::Failed;
                        i.error = Some(e.clone());
                    } else {
                        i.status = Status::Completed;
                        i.error = None;
                        // Browser cookies and passwords were only needed to fetch the file.
                        forget_credentials(i);
                        #[cfg(unix)]
                        if i.kind == ItemKind::Video {
                            // Tools can leave the output executable; a video never should be.
                            use std::os::unix::fs::PermissionsExt;
                            let _ = std::fs::set_permissions(
                                &path,
                                std::fs::Permissions::from_mode(0o644),
                            );
                        }
                    }
                    i.clone()
                });
                if let Some(item) = item {
                    let s = self.settings();
                    if item.status == Status::Completed {
                        if s.sounds.enabled && s.sounds.on_complete {
                            self.emit(CoreEvent::PlaySound {
                                kind: "complete".into(),
                            });
                        }
                        self.emit(CoreEvent::Completed(item));
                    } else {
                        self.emit(CoreEvent::Failed(item));
                    }
                }
            }
            jobs::JobResult::Paused { resume } => {
                self.with_item(&id, |i| {
                    if resume.is_some() {
                        i.resume = resume;
                    }
                    if i.status != Status::Completed {
                        i.status = Status::Paused;
                    }
                });
            }
            jobs::JobResult::Cancelled => {}
            jobs::JobResult::Failed {
                error,
                resume,
                resumable,
            } => {
                let item = self.with_item(&id, |i| {
                    i.status = Status::Failed;
                    i.error = Some(error.clone());
                    if resume.is_some() {
                        i.resume = resume;
                    }
                    if let Some(r) = resumable {
                        i.resumable = Some(r);
                    }
                    i.clone()
                });
                if let Some(item) = item {
                    let s = self.settings();
                    if s.sounds.enabled && s.sounds.on_failed {
                        self.emit(CoreEvent::PlaySound {
                            kind: "failed".into(),
                        });
                    }
                    self.emit(CoreEvent::Failed(item));
                }
            }
        }
        // Persist immediately: completion/failure must survive a crash right after.
        let item = self.item(&id);
        if let Some(item) = item {
            let _ = self.0.db.upsert_download(&item);
        }
    }

    // ----- torrent & clipboard ------------------------------------------------------------------

    pub(crate) async fn torrent_engine(&self) -> Result<&prism_torrent::TorrentEngine> {
        let data_dir = self.0.data_dir.clone();
        let default_dir = self.settings().downloads.default_dir;
        let keep = self.torrents_to_keep();
        let engine = self
            .0
            .torrent
            .get_or_try_init(|| async move {
                prism_torrent::TorrentEngine::start(
                    &data_dir.join("torrent-session"),
                    default_dir,
                    &|hash| keep(hash),
                )
                .await
            })
            .await?;
        let s = self.settings();
        let global = if s.downloads.speed_limiter_enabled {
            s.downloads.speed_limit
        } else {
            0
        };
        engine.set_limits(global, global);
        Ok(engine)
    }

    /// Which saved torrents the session should load again: the ones of downloads that aren't
    /// finished. A finished torrent loaded again holds its files open and brings deleted ones
    /// back (#82). Torrents no download knows are dropped too, unless an unfinished torrent's
    /// hash isn't known yet (it might be that one).
    fn torrents_to_keep(&self) -> impl Fn(&str) -> bool + Send + Sync + 'static {
        let mut active = std::collections::HashSet::new();
        let mut finished = std::collections::HashSet::new();
        let mut unknown = false;
        for i in self.0.state.lock().items.values() {
            if i.kind != ItemKind::Torrent {
                continue;
            }
            match (jobs::torrent_hash(i), i.status == Status::Completed) {
                (Some(h), false) => {
                    active.insert(h.to_ascii_lowercase());
                }
                (Some(h), true) => {
                    finished.insert(h.to_ascii_lowercase());
                }
                (None, false) => unknown = true,
                (None, true) => {}
            }
        }
        move |hash: &str| active.contains(hash) || (unknown && !finished.contains(hash))
    }

    fn start_clipboard_monitor(&self) {
        let weak = Arc::downgrade(&self.0);
        let weak2 = weak.clone();
        let monitor = crate::clipboard::ClipboardMonitor::spawn(
            move || {
                let inner = weak.upgrade()?;
                let st = inner.state.lock();
                st.settings
                    .clipboard
                    .monitor
                    .then(|| st.settings.capture.extensions.clone())
            },
            move |urls| {
                if let Some(inner) = weak2.upgrade() {
                    let _ = inner.events.send(CoreEvent::ClipboardUrls(urls));
                }
            },
        );
        *self.0.clipboard.lock() = Some(monitor);
    }

    /// Flush everything before exit.
    /// Shut down for an app restart (in-app update): remember what was downloading or waiting
    /// in a queue, so [`Core::start`] continues it in the new version.
    pub async fn shutdown_for_restart(&self) {
        let running: Vec<(String, bool)> = {
            let st = self.0.state.lock();
            st.items
                .values()
                .filter(|i| !i.trashed && (i.status.is_active() || i.status == Status::Queued))
                .map(|i| (i.id.clone(), i.status == Status::Queued))
                .collect()
        };
        if let Ok(json) = serde_json::to_vec(&running) {
            let _ = std::fs::write(self.0.data_dir.join(RESUME_FILE), json);
        }
        self.shutdown().await;
    }

    fn resume_after_restart(&self) {
        let path = self.0.data_dir.join(RESUME_FILE);
        let Ok(data) = std::fs::read(&path) else {
            return;
        };
        let _ = std::fs::remove_file(&path);
        let Ok(list) = serde_json::from_slice::<Vec<(String, bool)>>(&data) else {
            return;
        };
        let mut start = Vec::new();
        for (id, queued) in list {
            let Some(item) = self.item(&id) else { continue };
            if item.trashed || item.status == Status::Completed {
                continue;
            }
            if queued {
                // Waiting items go back to their queue, which admits them as before.
                self.with_item(&id, |i| i.status = Status::Queued);
            } else {
                start.push(id);
            }
        }
        if !start.is_empty() {
            tracing::info!("continuing {} download(s) after restart", start.len());
            self.start_items(&start);
        }
    }

    pub async fn shutdown(&self) {
        let ids: Vec<String> = self.0.state.lock().jobs.keys().cloned().collect();
        self.checkpoint().await;
        self.pause_items(&ids);
        // Give jobs a moment to flush and report.
        for _ in 0..20 {
            if self.0.state.lock().jobs.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        self.checkpoint().await;
        if let Some(t) = self.0.torrent.get() {
            t.shutdown().await;
        }
        self.0.ipc.lock().take();
        platform::set_keep_awake(false);
        // Fold the write-ahead log back into prism.db so the file on disk is complete.
        self.0.db.checkpoint();
    }

    // ----- import / export -----------------------------------------------------------------------

    /// Export the download list (without resume data) as JSON.
    pub fn export_list(&self) -> Result<String> {
        let st = self.0.state.lock();
        let mut items: Vec<&DownloadItem> = st.items.values().filter(|i| !i.trashed).collect();
        items.sort_by_key(|i| i.added_at);
        let list: Vec<serde_json::Value> = items
            .iter()
            .map(|i| {
                serde_json::json!({
                    "url": i.url, "fileName": i.file_name, "saveDir": i.save_dir, "category": i.category,
                    "description": i.description, "referer": i.referer, "status": i.status, "size": i.size,
                    "checksum": i.checksum, "kind": i.kind,
                })
            })
            .collect();
        Ok(serde_json::to_string_pretty(
            &serde_json::json!({"app": "prism", "version": 1, "items": list}),
        )?)
    }

    /// Import a list exported by [`export_list`] or a plain text file with one URL per line.
    pub fn import_list(&self, text: &str) -> Result<usize> {
        let urls: Vec<ImportedEntry> =
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
                v["items"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|i| {
                                Some((
                                    i["url"].as_str()?.to_string(),
                                    i["fileName"].as_str().map(String::from),
                                    i["saveDir"].as_str().map(PathBuf::from),
                                    i["description"].as_str().map(String::from),
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                text.lines()
                    .map(str::trim)
                    .filter(|l| l.contains("://") || l.starts_with("magnet:"))
                    .map(|l| (l.to_string(), None, None, None))
                    .collect()
            };
        let count = urls.len();
        let entries = urls
            .into_iter()
            .map(|(url, name, dir, desc)| capture::NewItem {
                url,
                file_name: name,
                save_dir: dir,
                description: desc.unwrap_or_default(),
                ..Default::default()
            })
            .collect();
        self.add_items(entries, StartMode::Later)?;
        Ok(count)
    }

    pub fn export_settings(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(&self.settings())?)
    }

    pub fn import_settings(&self, text: &str) -> Result<Settings> {
        let s: Settings = serde_json::from_str(text).context("not a Prism settings file")?;
        self.replace_settings(s)
    }

    /// Zip logs, settings (without proxy password) and a redacted item summary.
    pub fn export_diagnostics(&self, target: &Path) -> Result<()> {
        use std::io::Write;
        let file = std::fs::File::create(target)?;
        let mut zip = zip::ZipWriter::new(file);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let mut settings = self.settings();
        settings.proxy.password = String::new();
        if !settings.hugging_face.token.is_empty() {
            settings.hugging_face.token = "(set)".into();
        }
        zip.start_file("settings.json", opts)?;
        zip.write_all(serde_json::to_string_pretty(&settings)?.as_bytes())?;
        let summary: Vec<serde_json::Value> = {
            let st = self.0.state.lock();
            st.items
                .values()
                .map(|i| {
                    let host = url::Url::parse(&i.url).ok().and_then(|u| u.host_str().map(String::from));
                    serde_json::json!({"id": i.id, "kind": i.kind, "host": host, "status": i.status,
                        "error": i.error, "size": i.size, "downloaded": i.downloaded, "resumable": i.resumable})
                })
                .collect()
        };
        zip.start_file("items.json", opts)?;
        zip.write_all(serde_json::to_string_pretty(&summary)?.as_bytes())?;
        let mut info = serde_json::json!({
            "version": crate::APP_VERSION, "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
            "ipcPort": self.ipc_port(),
        });
        if let Some(session) = crate::platform::desktop_session() {
            info["desktop"] = session;
        }
        zip.start_file("system.json", opts)?;
        zip.write_all(serde_json::to_string_pretty(&info)?.as_bytes())?;
        if let Ok(dir) = std::fs::read_dir(self.0.data_dir.join("logs")) {
            for entry in dir.flatten() {
                let path = entry.path();
                if path.is_file()
                    && let Ok(bytes) = std::fs::read(&path)
                {
                    zip.start_file(
                        format!("logs/{}", entry.file_name().to_string_lossy()),
                        opts,
                    )?;
                    zip.write_all(&bytes)?;
                }
            }
        }
        zip.finish()?;
        Ok(())
    }
}

/// `(url, file name, folder, description)` of an imported list entry.
type ImportedEntry = (String, Option<String>, Option<PathBuf>, Option<String>);

fn platform_keep_awake(core: &Core) {
    let busy = !core.0.state.lock().jobs.is_empty();
    platform::set_keep_awake(busy);
}

use crate::platform;

/// Make a user-typed file name safe for every supported file system.
pub fn sanitize_name(name: &str) -> String {
    prism_engine::filename::sanitize(name)
}

fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        // Different volume: copy + delete.
        Err(_) => {
            std::fs::copy(from, to)?;
            std::fs::remove_file(from)
        }
    }
}

fn remove_item_files(item: &DownloadItem) {
    let _ = std::fs::remove_file(item.path());
    remove_partial_files(item);
}

/// What was running before a restart for an update (see [`Core::shutdown_for_restart`]).
const RESUME_FILE: &str = "resume-after-restart.json";

/// Data folder 0700 and database files 0600 (they hold download history and settings).
fn restrict_permissions(data_dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(data_dir, std::fs::Permissions::from_mode(0o700));
        for f in ["prism.db", "prism.db-wal", "prism.db-shm", "settings.json"] {
            let p = data_dir.join(f);
            if p.exists() {
                let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
            }
        }
    }
    #[cfg(not(unix))]
    let _ = data_dir;
}

/// Drops session cookies, passwords and auth headers from a finished item's saved request.
fn forget_credentials(item: &mut DownloadItem) {
    let Some(req) = item.request.as_mut().and_then(|r| r.as_object_mut()) else {
        return;
    };
    for key in ["cookies", "auth"] {
        req.remove(key);
    }
    if let Some(headers) = req.get_mut("headers").and_then(|h| h.as_array_mut()) {
        headers.retain(|h| {
            let name = h.get(0).and_then(|n| n.as_str()).unwrap_or_default();
            !name.eq_ignore_ascii_case("cookie") && !name.eq_ignore_ascii_case("authorization")
        });
    }
}

/// Where the engine keeps the in-progress file (may differ from the item's current name).
fn resume_part_path(item: &DownloadItem) -> Option<PathBuf> {
    let r = item.resume.as_ref()?;
    r.get("part_path")
        .or_else(|| r.get("partPath"))
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
}

/// The in-progress file, its sidecar directory and a video's temp folder.
fn remove_partial_files(item: &DownloadItem) {
    if let Some(part) = resume_part_path(item) {
        // Only Prism's own partial files/folders (the path may come from an imported list).
        let name = part
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.ends_with(".prismpart.d") && part.is_dir() {
            let _ = std::fs::remove_dir_all(&part);
        } else if name.ends_with(".prismpart") {
            let _ = std::fs::remove_file(&part);
        }
    }
    let mut part = item.path().into_os_string();
    part.push(".prismpart");
    let part = PathBuf::from(part);
    let _ = std::fs::remove_file(&part);
    let mut side = part.into_os_string();
    side.push(".d");
    let _ = std::fs::remove_dir_all(PathBuf::from(side));
    if item.kind == ItemKind::Video {
        let _ = std::fs::remove_dir_all(jobs::video_temp_dir(item));
    }
}
