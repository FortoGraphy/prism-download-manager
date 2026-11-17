# Prism browser extension

One TypeScript source, two Manifest V3 builds:

| Build | Background | Download capture |
|---|---|---|
| `build/chromium/` (Chrome, Edge, Brave, Opera, Vivaldi, Chromium) | service worker | `downloads.onDeterminingFilename` (before the Save-As dialog) |
| `build/firefox/` (Firefox ≥ 140, gecko id `prism@prismdl.site`) | event page (`background.scripts`) | blocking `webRequest.onHeadersReceived` for `Content-Disposition: attachment`, `downloads.onCreated` + cancel as backstop |

**Stable Chromium extension ID: `mmcdeiglkbfbicdjboocmhehodogilfl`** (from the public key in
`keys/`, see `keys/README.md`). The native host allows this origin by default.

## Build, test, install

```
npm install
npm run build        # build/chromium, build/firefox, build/prism-chromium.zip, build/prism-firefox.zip
npm run build:dev    # unminified + inline source maps
npm run typecheck    # tsc --noEmit (strict)
npm test             # vitest: rules, media classification, cookies, correlation, NativeClient
npm run e2e          # Playwright + real Chromium + real native host + mock app (see below)
npm run e2e:dismiss  # Playwright dismissal regressions; no native host or desktop app needed
```

Install for development:

1. Build the host: `cargo build -p prism-native-host --release`.
2. Register it: `target/release/prism-native-host register` (all browsers; per-user, no admin).
   The Prism app does this itself on every start / from *Settings › Browser integration › Repair*.
3. Chromium-family: `chrome://extensions` → Developer mode → *Load unpacked* → `extension/build/chromium`.
   Firefox: `about:debugging#/runtime/this-firefox` → *Load Temporary Add-on* → `extension/build/firefox/manifest.json`
   (or install `prism-firefox.zip` in Developer Edition/Nightly with signing disabled).

Note: other download-manager extensions (IDM, FDM…) also hook `onDeterminingFilename`; only one
of them can win, so disable them while using Prism.

## Features

* **Download capture** — a browser download is handed to Prism when capture is on and not paused,
  the bypass key was not held, the URL is http(s)/ftp (`blob:`/`data:` stay in the browser), the
  request was a GET (POST downloads stay), the page/URL isn't excluded, the extension *or* MIME type
  is in the lists and the size (when known) ≥ the minimum. The extension forwards URL, final URL,
  Referer (the header the browser really sent, via `webRequest.onSendHeaders` + `extraHeaders`),
  page URL, file name, size, MIME, the `Cookie` header (sent header merged with
  `cookies.getAll({url})` from the right — incl. incognito — store), Origin, user agent and the
  incognito flag. On acceptance the browser download is cancelled and erased before any Save-As
  dialog. **Fail open:** if the host/app is unreachable or the app declines, the browser keeps the
  download and the toolbar shows a red `!` badge.
* **Bypass key** (default Alt, configurable): a content script reports links clicked with the key
  held (URL + timestamp, sent on `mousedown`, before the navigation starts).
* **Media sniffing** — non-blocking `webRequest.onHeadersReceived`: HLS/DASH manifests,
  progressive video/audio (≥ 64 KiB), size from `Content-Length`/`Content-Range`; segments
  (`.ts`, `.m4s`) are dropped once a manifest is known; googlevideo chunks are ignored (YouTube is
  resolved by the app). Per-tab lists live in `storage.session` (survive service-worker restarts),
  are cleared on navigation, and the badge shows the count.
