//! # prism-video
//!
//! Video backend for Prism: a clean abstraction over **yt-dlp + ffmpeg (+ deno)**.
//!
//! * [`VideoService::ensure_tools`] / [`VideoService::update_ytdlp`] — managed, SHA-256
//!   verified tool installs (see [`tools`]).
//! * [`VideoService::probe`] — `yt-dlp -J` → [`Probe`] (single video with computed quality
//!   [`Choice`]s, or a flat playlist).
//! * [`VideoService::plan`] — [`Plan::Accelerated`] (the core downloads plain HTTP streams
//!   with its segmented engine, then calls [`VideoService::postprocess`]) or
//!   [`Plan::YtDlp`] (yt-dlp does everything, [`VideoService::run_ytdlp`]).
//!
//! The crate has no dependency on `prism-engine` / `prism-core`.

pub mod choices;
pub mod error;
pub mod ffmpeg;
pub mod model;
pub mod plan;
pub mod probe;
mod process;
pub mod progress;
pub mod sponsorblock;
pub mod template;
pub mod tools;
mod youtube_native;

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

pub use error::{Result, VideoError};
pub use ffmpeg::{
    AudioConversion, FfmpegInputs, Metadata, PostInput, PostProcess, SponsorBlockRequest,
};
pub use model::*;
pub use plan::{AcceleratedPlan, Plan, StreamRole, StreamSpec, YtDlpJob};
pub use progress::{Stage, YtProgress};
pub use tokio_util::sync::CancellationToken;
pub use tools::{Tool, ToolInfo, ToolPhase, ToolProgress, ToolSource, ToolStatus, UpdateOutcome};

use probe::ToolPaths;
use tokio::sync::mpsc::UnboundedSender;

/// The options for a retry after yt-dlp couldn't read the "Cookies from browser" cookies: a
/// locked or unreadable browser must not break videos that need no login.
fn without_browser_cookies(opts: &VideoOptions, err: &VideoError) -> VideoOptions {
    tracing::warn!("{err}; trying again without --cookies-from-browser");
    VideoOptions {
        cookies_from_browser: None,
        ..opts.clone()
    }
}

struct Inner {
    tools_dir: PathBuf,
    opts: RwLock<VideoOptions>,
    versions: tools::VersionCache,
    /// Serializes installs/updates.
    install_lock: tokio::sync::Mutex<()>,
}

/// Entry point used by the app core. Cheap to clone.
#[derive(Clone)]
pub struct VideoService {
    inner: Arc<Inner>,
}

impl VideoService {
    pub fn new(tools_dir: PathBuf, opts: VideoOptions) -> Self {
        tools::cleanup_tools_dir(&tools_dir);
        VideoService {
            inner: Arc::new(Inner {
                tools_dir,
                opts: RwLock::new(opts),
                versions: tools::VersionCache::default(),
                install_lock: tokio::sync::Mutex::new(()),
            }),
        }
    }

    pub fn set_options(&self, opts: VideoOptions) {
        if let Ok(mut o) = self.inner.opts.write() {
            *o = opts;
        }
    }

    pub fn options(&self) -> VideoOptions {
        self.inner
            .opts
            .read()
            .map(|o| o.clone())
            .unwrap_or_default()
    }

    pub fn tools_dir(&self) -> &Path {
        &self.inner.tools_dir
    }

    fn tool_paths(&self) -> ToolPaths {
        let o = self.options();
        let r = |t| tools::resolve(t, &self.inner.tools_dir, &o).0;
        ToolPaths {
            ytdlp: r(Tool::YtDlp),
            ffmpeg: r(Tool::Ffmpeg),
            ffprobe: r(Tool::Ffprobe),
            deno: r(Tool::Deno),
        }
    }

    fn require(&self, tool: Tool) -> Result<PathBuf> {
        tools::resolve(tool, &self.inner.tools_dir, &self.options())
            .0
            .ok_or(VideoError::ToolMissing(tool))
    }

    fn http(&self) -> Result<tools::Http> {
        tools::Http::new(&self.options())
    }

