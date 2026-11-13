# prism-native-host

Tiny Rust binary (≈1 MB, starts and answers in a few ms) that browsers start through
**native messaging** (`dev.prism.host`). It relays every message from the Prism extension
to the running Prism app over loopback HTTP and writes the app's reply back.

The crate is also a **library** the app core reuses (`default-features = false` drops the
HTTP client):

```toml
prism-native-host = { path = "../prism-native-host", default-features = false }
```

| Module | Use from the core |
|---|---|
| `registration` | `register(&RegisterOptions) -> Result<Vec<BrowserStatus>>`, `unregister(&[Browser], &Locations)`, `status(&[Browser], host_path, &Locations) -> Vec<BrowserStatus>` — the "Browser integration → Repair" page. Idempotent and fast: call `register` on every app start (self-healing). |
| `paths` | `data_dir()`, `ipc_file()`, `logs_dir()`, `manifests_dir()`, `app_executable()` — the location contract below. |
| `framing`, `protocol` | Native messaging framing and the envelope/error-code rules. |
| constants | `HOST_NAME = "dev.prism.host"`, `CHROME_EXTENSION_ID = "mmcdeiglkbfbicdjboocmhehodogilfl"`, `FIREFOX_EXTENSION_ID = "prism@prismdl.site"` (plus the older `prism@prism.dev` in `LEGACY_FIREFOX_EXTENSION_IDS`), `HOST_BINARY_NAME`, `APP_BINARY_NAME`. |

```rust
use prism_native_host::registration::{self, RegisterOptions, Browser};
let mut opts = RegisterOptions::new(host_exe_path);      // all browsers, default IDs
opts.chrome_extension_ids.push("<edge-store-id>".into()); // optional extra IDs
let statuses = registration::register(&opts)?;           // Vec<BrowserStatus>
// BrowserStatus { browser, name, installed, supported, registered, manifest_path,
//                 points_to_this_host, allowed, location, note }   (serde: camelCase)
```

## Location contract (the app core must match it)

App data dir:

| OS | Path |
|---|---|
| Windows | `%APPDATA%\Prism` |
| macOS | `~/Library/Application Support/Prism` |
| Linux / other Unix | `$XDG_DATA_HOME/prism` (if set and absolute), else `~/.local/share/prism` |
| any | **`PRISM_DATA_DIR`** env var overrides all of the above (tests, portable/dev setups) |

Files in the data dir:

| File | Written by | Content |
|---|---|---|
| `ipc.json` | app | `{"port": u16, "token": "<hex, 64 chars recommended>", "pid": u32, "version": "x.y.z"}`. Write it atomically (temp file + rename) **after** the IPC server listens; delete it on clean exit. The host `stat`s it on every request and re-reads it when it changes, so a removed file is detected instantly (a stale file pointing at a dead port costs one 600 ms connect timeout). |
| `app-path` | app (optional) | One line: absolute path of the app launcher. Needed for AppImage (mount path changes every run): the app copies the host to `<data dir>/bin/prism-native-host` (stable path for the manifests) and writes `$APPIMAGE` here. Harmless on other OSes. |
| `logs/native-host.log` (+ `.log.1`) | host | Rotated at 1 MiB. Debug lines only with `PRISM_HOST_DEBUG=1`. stdout is never used for anything but protocol frames. |
| `native-messaging/dev.prism.host.chromium.json`, `…firefox.json` | `register` (Windows) | Host manifests the registry keys point to. |

## Relay behaviour

1. Read a frame from stdin (`u32` native-endian length + UTF-8 JSON, ≤ 64 MiB).
2. Validate `{ "id": string|number, "type": string, ... }` (else reply `bad_json` / `bad_request`).
3. `type == "hostPing"` is answered locally: `{"id","ok":true,"host":{"version","dataDir"}}`.
4. Otherwise `POST http://127.0.0.1:<port>/v1/message` with `Authorization: Bearer <token>`,
   `Content-Type: application/json`, body = the **unchanged** message bytes. Timeout 20 s
   (`probeVideo` 90 s). Proxies are never used.
5. Reply to the browser = the app's JSON object with `"id"` set to the request id
   (non-object replies are wrapped as `{"ok":true,"data":…}`), must be < 1 MiB
   (else `reply_too_large`). A non-2xx answer whose body is a JSON object with `ok` is
   passed through; otherwise `app_error`. 401/403 → re-read `ipc.json` once, then `unauthorized`.
6. Every message is handled on its own thread, so a slow `probeVideo` never blocks a
   `download` on a long-lived `connectNative` port. Works the same for one-shot
   `sendNativeMessage`. On stdin EOF the host finishes in-flight requests and exits 0.

**Auto-launch:** if `ipc.json` is missing or the app does not accept the connection (and the
message does not carry `"noLaunch": true`), the host launches the app with `--background`
— detached, stdio to null, Windows `CREATE_NO_WINDOW | DETACHED_PROCESS |
CREATE_NEW_PROCESS_GROUP` (+ `CREATE_BREAKAWAY_FROM_JOB` when the browser's job allows it,
so the app survives the browser closing the port), own process group on Unix — then polls
for `ipc.json` + a successful `GET /v1/health` for up to **8 s**. Only one launch happens at a
time. App executable resolution order: env **`PRISM_APP_PATH`** → the path in
**`<data dir>/app-path`** (if the file and its target exist) → **`prism(.exe)` next to the
host binary** (on macOS the host sits in `Prism.app/Contents/MacOS/` next to `prism`).

