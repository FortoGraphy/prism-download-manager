// In-memory demo backend used when the UI runs outside Tauri (browser dev server,
// design review, marketing screenshots). It simulates downloads with live progress.

import type {
  BatchInfo,
  FoundFile,
  StartMode,
  Bootstrap,
  CaptureInfo,
  Category,
  CoreEvent,
  DownloadItem,
  GrabberProject,
  ItemDetails,
  Progress,
  Queue,
  SegmentInfo,
  Settings,
  Status,
  VideoCapture,
} from "./types";

const MB = 1024 * 1024;
const GB = 1024 * MB;
const now = Date.now();
/** Demo: when the browser extension last said hello (0 = never; `?ext=1` = already connected). */
let mockExtSeen = /[?&]ext=1/.test(location.search) ? Date.now() : 0;
/** The demo extension was "installed" from the Chrome Web Store (install type "normal"). */
let mockExtFromStore = false;
const listeners = new Set<(e: CoreEvent) => void>();
const emit = (e: CoreEvent) => listeners.forEach((l) => l(e));

const appListeners = new Map<string, Set<(p: unknown) => void>>();
const emitApp = (name: string, payload: unknown) => appListeners.get(name)?.forEach((l) => l(payload));
export function subscribeApp(name: string, handler: (p: unknown) => void): () => void {
  if (!appListeners.has(name)) appListeners.set(name, new Set());
  appListeners.get(name)!.add(handler);
  return () => appListeners.get(name)?.delete(handler);
}

export function subscribe(handler: (e: CoreEvent) => void): () => void {
  listeners.add(handler);
  return () => listeners.delete(handler);
}

const DEMO_EXTENSIONS: Record<string, string[]> = {
  video: ["mp4", "mkv", "webm", "mov"],
  music: ["mp3", "flac", "m4a"],
  documents: ["pdf", "txt", "docx"],
  programs: ["exe", "msi", "deb", "dmg"],
  compressed: ["zip", "7z", "rar", "iso", "tar", "gz"],
  images: ["jpg", "png", "webp"],
};

const categories: Category[] = [
  ["video", "Video", "cat-video"],
  ["music", "Music", "cat-music"],
  ["documents", "Documents", "cat-documents"],
  ["programs", "Programs", "cat-programs"],
  ["compressed", "Compressed", "cat-compressed"],
  ["images", "Images", "cat-images"],
  ["other", "Other", "cat-other"],
].map(([id, name, icon], position) => ({ id, name, icon, extensions: DEMO_EXTENSIONS[id] ?? [], builtin: true, position }));

const schedule = {
  enabled: true,
  startTime: "02:00",
  stopTime: "07:30",
  days: [1, 2, 3, 4, 5],
  retryFailed: true,
  onComplete: { type: "sleep" as const },
  wake: true,
};

const queues: Queue[] = [
  { id: "main", name: "Main queue", maxConcurrent: 3, speedLimit: 0, position: 0, builtin: true, running: true, schedule: { ...schedule, enabled: false, onComplete: { type: "none" } } },
  { id: "night", name: "Night downloads", maxConcurrent: 2, speedLimit: 0, position: 1, builtin: false, running: false, schedule },
];

type Seed = [string, string, number | null, Status, number, string, Partial<DownloadItem>?];
const seeds: Seed[] = [
  ["ubuntu-26.04-desktop-amd64.iso", "https://releases.ubuntu.com/26.04/ubuntu-26.04-desktop-amd64.iso", 6.2 * GB, "downloading", 0.62, "compressed", { connections: 16 }],
  ["Blender-5.1-windows-x64.msi", "https://download.blender.org/release/Blender5.1/blender-5.1.0-windows-x64.msi", 412 * MB, "downloading", 0.31, "programs"],
  ["Cosmos Laundromat (4K HDR).mkv", "https://www.youtube.com/watch?v=Y-rmzh0PI3c", 3.4 * GB, "downloading", 0.18, "video", { kind: "video", media: { title: "Cosmos Laundromat — First Cycle", quality: "2160p · HDR · AV1", site: "youtube.com", duration: 732 } }],
  ["Interstellar Soundscapes — Full Album.flac", "https://soundcloud.com/a/interstellar", 612 * MB, "processing", 1, "music", { kind: "video", media: { title: "Interstellar Soundscapes", quality: "Audio · FLAC", site: "soundcloud.com" } }],
  ["node-v24.18.0-x64.msi", "https://nodejs.org/dist/v24.18.0/node-v24.18.0-x64.msi", 31.2 * MB, "completed", 1, "programs"],
  ["Q3-financial-report-2026.pdf", "https://cdn.example.com/reports/Q3-2026.pdf", 8.7 * MB, "completed", 1, "documents"],
  ["dataset-imagenet-subset.tar.zst", "https://data.example.org/imagenet-subset.tar.zst", 18.4 * GB, "paused", 0.44, "compressed"],
  ["Big Buck Bunny 1080p.mp4", "https://vimeo.com/1084537", 728 * MB, "completed", 1, "video", { kind: "video", media: { title: "Big Buck Bunny", quality: "1080p60 · H.264", site: "vimeo.com", duration: 596 } }],
  ["debian-13.1.0-amd64-netinst.iso", "magnet:?xt=urn:btih:7a3c…", 754 * MB, "downloading", 0.83, "compressed", { kind: "torrent" }],
  ["photos-2026-summer.zip", "https://drive.example.com/s/7f2a/photos.zip", 2.1 * GB, "queued", 0, "compressed", { queueId: "main" }],
  ["VSCodeUserSetup-x64-1.112.0.exe", "https://update.code.visualstudio.com/latest/win32-x64-user/stable", 112 * MB, "queued", 0, "programs", { queueId: "main" }],
  ["lecture-08-distributed-systems.mp4", "https://ocw.example.edu/lectures/08.m3u8", 1.3 * GB, "failed", 0.27, "video", { kind: "hls", error: "Server returned 403 Forbidden — the link has expired. Use Refresh address." }],
  ["The Art of Computer Programming Vol 1.epub", "https://books.example.com/taocp1.epub", 14.2 * MB, "completed", 1, "documents"],
  ["wallpapers-4k-pack.7z", "https://cdn.example.net/wallpapers/4k-pack.7z", 940 * MB, "idle", 0, "images", { queueId: "night" }],
  ["llama-weights-q4.gguf", "https://huggingface.co/x/model/resolve/main/q4.gguf", 4.9 * GB, "idle", 0, "other", { queueId: "night" }],
  ["podcast-episode-212.mp3", "https://media.example.fm/ep212.mp3", 86 * MB, "completed", 1, "music"],
  ["GIMP-3.2.0-setup.exe", "https://download.gimp.org/gimp/v3.2/windows/gimp-3.2.0-setup.exe", 356 * MB, "completed", 1, "programs"],
  ["Sintel (2010) 4K.mkv", "https://www.youtube.com/watch?v=eRsGyueVLvQ", 2.6 * GB, "paused", 0.52, "video", { kind: "video", media: { title: "Sintel", quality: "2160p · VP9", site: "youtube.com", duration: 888 } }],
  ["fonts-collection.zip", "https://fonts.example.com/collection.zip", 64 * MB, "completed", 1, "compressed"],
  ["kernel-6.18.tar.xz", "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-6.18.tar.xz", 148 * MB, "completed", 1, "compressed"],
  ["presentation-keynote.pptx", "https://share.example.com/keynote.pptx", 42 * MB, "completed", 1, "documents"],
  ["mountain-timelapse-8k.mov", "https://videos.example.com/timelapse-8k.mov", 9.8 * GB, "downloading", 0.07, "video", { connections: 32 }],
];