    // ------------------------------------------------------------------ tools

    /// Path, version and source of every tool (versions are cached per file).
    pub async fn tool_status(&self) -> ToolStatus {
        let o = self.options();
        let d = &self.inner.tools_dir;
        let c = &self.inner.versions;
        let (yt_dlp, ffmpeg, ffprobe, deno) = tokio::join!(
            tools::tool_info(Tool::YtDlp, d, &o, c),
            tools::tool_info(Tool::Ffmpeg, d, &o, c),
            tools::tool_info(Tool::Ffprobe, d, &o, c),
            tools::tool_info(Tool::Deno, d, &o, c),
        );
        ToolStatus {
            yt_dlp,
            ffmpeg,
            ffprobe,
            deno,
        }
    }

    /// Download every missing tool (SHA-256 verified). A system ffmpeg is used
    /// when present (unless `prefer_system_ffmpeg` is false).
    pub async fn ensure_tools(
        &self,
        progress: impl Fn(ToolProgress) + Send + Sync,
    ) -> Result<ToolStatus> {
        let _guard = self.inner.install_lock.lock().await;
        let o = self.options();
        let d = self.inner.tools_dir.clone();
        let http = self.http()?;
        let mut errors = Vec::new();

        if tools::resolve(Tool::YtDlp, &d, &o).0.is_none()
            && let Err(e) = tools::install_ytdlp(&http, &d, o.channel, &progress).await
        {
            errors.push(format!("yt-dlp: {e}"));
        }
        if tools::resolve(Tool::Deno, &d, &o).0.is_none()
            && let Err(e) = tools::install_deno(&http, &d, &progress).await
        {
            errors.push(format!("deno: {e}"));
        }
        let (ff, ff_src, _) = tools::resolve(Tool::Ffmpeg, &d, &o);
        let fp = tools::resolve(Tool::Ffprobe, &d, &o).0;
        let need_ffmpeg = ff.is_none()
            || fp.is_none()
            || (!o.prefer_system_ffmpeg && ff_src == ToolSource::System);
        if need_ffmpeg && let Err(e) = tools::install_ffmpeg(&http, &d, &progress).await {
            errors.push(format!("ffmpeg: {e}"));
        }
        drop(_guard);
        let status = self.tool_status().await;
        if errors.is_empty() {
            Ok(status)
        } else {
            Err(VideoError::Other(errors.join("; ")))
        }
    }

    /// Force a managed (re)install of one tool, regardless of PATH copies.
    /// `Ffprobe` installs ffmpeg + ffprobe together.
    pub async fn install_tool(
        &self,
        tool: Tool,
        progress: impl Fn(ToolProgress) + Send + Sync,
    ) -> Result<ToolInfo> {
        let _guard = self.inner.install_lock.lock().await;
        let o = self.options();
        let d = self.inner.tools_dir.clone();
        let http = self.http()?;
        match tool {
            Tool::YtDlp => {
                tools::install_ytdlp(&http, &d, o.channel, &progress).await?;
            }
            Tool::Deno => {
                tools::install_deno(&http, &d, &progress).await?;
            }
            Tool::Ffmpeg | Tool::Ffprobe => tools::install_ffmpeg(&http, &d, &progress).await?,
        }
        Ok(tools::tool_info(tool, &d, &o, &self.inner.versions).await)
    }