`--background` contract for the app: start without showing a window (tray only), be
single-instance (forward to the running instance), start the IPC server, write `ipc.json`,
then serve the message the host retries.

### Error codes produced by the host

| code | meaning |
|---|---|
| `bad_json` | frame is not JSON (`id` is `null`) |
| `bad_request` | not an object / missing `id` / missing `type` |
| `app_not_running` | app not reachable and the message had `noLaunch: true` |
| `app_unreachable` | launch failed (binary missing) or the app did not come up within 8 s |
| `unauthorized` | the app rejected the token (even after re-reading `ipc.json`) |
| `app_error` | non-2xx answer without a structured body, or transport error mid-request |
| `bad_reply` | 2xx answer that is not JSON |
| `reply_too_large` | the app's reply would exceed the 1 MiB host→browser limit |
| `timeout` | the app accepted the connection but did not answer in time |
| `too_large` | the browser sent a frame > 64 MiB (the host then exits, the stream is desynchronized) |

## Command line

Browsers start the host as `prism-native-host chrome-extension://<id>/ [--parent-window=N]`
(Chromium) or `prism-native-host <manifest.json> <extension id>` (Firefox). Anything that
is not one of the subcommand keywords below runs the relay, so a browser can never trigger
a subcommand.

```
prism-native-host register   [--browsers all|chrome,edge,...] [--extension-id-chrome <id>]... \
                             [--extension-id-firefox <id>]... [--manifest-dir <dir>] [--host-path <exe>]
prism-native-host unregister [--browsers ...] [--manifest-dir <dir>]
prism-native-host status     [--browsers ...] [--manifest-dir <dir>] [--host-path <exe>]   # JSON
prism-native-host --version | --help
```

Browsers: `chrome`, `chromium`, `edge`, `brave`, `vivaldi`, `opera`, `chrome-for-testing`,
`firefox` (`all` = default). Default extension IDs: Chromium `mmcdeiglkbfbicdjboocmhehodogilfl`,
Firefox `prism@prismdl.site` (and `prism@prism.dev`, used before 1.1.6).

## Where registration writes

* **Windows** (per user, no admin): manifests in `<data dir>\native-messaging\`, registry
  default value of `HKCU\Software\<vendor>\NativeMessagingHosts\dev.prism.host`:
  Chrome, Brave, Vivaldi, Opera → `Google\Chrome` (they read Chrome's key); Edge →
  `Microsoft\Edge`; Chromium and Chrome for Testing → `Chromium` (verified with Playwright's
  Chrome for Testing 156); Firefox → `Mozilla`. Unregistering a subset of browsers keeps a
  key that a browser outside that subset shares (e.g. `--browsers brave` keeps Chrome's key).
* **Linux**: `dev.prism.host.json` in `$XDG_CONFIG_HOME` (default `~/.config`)
  `/google-chrome`, `/google-chrome-beta`, `/google-chrome-unstable` (Chrome Dev — every
  Chrome channel reads only its own dir), `/google-chrome-for-testing`, `/chromium`,
  `/microsoft-edge`, `/BraveSoftware/Brave-Browser`, `/vivaldi`, `/opera` + `/google-chrome`
  (Opera; unverified) `…/NativeMessagingHosts/`; Firefox `~/.mozilla/native-messaging-hosts/`.
  With no explicit `--browsers`, only browsers whose profile dir exists are registered.
  Flatpak browsers (`~/.var/app/<id>`) are reported `supported: false`; Snap Firefox works
  through the portal.
* **macOS**: `~/Library/Application Support/{Google/Chrome, Google/Chrome Dev,
  Google/Chrome Beta, Google/Chrome Canary, Google/Chrome for Testing, Chromium,
  Microsoft Edge, BraveSoftware/Brave-Browser, Vivaldi}/NativeMessagingHosts/` — each existing
  Chrome channel gets the manifest, because Chrome reads it only from the channel being run
  (registering only `Google/Chrome` makes Chrome Dev fail with "Specified native messaging
  host not found"). Opera → Chrome's dir + `Opera/` (unverified), Firefox →
  `~/Library/Application Support/Mozilla/NativeMessagingHosts/`.

## Tests

```
cargo test -p prism-native-host
```

* unit: framing (fragmented reads, 5 MiB frames, limits), argument detection (Chrome/Firefox
  invocations vs subcommands), data-dir and app-path resolution, manifest generation,
  a sandboxed register → status → unregister cycle (temp dir + `HKCU\Software\PrismTest<pid>`).
* `tests/relay.rs`: spawns the built binary against a std-only mock app (`PRISM_DATA_DIR`):
  relay + latency, malformed JSON, `hostPing`, concurrency (slow request doesn't block a fast
  one), oversized reply, structured app errors, 2 MiB incoming message, EOF → exit 0,
  `noLaunch` → `app_not_running`, missing app → `app_unreachable`, stale `ipc.json`,
  wrong token → `unauthorized`, and auto-launch of a fake app that publishes `ipc.json`.

Linux and macOS are type-checked with `cargo check --target x86_64-unknown-linux-gnu`
/ `x86_64-apple-darwin` (not executed there).
