# Building Prism (Windows & Linux)

macOS has its own guide: [`docs/BUILD_MACOS.md`](docs/BUILD_MACOS.md).

## Prerequisites

| Tool | Version | Notes |
|---|---|---|
| Rust | stable ≥ 1.88 (tested 1.99) | `rustup default stable` |
| Node.js | 24 LTS (npm 11) | builds the UI and the extension |
| Windows | Visual Studio 2022 Build Tools with "Desktop development with C++", Windows 10/11 SDK | WebView2 runtime is preinstalled on Windows 10/11 |
| Linux (Debian/Ubuntu 22.04+) | `sudo apt install build-essential curl wget file libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev patchelf rpm` | `rpm` only needed to build .rpm |
| Optional | `ffmpeg` on PATH | otherwise Prism downloads a verified build on first video download |

## One-time setup

```bash
git clone <repo> prism && cd prism
npm ci --prefix app
npm ci --prefix extension
```

## Development

```bash
# Terminal 1 – everything (sidecar + Vite + Tauri with hot reload)
cd app
npm run tauri:dev
```

The UI also runs in a normal browser with a built-in demo backend (`npm run dev` in `app/`,
then open http://localhost:1420) — handy for UI work without compiling Rust.

## Tests

```bash
cargo test --workspace --exclude prism-app        # engine, core, video, torrent, native host
cargo clippy --workspace --all-targets -- -D warnings
npm test --prefix app && npm run typecheck --prefix app
npm test --prefix extension && npm run typecheck --prefix extension
npm run e2e --prefix extension                    # browser ↔ native host ↔ app (Playwright)
```

## Release builds

```bash
npm run build --prefix extension      # extension/build/{chromium,firefox} + zips
cd app
npm run tauri:build                   # builds the native host sidecar, the UI and all bundles
```

Outputs (in `target/release/bundle/`):

| Platform | Artifacts |
|---|---|
| Windows | `nsis/Prism_<ver>_x64-setup.exe`, `msi/Prism_<ver>_x64_en-US.msi` |
| Linux | `deb/Prism_<ver>_amd64.deb`, `rpm/Prism-<ver>-1.x86_64.rpm`, `appimage/Prism_<ver>_amd64.AppImage` |

Windows portable zip (after `tauri:build`):

```powershell
./scripts/make-portable.ps1          # -> packaging/out/Prism-<ver>-windows-x64-portable.zip
```

Building only some bundles: `npx tauri build --bundles nsis` (or `msi`, `deb`, `rpm`, `appimage`).

## prism-web (standalone Linux binary)

`prism-web` is a self-hosted HTTP server. Its build embeds `app/dist`, so build the UI first:

```bash
npm ci --prefix app
npm run build --prefix app
cargo build --release -p prism-web --target x86_64-unknown-linux-gnu
./target/x86_64-unknown-linux-gnu/release/prism-web --host 127.0.0.1 --port 8080
```

Flags: `--host <addr>` (default `127.0.0.1`), `--port <u16>` (default `8080`),
`--no-auth-token`, `--download-dir <path>`, `--data-dir <path>`, and `--warmup`
(downloads yt-dlp/ffmpeg/deno, then exits). `PRISM_WEB_DATA_DIR` and
`PRISM_WEB_DOWNLOAD_DIR` override the default directories. `PRISM_WEB_TOKEN` sets a
fixed bearer token shared by the web UI and the browser extension: it is trimmed,
must be non-empty, and is never printed (`Access token: from PRISM_WEB_TOKEN`).
With it set, no `web-ui.json` or `ipc.json` is written, so the desktop app's native
host keeps working unchanged. Without it, prism-web generates a token (or accepts
`--no-auth-token` on loopback only) and publishes `ipc.json` as before.

Docker (the build context needs the prebuilt binary at the target path above):

```bash
export PRISM_WEB_TOKEN='a-long-random-secret'
docker build -t prism-web .
docker compose up -d
```

Compose requires `PRISM_WEB_TOKEN`, keeps state and downloads in named volumes, and
binds port 8080 to loopback only (`PRISM_WEB_BIND_HOST` defaults to `127.0.0.1`).
Binding to another interface must be explicit: set `PRISM_WEB_BIND_HOST` yourself,
and use TLS (a reverse proxy) beyond loopback — over plain HTTP the token and all
traffic are visible on the network. To connect the extension, open its Options page
and enter the server root URL (e.g. `http://127.0.0.1:8080`) and the same token;
leaving the URL blank keeps native messaging.

Release artifacts for `prism-web` are **Linux x64 only** (`prism-web-linux-x64.tar.gz`).
Desktop bundles remain Windows/Linux/macOS; tag pushes also publish the container to
`ghcr.io/<owner>/<repo>/prism-web`.

### Building Linux packages on Windows (WSL)

The Linux bundles were produced from WSL2 Ubuntu 22.04:

```bash
wsl -d Ubuntu-22.04
# inside WSL, on a Linux filesystem path (not /mnt/c, which is very slow):
git clone /mnt/c/path/to/prism ~/prism && cd ~/prism
curl --proto '=https' -sSf https://sh.rustup.rs | sh -s -- -y
# Node 24 via nvm or NodeSource, then the apt packages above
npm ci --prefix app && npm ci --prefix extension
npm run build --prefix extension && (cd app && npm run tauri:build)
```

## What the installers do

* **NSIS / MSI (Windows):** install per user (NSIS) or per machine (MSI), add Start-menu
  entries and an uninstaller, associate `.torrent` files, register the `magnet:` handler,
  and run `prism-native-host register` so Chrome, Edge, Brave, Opera, Vivaldi, Chromium
  and Firefox can reach the app. Uninstalling removes the registrations.
* **deb / rpm (Linux):** install `/usr/bin/prism` + `/usr/bin/prism-native-host`, a
  `.desktop` entry, and system-wide native messaging manifests for Chrome, Chromium, Edge
  and Firefox (`packaging/linux/postinst.sh`). Brave, Vivaldi and Opera get per-user
  manifests written by the app on every start.
* **AppImage:** the app copies the host to `~/.local/share/prism/bin/` (the AppImage mount
  path changes every run) and registers per-user manifests.
* **Autostart** is a setting (on by default) handled by the app itself.

## Continuous integration

`.github/workflows/ci.yml` runs tests, clippy and type checks on Windows, Linux and macOS.
`.github/workflows/release.yml` builds all bundles (Windows x64, Linux x64, macOS arm64 and
x64) on tag push and uploads them as workflow artifacts.

## Browser extension

See [`extension/README.md`](extension/README.md). In short: load
`extension/build/chromium` unpacked in Chromium-based browsers (Developer mode), and
`extension/build/firefox/manifest.json` as a temporary add-on in Firefox (or sign the
generated zip on addons.mozilla.org as an unlisted add-on for a permanent install).
