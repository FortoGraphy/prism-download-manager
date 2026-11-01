//! `invoke` commands exposed to the UI. Thin wrappers over `prism_core::Core`.

use crate::windows;
use prism_core::events::{BatchInfo, CaptureInfo, CaptureSource, VideoCapture};
use prism_core::grabber::GrabberConfig;
use prism_core::manager::{
    BatchDecision, Bootstrap, CaptureDecision, Core, ItemDetails, ItemPatch, NewItem, StartMode,
    VideoDecision,
};
use prism_core::model::*;
use prism_core::settings::Settings;
use serde::Deserialize;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

pub(crate) type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[tauri::command]
pub async fn bootstrap(core: State<'_, Core>) -> CmdResult<Bootstrap> {
    Ok(core.bootstrap())
}

#[tauri::command]
pub async fn update_settings(
    core: State<'_, Core>,
    app: AppHandle,
    patch: serde_json::Value,
) -> CmdResult<Settings> {
    let settings = core.update_settings(&patch).map_err(err)?;
    crate::apply_shell_settings(&app, &settings);
    Ok(settings)
}

#[tauri::command]
pub async fn toggle_speed_limiter(core: State<'_, Core>, app: AppHandle) -> CmdResult<bool> {
    let on = core.toggle_speed_limiter().map_err(err)?;
    crate::tray::refresh(&app);
    Ok(on)
}

// ----- adding downloads ---------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddUrlArgs {
    url: String,
    referer: Option<String>,
    username: Option<String>,
    password: Option<String>,
    file_name: Option<String>,
}

/// Open the "Download File" dialog for a URL typed or pasted by the user.
#[tauri::command]
pub async fn add_url(core: State<'_, Core>, args: AddUrlArgs) -> CmdResult<CaptureInfo> {
    let url = args.url.trim().to_string();
    if url.is_empty() {
        return Err("Enter an address".into());
    }
    if !(url.contains("://") || url.starts_with("magnet:")) {
        return Err("The address must start with http://, https://, ftp:// or magnet:".into());
    }
    let n = NewItem {
        url,
        referer: args.referer,
        username: args.username.filter(|u| !u.is_empty()),
        password: args.password,
        file_name: args.file_name.filter(|f| !f.is_empty()),
        ..Default::default()
    };
    Ok(core.capture(n, CaptureSource::Manual))
}

#[tauri::command]
pub async fn get_capture(core: State<'_, Core>, id: String) -> CmdResult<Option<CaptureInfo>> {
    Ok(core.pending_capture(&id))
}

#[tauri::command]
pub async fn confirm_capture(
    core: State<'_, Core>,
    decision: CaptureDecision,
) -> CmdResult<DownloadItem> {
    core.confirm_capture(decision).map_err(err)
}

#[tauri::command]
pub async fn cancel_capture(core: State<'_, Core>, id: String) -> CmdResult<()> {
    core.cancel_capture(&id);
    Ok(())
}

#[tauri::command]
pub async fn reprobe_capture(
    core: State<'_, Core>,
    id: String,
    username: Option<String>,
    password: Option<String>,
) -> CmdResult<()> {
    core.reprobe_capture(&id, username, password);
    Ok(())
}

#[tauri::command]
pub async fn batch_from_pattern(core: State<'_, Core>, pattern: String) -> CmdResult<BatchInfo> {
    core.batch_from_pattern(&pattern).map_err(err)
}

/// Batch from pasted text / a text file: every URL found becomes an entry.
#[tauri::command]
pub async fn batch_from_text(core: State<'_, Core>, text: String) -> CmdResult<BatchInfo> {
    let entries: Vec<prism_core::events::BatchEntry> = text
        .split_whitespace()
        .filter(|t| t.contains("://") || t.starts_with("magnet:"))
        .map(|u| prism_core::events::BatchEntry {
            url: u.to_string(),
            ..Default::default()
        })
        .collect();
    if entries.is_empty() {
        return Err("No addresses found".into());
    }
    Ok(core.open_batch("Links from text".into(), None, entries))
}

#[tauri::command]
pub async fn get_batch(core: State<'_, Core>, id: String) -> CmdResult<Option<BatchInfo>> {
    Ok(core.pending_batch(&id))
}

#[tauri::command]
pub async fn confirm_batch(core: State<'_, Core>, decision: BatchDecision) -> CmdResult<usize> {
    core.confirm_batch(decision).map(|v| v.len()).map_err(err)
}

