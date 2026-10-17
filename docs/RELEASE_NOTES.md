# Prism 1.0.8 — One beam. Many rays.

A fast, open-source download manager for Windows, Linux and macOS, with browser integration
and video downloads.

## What's new in 1.0.8 — security fixes

* **Linux: your browser cookies are no longer readable by other accounts on the computer.**
  When Prism handed a page's cookies to yt-dlp for a video, it wrote them to a file in `/tmp`
  that every account could read. The file is now private to you (0600) and has a random
  name, and files a crash left behind are removed. macOS and Windows keep temporary files per
  user and weren't affected.
* **The browser extension no longer adds SameSite=Strict cookies to downloads another site
  started.** A web page could start a download of a URL on another site, and Prism fetched it
  with that site's Strict cookies, which the browser had deliberately left out. Prism now adds
  them only where the browser itself would send them.
* The in-page *Download video* button now reacts only to your own clicks, not to clicks a page
  script fakes.
* Both issues were reported privately by [@pepopepopepo](https://github.com/pepopepopepo) —
  thank you!
* **After updating, click ↻ next to Prism on `chrome://extensions`** (or restart the browser)
  so it loads the new extension (1.0.8). If you installed it from a downloaded
  `prism-chromium.zip`, use the 1.0.8 zip.

### Also new since 1.0.6 (Prism 1.0.7)

* **LinkedIn: the Download video button finds the post in LinkedIn's new design** (feed, search
  and company pages), where nothing around the video names its post
  ([#1](https://github.com/FortoGraphy/prism-download-manager/issues/1)). Prism reads the post
  from its text and offers that post's video, never a neighbour's. For a post without text,
  Prism says to open the post.
* **Updates are found sooner:** Prism looks for a new version every hour, and again when you
  come back to it after half an hour (it was every 6 hours).
* **AppImage: the browser keeps the extension after Prism restarts.** The folder Prism showed
  for the extension disappeared with every restart of the AppImage. Prism now keeps it in
  `~/.local/share/prism/extension` — open Settings › Browser integration and load the extension
  from the folder shown there once.
* The reminder about an older browser extension also suggests restarting the browser.

### Prism 1.0.6

* Video menus open faster: Prism runs yt-dlp's unpacked build (about a third of the start-up
  time), reads the video while the pointer rests on it, answers at once for a page it read in
  the last 10 minutes and offers **Best quality** right away.
* Prism tells you when the browser still runs an older extension; Settings › Browser
  integration shows the extension's version.
* LinkedIn and Instagram feed, company, profile and Explore pages get a clear message at once;
  errors from pages Prism can't read say what to do. The macOS DMG guide points to the
  extension folder Prism keeps up to date.

## Downloads

| Your system | File |
|---|---|
| Windows 10 / 11 | `Prism_1.0.8_x64-setup.exe` (recommended) · `Prism_1.0.8_x64_en-US.msi` (all users) · `Prism-1.0.8-windows-x64-portable.zip` |
| macOS — Apple Silicon | `Prism_1.0.8_aarch64.dmg` |
| macOS — Intel | `Prism_1.0.8_x64.dmg` |
| Ubuntu / Debian / Mint | `Prism_1.0.8_amd64.deb` |
| Fedora / openSUSE | `Prism-1.0.8-1.x86_64.rpm` |
| Any Linux | `Prism_1.0.8_amd64.AppImage` |
| Browser extension | `prism-chromium.zip` (Chrome, Edge, Brave, Opera, Vivaldi) · `prism-firefox.zip` |

Step-by-step guides: [installing Prism](https://github.com/FortoGraphy/prism-download-manager/blob/main/docs/guide/installation.md) ·
[installing the browser extension](https://github.com/FortoGraphy/prism-download-manager/blob/main/docs/guide/browser-extension.md).
Verify downloads with `SHA256SUMS.txt`.

## What's inside

* **Multi-connection engine** written in Rust: up to 32 connections per file, dynamic
  segmentation (a freed connection splits the largest remaining range), automatic
  detection of server connection limits, writes straight into the final file.
* **Resume that survives crashes**: pause, quit, lose power — Prism continues where it
  stopped and validates the file against the server.
* **Browser integration**: click a download link and Prism takes over. A *Download video*
  button over video players, a popup listing every stream on the page, right-click menus.
* **Video and audio**: YouTube and every site yt-dlp supports. Choose **MP4 + quality** (up
  to 8K/HDR) or **MP3 only**. Subtitles, chapters, thumbnails, SponsorBlock.
* **More protocols**: FTP/FTPS, HLS and DASH streams, BitTorrent and magnet links.
* **Organisation**: categories, queues with schedules and speed limits, actions when a
  queue finishes (sleep, shut down…), site grabber, batch downloads, clipboard monitor.
* **Liquid-glass UI** with dark and light themes, six accent colours and a mini window.

## Known limitations

* The installers are **not code-signed** (signing certificates are paid):
  * Windows SmartScreen: click **More info → Run anyway**.
  * macOS: run `xattr -dr com.apple.quarantine /Applications/Prism.app` once (or System
    Settings → Privacy & Security → **Open Anyway**). In-app updates don't need this again.
* The extension is installed manually (Developer mode → *Load unpacked*); store listings
  are not available yet. Firefox keeps unsigned add-ons only until restart, except in
  Developer Edition / Nightly / ESR.
* macOS builds are produced by CI and tested there; feedback from real Macs is very welcome.
* Flatpak browsers on Linux can't talk to native apps (a browser sandbox limitation) — use
  the deb/rpm version of your browser.

Found a bug? [Open an issue](https://github.com/FortoGraphy/prism-download-manager/issues/new/choose).
Like Prism? A ⭐ helps a lot.
