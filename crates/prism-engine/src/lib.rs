//! # prism-engine
//!
//! The download engine of Prism: dynamic multi-connection HTTP(S) and FTP(S) downloads with
//! IDM-style work stealing, crash-safe resume, live speed limiting, mirrors, authentication,
//! proxies, checksums, plus HLS and DASH stream downloads.
//!
//! ```no_run
//! # async fn demo() -> Result<(), prism_engine::EngineError> {
//! use prism_engine::{DownloadRequest, Engine, EngineConfig};
//!
//! let engine = Engine::new(EngineConfig::default());
//! let req = DownloadRequest::new("https://example.com/file.zip".parse().unwrap(), "/downloads");
//! let handle = engine.start(req, None, vec![]);
//! let mut events = handle.events();
//! tokio::spawn(async move { while let Ok(ev) = events.recv().await { println!("{ev:?}") } });
//! // Persist `handle.checkpoint().await` every few seconds for crash safety.
//! let done = handle.wait().await?;
//! println!("saved {} ({} bytes)", done.path.display(), done.size);
//! # Ok(()) }
//! ```
//!
//! See the crate README for the design (segmentation, connection handling, resume contract).

#![warn(missing_docs)]

mod backoff;
mod checksum;
mod config;
mod engine;
mod error;
pub mod filename;
mod ftp;
mod hostlimits;
mod http;
mod job;
mod limiter;
mod media;
mod progress;
mod request;
mod resume;
mod segmented;
mod speed;
mod storage;
mod transfer;
pub mod util;

pub use config::EngineConfig;
pub use engine::Engine;
pub use error::EngineError;
pub use hostlimits::HostLimit;
pub use job::DownloadHandle;
pub use limiter::{MAX_ACQUIRE, RateLimiter};
pub use media::{VariantInfo, list_variants};
pub use progress::{
    Completed, DownloadEvent, DownloadState, ProbeInfo, ProgressSnapshot, SegmentStatus,
    SegmentView,
};
pub use request::{
    AuthScheme, ChecksumAlgo, ConflictPolicy, Credentials, Destination, DownloadRequest,
    ExpectedChecksum, HttpMethod, HttpPreference, MediaKind, MediaRequest, ProxyConfig,
    VariantProbeRequest, VariantSelection,
};
pub use resume::{MediaResumeState, RESUME_STATE_VERSION, ResumeKind, ResumeState, SegmentState};
pub use url::Url;
