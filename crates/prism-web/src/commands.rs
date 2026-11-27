//! HTTP equivalents of the Tauri `invoke` commands. Pure core calls are forwarded;
//! commands that need a desktop window or shell are answered with harmless no-ops.

use prism_core::manager::{Core, ItemPatch, NewItem, StartMode, VideoDecision};
use prism_core::events::{BatchEntry, CaptureSource};
use prism_core::model::PostAction;
use prism_torrent::TorrentSource;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::path::PathBuf;

fn arg<T: DeserializeOwned>(args: &Value, key: &str) -> Result<T, String> {
    let value = args.get(key).cloned().unwrap_or(Value::Null);
    serde_json::from_value(value).map_err(|e| format!("{key}: {e}"))
}

fn optional<T: DeserializeOwned>(args: &Value, key: &str) -> Result<Option<T>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => arg(args, key).map(Some),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddUrlArgs {
    url: String,
    referer: Option<String>,
    username: Option<String>,
    password: Option<String>,
    file_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StreamArgs {
    stream: prism_core::events::SniffedStream,
    page_url: Option<String>,
    title: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddTorrentArgs {
    source: TorrentSource,
    name: String,
    size: Option<u64>,
    only_files: Option<Vec<usize>>,
    save_dir: Option<PathBuf>,
    category: Option<String>,
    mode: Option<StartMode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HfPresentArgs {
    kind: prism_core::huggingface::RepoKind,
    id: String,
    sha: String,
    folder: PathBuf,
    files: Vec<prism_core::huggingface::RepoFile>,
}

pub async fn invoke(core: &Core, cmd: &str, args: Value) -> Result<Value, String> {
    let done = || Ok(Value::Null);
    match cmd {
        "bootstrap" => Ok(json!(core.bootstrap())),
        "update_settings" => Ok(json!(core
            .update_settings(&arg(&args, "patch")?)
            .map_err(|e| e.to_string())?)),
        "toggle_speed_limiter" => Ok(json!(core
            .toggle_speed_limiter()
            .map_err(|e| e.to_string())?)),

        "add_url" => {
            let a: AddUrlArgs = arg(&args, "args")?;
            let url = a.url.trim().to_string();
            if url.is_empty() {
                return Err("Enter an address".into());
            }
            if !(url.contains("://") || url.starts_with("magnet:")) {
                return Err("The address must start with http://, https://, ftp:// or magnet:".into());
            }
            let item = NewItem {
                url,
                referer: a.referer,
                username: a.username.filter(|u| !u.is_empty()),
                password: a.password,
                file_name: a.file_name.filter(|f| !f.is_empty()),
                ..Default::default()
            };
            Ok(json!(core.capture(item, CaptureSource::Manual)))
        }
        "get_capture" => Ok(json!(core.pending_capture(&arg::<String>(&args, "id")?))),
        "confirm_capture" => Ok(json!(core
            .confirm_capture(arg(&args, "decision")?)
            .map_err(|e| e.to_string())?)),
        "cancel_capture" => {
            core.cancel_capture(&arg::<String>(&args, "id")?);
            done()
        }
        "reprobe_capture" => {
            core.reprobe_capture(
                &arg::<String>(&args, "id")?,
                optional(&args, "username")?,
                optional(&args, "password")?,
            );
            done()
        }
        "batch_from_pattern" => Ok(json!(core
            .batch_from_pattern(&arg::<String>(&args, "pattern")?)
            .map_err(|e| e.to_string())?)),
        "batch_from_text" => {
            let text: String = arg(&args, "text")?;
            let entries: Vec<BatchEntry> = text
                .split_whitespace()
                .filter(|t| t.contains("://") || t.starts_with("magnet:"))
                .map(|u| BatchEntry {
                    url: u.to_string(),
                    ..Default::default()
                })
                .collect();
            if entries.is_empty() {
                return Err("No addresses found".into());
            }
            Ok(json!(core.open_batch(
                "Links from text".into(),
                None,
                entries
            )))
        }
        "get_batch" => Ok(json!(core.pending_batch(&arg::<String>(&args, "id")?))),
        "confirm_batch" => Ok(json!(core
            .confirm_batch(arg(&args, "decision")?)
            .map_err(|e| e.to_string())?
            .len())),
        "cancel_batch" => {
            core.cancel_batch(&arg::<String>(&args, "id")?);
            done()
        }
        "get_video" => Ok(json!(core.pending_video(&arg::<String>(&args, "id")?))),
        "open_video_url" => Ok(json!(core.open_video(
            arg::<String>(&args, "url")?,
            None,
            Vec::new(),
            None
        ))),
        "probe_video" => Ok(core
            .probe_video_for_window(&arg::<String>(&args, "url")?)
            .await
            .map_err(|e| format!("{e:#}"))?),
        "confirm_video" => {
            let d: VideoDecision = arg(&args, "decision")?;
            Ok(json!(core.confirm_video(d).map_err(|e| e.to_string())?.len()))
        }
        "cancel_video" => {
            core.cancel_video(&arg::<String>(&args, "id")?);
            done()
        }
        "add_stream" => {
            let a: StreamArgs = arg(&args, "args")?;
            Ok(json!(core
                .add_stream(a.stream, a.page_url, a.title, StartMode::Now)
                .map_err(|e| e.to_string())?))
        }
        "zip_contents" => Ok(json!(core
            .zip_contents(&arg::<String>(&args, "id")?)
            .await
            .map_err(|e| e.to_string())?)),
        "extract_zip_file" => Ok(json!(core
            .extract_zip_file(
                &arg::<String>(&args, "id")?,
                arg(&args, "index")?,
                &arg::<PathBuf>(&args, "dir")?
            )
            .await
            .map_err(|e| e.to_string())?)),
        "cancel_zip_file" => {
            core.cancel_zip_file(&arg::<String>(&args, "id")?, arg(&args, "index")?);
            done()
        }

        "inspect_torrent" => Ok(json!(core
            .inspect_torrent(arg(&args, "source")?)
            .await
            .map_err(|e| format!("{e:#}"))?)),
        "add_torrent" => {
            let a: AddTorrentArgs = arg(&args, "args")?;
            Ok(json!(core
                .add_torrent(
                    a.source,
                    a.name,
                    a.size,
                    a.only_files,
                    a.save_dir,
                    a.category,
                    a.mode.unwrap_or_default()
                )
                .map_err(|e| e.to_string())?))
        }

        "hf_context" => Ok(json!(core.hf_context())),
        "hf_repo" => Ok(json!(core
            .hf_repo(&arg::<String>(&args, "input")?)
            .await
            .map_err(|e| e.to_string())?)),
        "hf_search" => Ok(json!(core
            .hf_search(&arg::<String>(&args, "query")?)
            .await
            .map_err(|e| e.to_string())?)),
        "hf_present" => {
            let a: HfPresentArgs = arg(&args, "args")?;
            let core = core.clone();
            let present = tokio::task::spawn_blocking(move || {
                core.hf_present(a.kind, &a.id, &a.sha, &a.folder, &a.files)
            })
            .await
            .map_err(|e| e.to_string())?;
            Ok(json!(present))
        }
        "hf_add" => Ok(json!(core
            .hf_add(arg(&args, "args")?)
            .map_err(|e| e.to_string())?)),

        "media_inspect" => Ok(json!(core.media_inspect(arg(&args, "paths")?).await)),
        "media_plan" => Ok(json!(core.media_plan(
            &arg::<Vec<prism_core::media::MediaInfo>>(&args, "files")?,
            arg(&args, "targetBytes")?
        ))),
        "media_start" => Ok(json!(core
            .media_start(arg(&args, "request")?)
            .map_err(|e| e.to_string())?)),
        "media_cancel" => {
            core.media_cancel(&arg::<String>(&args, "id")?);
            done()
        }
        "media_jobs" => Ok(json!(core.media_jobs())),
        "media_clear" => Ok(json!(core.media_clear())),

        "start_items" => {
            core.start_items(&arg::<Vec<String>>(&args, "ids")?);
            done()
        }
        "pause_items" => {
            core.pause_items(&arg::<Vec<String>>(&args, "ids")?);
            done()
        }
        "pause_all" => {
            core.pause_all();
            done()
        }
        "resume_all" => {
            core.resume_all();
            done()
        }
        "delete_items" => {
            core.delete_items(
                &arg::<Vec<String>>(&args, "ids")?,
                arg(&args, "deleteFiles")?,
                arg(&args, "permanent")?,
            )
            .await
            .map_err(|e| e.to_string())?;
            done()
        }
        "restore_items" => {
            core.restore_items(&arg::<Vec<String>>(&args, "ids")?);
            done()
        }
        "empty_trash" => {
            core.empty_trash().await.map_err(|e| e.to_string())?;
            done()
        }
        "remove_completed" => {
            core.remove_completed().await.map_err(|e| e.to_string())?;
            done()
        }
        "redownload" => {
            core.redownload(&arg::<String>(&args, "id")?)
                .map_err(|e| e.to_string())?;
            done()
        }
        "update_item" => Ok(json!(core
            .update_item(&arg::<String>(&args, "id")?, arg::<ItemPatch>(&args, "patch")?)
            .map_err(|e| e.to_string())?)),
        "item_details" => Ok(json!(core.item_details(&arg::<String>(&args, "id")?))),
        "remove_deleted_files" => Ok(json!(core.remove_deleted_files().await)),

        "assign_to_queue" => {
            core.assign_to_queue(&arg::<Vec<String>>(&args, "ids")?, arg(&args, "queueId")?);
            done()
        }
        "move_in_queue" => {
            core.move_in_queue(&arg::<Vec<String>>(&args, "ids")?, arg(&args, "delta")?);
            done()
        }
        "upsert_queue" => {
            let q: prism_core::model::Queue = arg(&args, "queue")?;
            // Only harmless finish actions may come from a browser; shell and power actions stay desktop-only.
            if !matches!(q.schedule.on_complete, PostAction::None | PostAction::Sound) {
                return Err("queue finish action is not available in prism-web".into());
            }
            let r = core.upsert_queue(q).map_err(|e| e.to_string())?;
            core.arm_wake_timers();
            Ok(json!(r))
        }
        "delete_queue" => Ok(json!(core
            .delete_queue(&arg::<String>(&args, "id")?)
            .map_err(|e| e.to_string())?)),
        "start_queue" => {
            core.start_queue(&arg::<String>(&args, "id")?);
            done()
        }
        "stop_queue" => {
            core.stop_queue(&arg::<String>(&args, "id")?);
            done()
        }
        "next_schedule_runs" => Ok(json!(core.next_schedule_runs())),
        "cancel_post_action" => {
            core.cancel_post_action();
            done()
        }
        "upsert_category" => Ok(json!(core
            .upsert_category(arg(&args, "category")?)
            .map_err(|e| e.to_string())?)),
        "delete_category" => Ok(json!(core
            .delete_category(&arg::<String>(&args, "id")?)
            .map_err(|e| e.to_string())?)),

        "grabber_projects" => Ok(json!(core.grabber_projects())),
        "start_grabber" => Ok(json!(core
            .start_grabber(
                optional(&args, "projectId")?,
                arg(&args, "name")?,
                arg(&args, "config")?
            )
            .map_err(|e| e.to_string())?)),
        "stop_grabber" => {
            core.stop_grabber(&arg::<String>(&args, "projectId")?);
            done()
        }
        "delete_grabber_project" => {
            core.delete_grabber_project(&arg::<String>(&args, "projectId")?)
                .map_err(|e| e.to_string())?;
            done()
        }
        "download_grabbed" => Ok(json!(core
            .download_grabbed(
                &arg::<String>(&args, "projectId")?,
                &arg::<Vec<String>>(&args, "urls")?,
                optional(&args, "saveDir")?,
                arg(&args, "keepStructure")?,
                optional(&args, "mode")?.unwrap_or_default()
            )
            .map_err(|e| e.to_string())?
            .len())),

        "free_space" => {
            let path: PathBuf = arg(&args, "path")?;
            let state = core.bootstrap();
            let allowed = path.starts_with(core.data_dir())
                || path.starts_with(&state.settings.downloads.default_dir)
                || state.categories.iter().filter_map(|c| c.save_dir.as_ref()).any(|dir| path.starts_with(dir));
            if !allowed {
                return Err("free space is only available for Prism folders".into());
            }
            Ok(json!(prism_core::platform::available_space(&path)))
        }
        // Browser clients cannot pick local files; path-taking commands stay desktop-only.
        "read_torrent_file" | "export_list" | "import_list" | "export_settings"
        | "import_settings" | "export_diagnostics" => {
            Err("file commands are not available in prism-web".into())
        }

        "tool_status" => Ok(core.tool_status().await),
        "ensure_tools" => core.ensure_tools().await.map_err(|e| format!("{e:#}")),
        "update_ytdlp" => core
            .update_ytdlp()
            .await
            .map_err(|e| format!("{e:#}"))
            .and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string())),

        "integration_status" => Ok(integration_status(core)),
        "repair_integration" => Ok(integration_status(core)),

        // Desktop-only commands. The browser cannot open server files or windows.
        "native_material" => Ok(json!(false)),
        "check_update" => Ok(Value::Null),
        "install_update" => Err("updates are not available in prism-web".into()),
        "extension_dir" | "extension_folder" => Ok(Value::Null),
        "open_browser" | "open_extension_store" | "open_file" | "reveal_path" | "open_path"
        | "open_item" | "reveal_item" | "start_file_drag" | "open_details" | "open_window"
        | "open_tools" | "show_main" | "set_ui_language" | "play_sound" => done(),
        "toggle_mini" | "toggle_dropzone" | "set_autostart" => Ok(json!(false)),
        "main_window_starts_visible" => Ok(json!(true)),
        "app_run_id" => Ok(json!(1)),
        "quit_app" | "clipboard_write" => done(),
        "clipboard_read" => Ok(json!("")),
        "data_dir" => Ok(json!(core.data_dir())),

        other => Err(format!("unknown command: {other}")),
    }
}

fn integration_status(core: &Core) -> Value {
    let seen = core.extension_last_seen();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    json!({
        "browsers": [],
        "lastSeen": seen,
        "extensionConnected": seen.is_some_and(|t| now - t < 10 * 60 * 1000),
        "extensionVersion": core.extension_version(),
        "extensionInstallType": core.extension_install_type(),
        "extensionOutdated": core.extension_outdated(),
        "appVersion": prism_core::APP_VERSION,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dispatches_core_commands() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::start(prism_core::manager::CoreOptions {
            data_dir: dir.path().to_path_buf(),
            ipc: false,
            clipboard: false,
        })
        .await
        .unwrap();

        let boot = invoke(&core, "bootstrap", json!({})).await.unwrap();
        assert!(boot["items"].is_array());
        invoke(
            &core,
            "update_settings",
            json!({"patch": {"downloads": {"defaultDir": "/tmp/downloads"}}}),
        )
        .await
        .unwrap();
        assert_eq!(
            core.settings().downloads.default_dir,
            PathBuf::from("/tmp/downloads")
        );
        assert_eq!(
            invoke(&core, "native_material", json!({})).await.unwrap(),
            json!(false)
        );
        assert!(invoke(&core, "nope", json!({})).await.is_err());
        core.shutdown().await;
    }

    #[tokio::test]
    async fn web_rejects_unsafe_queue_finish_actions() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::start(prism_core::manager::CoreOptions {
            data_dir: dir.path().to_path_buf(),
            ipc: false,
            clipboard: false,
        })
        .await
        .unwrap();

        let queue = |action| {
            json!({
                "id": "web-test", "name": "Web queue", "maxConcurrent": 1,
                "speedLimit": 0, "position": 99, "builtin": false, "running": false,
                "schedule": {"enabled": true, "onComplete": action}
            })
        };
        let before = core.queues().len();
        for action in [
            json!({"type": "command", "value": "sh -c touch /tmp/pwned"}),
            json!({"type": "shutdown"}),
            json!({"type": "sleep"}),
            json!({"type": "lock"}),
            json!({"type": "exit"}),
        ] {
            assert!(invoke(&core, "upsert_queue", json!({"queue": queue(action)}))
                .await
                .is_err());
        }
        assert_eq!(core.queues().len(), before);

        let queue = queue(json!({"type": "sound"}));
        invoke(&core, "upsert_queue", json!({"queue": queue}))
            .await
            .unwrap();
        assert_eq!(core.queues().len(), before + 1);
        assert!(core
            .queues()
            .iter()
            .any(|q| q.schedule.on_complete == PostAction::Sound));
        core.shutdown().await;
    }

    #[tokio::test]
    async fn web_rejects_path_taking_commands() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::start(prism_core::manager::CoreOptions {
            data_dir: dir.path().to_path_buf(),
            ipc: false,
            clipboard: false,
        })
        .await
        .unwrap();
        let victim = dir.path().join("victim.txt");
        std::fs::write(&victim, b"keep").unwrap();

        for cmd in [
            "read_torrent_file",
            "export_list",
            "import_list",
            "export_settings",
            "import_settings",
            "export_diagnostics",
        ] {
            assert!(
                invoke(&core, cmd, json!({"path": victim.to_str().unwrap()}))
                    .await
                    .is_err(),
                "{cmd} should be unavailable over HTTP"
            );
        }
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
        assert!(invoke(&core, "free_space", json!({"path": victim})).await.is_ok());
        assert!(invoke(&core, "free_space", json!({"path": "/etc/passwd"})).await.is_err());
        core.shutdown().await;
    }
}
