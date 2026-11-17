# prism-engine

The download engine of Prism: dynamic multi-connection HTTP(S) and FTP(S) downloads with
IDM-style work stealing, crash-safe resume, live speed limits, mirrors, authentication,
proxies and checksums, plus HLS and DASH stream downloads. It has no UI and no database. The
application core (`prism-core`) owns persistence and scheduling.

## Public API

```rust
use prism_engine::*;

// Engine (cheap to clone; jobs run on the calling Tokio runtime)
Engine::new(config: EngineConfig) -> Engine
Engine::config(&self) -> Arc<EngineConfig>             // shared snapshot of the current config
Engine::update_config(&self, config: EngineConfig)     // jobs started afterwards use it
Engine::global_limiter(&self) -> Arc<RateLimiter>
Engine::host_limits(&self) -> Vec<HostLimit>           // learned per-host connection limits
Engine::set_host_limit(&self, host: &str, connections: Option<u32>)
async Engine::probe(&self, req: &DownloadRequest) -> Result<ProbeInfo, EngineError>
Engine::start(&self, req: DownloadRequest, resume: Option<ResumeState>,
              extra_limiters: Vec<Arc<RateLimiter>>) -> DownloadHandle      // http(s), ftp(s)
Engine::start_media(&self, req: MediaRequest, resume: Option<ResumeState>,
                    extra_limiters: Vec<Arc<RateLimiter>>) -> DownloadHandle // HLS / DASH

// DownloadHandle (Clone; dropping handles does not stop the job)
id() -> Uuid
request_pause()                                  // fire-and-forget
async pause() -> Option<ResumeState>             // waits; returns the final durable state
cancel(delete_file: bool)
set_connections(n: u8)
set_speed_limit(bytes_per_sec: u64)  /  speed_limit() -> u64
async change_url(url: Url) -> Result<(), EngineError>   // "refresh address" (HTTP only)
snapshot() -> ProgressSnapshot                   // poll it, e.g. 4×/s
state() -> DownloadState
resume_state() -> Option<ResumeState>            // consistent, not necessarily fsynced
async checkpoint() -> Option<ResumeState>        // flush + fsync, then the state: persist this
events() -> broadcast::Receiver<DownloadEvent>   // the first call also gets events since start
is_finished() -> bool
async wait() -> Result<Completed, EngineError>   // Err(Paused) / Err(Cancelled) when stopped
```

Data types (all `serde` Serialize/Deserialize):

* `EngineConfig`: `user_agent`, `max_connections_cap` (32), `default_connections` (8),
  `initial_connections` (4), `min_split_size` (1 MiB), `min_request_size` (4 MiB),
  `max_request_size` (16 MiB), `write_buffer` (1 MiB), `memory_budget` (64 MiB),
  `connect_timeout` (15 s), `read_timeout` (30 s, stall detection), `max_retries` (10),
  `temp_suffix` (`.prismpart`), `default_proxy` (System), `max_connections_per_host`
  (None), `host_limit_ttl` (24 h), `ffmpeg_path` (None = `ffmpeg` on PATH).
* `DownloadRequest`: `url`, `mirrors`, `method` (Get/Post), `body: Option<String>`,
  `headers: Vec<(String,String)>`, `cookies`, `referer`, `user_agent`,
  `auth: Option<Credentials {username, password, scheme: Auto|Basic|Digest|Ntlm}>`,
  `proxy: Option<ProxyConfig>` (`None` = engine default; `ProxyConfig::None|System|Http{url,
  username,password}|Socks5{..}`),
  `destination: Destination {dir, file_name, conflict: Rename|Overwrite|Fail}`,
  `connections` (0 = default), `http: HttpPreference` (`Http1Only` default | `Auto` |
  `Http3Experimental`), `checksum: Option<ExpectedChecksum {algo: Md5|Sha1|Sha256, hex}>`,
  `speed_limit` (0 = unlimited), `max_retries`, `max_request_size` (for example 10 MiB for
  YouTube formats). `DownloadRequest::new(url, dir)` fills in the defaults.
* `MediaRequest`: `url`, `kind` (Auto|Hls|Dash), `variant` (Best | MaxHeight(h) |
  Index(i)), `headers`, `cookies`, `referer`, `user_agent`, `proxy`, `destination`,
  `connections` (parallel segments), `remux` (default true), `speed_limit`, `max_retries`.
* `ResumeState`: `version`, `kind` (Http|Ftp|Media), `url`, `final_url`, `part_path`,
  `final_path`, `total_size`, `etag`, `last_modified`, `resumable`,
  `segments: Vec<SegmentState {start, pos, end}>`, `media: Option<MediaResumeState>`. Also
  `to_json` / `from_json`, `downloaded()`, `validate()`, `verify_against_disk()` and
  `with_url(url)` (refresh the address of a paused download).
