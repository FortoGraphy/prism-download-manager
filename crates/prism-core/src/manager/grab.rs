//! Site Grabber projects: run crawls, persist results, queue selected files.

use super::Core;
use super::capture::{NewItem, StartMode};
use crate::events::CoreEvent;
use crate::grabber::{self, CancelFlag, FoundFile, GrabberConfig, GrabberEvent};
use crate::model::{DownloadItem, GrabberProject, now_ms};
use anyhow::{Result, anyhow};
use std::path::PathBuf;
use tokio::sync::mpsc;

impl Core {
    pub fn grabber_projects(&self) -> Vec<GrabberProject> {
        self.0.db.grabber_projects().unwrap_or_default()
    }

    /// Create (or re-run) a project and start crawling. Events stream as `CoreEvent::Grabber`.
    pub fn start_grabber(
        &self,
        project_id: Option<String>,
        name: String,
        config: GrabberConfig,
    ) -> Result<GrabberProject> {
        let _rt = crate::rt::enter();
        let now = now_ms();
        let project = GrabberProject {
            id: project_id.unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string()),
            name: if name.trim().is_empty() {
                url::Url::parse(&config.start_url)
                    .ok()
                    .and_then(|u| u.host_str().map(String::from))
                    .unwrap_or_else(|| "Grabber project".into())
            } else {
                name
            },
            config: serde_json::to_value(&config)?,
            results: serde_json::json!({"files": [], "pages": 0, "running": true}),
            created_at: now,
            updated_at: now,
        };
        self.0.db.upsert_grabber_project(&project)?;
        let cancel = CancelFlag::default();
        if let Some(old) = self
            .0
            .state
            .lock()
            .grabbers
            .insert(project.id.clone(), cancel.clone())
        {
            old.cancel();
        }
        let (tx, mut rx) = mpsc::unbounded_channel();
        let cfg = config.clone();
        tokio::spawn(grabber::crawl(cfg, cancel, tx));
        let core = self.clone();
        let pid = project.id.clone();
        let mut stored = project.clone();
        tokio::spawn(async move {
            let mut files: Vec<FoundFile> = Vec::new();
            let mut pages = 0u32;
            while let Some(event) = rx.recv().await {
                match &event {
                    GrabberEvent::FileFound { file } => files.push(file.clone()),
                    GrabberEvent::PageVisited { pages: p, .. } => pages = *p,
                    GrabberEvent::Finished { .. } => {}
                    GrabberEvent::PageError { .. } => {}
                }
                let finished = matches!(event, GrabberEvent::Finished { .. });
                core.emit(CoreEvent::Grabber {
                    project_id: pid.clone(),
                    event,
                });
                if finished {
                    break;
                }
            }
            if config.fetch_sizes && !files.is_empty() {
                let start = url::Url::parse(config.start_url.trim()).ok();
                files = grabber::fetch_sizes(
                    files,
                    &config.user_agent,
                    &config.cookies,
                    start.as_ref(),
                )
                .await;
            }
            stored.results = serde_json::json!({"files": files, "pages": pages, "running": false});
            stored.updated_at = now_ms();
            if let Err(e) = core.0.db.upsert_grabber_project(&stored) {
                tracing::error!("save grabber project: {e}");
            }
            core.0.state.lock().grabbers.remove(&pid);
            core.emit(CoreEvent::Grabber {
                project_id: pid.clone(),
                event: GrabberEvent::Finished {
                    pages,
                    files: files.len() as u32,
                    cancelled: false,
                },
            });
        });
        Ok(project)
    }

    pub fn stop_grabber(&self, project_id: &str) {
        if let Some(flag) = self.0.state.lock().grabbers.get(project_id) {
            flag.cancel();
        }
    }

    pub fn delete_grabber_project(&self, project_id: &str) -> Result<()> {
        self.stop_grabber(project_id);
        self.0.db.delete_grabber_project(project_id)?;
        Ok(())
    }

    /// Queue the selected files of a project.
    pub fn download_grabbed(
        &self,
        project_id: &str,
        urls: &[String],
        save_dir: Option<PathBuf>,
        keep_structure: bool,
        mode: StartMode,
    ) -> Result<Vec<DownloadItem>> {
        let project = self
            .grabber_projects()
            .into_iter()
            .find(|p| p.id == project_id)
            .ok_or_else(|| anyhow!("unknown grabber project"))?;
        let files: Vec<FoundFile> =
            serde_json::from_value(project.results["files"].clone()).unwrap_or_default();
        let config: GrabberConfig =
            serde_json::from_value(project.config.clone()).unwrap_or_default();
        let wanted: std::collections::HashSet<&String> = urls.iter().collect();
        let start = url::Url::parse(config.start_url.trim()).ok();
        let entries = files
            .into_iter()
            .filter(|f| wanted.contains(&f.url))
            .map(|f| {
                // Keep the site's cookie and HTTP auth on the site's own hosts.
                let with_credentials = match (&start, url::Url::parse(&f.url).ok()) {
                    (Some(s), Some(u)) => grabber::credentials_allowed(s, &u),
                    _ => false,
                };
                let auth = with_credentials
                    .then(|| config.login.as_ref().filter(|l| l.form_url.is_none()))
                    .flatten();
                // Mirror the site's folder structure below the chosen directory.
                let dir = save_dir.as_ref().map(|base| {
                    if !keep_structure {
                        return base.clone();
                    }
                    let mut d = base.clone();
                    if let Ok(u) = url::Url::parse(&f.url) {
                        if let Some(host) = u.host_str() {
                            d.push(host);
                        }
                        let segments: Vec<&str> =
                            u.path_segments().map(|s| s.collect()).unwrap_or_default();
                        for seg in segments.iter().take(segments.len().saturating_sub(1)) {
                            d.push(super::sanitize_name(
                                &percent_encoding::percent_decode_str(seg).decode_utf8_lossy(),
                            ));
                        }
                    }
                    d
                });
                NewItem {
                    url: f.url,
                    file_name: Some(f.file_name),
                    save_dir: dir,
                    referer: Some(f.referer),
                    size: f.size,
                    cookies: (!config.cookies.is_empty() && with_credentials)
                        .then(|| config.cookies.clone()),
                    username: auth.map(|l| l.username.clone()),
                    password: auth.map(|l| l.password.clone()),
                    grabber_project: Some(project_id.to_string()),
                    conflict: Some(crate::settings::ConflictAction::Rename),
                    ..Default::default()
                }
            })
            .collect();
        self.add_items(entries, mode)
    }
}