    /// Check the configured channel's latest release and replace the managed
    /// yt-dlp if its SHA-256 differs from the published one. Installs it when missing.
    pub async fn update_ytdlp(&self) -> Result<UpdateOutcome> {
        let _guard = self.inner.install_lock.lock().await;
        let o = self.options();
        let d = self.inner.tools_dir.clone();
        let (path, source, _) = tools::resolve(Tool::YtDlp, &d, &o);
        match source {
            ToolSource::System | ToolSource::Custom => {
                return Ok(UpdateOutcome::NotManaged { source, path });
            }
            ToolSource::Managed | ToolSource::Missing => {}
        }
        let http = self.http()?;
        let from = match &path {
            Some(p) => self.inner.versions.version(Tool::YtDlp, p).await,
            None => None,
        };
        let rel = tools::ytdlp_latest(&http, o.channel).await?;
        let expected = rel.preferred_sha256().unwrap_or_default().to_string();
        if let Some(p) = &path {
            // A single-file install differs from the unpacked build's archive, so it moves to
            // the unpacked build here (faster start). The archive's hash says nothing about files
            // removed since (an antivirus taking one of its libraries): one that doesn't start is
            // reinstalled.
            let runs = process::run_capture(
                Tool::YtDlp,
                p,
                ["--version"],
                std::time::Duration::from_secs(60),
            )
            .await
            .is_ok_and(|out| out.status.success());
            let dir = d.clone();
            let local = tokio::task::spawn_blocking(move || tools::installed_ytdlp_sha256(&dir))
                .await
                .map_err(|e| VideoError::Other(e.to_string()))?;
            if runs && local.is_some_and(|l| l.eq_ignore_ascii_case(&expected)) {
                return Ok(UpdateOutcome::UpToDate {
                    version: from.or(Some(rel.tag)),
                });
            }
        }
        tracing::info!(tag = %rel.tag, "updating yt-dlp");
        tools::install_ytdlp_release(&http, &d, &rel, &|_| {}).await?;
        let to = self
            .inner
            .versions
            .version(Tool::YtDlp, &tools::managed_path(&d, Tool::YtDlp))
            .await
            .or(Some(rel.tag));
        Ok(UpdateOutcome::Updated { from, to })
    }

    // ------------------------------------------------------------------ probe

    /// `yt-dlp -J` the URL. Playlists/channels (or `list=` URLs with
    /// `ctx.want_playlist`) are listed flat.
    pub async fn probe(&self, url: &str, ctx: &RequestContext) -> Result<Probe> {
        let opts = self.options();
        // Native YouTube fast path: formats straight from the Innertube API as the
        // logged-in session, no yt-dlp + deno. Declines silently per-format-risk.
        if opts.use_native_youtube
            && probe::is_youtube(url)
            && let Some(json) = youtube_native::try_resolve(url, ctx, &opts).await
            && let Ok(probe) = model::parse_probe_json(&json)
        {
            return Ok(probe);
        }
        match self.probe_with(url, ctx, &opts).await {
            Err(e) if e.is_cookie_read_failure() && opts.cookies_from_browser.is_some() => self
                .probe_with(url, ctx, &without_browser_cookies(&opts, &e))
                .await
                .map_err(|e2| e2.after_cookie_read_failure(&e)),
            r => r,
        }
    }

    async fn probe_with(
        &self,
        url: &str,
        ctx: &RequestContext,
        opts: &VideoOptions,
    ) -> Result<Probe> {
        let ytdlp = self.require(Tool::YtDlp)?;
        let mut target = url.to_string();
        for _ in 0..2 {
            let cookie_file = probe::write_cookie_file(&target, ctx)?;
            let args = self.build_args(
                &target,
                ctx,
                cookie_file.as_ref().map(|f| f.0.as_path()),
                probe::probe_args(&target, ctx),
                opts,
            );
            let json = probe::run_probe_json(&ytdlp, args).await?;
            match probe::redirect_url(&json) {
                Some(next) if next != target => target = next,
                _ => return model::parse_probe_json(&json),
            }
        }
        Err(VideoError::Other(format!(
            "too many redirects while probing {url}"
        )))
    }

    fn build_args(
        &self,
        url: &str,
        ctx: &RequestContext,
        cookie_file: Option<&Path>,
        tail: Vec<String>,
        o: &VideoOptions,
    ) -> Vec<String> {
        let mut a = probe::global_args(&self.tool_paths(), o);
        a.extend(probe::context_args(url, ctx, o, cookie_file));
        a.extend(tail);
        a
    }

    // ------------------------------------------------------------------ plan