* **Floating video button** — Shadow-DOM pill above the top-right of a hovered `<video>` (YouTube
  player / Shorts aware, skips players < 200×120 and small muted autoplay loops, follows
  scroll/resize, moves into the fullscreen element, handles `yt-navigate-finish`, `popstate`,
  Navigation API and DOM mutations). On `videoSites` (YouTube, Vimeo, …) the menu shows the app's
  `probeVideo` formats — on a feed, profile or listing (X, Instagram, Facebook, Reddit, TikTok,
  LinkedIn…) for the hovered post (the tile's link, the single-post link of its `<article>`, or
  LinkedIn's post id in the attributes of its container, or in LinkedIn's newer pages the
  `componentkey` of the post's text block inside the video's `[role=listitem]` card), not the page (the app refuses
  LinkedIn / Instagram list pages at once, without yt-dlp). The probe starts once the pointer rests on a video (~0.4 s; one at a time) and is
  kept 10 minutes; while it runs, single post / video pages offer *Best quality* at once
  (`formatId: "best"`: the app picks the best format within the preferred quality). Elsewhere first *This video* — what the hovered element itself plays
  (`currentSrc`, `src`, `<source>` children, a `data-sources` quality list as on LinkedIn), best
  quality first — then the tab's other sniffed streams (other posts, autoplaying ads).
  The **X** beside Download hides the button and its quality menu for that video/post until
  the page is reloaded. Hovering again, scrolling or rerendering the same post does not undo
  dismissal; other posts and new sources in recycled players retain their buttons. Choices
  stay in memory for the page lifetime. The X supports keyboard activation, and hidden
  controls leave the focus order. **Download with Prism** in the browser context menu remains
  available for hidden videos (blob players can explicitly reopen the quality menu).
  Before/after screenshots: [dark page](docs/screenshots/video-download-dismiss-dark.png),
  [light page](docs/screenshots/video-download-dismiss-light.png).
* **Popup** — connection state (with a fix hint), pause-capture toggle, media on the page with
  per-item Download, "All links / images / videos", Open Prism, Options. Follows the system theme.