* `ProgressSnapshot {state, downloaded, total, speed (EWMA), eta, active_connections,
  target_connections, resumable, file_name, path, final_url, elapsed, segments:
  Vec<SegmentView {start, pos, end, speed, status: Active|Idle|Done|Retrying|Failed}>}`.
  For media jobs each `SegmentView` is one media segment (`start = index`).
* `DownloadEvent`: `Probed(ProbeInfo)`, `StateChanged{state}`, `SegmentSplit{at, end,
  race}`, `Retrying{reason, attempt, delay}`, `ServerLimitDetected{connections}`,
  `LinkExpired`, `UrlChanged{url}`, `Restarted{reason}`, `Completed(Completed)`,
  `Failed{error}`. Progress is not an event; poll `snapshot()`.
* `Completed {path, size, checksum_ok, elapsed, transferred, extra_files, warning}`.
* `EngineError` (thiserror, Clone, serde-tagged): `Http{status}`, `Network`, `Timeout`,
  `Io`, `NoSpace`, `ValidatorMismatch`, `RangeNotSupported`, `AuthRequired`, `LinkExpired`,
  `ChecksumMismatch{expected, actual, path}`, `Cancelled`, `Paused`, `InvalidUrl`,
  `FileExists`, `Ftp`, `Hls`, `Dash`, `Remux`, `Unsupported`, `InvalidResumeState`,
  `Internal`. It provides `is_transient()` and `user_message()`.
* `RateLimiter::new(bps)`, `unlimited()`, `set_rate(bps)`, `rate()`, `async acquire(bytes)`.
* `filename::{sanitize, parse_content_disposition, derive_file_name, name_from_url,
  extension_for_mime, numbered_name}` and `util::{expand_batch_pattern, BatchError,
  MAX_BATCH_EXPANSION}`.

## Design notes

**Probe without an extra round trip.** The first request is
`GET Range: bytes=0-<4 MiB-1>`. A `206` with a total size proves that ranges work, and its body
becomes worker 0's first stream. A `200` switches to a single connection, and a failure then
restarts from zero. POST downloads always use a single connection.

**Dynamic segmentation** (`segmented/table.rs`). The file is always partitioned into segments
`{start, pos, end}`. An idle worker takes an unowned segment first. Otherwise it splits the
owned segment with the largest remainder, but only if at least `2 × min_split` remains. The
split happens at the 4 KiB-aligned midpoint, or behind the victim's in-flight request when
that request reaches past the midpoint, so the victim's socket survives. When nothing can be
split, an idle worker races a slow tail (more than 3 s left and 4× slower than a typical
connection). Both workers write identical bytes and the first to reach `end` wins. One mutex
protects the table, taken once per network chunk.

**Connections.** Each worker owns its own `reqwest::Client`, HTTP/1.1 by default, and all
workers share one rustls config (aws-lc-rs plus the platform verifier), so TLS sessions are
resumed. Workers request bounded ranges sized to about 2 s of transfer (4–16 MiB) and reuse
their keep-alive connection for the next range. The tests check this through the test
server's per-connection request counts. Ramp-up: `initial_connections` start at once, and the
rest open as soon as those deliver data. Every worker holds a per-host permit (`HostLimits`,
shared engine-wide), which enforces learned limits and the optional global per-host cap.

**Server limits and retries.** An explicit refusal of an additional connection (`429`, `503`,
`403` after a successful probe, FTP `421`) lowers the target to the number of working
connections, records the limit for the host, emits `ServerLimitDetected` and later probes one
connection more, backing off exponentially (5 min after `Retry-After`). A reset or refusal
before any response counts as a limit signal only if a worker that never received data fails
twice in a row. Dropped connections after progress are re-opened immediately. Consecutive
failures without progress back off with `0.5 s·2ⁿ` (capped at 30 s, equal jitter) up to
`max_retries`. After `429`/`503`, every request covers the whole remaining segment, which
keeps the request count low. Stall detection drops a connection that delivers no data for
`read_timeout`.

**Disk I/O and crash safety** (`segmented/writer.rs`, `storage.rs`). Data goes straight into
`<name>.prismpart` at its final offset. One writer thread per download coalesces each
worker's chunks into 1 MiB positioned writes (`seek_write` / `write_all_at`). An engine-wide
64 MiB budget applies backpressure. On Windows the file is marked sparse before `set_len`;
on Linux it is `fallocate`d. Free space is checked first. `checkpoint()` is a barrier in the
writer: it flushes, fsyncs and then captures the segment states. A persisted state therefore
never claims bytes that are not durably on disk, and the crash test verifies this byte for
byte. On completion the engine fsyncs, checks the size, verifies the checksum and renames
atomically, honouring the conflict policy.