    /// Recompute picker choices for another container.
    pub fn choices(&self, info: &VideoInfo, container: Container) -> Vec<Choice> {
        choices::compute_choices(info, container)
    }

    /// Accelerated plan when every needed format is a plain HTTP file, else a yt-dlp job.
    pub fn plan(
        &self,
        info: &VideoInfo,
        choice: &Choice,
        opts: &OutputOptions,
        ctx: &RequestContext,
    ) -> Plan {
        let o = self.options();
        plan::plan(
            info,
            choice,
            opts,
            ctx,
            o.user_agent.as_deref(),
            o.delete_temp_inputs,
            o.concurrent_fragments,
        )
    }

    /// yt-dlp job regardless of format protocols (fallback after an accelerated failure, e.g. 403).
    pub fn plan_ytdlp_forced(
        &self,
        info: &VideoInfo,
        choice: &Choice,
        opts: &OutputOptions,
        ctx: &RequestContext,
    ) -> YtDlpJob {
        plan::plan_ytdlp(
            info,
            choice,
            opts,
            ctx,
            self.options().concurrent_fragments,
            "forced".into(),
        )
    }

    /// Full yt-dlp argument list for a job (`cookie_file` = path of the temp cookies.txt, if any).
    pub fn ytdlp_command_line(&self, job: &YtDlpJob, cookie_file: Option<&Path>) -> Vec<String> {
        self.command_line_with(job, cookie_file, &self.options())
    }

    fn command_line_with(
        &self,
        job: &YtDlpJob,
        cookie_file: Option<&Path>,
        o: &VideoOptions,
    ) -> Vec<String> {
        let mut a = probe::global_args(&self.tool_paths(), o);
        a.extend(probe::context_args(&job.url, &job.context, o, cookie_file));
        a.extend(job.args.iter().cloned());
        a.extend(o.extra_ytdlp_args.iter().cloned());
        a.push("--".into());
        a.push(job.url.clone());
        a
    }

    // ------------------------------------------------------------------ run

    /// Run a yt-dlp job; progress is streamed to `progress`; returns the final file path.
    pub async fn run_ytdlp(
        &self,
        job: &YtDlpJob,
        progress: UnboundedSender<YtProgress>,
        cancel: CancellationToken,
    ) -> Result<PathBuf> {
        let opts = self.options();
        match self
            .run_ytdlp_with(job, progress.clone(), cancel.clone(), &opts)
            .await
        {
            // yt-dlp reads the cookies before it downloads anything, so a retry starts clean.
            Err(e) if e.is_cookie_read_failure() && opts.cookies_from_browser.is_some() => self
                .run_ytdlp_with(job, progress, cancel, &without_browser_cookies(&opts, &e))
                .await
                .map_err(|e2| e2.after_cookie_read_failure(&e)),
            r => r,
        }
    }

    async fn run_ytdlp_with(
        &self,
        job: &YtDlpJob,
        progress: UnboundedSender<YtProgress>,
        cancel: CancellationToken,
        opts: &VideoOptions,
    ) -> Result<PathBuf> {
        let ytdlp = self.require(Tool::YtDlp)?;
        if !job.output_dir.as_os_str().is_empty() {
            tokio::fs::create_dir_all(&job.output_dir).await?;
        }
        let cookie_file = probe::write_cookie_file(&job.url, &job.context)?;
        let args = self.command_line_with(job, cookie_file.as_ref().map(|f| f.0.as_path()), opts);
        let res = probe::run_job(&ytdlp, args, progress, cancel).await;
        drop(cookie_file);
        res
    }

    /// ffmpeg mux / convert / embed for the accelerated path. Progress is optional.
    pub async fn postprocess(
        &self,
        job: &PostProcess,
        progress: Option<UnboundedSender<YtProgress>>,
        cancel: CancellationToken,
    ) -> Result<PathBuf> {
        let ffmpeg = self.require(Tool::Ffmpeg)?;
        let client = tools::http_client(&self.options(), true)?;
        ffmpeg::postprocess(&ffmpeg, &client, job, progress.as_ref(), &cancel).await
    }
}
