# Installing Prism

Download the latest version from the **[Releases page](https://github.com/FortoGraphy/prism-download-manager/releases/latest)**.

| Your system | Download | Notes |
|---|---|---|
| Windows 10 / 11 (64-bit) | `Prism_<version>_x64-setup.exe` | Recommended. Installs for your user account, no admin rights needed. |
| Windows (IT / all users) | `Prism_<version>_x64_en-US.msi` | Installs for all users; needs administrator rights. |
| Windows (no install) | `Prism-<version>-windows-x64-portable.zip` | Unzip anywhere (USB stick works). Settings stay next to `prism.exe`. |
| macOS 11+ — Apple Silicon (M1–M4) | `Prism_<version>_aarch64.dmg` | |
| macOS 11+ — Intel | `Prism_<version>_x64.dmg` | |
| Ubuntu, Debian, Mint, Pop!_OS, elementary | `Prism_<version>_amd64.deb` | |
| Fedora, openSUSE, RHEL | `Prism-<version>-1.x86_64.rpm` | |
| Any other Linux | `Prism_<version>_amd64.AppImage` | |
| Linux x64 only — self-hosted | `prism-web-linux-x64.tar.gz` | Standalone HTTP server, no desktop shell. See [Prism Web](#prism-web-linux-x64-only) below. |

Every release also contains `SHA256SUMS.txt` so you can verify downloads.

After installing the app, **install the browser extension** — that is what lets Prism catch
downloads and videos from your browser: [browser-extension.md](browser-extension.md).

---

## Windows

1. Run `Prism_<version>_x64-setup.exe`.
2. Windows SmartScreen may say *"Windows protected your PC"* because the installer is not
   code-signed with a paid certificate. Click **More info → Run anyway**.
3. Prism starts and shows a short welcome tour. It sits in the system tray (bottom-right)
   while it works.

The installer also:
* registers Prism with Chrome, Edge, Brave, Opera, Vivaldi, Chromium and Firefox (so the
  extension can talk to the app),
* opens `magnet:` links and `.torrent` files with Prism,
* adds a Start-menu entry and an uninstaller (Settings → Apps → Prism → Uninstall).

**Portable version:** unzip, run `prism.exe`. Everything (settings, history, tools) stays in
the `data` folder next to it.

## macOS

1. Open the `.dmg` and drag **Prism** into **Applications**.
2. The app is not notarized by Apple (that needs a paid developer account), so macOS blocks
   it the first time — it may refuse to open, say it *"is damaged"*, or close it after a few
   seconds. Allow it once, either way:
   * **Terminal (simplest):**
     ```bash
     xattr -dr com.apple.quarantine /Applications/Prism.app
     ```
   * or open Prism once, then **System Settings → Privacy & Security** → scroll down →
     **Open Anyway** next to "Prism was blocked" → confirm with your password.

   Updates installed from inside Prism (it offers them by itself) don't need this again.
3. Optional but recommended for video downloads: `brew install ffmpeg` (Prism can also fetch a
   verified ffmpeg build by itself — Settings → Video & streams → Check tools).

## Linux

**Debian / Ubuntu family**

```bash
sudo apt install ./Prism_<version>_amd64.deb
```

**Fedora / openSUSE**

```bash
sudo dnf install ./Prism-<version>-1.x86_64.rpm      # openSUSE: sudo zypper install ./Prism-*.rpm
```

The package needs any `ffmpeg` (`/usr/bin/ffmpeg`): Fedora's `ffmpeg-free` or the full
`ffmpeg` from RPM Fusion both work, and Prism uses whichever is installed.

**AppImage (any distribution)**

```bash
chmod +x Prism_<version>_amd64.AppImage
./Prism_<version>_amd64.AppImage
```

The deb/rpm packages register Prism with Chrome, Chromium, Edge and Firefox for all users;
the app itself also registers Brave, Vivaldi and Opera for your user on every start.
`ffmpeg` is installed as a dependency of the deb/rpm packages (AppImage users: install
`ffmpeg` from your distribution for video features).

The tray icon needs an AppIndicator-capable desktop (KDE, Cinnamon, XFCE, MATE work out of the
box; on GNOME install the *AppIndicator and KStatusNotifierItem Support* extension).

## Prism Web (Linux x64 only)

`prism-web` is a standalone HTTP server that serves the same Prism UI as the desktop app, without
the desktop shell (there are no file or folder opening controls). It is Linux x64 only and ships
as `prism-web-linux-x64.tar.gz`.

Authentication depends on `PRISM_WEB_TOKEN`. Set it to a long random secret and the UI and the
extension share that one bearer token; no token files are written. Without it, prism-web generates
a UI token, printed as `Access token:` and saved in `web-ui.json` in its data dir, and a separate
extension token in `ipc.json` (the path is printed as `IPC info:`). Keep the tokens private —
anyone with one can control your downloads. Commands and setup are in
[BUILD.md — prism-web](../../BUILD.md#prism-web-standalone-linux-binary).

The extension uses native messaging by default. To connect it to prism-web instead, open its
Options page and enter the server URL and the token; see the
[browser extension guide](browser-extension.md) and the [BUILD.md section](../../BUILD.md#prism-web-standalone-linux-binary).

The server binds to loopback (`127.0.0.1`) by default. Beyond loopback, put it behind a reverse
proxy with TLS: over plain HTTP the token and all traffic are visible on the network.

## Updating

Install the newer version over the old one — your downloads, queues and settings are kept.
yt-dlp (used for video sites) updates itself automatically in the background.

## Uninstalling

* **Windows:** Settings → Apps → Prism → Uninstall (removes the browser registration too).
* **macOS:** drag Prism from Applications to the Trash. Settings live in
  `~/Library/Application Support/Prism` if you want to remove them as well.
* **Linux:** `sudo apt remove prism` / `sudo dnf remove prism`; settings live in
  `~/.local/share/prism`.
