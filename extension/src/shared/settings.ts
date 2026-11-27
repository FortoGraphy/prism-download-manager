import type { BypassKey, CaptureSettings } from './protocol';

/** IDM-like default extension list. */
export const DEFAULT_EXTENSIONS = (
  '3GP 7Z AAC ACE AIF APK ARJ ASF AVI BIN BZ2 EXE GZ GZIP IMG ISO LZH M4A M4V MKV MOV MP3 MP4 MPA MPE MPEG MPG ' +
  'MSI MSU OGG OGV PDF PLJ PPS PPT QT R0* R1* RA RAR RM RMVB SEA SIT SITX TAR TIF TIFF WAV WMA WMV Z ZIP DMG DEB ' +
  'RPM APPIMAGE FLAC OPUS WEBM ZST XZ'
).split(' ');

export const DEFAULT_MIME_TYPES = [
  'video/',
  'audio/',
  'application/octet-stream',
  'application/zip',
  'application/x-zip-compressed',
  'application/x-7z-compressed',
  'application/x-rar-compressed',
  'application/vnd.rar',
  'application/x-tar',
  'application/gzip',
  'application/x-gzip',
  'application/x-bzip2',
  'application/x-xz',
  'application/zstd',
  'application/x-msdownload',
  'application/x-msi',
  'application/x-ms-installer',
  'application/x-iso9660-image',
  'application/x-apple-diskimage',
  'application/vnd.android.package-archive',
  'application/vnd.debian.binary-package',
  'application/x-debian-package',
  'application/x-rpm',
  'application/x-redhat-package-manager',
];

/** Same list as `VIDEO_SITES` in crates/prism-core/src/manager/capture.rs (the app sends its copy; test/video-sites.test.ts keeps them equal). */
export const DEFAULT_VIDEO_SITES = [
  'youtube.com',
  'youtu.be',
  'youtube-nocookie.com',
  'vimeo.com',
  'dailymotion.com',
  'twitch.tv',
  'x.com',
  'twitter.com',
  'tiktok.com',
  'instagram.com',
  'facebook.com',
  'fb.watch',
  'reddit.com',
  'bilibili.com',
  'soundcloud.com',
  'streamable.com',
  'rumble.com',
  'vk.com',
  'vk.ru',
  'vkvideo.ru',
  'ok.ru',
  'nicovideo.jp',
  'bandcamp.com',
  'ted.com',
  'odysee.com',
  'linkedin.com',
];

export const DEFAULT_SETTINGS: CaptureSettings = {
  captureEnabled: true,
  extensions: DEFAULT_EXTENSIONS,
  mimeTypes: DEFAULT_MIME_TYPES,
  minSize: 0,
  excludedSites: [],
  bypassKey: 'alt',
  showVideoButton: true,
  videoSites: DEFAULT_VIDEO_SITES,
};

const BYPASS_KEYS: BypassKey[] = ['alt', 'shift', 'ctrl', 'meta', 'none'];

function stringList(v: unknown, fallback: string[], map: (s: string) => string): string[] {
  if (typeof v === 'string') v = v.split(/[\s,;]+/);
  if (!Array.isArray(v)) return fallback;
  const out: string[] = [];
  for (const x of v) {
    if (typeof x !== 'string') continue;
    const s = map(x.trim());
    if (s && !out.includes(s)) out.push(s);
  }
  return out;
}

/** Validates a partial settings object and fills in defaults. */
export function normalizeSettings(input: unknown, base: CaptureSettings = DEFAULT_SETTINGS): CaptureSettings {
  const p = (input && typeof input === 'object' ? input : {}) as Record<string, unknown>;
  const bool = (k: keyof CaptureSettings, d: boolean) => (typeof p[k] === 'boolean' ? (p[k] as boolean) : d);
  const minSize = typeof p.minSize === 'number' && Number.isFinite(p.minSize) && p.minSize >= 0 ? Math.floor(p.minSize) : base.minSize;
  const bypassKey = BYPASS_KEYS.includes(p.bypassKey as BypassKey) ? (p.bypassKey as BypassKey) : base.bypassKey;
  return {
    captureEnabled: bool('captureEnabled', base.captureEnabled),
    extensions: stringList(p.extensions, base.extensions, (s) => s.replace(/^\*?\./, '').toUpperCase()),
    mimeTypes: stringList(p.mimeTypes, base.mimeTypes, (s) => s.toLowerCase()),
    minSize,
    excludedSites: stringList(p.excludedSites, base.excludedSites, (s) => s.toLowerCase()),
    bypassKey,
    showVideoButton: bool('showVideoButton', base.showVideoButton),
    videoSites: stringList(p.videoSites, base.videoSites, (s) => s.toLowerCase().replace(/^\*\./, '')),
  };
}

/** Base URL of a prism-web server; empty means use native messaging. */
export function normalizeRemoteUrl(v: unknown): string {
  if (typeof v !== 'string') return '';
  try {
    const u = new URL(v.trim());
    if ((u.protocol !== 'http:' && u.protocol !== 'https:') || u.username || u.password) return '';
    return u.origin;
  } catch {
    return '';
  }
}

export function normalizeRemoteToken(v: unknown): string {
  return typeof v === 'string' ? v.trim() : '';
}

/** Persisted extension state (chrome.storage.local key `prism.state`). */
export interface StoredState {
  settings: CaptureSettings;
  /** Popup toggle: temporarily pause capture (local only). */
  paused: boolean;
  /** `app`: settings follow the app (refreshed on every hello); `local`: user edited them in Options. */
  settingsSource: 'app' | 'local';
  appVersion?: string;
  /** Root URL of a prism-web server (e.g. `http://127.0.0.1:8787`); empty = native messaging. */
  remoteUrl: string;
  /** Bearer token sent to that server; stored locally, never logged. */
  remoteToken: string;
}

export const STATE_KEY = 'prism.state';

export function defaultState(): StoredState {
  return { settings: { ...DEFAULT_SETTINGS }, paused: false, settingsSource: 'app', remoteUrl: '', remoteToken: '' };
}

export function normalizeState(raw: unknown): StoredState {
  const d = defaultState();
  if (!raw || typeof raw !== 'object') return d;
  const r = raw as Record<string, unknown>;
  return {
    settings: normalizeSettings(r.settings),
    paused: typeof r.paused === 'boolean' ? r.paused : d.paused,
    settingsSource: r.settingsSource === 'local' ? 'local' : 'app',
    remoteUrl: normalizeRemoteUrl(r.remoteUrl),
    remoteToken: normalizeRemoteToken(r.remoteToken),
    ...(typeof r.appVersion === 'string' ? { appVersion: r.appVersion } : {}),
  };
}