**Resume.** The first request carries `If-Range`: the strong ETag, else Last-Modified. A
`200`, a different total or a different strong ETag returns `ValidatorMismatch`. Without
validators, the engine re-downloads up to 64 KiB just before a resume point and compares it
with the bytes on disk.

**Speed limiting** (`limiter.rs`). The limiter uses a GCRA token bucket with a single FIFO
timeline per limiter. Slots are computed from the monotonic clock, never from sleep lengths.
The burst tolerance is 250 ms (at least 64 KiB), so oversleeping (for example macOS timer
coalescing) is credited back. Each acquire reserves at most 64 KiB, and a waiter sleeps once
per reservation. Workers acquire from `[download, extra…, global]`. All limits can be changed
live, and `0` means unlimited.

**Redirects, mirrors, URL refresh.** The engine follows up to 10 redirects and reuses the final
URL. On `403`/`404`/`410` it falls back to the original URL, which yields a fresh signed URL.
Mirrors are validated in the background (same size) and used round-robin. A failing mirror is
benched for 60 s. `change_url` validates the size and strong ETag, then every worker re-opens
on the new URL.

**Authentication.** Basic and Digest use the `http-auth` crate (MD5/SHA-256, `qop=auth`,
nonce counting). NTLM uses `ntlmclient`, with the 3-leg handshake on the worker's own
connection.

**FTP/FTPS** (`ftp.rs`, `suppaftp`). The engine runs `SIZE`, probes `REST` support and then
`REST`+`RETR` per segment on its own control connection, in passive mode with the NAT
workaround. FTPS uses explicit `AUTH TLS`. Credentials come from the URL, then the request,
then anonymous. Resume checks the size and `MDTM`.

**HLS/DASH** (`media/`). HLS uses `m3u8-rs`: variant selection, a separate audio rendition,
AES-128 (IV from the tag or the media sequence), `EXT-X-MAP` and byte ranges. DASH uses the
`dash-mpd-core` parser and resolves the segments itself: `SegmentTemplate` with
`$Number%0Nd$`/`$Time$`/`$Bandwidth$`, `SegmentTimeline`, `SegmentList`, and single-file
`SegmentBase`/`BaseURL`. Live, multi-period and DRM manifests return `Unsupported`. N
parallel workers write each segment to `<final>.prismpart.d/seg-NNNNNN.bin` (write, fsync,
rename), so resuming skips completed segments. Each track is then concatenated and muxed with
`ffmpeg -c copy` into `.mp4`. Without ffmpeg, or with `remux = false`, the raw `.ts` (HLS) or
fMP4 (DASH) is kept, any extra track is saved as `<name>.audio.<ext>`, and
`Completed::warning` says so.

**HTTP/3.** HTTP/3 sits behind the `http3` cargo feature. Build it with
`RUSTFLAGS="--cfg reqwest_unstable" cargo build -p prism-engine --features http3`.
`HttpPreference::Http3Experimental` then uses `http3_prior_knowledge()`. Without the
feature, it behaves like `Auto`.

## Tests and tools

```sh
cargo test -p prism-engine -p prism-testserver
cargo clippy -p prism-engine -p prism-testserver --all-targets -- -D warnings
```

The integration tests (`tests/http.rs`, `tests/media.rs`, `tests/ftp.rs`) run against
`prism-testserver` in-process, using random ports and temp dirs, and verify the SHA-256 of
every result. The ffmpeg-based media tests skip with a message when ffmpeg is missing. The
FTP tests embed a `libunftp` server.

Benchmarks:

```sh
cargo run --release -p prism-testserver -- --port 8080 --size 1G --throttle 4M
cargo run --release -p prism-engine --example prism-get -- http://127.0.0.1:8080/file/x.bin -c 16 -o /tmp
cargo run --release -p prism-engine --example prism-get -- <url> -c 8 --limit 5M --sha256 <hex>
```

## Known limitations

* NTLM is covered only by unit tests (message flow); there is no live NTLM server test.
* FTPS supports explicit TLS only (no implicit port 990). The tests cover plain FTP only.
* Proxies: client construction is tested, but no test runs against a live HTTP or SOCKS
  proxy. The system proxy means environment variables, the Windows registry and macOS
  settings. PAC/WPAD is not supported.
* HLS: no live playlists and no SAMPLE-AES. An `EXT-X-MAP` init section is decrypted with the
  AES-128 key in effect at its tag (none if the MAP comes first). Subtitles are ignored. `MediaRequest` has no credentials field (cookies and
  headers are supported).
* DASH: static, single-period, unencrypted manifests only. `SegmentBase` representations are
  fetched as one file (no `sidx` parsing).
* `change_url` works only for running HTTP downloads. For paused downloads use
  `ResumeState::with_url`.
* Shrinking `memory_budget` through `update_config` is best effort (only unused memory is
  released).
