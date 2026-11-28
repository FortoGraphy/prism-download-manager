# Installing the Prism browser extension

The extension is what makes Prism feel like magic in the browser:

* clicking a download link opens Prism's **Download File** window instead of the browser's
  download bar (hold **Alt** while clicking to let the browser handle it),
* a **Download video** button appears over video players — choose **MP4 + quality** or
  **MP3**,
* right-click menu: *Download with Prism*, *Download all links / images / videos*,
* the toolbar popup lists every video and audio stream found on the page.

> For the default **native messaging** transport, the Prism **app must be installed first**:
> the extension talks to it through a small helper that the installer registers. Prism →
> **Settings → Browser integration** shows the status of every browser and repairs it with
> one click. With **prism-web**, no desktop app or native host is needed — see
> [Connecting to prism-web](#connecting-to-prism-web).

> **Easiest way for native messaging:** Prism guides you through this on first start, and any
> time from **Settings → Browser integration**: it opens the store page in the browser you
> pick and turns green when the extension connects.

## Chrome, Edge, Brave, Opera, Vivaldi, Helium (Chrome Web Store)

**[Prism Download Manager in the Chrome Web Store](https://chromewebstore.google.com/detail/prism-download-manager/mmcdeiglkbfbicdjboocmhehodogilfl)**

1. Open the link above in your browser.
2. Click **Add to Chrome** (Brave: *Add to Brave*, Opera: *Add to Opera*), then **Add
   extension**.
3. Pin the Prism icon: click the puzzle icon in the toolbar → pin **Prism**.
4. Click the Prism icon — it should say **Connected**.

The store keeps the extension up to date by itself.

* **Edge:** the first time, Edge asks to *Allow extensions from other stores* at the top of
  the page — click **Allow**, then **Add to Chrome**.
* **Opera:** Opera first offers its free *Install Chrome Extensions* add-on — add it, then come
  back to the store page.

### Added Prism by hand before?

Versions before 1.1.0 loaded the extension with *Developer mode → Load unpacked*. That copy
doesn't update by itself, and the browser won't add the store version while it's installed:
open `chrome://extensions` (`edge://extensions`, `brave://extensions`, …), click **Remove**
under Prism, then add it from the store. Prism keeps working with both, so nothing else
changes.

## Firefox, Zen and Waterfox

Open [Prism in Firefox Add-ons](https://addons.mozilla.org/firefox/addon/prism-download-manager/)
and click **Add to Firefox** (Firefox 140 or later). Zen and Waterfox are built on Firefox: the
same page works there, and the add-on updates itself.

### Prefer to add it by hand?

Firefox keeps an extension added by hand only until it restarts, unless it is a Developer
Edition, Nightly or ESR:

* **Try it now (until restart):** open **`about:debugging#/runtime/this-firefox`** →
  **Load Temporary Add-on…** → pick `manifest.json` inside the **`firefox`** folder. Prism →
  Settings → Browser integration shows the folder (it's also in `prism-firefox.zip` on the
  [latest release](https://github.com/FortoGraphy/prism-download-manager/releases/latest)).
* **Permanent (Firefox Developer Edition / Nightly / ESR):** open `about:config`, set
  `xpinstall.signatures.required` to `false`, then open `about:addons` → gear icon →
  **Install Add-on From File…** → pick `prism-firefox.zip`.

## Already using IDM or another download manager?

Their browser extensions compete for the same downloads. Disable (or remove) the
*IDM Integration Module* or similar extensions in `chrome://extensions`, otherwise they may
grab downloads before Prism sees them.

## Connecting to prism-web

Prism-web is a self-hosted HTTP server; the desktop app and its native host are not needed
for this path. Leave the extension's server URL blank (the default) to use native messaging
instead.

1. Start prism-web (by default it listens on `127.0.0.1:8080`; use `--host` / `--port` to
   change that). Set `PRISM_WEB_TOKEN` to the token shared by the web UI and the extension.
   Without it, prism-web generates a token for the extension and writes it to its `web-ipc.json`
   discovery file: look for `IPC info: <path>` in the startup output to find that file. The
   `Access token:` printed at startup is for the web UI.
2. Add the extension, then open its **Options** page.
3. Set **Prism web server root URL** to the server root, e.g. `http://127.0.0.1:8080` —
   not a path prefix.
4. Set **Bearer token** to the `PRISM_WEB_TOKEN` value, or, if you left it unset, to the
   generated extension token in that `web-ipc.json` file.

The token and download metadata cross the network between the browser and the server. For
anything beyond loopback, put prism-web behind HTTPS (for example a reverse proxy): over
plain HTTP they are visible on the network. Never paste the token into issues.

## Troubleshooting

| Symptom | Fix |
|---|---|
| Popup says **Not connected** / *Prism isn't reachable* | Start Prism (it lives in the tray), then click **Retry** in the popup. Still failing → Prism → Settings → Browser integration → **Repair**, then restart the browser. |
| *Prism isn't reachable* when using prism-web | Check **Prism web server root URL** is the root (`http://127.0.0.1:8080`), not a path prefix; check **Bearer token** matches `PRISM_WEB_TOKEN` or the generated token in `web-ipc.json`; confirm the server is running and reachable. Beyond loopback use HTTPS via a reverse proxy — plain HTTP exposes the token. |
| *Failed to start native messaging host* (Windows) while Prism runs and Browser integration is green | The browser couldn't start Prism's helper. Update Prism: since 1.2.6 the helper no longer needs the Visual C++ Redistributable. Still failing → Windows lacks the `ComSpec` variable, which Chrome needs to start any helper: run `setx ComSpec "%SystemRoot%\system32\cmd.exe"` in a terminal, then quit the browser completely and start it again. Chrome started with `--enable-logging --v=1` writes the reason to its log (search for `launch_context_win`). |
| Downloads still go to the browser | Make sure *Capture downloads* is on in the popup; check the file type is in Settings → File types (e.g. images are not captured by default); `blob:` downloads (some web apps) can't be handed over. |
| No **Download video** button | Hover the video; check *Show video button* in the extension options; reload the page after installing. |
| The store says Prism is already added, but it's the old copy | Remove the copy you added by hand (see *Added Prism by hand before?*), then add it from the store. |
| "Extension ID" mismatch in Browser integration | You loaded a modified copy — use the store version; its ID is `mmcdeiglkbfbicdjboocmhehodogilfl`. |
| Linux Flatpak/Snap browsers | Flatpak browsers can't run native helpers (browser limitation). Snap Firefox asks for permission once. Use a deb/rpm browser for full integration. |

## Adding it by hand instead

Rather not use the store? Download `prism-chromium.zip` from the
[latest release](https://github.com/FortoGraphy/prism-download-manager/releases/latest) (or
build it with `npm run build --prefix extension`), unzip it into a folder that stays, then on
the browser's extensions page turn on **Developer mode** → **Load unpacked** → that folder.
Prism → Settings → Browser integration → *Add it by hand instead* walks you through it with
the folder Prism keeps up to date. A copy added by hand doesn't update by itself (click ↻ next
to Prism after a Prism update), and it has the same ID as the store version, so only one of
them can be installed at a time.

## Privacy

The extension sends a download's address, the page it came from, the browser's user agent
and that site's cookies only to the transport you configured; that is what lets downloads
needing your login continue in Prism.

* **Native messaging (default):** only to the Prism app on your own computer — no network,
  no servers, no analytics.
* **Direct HTTP:** to the prism-web server root you entered in Options. The bearer token and
  download metadata travel over the network to that server; use HTTPS beyond loopback.