const items: Record<string, DownloadItem> = {};
seeds.forEach(([fileName, url, size, status, frac, category, extra], idx) => {
  const id = `demo${idx}`;
  items[id] = {
    id,
    kind: "http",
    url,
    finalUrl: null,
    fileName,
    saveDir: `C:\\Users\\you\\Downloads\\${categories.find((c) => c.id === category)?.name ?? ""}`,
    size,
    downloaded: Math.round((size ?? 0) * frac),
    status,
    error: null,
    category,
    queueId: null,
    description: "",
    referer: null,
    addedAt: now - idx * 3_600_000 * 2.3,
    lastTryAt: now - idx * 1_800_000,
    finishedAt: status === "completed" ? now - idx * 1_700_000 : null,
    resumable: true,
    connections: 8,
    speedLimit: 0,
    checksum: null,
    trashed: false,
    position: idx,
    mirrors: [],
    grabberProject: null,
    media: null,
    ...extra,
  };
});

/** Abstract "video frame" thumbnails as SVG data URIs (no network in the demo). */
function demoThumb(a: string, b: string, c: string): string {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 160 90"><defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="${a}"/><stop offset="1" stop-color="${b}"/></linearGradient><radialGradient id="r" cx=".65" cy=".35" r=".5"><stop offset="0" stop-color="${c}" stop-opacity=".85"/><stop offset="1" stop-color="${c}" stop-opacity="0"/></radialGradient></defs><rect width="160" height="90" fill="url(#g)"/><rect width="160" height="90" fill="url(#r)"/><path d="M0 70 Q40 55 80 66 T160 60 V90 H0Z" fill="#000" fill-opacity=".35"/></svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}
const THUMBS: Record<string, string> = {
  demo2: demoThumb("#120a2e", "#2a1050", "#ff7a59"),
  demo3: demoThumb("#071226", "#0b2a3a", "#50f0be"),
  demo7: demoThumb("#8fc7f0", "#4d7f2c", "#f5f1e6"),
  demo17: demoThumb("#2a1408", "#5a2e10", "#ffb35c"),
  demo21: demoThumb("#0a1022", "#2b1c33", "#ffd27a"),
};
for (const [id, thumb] of Object.entries(THUMBS)) if (items[id]) items[id].media = { ...(items[id].media ?? {}), thumbnail: thumb };

// `?demo=1000` adds bulk items to exercise the virtualized list.
const bulk = typeof location !== "undefined" ? Number(new URLSearchParams(location.search).get("demo") ?? 0) : 0;
for (let i = 0; i < bulk; i++) {
  const id = `bulk${i}`;
  const done = i % 3 !== 0;
  items[id] = { ...items.demo4, id, fileName: `archive-part-${String(i + 1).padStart(4, "0")}.zip`, url: `https://files.example.com/parts/${i}.zip`, size: (5 + (i % 40)) * MB, downloaded: done ? (5 + (i % 40)) * MB : 0, status: done ? "completed" : "idle", category: "compressed", addedAt: now - (i + 30) * 600_000, position: 100 + i };
}

function demoParam(name: string): string | null {
  return typeof location === "undefined" ? null : new URLSearchParams(location.search).get(name);
}

const settings: Settings = {
  general: { launchAtStartup: true, startMinimized: false, closeToTray: true, showCaptureDialog: true, showProgressWindow: false, showCompletionDialog: true, notifications: true, confirmDelete: true, useTrash: true, checkUpdates: true, language: "", removeDeletedFiles: false },
  appearance: { theme: (demoParam("theme") as "dark" | "light" | null) ?? "system", accent: "prism", density: demoParam("density") === "compact" ? "compact" : "comfortable", motion: "system", reduceTransparency: false, windowMaterial: false, visibleColumns: ["name", "size", "status", "progress", "speed", "eta", "added"], compactView: demoParam("compact") === "1" },
  downloads: { defaultDir: "C:\\Users\\you\\Downloads", useCategoryFolders: true, defaultConnections: 8, maxConnections: 32, conflict: "ask", maxRetries: 20, speedLimit: 5 * MB, speedLimiterEnabled: false, verifyChecksums: true, autoStart: true },
  connection: { userAgent: "", connectTimeoutSecs: 15, readTimeoutSecs: 30, httpVersion: "http1", maxConnectionsPerHost: 0, dnsOverrides: [] },
  proxy: { mode: "system", address: "", username: "", password: "", bypass: [] },
  capture: { enabled: true, extensions: ["zip", "rar", "7z", "exe", "msi", "iso", "mp4", "mkv", "mp3", "pdf"], mimeTypes: ["application/octet-stream"], excludedSites: ["*.update.microsoft.com"], minSizeKb: 0, bypassKey: "alt", videoButton: true, videoMinDurationSecs: 0 },
  sounds: { enabled: true, onComplete: true, onFailed: true, onQueueDone: true, volume: 0.6, completeSound: "classic" },
  clipboard: { monitor: true, ask: true },
  grabber: { defaultDepth: 2, maxPages: 2000, concurrentRequests: 6, respectRobots: true },
  antivirus: { enabled: false, kind: "auto", command: "", args: "", extensions: [] },
  video: { ytdlpPath: "", ffmpegPath: "", autoUpdate: true, preferredHeight: 1080, container: "mp4", audioFormat: "m4a", embedSubtitles: false, subtitleLangs: "en.*", embedThumbnail: true, embedMetadata: true, embedChapters: true, sponsorblock: "off", accelerate: true, cookiesFromBrowser: "", fileTemplate: "%(title)s [%(id)s].%(ext)s", numberPlaylistVideos: false },
  huggingFace: { endpoint: "", token: "", folder: "" },
  shortcuts: {},
  // The demo bootstrap version: "What's new" opens with ?sheet=whatsnew, or by itself with ?lastseen=1.0.9 (an update).
  lastSeenVersion: (typeof location === "undefined" ? null : new URLSearchParams(location.search).get("lastseen")) ?? "1.1.7",
  onboardingDone: typeof location === "undefined" || !new URLSearchParams(location.search).has("onboarding"),
};

const grabberProjects: GrabberProject[] = [
  {
    id: "g1",
    name: "docs.example.org manuals",
    config: { startUrl: "https://docs.example.org/manuals/", depth: 3, scope: "sameDomain", extensions: ["pdf", "epub"], includeEmbedded: false, filesAnyHost: true, maxPages: 2000, concurrency: 6, exclude: ["logout"], fetchSizes: true, login: null, userAgent: "", cookies: "" },
    results: {
      pages: 148,
      running: false,
      files: [
        "admin-guide.pdf", "install/quickstart.pdf", "install/upgrade-notes.pdf", "api/reference-v3.pdf", "api/reference-v2.pdf", "api/examples.epub", "tutorials/getting-started.pdf", "tutorials/advanced-routing.pdf",
      ].map((p, i) => ({ url: `https://docs.example.org/manuals/${p}`, fileName: p.split("/").pop()!, size: (i + 2) * 1.7 * MB, mime: "application/pdf", referer: "https://docs.example.org/manuals/", depth: p.includes("/") ? 2 : 1 })),
    },
    createdAt: now - 86_400_000,
    updatedAt: now - 3_600_000,
  },
];

// ----- live simulation -----------------------------------------------------------------

const speeds: Record<string, number> = {};
const history: Record<string, number[]> = {};
const segState: Record<string, SegmentInfo[]> = {};

function makeSegments(item: DownloadItem): SegmentInfo[] {
  const size = item.size ?? 0;
  const n = item.connections;
  const segs: SegmentInfo[] = [];
  for (let k = 0; k < n; k++) {
    const start = Math.floor((size * k) / n);
    const end = Math.floor((size * (k + 1)) / n);
    const done = (item.downloaded / size) * (0.6 + 0.8 * Math.random());
    segs.push({ start, end, pos: Math.min(end, start + Math.floor((end - start) * Math.min(1, done))), speed: 0, state: "active" });
  }
  return segs;
}

function tick() {
  const progress: Progress[] = [];
  for (const item of Object.values(items)) {
    if (!["downloading", "connecting", "processing"].includes(item.status)) continue;
    const size = item.size ?? 0;
    const target = item.kind === "torrent" ? 6.5 * MB : 9 * MB + (item.connections > 8 ? 14 * MB : 0);
    speeds[item.id] = (speeds[item.id] ?? target) * 0.85 + target * (0.7 + Math.random() * 0.6) * 0.15;
    if (item.status === "processing") speeds[item.id] = 0;
    const speed = Math.round(speeds[item.id]);
    const h = (history[item.id] ??= Array.from({ length: 120 }, (_, k) => Math.max(0, speed * (0.75 + 0.25 * Math.sin(k / 7)) + (Math.random() - 0.5) * speed * 0.3)));
    h.push(speed);
    if (h.length > 120) h.shift();
    item.downloaded = Math.min(size, item.downloaded + speed / 4);
    const segs = (segState[item.id] ??= makeSegments(item));
    for (const s of segs) {
      if (s.pos >= s.end) {
        s.state = "done";
        s.speed = 0;
        continue;
      }
      s.speed = Math.round((speed / segs.length) * (0.6 + Math.random() * 0.8));
      s.pos = Math.min(s.end, s.pos + s.speed / 4);
    }
    progress.push({
      id: item.id,
      status: item.status,
      downloaded: item.downloaded,
      size: item.size,
      speed,
      etaSecs: speed > 0 ? (size - item.downloaded) / speed : null,
      connections: segs.filter((s) => s.state === "active").length,
      detail: item.status === "processing" ? "Muxing video and audio" : null,
    });
    // Loop demo items so screenshots always show activity.
    if (item.downloaded >= size) {
      item.downloaded = size * 0.05;
      segState[item.id] = makeSegments(item);
    }
  }
  if (progress.length) emit({ type: "progress", payload: progress });
}
setInterval(tick, 250);

// ----- commands --------------------------------------------------------------------------

let captureSeq = 0;
let itemSeq = 0;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const captures: Record<string, CaptureInfo> = {};
const batches: Record<string, BatchInfo> = {};

function newItem(fileName: string, url: string, size: number | null, category: string, extra: Partial<DownloadItem> = {}): DownloadItem {
  const id = `new${++itemSeq}`;
  const item: DownloadItem = {
    ...items.demo4,
    id,
    fileName,
    url,
    size,
    downloaded: 0,
    status: "downloading",
    category,
    saveDir: `C:\\Users\\you\\Downloads\\${categories.find((c) => c.id === category)?.name ?? "Other"}`,
    addedAt: Date.now(),
    lastTryAt: Date.now(),
    finishedAt: null,
    position: Object.keys(items).length,
    queueId: null,
    media: null,
    ...extra,
  };
  items[id] = item;
  emit({ type: "itemsAdded", payload: [{ ...item }] });
  return item;
}

function startStatus(mode: StartMode): Partial<DownloadItem> {
  return mode.type === "now" ? { status: "downloading" } : mode.type === "queue" ? { status: "queued", queueId: mode.queueId } : { status: "idle" };
}

function categoryFor(name: string): string {
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  if (["mp4", "mkv", "webm", "mov"].includes(ext)) return "video";
  if (["mp3", "flac", "m4a", "wav"].includes(ext)) return "music";
  if (["pdf", "epub", "docx", "txt"].includes(ext)) return "documents";
  if (["exe", "msi", "dmg"].includes(ext)) return "programs";
  if (["zip", "7z", "rar", "iso", "gz", "xz", "zst"].includes(ext)) return "compressed";
  if (["jpg", "jpeg", "png", "gif", "webp"].includes(ext)) return "images";
  return "other";
}

function storeBatch(title: string, urls: string[]): BatchInfo {
  const id = `batch${++captureSeq}`;
  batches[id] = {
    id,
    title,
    pageUrl: null,
    entries: urls.map((url) => {
      const fileName = decodeURIComponent(url.split(/[?#]/)[0].split("/").pop() || "file");
      return { url, fileName, size: null, mime: null, category: categoryFor(fileName), referer: null, text: null };
    }),
  };
  return batches[id];
}

function removeWhere(pred: (i: DownloadItem) => boolean) {
  const ids = Object.values(items).filter(pred).map((i) => i.id);
  ids.forEach((id) => delete items[id]);
  if (ids.length) emit({ type: "itemsRemoved", payload: ids });
}

const crawlStop: Record<string, boolean> = {};

/** Simulated site crawl: emits pageVisited / fileFound / finished like the real grabber. */
function startDemoCrawl(projectId: string | null, name: string, config: GrabberProject["config"]): GrabberProject {
  const id = projectId ?? `g${Date.now().toString(36)}`;
  let proj = grabberProjects.find((g) => g.id === id);
  if (!proj) {
    proj = { id, name, config, results: { files: [], pages: 0, running: true }, createdAt: Date.now(), updatedAt: Date.now() };
    grabberProjects.push(proj);
  } else Object.assign(proj, { name, config, results: { files: [], pages: 0, running: true } });
  crawlStop[id] = false;
  const base = config.startUrl.replace(/\/$/, "");
  const exts = config.extensions.length ? config.extensions : ["jpg"];
  const files: FoundFile[] = [];
  const total = 36;
  let pages = 0;
  const project = proj;
  const t = setInterval(() => {
    if (crawlStop[id] || pages >= total) {
      clearInterval(t);
      project.results = { files, pages, running: false };
      emit({ type: "grabber", payload: { project_id: id, event: { type: "finished", pages, files: files.length, cancelled: !!crawlStop[id] } } });
      return;
    }
    pages++;
    const url = `${base}/page-${pages}.html`;
    emit({ type: "grabber", payload: { project_id: id, event: { type: "pageVisited", url, depth: 1 + (pages % 3), pages, queued: total - pages } } });
    for (let k = 0; k < (pages % 3) + 1; k++) {
      const ext = exts[(pages + k) % exts.length];
      const file: FoundFile = { url: `${base}/files/item-${pages}-${k}.${ext}`, fileName: `item-${pages}-${k}.${ext}`, size: (1 + ((pages * 7 + k * 3) % 12)) * MB, mime: null, referer: url, depth: 1 + (pages % 3) };
      files.push(file);
      emit({ type: "grabber", payload: { project_id: id, event: { type: "fileFound", file } } });
    }
    if (pages % 11 === 0) emit({ type: "grabber", payload: { project_id: id, event: { type: "pageError", url, error: "404 Not Found" } } });
  }, 160);
  return proj;
}


function demoCapture(url: string): CaptureInfo {
  const name = decodeURIComponent(url.split(/[?#]/)[0].split("/").pop() || "download.bin");
  return {
    id: `cap${++captureSeq}`,
    source: "browser",
    kind: "http",
    url,
    finalUrl: url,
    fileName: name,
    size: 1.84 * GB,
    mime: "application/x-iso9660-image",
    resumable: true,
    category: "compressed",
    saveDir: "C:\\Users\\you\\Downloads\\Compressed",
    referer: "https://www.debian.org/distrib/",
    pageUrl: "https://www.debian.org/distrib/",
    description: "",
    probing: false,
    probeError: null,
    exists: false,
    needsAuth: false,
  };
}

const demoVideo: VideoCapture = {
  id: "vid1",
  // ?playlist shows the video window for a playlist.
  pageUrl:
    typeof location !== "undefined" && new URLSearchParams(location.search).has("playlist")
      ? "https://www.youtube.com/playlist?list=PLdemo"
      : "https://www.youtube.com/watch?v=aqz-KE-bpKQ",
  title: "Big Buck Bunny 60fps 4K — Official Blender Foundation Short Film",
  streams: [],
  preselected: null,
};

const demoBatch: BatchInfo = {
  id: "batch1",
  title: "Links from releases.example.org",
  pageUrl: "https://releases.example.org/",
  entries: [
    ["app-2.4.0-win-x64.zip", 84, "compressed"],
    ["app-2.4.0-linux-x64.tar.gz", 79, "compressed"],
    ["app-2.4.0-macos-arm64.dmg", 92, "programs"],
    ["app-2.4.0-checksums.txt", 0.01, "documents"],
    ["release-notes-2.4.pdf", 1.2, "documents"],
    ["screenshot-dark.png", 2.4, "images"],
    ["screenshot-light.png", 2.3, "images"],
    ["demo-walkthrough.mp4", 148, "video"],
  ].map(([n, s, c]) => ({ url: `https://releases.example.org/2.4.0/${n}`, fileName: n as string, size: (s as number) * MB, category: c as string, mime: null, referer: null, text: null })),
};

function details(id: string): ItemDetails | null {
  const item = items[id];
  if (!item) return null;
  const segs = (segState[id] ??= makeSegments(item));
  return {
    item,
    segments: segs,
    progress: null,
    speedHistory: history[id] ?? [],
  };
}

const commands: Record<string, (args: Record<string, unknown>) => unknown> = {
  bootstrap: (): Bootstrap => ({
    version: "1.1.7",
    items: Object.values(items),
    categories,
    queues,
    settings,
    grabberProjects,
    dataDir: "C:\\Users\\you\\AppData\\Roaming\\Prism",
    captures: [],
  }),
  update_settings: ({ patch }) => {
    deepMerge(settings as unknown as Record<string, unknown>, patch as Record<string, unknown>);
    // A fresh copy, as the real backend sends: the same object wouldn't re-render the UI.
    emit({ type: "settingsChanged", payload: structuredClone(settings) });
    return structuredClone(settings);
  },
  toggle_speed_limiter: () => {
    settings.downloads.speedLimiterEnabled = !settings.downloads.speedLimiterEnabled;
    emit({ type: "settingsChanged", payload: settings });
    return settings.downloads.speedLimiterEnabled;
  },
  add_url: ({ args }) => {
    const c = demoCapture((args as { url: string }).url);
    captures[c.id] = c;
    return c;
  },
  // Any id works in the demo; the probe "finishes" after a moment (captureUpdated).
  get_capture: ({ id }) => {
    const key = String(id);
    if (!captures[key]) {
      const c = { ...demoCapture("https://cdimage.debian.org/debian-cd/current/amd64/iso-dvd/debian-13.1.0-amd64-DVD-1.iso"), id: key, probing: true, size: null, resumable: null };
      captures[key] = c;
      setTimeout(() => {
        captures[key] = { ...c, probing: false, size: 3.72 * GB, resumable: true };
        emit({ type: "captureUpdated", payload: captures[key] });
      }, 700);
    }
    return captures[key];
  },
  confirm_capture: ({ decision }) => {
    const d = decision as { id: string; fileName?: string; saveDir?: string; category?: string; description?: string; mode: StartMode };
    const c = captures[d.id] ?? demoCapture("https://example.com/file.bin");
    delete captures[d.id];
    return newItem(d.fileName || c.fileName, c.url, c.size ?? null, d.category || c.category, { saveDir: d.saveDir || c.saveDir, description: d.description ?? "", ...startStatus(d.mode) });
  },
  cancel_capture: ({ id }) => void delete captures[String(id)],
  reprobe_capture: () => undefined,
  batch_from_pattern: ({ pattern }) => {
    const p = String(pattern);
    const m = p.match(/\[(\d+)-(\d+)\]/);
    if (!m) throw new Error("The pattern needs a range like [001-100]");
    const [a, b] = [parseInt(m[1], 10), parseInt(m[2], 10)];
    const urls = Array.from({ length: Math.min(500, Math.abs(b - a) + 1) }, (_, i) => p.replace(m[0], String(Math.min(a, b) + i).padStart(m[1].length, "0")));
    return storeBatch("Pattern download", urls);
  },
  batch_from_text: ({ text }) => {
    const urls = String(text)
      .split(/\s+/)
      .filter((t) => t.includes("://") || t.startsWith("magnet:"));
    if (!urls.length) throw new Error("No addresses found");
    return storeBatch("Links from text", urls);
  },
  get_batch: ({ id }) => batches[String(id)] ?? demoBatch,
  confirm_batch: ({ decision }) => {
    const d = decision as { id: string; urls: string[]; mode: StartMode };
    const b = batches[d.id] ?? demoBatch;
    for (const u of d.urls) {
      const e = b.entries.find((x) => x.url === u);
      newItem(e?.fileName || u.split("/").pop() || "file", u, e?.size ?? null, e?.category || "other", startStatus(d.mode));
    }
    return d.urls.length;
  },
  cancel_batch: () => undefined,
  get_video: () => demoVideo,
  open_video_url: ({ url }) => ({ ...demoVideo, pageUrl: String(url) }),
  confirm_video: ({ decision }) => {
    const d = decision as {
      title?: string;
      entries: (string | { url: string; title?: string | null; index?: number })[];
      choice: { label: string; kind: string };
      options?: { container?: string };
      mode: StartMode;
      duration?: number;
      thumbnail?: string;
      pageUrl: string;
    };
    // The last request, for checks in the demo.
    (globalThis as { __prismLastVideo?: unknown }).__prismLastVideo = d;
    const list = d.entries.length ? d.entries.map((e) => (typeof e === "string" ? { url: e, title: null } : e)) : [{ url: d.pageUrl, title: d.title }];
    const ext = d.choice.kind === "audio" ? "mp3" : (d.options?.container ?? "mp4");
    list.forEach((e) =>
      newItem(`${e.title ?? "Video"}.${ext}`, e.url, 400 * MB, d.choice.kind === "audio" ? "music" : "video", {
        kind: "video",
        media: { title: e.title ?? undefined, quality: d.choice.label, site: "youtube.com", duration: d.duration, thumbnail: d.thumbnail },
        ...startStatus(d.mode),
      }),
    );
    return list.length;
  },
  cancel_video: () => undefined,
  add_stream: ({ args }) => {
    const a = args as { stream: { url: string; kind: string; size?: number }; title?: string };
    return newItem(`${a.title ?? "stream"}.mp4`, a.stream.url, a.stream.size ?? null, "video", { kind: a.stream.kind === "dash" ? "dash" : "hls" });
  },
  read_torrent_file: ({ path }) => ({ type: "file", value: btoa(String(path)) }),
  inspect_torrent: async () => {
    await sleep(600);
    const files: [string, number][] = [
      ["debian-13.1.0-amd64-DVD-1.iso", 3.9 * GB],
      ["debian-13.1.0-amd64-DVD-2.iso", 4.4 * GB],
      ["docs/README.txt", 4 * 1024],
      ["docs/install-guide.pdf", 2.1 * MB],
      ["docs/release-notes.pdf", 640 * 1024],
      ["checksums/SHA256SUMS", 2 * 1024],
      ["checksums/SHA256SUMS.sign", 1024],
    ];
    return {
      infoHash: "7a3c1f9e0b2d4c6e8f00112233445566778899aa",
      name: "debian-13.1.0-amd64-DVD",
      totalSize: files.reduce((s, f) => s + f[1], 0),
      files: files.map(([path, size], index) => ({ index, path, size })),
    };
  },
  native_material: () => true,
  hf_context: () => ({ endpoint: "https://huggingface.co", token: null, folder: "C:\\Users\\demo\\Downloads\\Models", lmStudio: "C:\\Users\\demo\\.lmstudio\\models" }),
  hf_search: async ({ query }) => {
    await sleep(250);
    const all = [
      { id: "unsloth/Qwen3-8B-GGUF", downloads: 2_410_000, likes: 310, pipelineTag: "text-generation", gguf: true, gated: false },
      { id: "Qwen/Qwen3-8B", downloads: 8_095_680, likes: 2152, pipelineTag: "text-generation", gguf: false, gated: false },
      { id: "meta-llama/Llama-3.1-8B-Instruct", downloads: 6_120_000, likes: 4800, pipelineTag: "text-generation", gguf: false, gated: true },
      { id: "google/gemma-3-4b-it-qat-q4_0-gguf", downloads: 940_000, likes: 520, pipelineTag: "image-text-to-text", gguf: true, gated: true },
      { id: "openai/gpt-oss-20b", downloads: 5_300_000, likes: 3900, pipelineTag: "text-generation", gguf: false, gated: false },
      { id: "black-forest-labs/FLUX.1-schnell", downloads: 1_800_000, likes: 4100, pipelineTag: "text-to-image", gguf: false, gated: false },
    ];
    const q = String(query ?? "").toLowerCase();
    return q ? all.filter((h) => h.id.toLowerCase().includes(q.split(" ")[0])) : all;
  },
  hf_repo: async ({ input }) => {
    await sleep(450);
    const id = String(input).replace(/^https?:\/\/[^/]+\//, "").split("/").slice(0, 2).join("/");
    if (!/^[\w.-]+\/[\w.-]+$/.test(id)) throw new Error("hf:notFound");
    const gguf = /gguf/i.test(id);
    const files: [string, number][] = gguf
      ? [
          [".gitattributes", 2_100], ["README.md", 9_800], ["Qwen3-8B-Q2_K.gguf", 3.28 * GB], ["Qwen3-8B-Q3_K_M.gguf", 4.12 * GB],
          ["Qwen3-8B-Q4_K_M.gguf", 5.03 * GB], ["Qwen3-8B-Q5_K_M.gguf", 5.85 * GB], ["Qwen3-8B-Q6_K.gguf", 6.73 * GB],
          ["Qwen3-8B-Q8_0.gguf", 8.71 * GB], ["Qwen3-8B-BF16.gguf", 16.4 * GB],
        ]
      : [
          [".gitattributes", 1_600], ["README.md", 16_000], ["config.json", 728], ["generation_config.json", 239],
          ["model-00001-of-00005.safetensors", 3.99 * GB], ["model-00002-of-00005.safetensors", 3.99 * GB], ["model-00003-of-00005.safetensors", 3.96 * GB],
          ["model-00004-of-00005.safetensors", 3.19 * GB], ["model-00005-of-00005.safetensors", 1.24 * GB], ["model.safetensors.index.json", 32_900],
          ["tokenizer.json", 11.4 * MB], ["tokenizer_config.json", 9_700], ["merges.txt", 1.67 * MB], ["vocab.json", 2.78 * MB],
        ];
    const variants = gguf
      ? files.filter(([p]) => p.endsWith(".gguf")).map(([p, size]) => ({ label: p.replace(/^.*-(\w+)\.gguf$/, "$1"), files: [p], size }))
      : [];
    const suggested = gguf ? ["Qwen3-8B-Q4_K_M.gguf"] : files.map(([p]) => p).filter((p) => p !== ".gitattributes");
    return {
      kind: "model", id, sha: "b968826d9c46dd6066d109eabc6255188de91218", revision: "main", gated: /llama|gemma/i.test(id), private: false,
      downloads: gguf ? 2_410_000 : 8_095_680, likes: gguf ? 310 : 2152, pipelineTag: "text-generation", library: gguf ? null : "transformers", gguf,
      files: files.map(([path, size]) => ({ path, size, sha256: null })), path: null, pageUrl: `https://huggingface.co/${id}`,
      folder: `C:\\Users\\demo\\Downloads\\Models\\${id.replace("/", "\\")}`,
      suggestion: { variants, variant: gguf ? "Q4_K_M" : null, extras: [], files: suggested },
    };
  },
  hf_present: () => ({ onDisk: [], listed: [] }),
  hf_add: ({ args }) => {
    const a = args as { id: string; files: { path: string; size: number }[]; mode?: StartMode | null };
    for (const f of a.files) newItem(f.path.split("/").pop()!, `https://huggingface.co/${a.id}/resolve/main/${f.path}`, f.size, "other", startStatus(a.mode ?? { type: "now" }));
    return a.files.length;
  },
  add_torrent: ({ args }) => {
    const a = args as { name: string; size?: number; mode?: StartMode };
    return newItem(a.name, "magnet:?xt=urn:btih:7a3c1f9e", a.size ?? null, "compressed", { kind: "torrent", ...startStatus(a.mode ?? { type: "now" }) });
  },
  update_item: ({ id, patch }) => {
    const it = items[String(id)];
    if (!it) throw new Error("unknown item");
    Object.assign(it, patch);
    emit({ type: "itemUpdated", payload: { ...it } });
    return it;
  },
  redownload: ({ id }) => {
    const it = items[String(id)];
    if (it) {
      Object.assign(it, { downloaded: 0, status: "downloading", error: null });
      emit({ type: "itemUpdated", payload: { ...it } });
    }
  },
  remove_completed: () => removeWhere((i) => i.status === "completed" && !i.trashed),
  empty_trash: () => removeWhere((i) => i.trashed),
  assign_to_queue: ({ ids, queueId }) => {
    for (const id of ids as string[]) {
      items[id].queueId = (queueId as string | null) ?? null;
      if (queueId && items[id].status === "idle") items[id].status = "queued";
      emit({ type: "itemUpdated", payload: { ...items[id] } });
    }
  },
  move_in_queue: ({ ids, delta }) => {
    const it = items[(ids as string[])[0]];
    if (!it?.queueId) return;
    const list = Object.values(items)
      .filter((i) => i.queueId === it.queueId)
      .sort((a, b) => a.position - b.position);
    const from = list.indexOf(it);
    const to = Math.max(0, Math.min(list.length - 1, from + (delta as number)));
    list.splice(to, 0, ...list.splice(from, 1));
    list.forEach((x, i) => {
      x.position = i;
      emit({ type: "itemUpdated", payload: { ...x } });
    });
  },
  upsert_queue: ({ queue }) => {
    const q = queue as Queue;
    const i = queues.findIndex((x) => x.id === q.id);
    if (i >= 0) queues[i] = { ...q, running: queues[i].running };
    else queues.push(q);
    emit({ type: "queuesChanged", payload: [...queues] });
    return [...queues];
  },
  delete_queue: ({ id }) => {
    const i = queues.findIndex((x) => x.id === id);
    if (i >= 0 && !queues[i].builtin) queues.splice(i, 1);
    emit({ type: "queuesChanged", payload: [...queues] });
    return [...queues];
  },
  upsert_category: ({ category }) => {
    const c = category as Category;
    const i = categories.findIndex((x) => x.id === c.id);
    if (i >= 0) categories[i] = c;
    else categories.push(c);
    emit({ type: "categoriesChanged", payload: [...categories] });
    return [...categories];
  },
  delete_category: ({ id }) => {
    const i = categories.findIndex((x) => x.id === id);
    if (i >= 0 && !categories[i].builtin) categories.splice(i, 1);
    emit({ type: "categoriesChanged", payload: [...categories] });
    return [...categories];
  },
  start_grabber: ({ projectId, name, config }) => startDemoCrawl((projectId as string | null) ?? null, String(name), config as GrabberProject["config"]),
  stop_grabber: ({ projectId }) => void (crawlStop[String(projectId)] = true),
  delete_grabber_project: ({ projectId }) => {
    const i = grabberProjects.findIndex((g) => g.id === projectId);
    if (i >= 0) grabberProjects.splice(i, 1);
  },
  download_grabbed: ({ projectId, urls }) => {
    for (const u of urls as string[]) newItem(u.split("/").pop() || "file", u, 3 * MB, categoryFor(u), { grabberProject: String(projectId), status: "queued", queueId: "main" });
    return (urls as string[]).length;
  },
  ensure_tools: async () => {
    await sleep(1200);
    return commands.tool_status({});
  },
  update_ytdlp: async () => {
    await sleep(1200);
    return { updated: false, version: "2026.10.09" };
  },
  repair_integration: async () => {
    await sleep(500);
    return commands.integration_status({});
  },
  check_update: () => (demoParam("update") ? { version: "1.0.3", current: "1.0.2", notes: "Faster, friendlier, fewer bugs.", canInstall: true } : null),
  install_update: async () => {
    for (let i = 1; i <= 20; i++) {
      await sleep(120);
      emitApp("update-progress", [i * 800_000, 16_000_000]);
    }
  },
  extension_dir: () => "C:\\Program Files\\Prism\\extension",
  extension_folder: ({ firefox }) => `C:\\Users\\you\\AppData\\Local\\Prism\\extension\\${firefox ? "firefox" : "chromium"}`,
  open_browser: () => {
    // Demo: pretend the user installs the extension a few seconds after the page opens.
    if (!mockExtSeen) setTimeout(() => (mockExtSeen = Date.now()), 6000);
  },
  open_extension_store: () => {
    // Demo: pretend the user clicks "Add to Chrome" a few seconds after the store page opens.
    mockExtFromStore = true;
    if (!mockExtSeen) setTimeout(() => (mockExtSeen = Date.now()), 4000);
  },
  cancel_post_action: () => undefined,
  item_details: ({ id }) => details(id as string),
  grabber_projects: () => grabberProjects,
  next_schedule_runs: () => [["night", "2026-10-15 02:00"]],
  probe_video: async ({ url }) => {
    await sleep(900);
    if (String(url).includes("list=")) {
      return {
        kind: "playlist",
        id: "PLdemo",
        title: "Blender Open Movies",
        uploader: "Blender",
        webpageUrl: url,
        extractor: "youtube:tab",
        entries: ["Big Buck Bunny", "Sintel", "Tears of Steel", "Cosmos Laundromat", "Spring", "Agent 327: Operation Barbershop"].map((t, i) => ({
          id: `e${i}`,
          url: `https://www.youtube.com/watch?v=demo${i}`,
          title: t,
          duration: 300 + i * 97,
          thumbnail: Object.values(THUMBS)[i % 5],
          uploader: "Blender",
        })),
      };
    }
    const choice = (id: string, kind: "video" | "audio", label: string, subLabel: string, est: number, extra: Record<string, unknown> = {}) => ({
      id, kind, label, subLabel, estSize: est, hdr: false, container: "mp4", accelerable: kind === "video", ...extra,
    });
    return {
      kind: "video",
      id: "aqz-KE-bpKQ",
      title: demoVideo.title,
      uploader: "Blender",
      channel: "Blender",
      duration: 635,
      uploadDate: "20140110",
      thumbnail: THUMBS.demo7,
      webpageUrl: demoVideo.pageUrl,
      extractor: "youtube",
      isLive: false,
      chapters: [],
      formats: [],
      hasDrm: false,
      subtitles: { en: { name: "English", auto: false }, de: { name: "German", auto: false }, cs: { name: "Czech", auto: true } },
      choices: [
        choice("v2160p60-hdr", "video", "2160p60 · HDR · AV1", "MP4 · ~1.9 GB", 1.9 * GB, { height: 2160, fps: 60, hdr: true, vcodec: "AV1" }),
        choice("v2160p60", "video", "2160p60 · VP9", "MKV · ~1.6 GB", 1.6 * GB, { height: 2160, fps: 60, vcodec: "VP9", container: "mkv" }),
        choice("v1440p60", "video", "1440p60 · VP9", "MKV · ~820 MB", 820 * MB, { height: 1440, fps: 60, vcodec: "VP9", container: "mkv" }),
        choice("v1080p60", "video", "1080p60 · H.264", "MP4 · ~412 MB", 412 * MB, { height: 1080, fps: 60, vcodec: "H.264" }),
        choice("v720p60", "video", "720p60 · H.264", "MP4 · ~198 MB", 198 * MB, { height: 720, fps: 60, vcodec: "H.264" }),
        choice("v480p", "video", "480p · H.264", "MP4 · ~74 MB", 74 * MB, { height: 480, vcodec: "H.264" }),
        choice("a-m4a", "audio", "Audio · M4A", "AAC 129 kbps · original", 10 * MB, { audioFormat: "m4a", acodec: "AAC" }),
        choice("a-mp3", "audio", "Audio · MP3", "320 kbps · converted", 25 * MB, { audioFormat: "mp3", audioBitrateKbps: 320 }),
        choice("a-opus", "audio", "Audio · Opus", "160 kbps · original", 12 * MB, { audioFormat: "opus", acodec: "Opus" }),
      ],
    };
  },
  tool_status: () => ({
    ytDlp: { tool: "yt-dlp", source: "managed", version: "2026.10.09", path: "…\tools\yt-dlp.exe" },
    ffmpeg: { tool: "ffmpeg", source: "system", version: "9.0.1", path: "C:\ffmpeg\bin\ffmpeg.exe" },
    ffprobe: { tool: "ffprobe", source: "system", version: "9.0.1", path: "C:\ffmpeg\bin\ffprobe.exe" },
    deno: { tool: "deno", source: "managed", version: "2.9.7", path: "…\tools\deno.exe" },
  }),
  integration_status: () => ({
    hostPath: "C:\\Program Files\\Prism\\prism-native-host.exe",
    extensionConnected: mockExtSeen > 0,
    extensionVersion: mockExtSeen > 0 ? (demoParam("oldext") ? "1.0.0" : "1.1.7") : null,
    extensionInstallType: mockExtSeen > 0 ? (demoParam("exttype") ?? (mockExtFromStore ? "normal" : "development")) : null,
    extensionOutdated: mockExtSeen > 0 && !!demoParam("oldext") && !["normal", "admin"].includes(demoParam("exttype") ?? ""),
    appVersion: "1.1.7",
    lastSeen: mockExtSeen || null,
    browsers: (
      [
        ["chrome", "Google Chrome", true, true],
        ["edge", "Microsoft Edge", true, true],
        ["brave", "Brave", true, true],
        ["firefox", "Firefox", true, false],
        ["opera", "Opera", false, false],
        ["vivaldi", "Vivaldi", false, false],
        ["helium", "Helium", false, false],
        ["chromium", "Chromium", false, false],
        ["zen", "Zen Browser", true, false],
      ] as const
    ).map(([browser, name, installed, registered]) => ({ browser, name, installed, supported: true, registered, pointsToThisHost: registered, manifestPath: null })),
  }),
  free_space: () => 412 * GB,
  data_dir: () => "C:\\Users\\you\\AppData\\Roaming\\Prism",
  pause_items: ({ ids }) => setStatus(ids as string[], "paused"),
  start_items: ({ ids }) => setStatus(ids as string[], "downloading"),
  pause_all: () => setStatus(Object.values(items).filter((i) => i.status === "downloading").map((i) => i.id), "paused"),
  resume_all: () => setStatus(Object.values(items).filter((i) => i.status === "paused").map((i) => i.id), "downloading"),
  delete_items: ({ ids, permanent }) => {
    if (permanent) return removeWhere((i) => (ids as string[]).includes(i.id));
    for (const id of ids as string[]) {
      items[id].trashed = true;
      emit({ type: "itemUpdated", payload: { ...items[id] } });
    }
  },
  restore_items: ({ ids }) => {
    for (const id of ids as string[]) {
      items[id].trashed = false;
      emit({ type: "itemUpdated", payload: { ...items[id] } });
    }
  },
  start_queue: ({ id }) => setQueue(id as string, true),
  stop_queue: ({ id }) => setQueue(id as string, false),
};

function setQueue(id: string, running: boolean) {
  const q = queues.find((x) => x.id === id);
  if (q) q.running = running;
  emit({ type: "queuesChanged", payload: [...queues] });
}

function setStatus(ids: string[], status: Status) {
  for (const id of ids) {
    const i = items[id];
    if (!i || i.status === "completed") continue;
    i.status = status;
    emit({ type: "itemUpdated", payload: { ...i } });
  }
}

function deepMerge(target: Record<string, unknown>, patch: Record<string, unknown>) {
  for (const [k, v] of Object.entries(patch ?? {})) {
    if (v && typeof v === "object" && !Array.isArray(v) && typeof target[k] === "object") {
      deepMerge(target[k] as Record<string, unknown>, v as Record<string, unknown>);
    } else {
      target[k] = v;
    }
  }
}

export async function invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  const fn = commands[cmd];
  if (!fn) {
    console.info(`[demo] ${cmd}`, args);
    return undefined as T;
  }
  return (await fn(args)) as T;
}
