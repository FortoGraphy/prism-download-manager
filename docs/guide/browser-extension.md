# Installing the Prism browser extension

The extension is what makes Prism feel like magic in the browser:

* clicking a download link opens Prism's **Download File** window instead of the browser's
  download bar (hold **Alt** while clicking to let the browser handle it),
* a **Download video** button appears over video players — choose **MP4 + quality** or
  **MP3**,
* right-click menu: *Download with Prism*, *Download all links / images / videos*,
* the toolbar popup lists every video and audio stream found on the page.

> The Prism **app must be installed first** — the extension talks to it through a small
> helper that the installer registers. Prism → **Settings → Browser integration** shows the
> status of every browser and repairs it with one click.

> **Easiest way:** Prism guides you through this on first start, and any time from
> **Settings → Browser integration**: it opens the store page in the browser you pick and
> turns green when the extension connects.

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

Zen and Waterfox are built on Firefox: the same steps work there. Prism isn't in Firefox Add-ons yet, and
Firefox only keeps unsigned extensions until it restarts, so choose one of:

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

## Troubleshooting

| Symptom | Fix |
|---|---|
| Popup says **Not connected** / *Prism isn't reachable* | Start Prism (it lives in the tray), then click **Retry** in the popup. Still failing → Prism → Settings → Browser integration → **Repair**, then restart the browser. |
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
and that site's cookies **only to the Prism app on your own computer** (through native
messaging — no network, no servers, no analytics). This is what lets downloads that need
your login continue in Prism.
