//! Engine-wide configuration.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::request::ProxyConfig;

/// Engine-wide settings. Per-download settings live in [`crate::DownloadRequest`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EngineConfig {
    /// `User-Agent` used when the request does not specify one.
    pub user_agent: String,
    /// Hard upper bound for connections per download (default 32).
    pub max_connections_cap: u8,
    /// Connections used when a request asks for 0 (default 8).
    pub default_connections: u8,
    /// Connections opened immediately; the rest ramp up as connections prove healthy (default 4).
    pub initial_connections: u8,
    /// Minimum size of a split piece; a range is only split when at least twice this remains
    /// (default 1 MiB). Split points are 4 KiB aligned.
    pub min_split_size: u64,
    /// Smallest bounded range requested per HTTP request (default 4 MiB).
    pub min_request_size: u64,
    /// Largest bounded range requested per HTTP request (default 16 MiB). Requests are sized
    /// adaptively (~2 s of transfer at the connection's speed) within these bounds, so a split
    /// rarely aborts a socket and keep-alive connections are reused.
    pub max_request_size: u64,
    /// Write coalescing buffer per stream in the writer thread (default 1 MiB).
    pub write_buffer: usize,
    /// Global budget for received-but-not-yet-written data across all downloads (default
    /// 64 MiB). When exhausted, connections stop reading (TCP backpressure).
    pub memory_budget: usize,
    /// TCP/TLS connect timeout (default 15 s).
    pub connect_timeout: Duration,
    /// Stall detection: a connection that delivers no bytes for this long is dropped and
    /// re-opened (default 30 s).
    pub read_timeout: Duration,
    /// Consecutive failures without progress before a connection gives up (default 10).
    pub max_retries: u32,
    /// Suffix of in-progress files (default `.prismpart`).
    pub temp_suffix: String,
    /// Proxy used when a request does not specify one (default: system proxy).
    pub default_proxy: ProxyConfig,
    /// Optional global cap of simultaneous connections to one host across all downloads.
    pub max_connections_per_host: Option<u16>,
    /// How long a learned per-host connection limit is remembered (default 24 h).
    pub host_limit_ttl: Duration,
    /// ffmpeg executable for HLS/DASH remuxing (`None` = `ffmpeg` on `PATH`).
    pub ffmpeg_path: Option<PathBuf>,
    /// Global host-to-IP resolution overrides (custom DNS / DoH fallback).
    pub default_dns_overrides: Vec<(String, std::net::SocketAddr)>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            user_agent: format!("Prism/{}", env!("CARGO_PKG_VERSION")),
            max_connections_cap: 32,
            default_connections: 8,
            initial_connections: 4,
            min_split_size: 1 << 20,
            min_request_size: 4 << 20,
            max_request_size: 16 << 20,
            write_buffer: 1 << 20,
            memory_budget: 64 << 20,
            connect_timeout: Duration::from_secs(15),
            read_timeout: Duration::from_secs(30),
            max_retries: 10,
            temp_suffix: ".prismpart".into(),
            default_proxy: ProxyConfig::System,
            max_connections_per_host: None,
            host_limit_ttl: Duration::from_secs(24 * 3600),
            ffmpeg_path: None,
            default_dns_overrides: Vec::new(),
        }
    }
}

impl EngineConfig {
    /// Returns a copy with all values clamped into sane ranges.
    pub(crate) fn normalized(mut self) -> Self {
        self.max_connections_cap = self.max_connections_cap.clamp(1, 64);
        self.default_connections = self.default_connections.clamp(1, self.max_connections_cap);
        self.initial_connections = self.initial_connections.clamp(1, self.max_connections_cap);
        self.min_split_size = self.min_split_size.max(16 * 1024);
        self.min_request_size = self.min_request_size.max(64 * 1024);
        self.max_request_size = self.max_request_size.max(self.min_request_size);
        self.write_buffer = self.write_buffer.clamp(16 * 1024, 64 << 20);
        self.memory_budget = self.memory_budget.clamp(4 << 20, 1 << 30);
        if self.temp_suffix.is_empty() {
            self.temp_suffix = ".prismpart".into();
        }
        self
    }
}