* **Context menus** — Download with Prism (link/image/video/audio; a video that plays a `blob:`
  stream opens the Prism menu over it, or the app's video window without the in-page button), Download selection links,
  all links, all images, all videos (collected in every frame with `scripting.executeScript`).
* **Options** — capture on/off, video button, bypass key, extension list, MIME list, min size,
  excluded sites, video sites, *Sync from Prism*.
* **Native port** — one lazy `connectNative('dev.prism.host')` port kept open (keeps the MV3
  worker alive), id-correlated requests with timeouts, reconnect on demand (5 s back-off when the
  host is missing), one-shot `sendNativeMessage` fallback if the port dies mid-request, 1-min
  `alarms` watchdog (`hello`).

## Protocol (v1) — what the app must implement

Transport: extension ⇄ `prism-native-host` (native messaging) ⇄ app
`POST http://127.0.0.1:<port>/v1/message` (`Authorization: Bearer <token>`, JSON body = the
extension's message verbatim). `GET /v1/health` (same auth) → `200 {"ok":true,"version":"x.y.z"}`.
Host details (ipc.json, launch, error codes): `crates/prism-native-host/README.md`.

**Envelope** (every request): `id` (string, e.g. `"k4fqpc-13"`), `type`, optional
`noLaunch: true` (a hint for the host — "don't start the app for this"; the app can ignore it).
**Every reply** is HTTP 200 + a JSON object: `{"ok": true, ...}` or
`{"ok": false, "error": {"code": "<snake_case>", "message": "<human text>"}}`. The host adds
`id`. Replies must be < 1 MiB. **Answer immediately** — never block on a dialog: `download`,
`batch` and `video` hold the user's click (a captured browser download is paused until the reply);
show UI asynchronously after replying.

Extension timeouts: `hello` 4 s, `status` 2.5 s, `download`/`batch`/`video`/`open` 5 s when the
app is known to be up, 12 s otherwise (lets the host launch it), `probeVideo` 60 s.

### `hello`
Sent on worker start, browser start, install, every minute (all with `noLaunch: true`) and on
*Sync from Prism* (without `noLaunch`).
```json
{ "id": "…", "type": "hello", "noLaunch": true, "protocol": 1, "version": "1.0.0",
  "browser": "chrome|edge|brave|opera|vivaldi|chromium|firefox", "userAgent": "Mozilla/5.0 …",
  "installType": "development|normal|admin|…" }
```
`installType` (since 1.0.7) is `management.getSelf().installType`: the app reminds about an older
extension only when the browser doesn't update it by itself (not for `normal` / `admin`).
Reply:
```json
{ "ok": true, "app": { "name": "Prism", "version": "1.0.0" }, "protocol": 1,
  "settings": {
    "captureEnabled": true,
    "extensions": ["ZIP", "ISO", "R0*", "…"],
    "mimeTypes": ["video/", "audio/", "application/octet-stream", "application/x-", "…"],
    "minSize": 0,
    "excludedSites": ["example.com", "*.bank.com", "https://x.com/dl/*"],
    "bypassKey": "alt",
    "showVideoButton": true,
    "videoSites": ["youtube.com", "vimeo.com", "…"] } }
```
All `settings` fields are optional (missing → extension defaults, see `src/shared/settings.ts`).
`extensions`: upper-case, no dot, `*`/`?` wildcards. `mimeTypes`: entries ending in `/`, `-` or
`*` are prefixes, others exact. `minSize`: bytes, 0 = none. `excludedSites`: `host` (incl.
subdomains), `*.host`, or a URL glob. `bypassKey`: `alt|shift|ctrl|meta|none`. The extension
applies app settings unless the user saved local overrides in Options (*Sync from Prism* reverts
to the app's).

### `download`
```json
{ "id": "…", "type": "download",
  "url": "https://host/dl?id=1",            // original request URL (before redirects)
  "finalUrl": "https://cdn/file.zip",       // only when different from url
  "referrer": "https://host/page",          // Referer header the browser sent
  "pageUrl": "https://host/page",           // top-level page
  "filename": "file.zip",                   // browser's suggested name (Content-Disposition aware)
  "fileSize": 2097152,                      // bytes, omitted when unknown
  "mime": "application/zip",
  "method": "GET",
  "headers": { "Origin": "https://host" },  // optional extra headers to replay
  "cookies": "session=abc; theme=dark",     // Cookie header for url ('' if none)
  "finalUrlCookies": "cdn_token=…",         // only when finalUrl is on another origin
  "userAgent": "Mozilla/5.0 …",
  "incognito": false,
  "source": "capture" }                     // "capture" (intercepted) | "contextMenu" (Download with Prism)
```
Reply: `{ "ok": true, "accepted": true, "downloadId": "…" }`. `accepted: false` → for
`capture` the browser keeps the download itself. Any error → also fail-open.

### `batch`
"Download all links / images / videos / selection links" (context menu or popup).
```json
{ "id": "…", "type": "batch", "pageUrl": "https://host/page", "title": "Page title",
  "kind": "links|images|videos|selection",
  "items": [ { "url": "https://host/a.zip", "referrer": "https://host/page",
               "kind": "link|image|video|audio", "filename": "a.zip",
               "text": "link text / alt text", "cookies": "session=abc" } ],
  "userAgent": "…", "incognito": false }
```
Items are de-duplicated, http(s)/ftp only, collected from all frames (`selection`: only the
clicked frame; includes plain-text URLs in the selection). `filename`/`text` are optional.
Expected UX: a selection dialog with filters. Reply: `{ "ok": true, "accepted": true, "count": 12 }`.

### `probeVideo`
Sent when the video button's menu opens on a `videoSites` page (also embedded players in
iframes — then `pageUrl` is the iframe URL, e.g. `https://www.youtube.com/embed/ID`).
```json
{ "id": "…", "type": "probeVideo", "pageUrl": "https://www.youtube.com/watch?v=…",
  "cookies": "…", "userAgent": "…" }
```
Reply (formats shown in this order; `kind: "audio"` is grouped under *Audio only*, everything
else under *Video*; `label` is displayed verbatim, `note` as a second line):
```json
{ "ok": true, "title": "Video title", "thumbnail": "https://…", "duration": 213,
  "formats": [
    { "id": "137+140", "label": "1080p60 · MP4 · 145 MB", "kind": "video", "ext": "mp4", "height": 1080, "fps": 60, "size": 152043520 },
    { "id": "22", "label": "720p · MP4 · 61 MB", "kind": "muxed", "ext": "mp4", "height": 720 },
    { "id": "140", "label": "Audio · M4A · 4 MB", "kind": "audio", "ext": "m4a", "size": 4194304, "note": "AAC 128k" } ] }
```
`ok: false` or an empty list → the menu falls back to sniffed streams and shows the message.

### `video`
Sent when the user picks an entry in the video-button menu or clicks *Download* on a popup media
item. Two shapes:
```json
// a probeVideo format was chosen (cookies are for pageUrl)
{ "id": "…", "type": "video", "pageUrl": "https://www.youtube.com/watch?v=…", "title": "Video title",
  "formatId": "137+140", "formatLabel": "1080p60 · MP4 · 145 MB", "streams": [],
  "cookies": "…", "userAgent": "…", "incognito": false }

// a sniffed stream was chosen (cookies are for the stream URL)
{ "id": "…", "type": "video", "pageUrl": "https://site/page", "title": "Page title",
  "streams": [ { "url": "https://cdn/master.m3u8", "kind": "hls|dash|direct",
                 "mime": "application/vnd.apple.mpegurl", "size": 307200, "quality": "1080p",
                 "headers": { "Referer": "https://site/page", "Origin": "https://site" } } ],
  "cookies": "…", "userAgent": "…", "incognito": false }
```
`size`, `quality`, `mime`, `headers` are optional. `kind: "direct"` is a progressive file
(treat like a download with those headers). Reply: `{ "ok": true, "accepted": true }`.

### `status`
Popup open / *Retry*: `{ "id": "…", "type": "status", "noLaunch": true }` →
`{ "ok": true, "app": { "name": "Prism", "version": "1.0.0" }, "captureEnabled": true }`.

### `open`
Popup *Open Prism* / *Start Prism* (the host launches the app if needed):
`{ "id": "…", "type": "open", "page": "main|browserIntegration|settings" }` → `{ "ok": true }`
(bring the main window to front, optionally on that page).

### Errors the extension understands
App/host codes are shown to the user; these drive the connection indicator:
`host_missing`/`host_forbidden` (native host not registered → "Repair" hint),
`app_not_running`, `app_unreachable`, `timeout`, `host_exited`, `disconnected`, `unauthorized`
(→ "Prism isn't reachable"). Any other `ok:false` code means the app is up but refused the request.

## Layout

```
build.mjs                 esbuild bundling, per-browser manifests, zips
src/background/           index (router), native (port + correlation), capture, requests (header map),
                          sniffer, video, collect, menus, state, badge, context
src/content/              index, bypass (modifier tracking), video-button (+ styles)
src/popup/, src/options/  vanilla TS + CSS (shared tokens in src/shared/ui.css)
src/shared/               protocol types, settings, rules, media, cookies, correlator, bypass, icons, api shim
scripts/gen-icons.mjs     renders the Prism mark to static/icons/*.png (no deps)
test/                     vitest unit tests
e2e/                      Playwright end-to-end test, mock app, test site
```

## End-to-end test

`npm run e2e` (needs `npm run build`, `cargo build -p prism-native-host --release`,
`npx playwright install chromium`). It starts a mock app (writes `ipc.json` into a temp
`PRISM_DATA_DIR`), registers the real host for Playwright's Chromium
(`HKCU\Software\Chromium\NativeMessagingHosts\dev.prism.host` on Windows, temp manifest,
backed up/restored afterwards), serves a local page, launches headless Chromium with the unpacked
extension and checks: stable ID; `hello` over the native port; .zip click → `download` at the app
with url/referrer/pageUrl/cookies/size/mime and the browser download cancelled+erased (prints
click → app latency); Alt+click bypass; video button → sniffed stream `video`; video button on a
video site → `probeVideo` formats → `video` with `formatId`; links/images/selection `batch`;
popup connected state, media list and "All videos"; app down → fail-open + `!` badge + popup fix hint.

Env: `PRISM_E2E_HEADED=1`, `PRISM_HOST_EXE=…`, `PRISM_E2E_CHROME_PATH=…` (use a specific
Chromium / Chrome-for-Testing binary; branded Chrome/Edge cannot side-load extensions any more),
`PRISM_E2E_BROWSERS=chromium[,chrome]` (registry keys to register). Screenshots land in
`e2e/artifacts/`. The registry part is Windows-only; on Linux/macOS register into the matching
Chromium config dir.

`node e2e/mock-app.mjs <dataDir>` runs the mock app standalone (prints every message) for manual
testing with a real browser.

## Limitations

* Firefox build: compiles, passes `web-ext lint` (0 errors), same features, but has not been run
  end-to-end here. The blocking-webRequest path only handles `Content-Disposition: attachment`
  responses; other matches go through `downloads.onCreated` + cancel (a download entry flashes).
* Opera/Vivaldi/Brave use Chrome's registration; Opera's Linux/macOS directories are unverified.
* Partitioned (CHIPS) cookies are only included when the browser sent them on the captured request.
* Downloads started by `<a download>` on `blob:`/`data:` URLs, and POST form downloads, always stay in the browser.
* The video button cannot overlay a `<video>` element that is itself fullscreen (it hides then).