#[tauri::command]
pub async fn cancel_batch(core: State<'_, Core>, id: String) -> CmdResult<()> {
    core.cancel_batch(&id);
    Ok(())
}

#[tauri::command]
pub async fn get_video(core: State<'_, Core>, id: String) -> CmdResult<Option<VideoCapture>> {
    Ok(core.pending_video(&id))
}

/// Open the video quality picker for a page URL typed by the user.
#[tauri::command]
pub async fn open_video_url(core: State<'_, Core>, url: String) -> CmdResult<VideoCapture> {
    Ok(core.open_video(url, None, Vec::new(), None))
}

#[tauri::command]
pub async fn probe_video(core: State<'_, Core>, url: String) -> CmdResult<serde_json::Value> {
    // The browser tab's session when the window came from its in-page button.
    core.probe_video_for_window(&url)
        .await
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn confirm_video(core: State<'_, Core>, decision: VideoDecision) -> CmdResult<usize> {
    core.confirm_video(decision).map(|v| v.len()).map_err(err)
}

#[tauri::command]
pub async fn cancel_video(core: State<'_, Core>, id: String) -> CmdResult<()> {
    core.cancel_video(&id);
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamArgs {
    stream: prism_core::events::SniffedStream,
    page_url: Option<String>,
    title: Option<String>,
}

#[tauri::command]
pub async fn add_stream(core: State<'_, Core>, args: StreamArgs) -> CmdResult<DownloadItem> {
    core.add_stream(args.stream, args.page_url, args.title, StartMode::Now)
        .map_err(err)
}

#[tauri::command]
pub async fn inspect_torrent(
    core: State<'_, Core>,
    source: prism_torrent::TorrentSource,
) -> CmdResult<prism_torrent::TorrentMeta> {
    core.inspect_torrent(source)
        .await
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn read_torrent_file(path: PathBuf) -> CmdResult<prism_torrent::TorrentSource> {
    let bytes = tokio::fs::read(&path).await.map_err(err)?;
    Ok(prism_torrent::TorrentSource::File(bytes))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddTorrentArgs {
    source: prism_torrent::TorrentSource,
    name: String,
    size: Option<u64>,
    only_files: Option<Vec<usize>>,
    save_dir: Option<PathBuf>,
    category: Option<String>,
    mode: Option<StartMode>,
}

#[tauri::command]
pub async fn add_torrent(core: State<'_, Core>, args: AddTorrentArgs) -> CmdResult<DownloadItem> {
    core.add_torrent(
        args.source,
        args.name,
        args.size,
        args.only_files,
        args.save_dir,
        args.category,
        args.mode.unwrap_or_default(),
    )
    .map_err(err)
}

// ----- "What's inside" a ZIP in the Download File window -------------------------------------

/// The files in the ZIP a Download File window is about to download. Errors are codes the
/// window explains (`zip:noRanges`, `zip:notZip`, `zip:protected`, …).
#[tauri::command]
pub async fn zip_contents(
    core: State<'_, Core>,
    id: String,
) -> CmdResult<Vec<prism_core::manager::ZipFile>> {
    core.zip_contents(&id).await.map_err(err)
}

/// Downloads one file of that ZIP into `dir`; returns where it was saved.
#[tauri::command]
pub async fn extract_zip_file(
    core: State<'_, Core>,
    id: String,
    index: u32,
    dir: PathBuf,
) -> CmdResult<PathBuf> {
    core.extract_zip_file(&id, index, &dir).await.map_err(err)
}

#[tauri::command]
pub async fn cancel_zip_file(core: State<'_, Core>, id: String, index: u32) -> CmdResult<()> {
    core.cancel_zip_file(&id, index);
    Ok(())
}

// ----- Hugging Face models (#29) ------------------------------------------------------------

/// Whether the OS puts a material (Mica / vibrancy) behind the window (see main.rs).
#[tauri::command]
pub fn native_material() -> bool {
    crate::native_material_supported()
}

#[tauri::command]
pub async fn hf_context(core: State<'_, Core>) -> CmdResult<prism_core::huggingface::Context> {
    Ok(core.hf_context())
}

/// Errors are codes the window explains (`hf:notFound`, `hf:needsToken`, …).
#[tauri::command]
pub async fn hf_repo(
    core: State<'_, Core>,
    input: String,
) -> CmdResult<prism_core::huggingface::Repo> {
    core.hf_repo(&input).await.map_err(err)
}

#[tauri::command]
pub async fn hf_search(
    core: State<'_, Core>,
    query: String,
) -> CmdResult<Vec<prism_core::huggingface::Hit>> {
    core.hf_search(&query).await.map_err(err)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HfPresentArgs {
    kind: prism_core::huggingface::RepoKind,
    id: String,
    sha: String,
    folder: PathBuf,
    files: Vec<prism_core::huggingface::RepoFile>,
}

#[tauri::command]
pub async fn hf_present(
    core: State<'_, Core>,
    args: HfPresentArgs,
) -> CmdResult<prism_core::huggingface::Present> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        core.hf_present(args.kind, &args.id, &args.sha, &args.folder, &args.files)
    })
    .await
    .map_err(err)
}

#[tauri::command]
pub async fn hf_add(
    core: State<'_, Core>,
    args: prism_core::huggingface::AddModel,
) -> CmdResult<usize> {
    core.hf_add(args).map_err(err)
}

// ----- Tools: convert, make smaller, ZIP -----------------------------------------------------

/// Read files for the Tools window (ffmpeg is downloaded first if there's none).
#[tauri::command]
pub async fn media_inspect(
    core: State<'_, Core>,
    paths: Vec<PathBuf>,
) -> CmdResult<Vec<prism_core::media::MediaInfo>> {
    Ok(core.media_inspect(paths).await)
}

/// The "Make smaller" estimate (no work done).
#[tauri::command]
pub fn media_plan(
    core: State<'_, Core>,
    files: Vec<prism_core::media::MediaInfo>,
    target_bytes: u64,
) -> Vec<prism_core::media::ShrinkPlan> {
    core.media_plan(&files, target_bytes)
}

#[tauri::command]
pub async fn media_start(
    core: State<'_, Core>,
    request: prism_core::media::MediaRequest,
) -> CmdResult<Vec<prism_core::media::MediaJob>> {
    core.media_start(request).map_err(err)
}

#[tauri::command]
pub fn media_cancel(core: State<'_, Core>, id: String) {
    core.media_cancel(&id);
}

#[tauri::command]
pub fn media_jobs(core: State<'_, Core>) -> Vec<prism_core::media::MediaJob> {
    core.media_jobs()
}

#[tauri::command]
pub fn media_clear(core: State<'_, Core>) -> Vec<prism_core::media::MediaJob> {
    core.media_clear()
}

/// Open (or bring forward) the Tools window: `{tab, paths?, preset?}`.
#[tauri::command]
pub async fn open_tools(app: AppHandle, request: serde_json::Value) -> CmdResult<()> {
    windows::open_tools(&app, &request).map_err(err)
}

/// Open a file (a Tools result). A file that's gone is an error, not a new folder.
#[tauri::command]
pub async fn open_file(app: AppHandle, path: PathBuf) -> CmdResult<()> {
    if !path.is_file() {
        return Err("The file isn’t there any more".into());
    }
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(err)
}

/// Show a file in its folder (or the folder, if the file is gone).
#[tauri::command]
pub async fn reveal_path(app: AppHandle, path: PathBuf) -> CmdResult<()> {
    if path.exists() {
        return app.opener().reveal_item_in_dir(&path).map_err(err);
    }
    let dir = path
        .parent()
        .filter(|d| d.is_dir())
        .ok_or("The folder isn’t there any more")?;
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(err)
}

// ----- item control -----------------------------------------------------------------------

#[tauri::command]
pub async fn start_items(core: State<'_, Core>, ids: Vec<String>) -> CmdResult<()> {
    core.start_items(&ids);
    Ok(())
}

#[tauri::command]
pub async fn pause_items(core: State<'_, Core>, ids: Vec<String>) -> CmdResult<()> {
    core.pause_items(&ids);
    Ok(())
}

#[tauri::command]
pub async fn pause_all(core: State<'_, Core>) -> CmdResult<()> {
    core.pause_all();
    Ok(())
}

#[tauri::command]
pub async fn resume_all(core: State<'_, Core>) -> CmdResult<()> {
    core.resume_all();
    Ok(())
}

#[tauri::command]
pub async fn delete_items(
    core: State<'_, Core>,
    ids: Vec<String>,
    delete_files: bool,
    permanent: bool,
) -> CmdResult<()> {
    core.delete_items(&ids, delete_files, permanent)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn restore_items(core: State<'_, Core>, ids: Vec<String>) -> CmdResult<()> {
    core.restore_items(&ids);
    Ok(())
}

#[tauri::command]
pub async fn empty_trash(core: State<'_, Core>) -> CmdResult<()> {
    core.empty_trash().await.map_err(err)
}

#[tauri::command]
pub async fn remove_completed(core: State<'_, Core>) -> CmdResult<()> {
    core.remove_completed().await.map_err(err)
}

#[tauri::command]
pub async fn redownload(core: State<'_, Core>, id: String) -> CmdResult<()> {
    core.redownload(&id).map_err(err)
}

#[tauri::command]
pub async fn update_item(
    core: State<'_, Core>,
    id: String,
    patch: ItemPatch,
) -> CmdResult<DownloadItem> {
    core.update_item(&id, patch).map_err(err)
}

#[tauri::command]
pub async fn item_details(core: State<'_, Core>, id: String) -> CmdResult<Option<ItemDetails>> {
    Ok(core.item_details(&id))
}

#[tauri::command]
pub async fn open_item(core: State<'_, Core>, app: AppHandle, id: String) -> CmdResult<()> {
    let item = core.item(&id).ok_or("unknown item")?;
    app.opener()
        .open_path(item.path().to_string_lossy(), None::<&str>)
        .map_err(err)
}

#[tauri::command]
pub async fn reveal_item(core: State<'_, Core>, app: AppHandle, id: String) -> CmdResult<()> {
    let item = core.item(&id).ok_or("unknown item")?;
    let path = item.path();
    if path.exists() {
        app.opener().reveal_item_in_dir(&path).map_err(err)
    } else {
        std::fs::create_dir_all(&item.save_dir).ok();
        app.opener()
            .open_path(item.save_dir.to_string_lossy(), None::<&str>)
            .map_err(err)
    }
}

#[tauri::command]
pub async fn open_path(app: AppHandle, path: PathBuf) -> CmdResult<()> {
    if !path.exists() {
        std::fs::create_dir_all(&path).map_err(err)?;
    }
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(err)
}

// ----- queues & categories ----------------------------------------------------------------

#[tauri::command]
pub async fn assign_to_queue(
    core: State<'_, Core>,
    ids: Vec<String>,
    queue_id: Option<String>,
) -> CmdResult<()> {
    core.assign_to_queue(&ids, queue_id);
    Ok(())
}

#[tauri::command]
pub async fn move_in_queue(core: State<'_, Core>, ids: Vec<String>, delta: i64) -> CmdResult<()> {
    core.move_in_queue(&ids, delta);
    Ok(())
}

#[tauri::command]
pub async fn upsert_queue(
    core: State<'_, Core>,
    app: AppHandle,
    queue: Queue,
) -> CmdResult<Vec<Queue>> {
    let r = core.upsert_queue(queue).map_err(err);
    core.arm_wake_timers();
    let _ = app;
    r
}

#[tauri::command]
pub async fn delete_queue(core: State<'_, Core>, id: String) -> CmdResult<Vec<Queue>> {
    core.delete_queue(&id).map_err(err)
}

#[tauri::command]
pub async fn start_queue(core: State<'_, Core>, id: String) -> CmdResult<()> {
    core.start_queue(&id);
    Ok(())
}

#[tauri::command]
pub async fn stop_queue(core: State<'_, Core>, id: String) -> CmdResult<()> {
    core.stop_queue(&id);
    Ok(())
}

#[tauri::command]
pub async fn next_schedule_runs(core: State<'_, Core>) -> CmdResult<Vec<(String, Option<String>)>> {
    Ok(core.next_schedule_runs())
}

#[tauri::command]
pub async fn cancel_post_action(core: State<'_, Core>) -> CmdResult<()> {
    core.cancel_post_action();
    Ok(())
}

#[tauri::command]
pub async fn upsert_category(
    core: State<'_, Core>,
    category: Category,
) -> CmdResult<Vec<Category>> {
    core.upsert_category(category).map_err(err)
}

#[tauri::command]
pub async fn delete_category(core: State<'_, Core>, id: String) -> CmdResult<Vec<Category>> {
    core.delete_category(&id).map_err(err)
}

// ----- grabber ------------------------------------------------------------------------------

#[tauri::command]
pub async fn grabber_projects(core: State<'_, Core>) -> CmdResult<Vec<GrabberProject>> {
    Ok(core.grabber_projects())
}

#[tauri::command]
pub async fn start_grabber(
    core: State<'_, Core>,
    project_id: Option<String>,
    name: String,
    config: GrabberConfig,
) -> CmdResult<GrabberProject> {
    core.start_grabber(project_id, name, config).map_err(err)
}

#[tauri::command]
pub async fn stop_grabber(core: State<'_, Core>, project_id: String) -> CmdResult<()> {
    core.stop_grabber(&project_id);
    Ok(())
}

#[tauri::command]
pub async fn delete_grabber_project(core: State<'_, Core>, project_id: String) -> CmdResult<()> {
    core.delete_grabber_project(&project_id).map_err(err)
}

#[tauri::command]
pub async fn download_grabbed(
    core: State<'_, Core>,
    project_id: String,
    urls: Vec<String>,
    save_dir: Option<PathBuf>,
    keep_structure: bool,
    mode: Option<StartMode>,
) -> CmdResult<usize> {
    core.download_grabbed(
        &project_id,
        &urls,
        save_dir,
        keep_structure,
        mode.unwrap_or(StartMode::Now),
    )
    .map(|v| v.len())
    .map_err(err)
}

// ----- import / export ----------------------------------------------------------------------

#[tauri::command]
pub async fn export_list(core: State<'_, Core>, path: PathBuf) -> CmdResult<()> {
    std::fs::write(path, core.export_list().map_err(err)?).map_err(err)
}

#[tauri::command]
pub async fn import_list(core: State<'_, Core>, path: PathBuf) -> CmdResult<usize> {
    let text = std::fs::read_to_string(path).map_err(err)?;
    core.import_list(&text).map_err(err)
}

#[tauri::command]
pub async fn export_settings(core: State<'_, Core>, path: PathBuf) -> CmdResult<()> {
    std::fs::write(path, core.export_settings().map_err(err)?).map_err(err)
}

#[tauri::command]
pub async fn import_settings(
    core: State<'_, Core>,
    app: AppHandle,
    path: PathBuf,
) -> CmdResult<Settings> {
    let text = std::fs::read_to_string(path).map_err(err)?;
    let s = core.import_settings(&text).map_err(err)?;
    crate::apply_shell_settings(&app, &s);
    Ok(s)
}

#[tauri::command]
pub async fn export_diagnostics(core: State<'_, Core>, path: PathBuf) -> CmdResult<()> {
    core.export_diagnostics(&path).map_err(err)
}

// ----- tools & integration -----------------------------------------------------------------

#[tauri::command]
pub async fn tool_status(core: State<'_, Core>) -> CmdResult<serde_json::Value> {
    Ok(core.tool_status().await)
}

#[tauri::command]
pub async fn ensure_tools(core: State<'_, Core>) -> CmdResult<serde_json::Value> {
    core.ensure_tools().await.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn update_ytdlp(core: State<'_, Core>) -> CmdResult<serde_json::Value> {
    core.update_ytdlp().await.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn integration_status(core: State<'_, Core>) -> CmdResult<serde_json::Value> {
    Ok(with_extension_state(crate::integration::status(), &core))
}

#[tauri::command]
pub async fn repair_integration(core: State<'_, Core>) -> CmdResult<serde_json::Value> {
    crate::integration::register()
        .map(|s| with_extension_state(s, &core))
        .map_err(err)
}

fn with_extension_state(mut status: serde_json::Value, core: &Core) -> serde_json::Value {
    let seen = core.extension_last_seen();
    status["lastSeen"] = serde_json::json!(seen);
    // The extension says hello on browser start and every few minutes while active.
    status["extensionConnected"] =
        serde_json::json!(seen.is_some_and(|t| now_ms() - t < 10 * 60 * 1000));
    // The browser keeps running an unpacked extension's old files until it is reloaded.
    status["extensionVersion"] = serde_json::json!(core.extension_version());
    status["extensionInstallType"] = serde_json::json!(core.extension_install_type());
    status["extensionOutdated"] = serde_json::json!(core.extension_outdated());
    status["appVersion"] = serde_json::json!(prism_core::APP_VERSION);
    status
}

#[tauri::command]
pub async fn extension_dir(app: AppHandle) -> CmdResult<Option<PathBuf>> {
    Ok(crate::integration::extension_dir(&app))
}

#[tauri::command]
pub async fn extension_folder(app: AppHandle, firefox: bool) -> CmdResult<Option<PathBuf>> {
    Ok(crate::integration::extension_folder(&app, firefox))
}

#[tauri::command]
pub async fn open_browser(browser: String) -> CmdResult<()> {
    crate::integration::open_browser(&browser)
}

/// Opens Prism's Chrome Web Store page in `browser`, or in the default browser when that one
/// can't be started.
/// Linux: plays a sound through the system's player (Settings › Sounds' preview); elsewhere the
/// window plays it itself.
#[tauri::command]
pub async fn play_sound(
    core: State<'_, Core>,
    kind: String,
    complete_sound: Option<String>,
) -> CmdResult<()> {
    #[cfg(target_os = "linux")]
    {
        let s = core.settings().sounds;
        let complete = complete_sound.unwrap_or(s.complete_sound);
        crate::sound::play(&kind, &complete, s.volume);
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (core, kind, complete_sound);
    Ok(())
}

#[tauri::command]
pub async fn open_extension_store(app: AppHandle, browser: String) -> CmdResult<()> {
    if crate::integration::open_extension_store(&browser).is_ok() {
        return Ok(());
    }
    app.opener()
        .open_url(crate::integration::store_url(&browser), None::<&str>)
        .map_err(err)
}

// ----- shell / windows ---------------------------------------------------------------------

#[tauri::command]
pub async fn open_details(app: AppHandle, id: String) -> CmdResult<()> {
    windows::open_details(&app, &id).map_err(err)
}

#[tauri::command]
pub async fn open_window(app: AppHandle, kind: String, id: Option<String>) -> CmdResult<()> {
    windows::open(&app, &kind, id.as_deref()).map_err(err)
}

#[tauri::command]
pub async fn toggle_mini(app: AppHandle) -> CmdResult<bool> {
    windows::toggle_mini(&app).map_err(err)
}

#[tauri::command]
pub async fn toggle_dropzone(app: AppHandle) -> CmdResult<bool> {
    windows::toggle_dropzone(&app).map_err(err)
}

#[tauri::command]
pub async fn main_window_starts_visible(core: State<'_, Core>) -> CmdResult<bool> {
    // The main window is created hidden and the UI shows it after its first paint, unless
    // Prism started in the background (autostart) or "Start minimized" is on: then it stays
    // in the tray until the user opens it. A main window created again (after it was closed in
    // the tray, see windows::release_main_later) is one the user asked for.
    use std::sync::atomic::Ordering;
    let background = crate::STARTED_IN_BACKGROUND.load(Ordering::Relaxed);
    Ok(crate::windows::MAIN_WANTED.load(Ordering::SeqCst)
        || (!background && !core.settings().general.start_minimized))
}

/// Identifies this run of Prism, so a main window created again after a while in the tray can
/// keep choices that last until Prism quits (an update postponed with "Later").
#[tauri::command]
pub fn app_run_id() -> u64 {
    static RUN: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *RUN.get_or_init(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(1)
    })
}

/// The language the UI shows (already resolved from the setting), for the tray and notifications.
#[tauri::command]
pub async fn set_ui_language(app: AppHandle, language: String) -> CmdResult<()> {
    if crate::i18n::set_language(&language) {
        crate::tray::relabel(&app);
    }
    Ok(())
}

/// Settings › General › "Remove downloads whose files were deleted" (the main window asks when it
/// comes to the front). Returns how many were removed.
#[tauri::command]
pub async fn remove_deleted_files(core: State<'_, Core>) -> CmdResult<usize> {
    Ok(core.remove_deleted_files().await)
}

#[tauri::command]
pub async fn show_main(app: AppHandle) -> CmdResult<()> {
    windows::show_main(&app);
    Ok(())
}

#[tauri::command]
pub async fn set_autostart(app: AppHandle, enabled: bool) -> CmdResult<bool> {
    use tauri_plugin_autostart::ManagerExt;
    let al = app.autolaunch();
    if enabled { al.enable() } else { al.disable() }.map_err(err)?;
    al.is_enabled().map_err(err)
}

#[tauri::command]
pub async fn quit_app(app: AppHandle) {
    crate::quit(&app).await
}

#[tauri::command]
pub async fn free_space(path: PathBuf) -> CmdResult<Option<u64>> {
    Ok(prism_core::platform::available_space(&path))
}

#[tauri::command]
pub async fn clipboard_read() -> CmdResult<String> {
    Ok({
        arboard::Clipboard::new()
            .and_then(|mut c| c.get_text())
            .unwrap_or_default()
    })
}

#[tauri::command]
pub async fn clipboard_write(text: String) -> CmdResult<()> {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(text))
        .map_err(err)
}

#[tauri::command]
pub async fn data_dir(app: AppHandle) -> CmdResult<PathBuf> {
    Ok(app.state::<Core>().data_dir().to_path_buf())
}
