// Types mirroring prism-core (serde camelCase). Keep in sync with
// crates/prism-core/src/{model,settings,events}.rs.

export type ItemKind = "http" | "ftp" | "hls" | "dash" | "video" | "torrent";
export type Status =
  | "idle"
  | "queued"
  | "connecting"
  | "downloading"
  | "retrying"
  | "paused"
  | "processing"
  | "completed"
  | "failed";

export interface MediaInfo {
  title?: string | null;
  thumbnail?: string | null;
  duration?: number | null;
  quality?: string | null;
  site?: string | null;
  pageUrl?: string | null;
  /** 1-based position in the playlist it was picked from. */
  playlistIndex?: number | null;
}

export interface DownloadItem {
  id: string;
  kind: ItemKind;
  url: string;
  finalUrl?: string | null;
  fileName: string;
  saveDir: string;
  size?: number | null;
  downloaded: number;
  status: Status;
  error?: string | null;
  category: string;
  queueId?: string | null;
  description: string;
  referer?: string | null;
  addedAt: number;
  lastTryAt?: number | null;
  finishedAt?: number | null;
  resumable?: boolean | null;
  connections: number;
  speedLimit: number;
  checksum?: string | null;
  trashed: boolean;
  position: number;
  mirrors: string[];
  grabberProject?: string | null;
  media?: MediaInfo | null;
}

export interface Progress {
  id: string;
  status: Status;
  downloaded: number;
  size?: number | null;
  speed: number;
  etaSecs?: number | null;
  connections: number;
  detail?: string | null;
}

export interface SegmentInfo {
  start: number;
  pos: number;
  end: number;
  speed: number;
  state: string;
}

export interface ItemDetails {
  item: DownloadItem;
  segments: SegmentInfo[];
  progress?: Progress | null;
  speedHistory: number[];
}

export interface Category {
  id: string;
  name: string;
  icon: string;
  extensions: string[];
  saveDir?: string | null;
  builtin: boolean;
  position: number;
}

export type PostAction =
  | { type: "none" }
  | { type: "exit" }
  | { type: "sleep" }
  | { type: "hibernate" }
  | { type: "shutdown" }
  | { type: "lock" }
  | { type: "sound" }
  | { type: "command"; value: string };

export interface Schedule {
  enabled: boolean;
  startTime?: string | null;
  stopTime?: string | null;
  days: number[];
  onceDate?: string | null;
  retryFailed: boolean;
  onComplete: PostAction;
  wake: boolean;
}

export interface Queue {
  id: string;
  name: string;
  maxConcurrent: number;
  speedLimit: number;
  position: number;
  builtin: boolean;
  schedule: Schedule;
  running: boolean;
}

export interface GrabberProject {
  id: string;
  name: string;
  config: GrabberConfig;
  results: { files?: FoundFile[]; pages?: number; running?: boolean };
  createdAt: number;
  updatedAt: number;
}

export type GrabberScope = "sameHost" | "sameDomain" | "subPath" | "any";

export interface GrabberConfig {
  startUrl: string;
  depth: number;
  scope: GrabberScope;
  extensions: string[];
  includeEmbedded: boolean;
  filesAnyHost: boolean;
  maxPages: number;
  concurrency: number;
  exclude: string[];
  fetchSizes: boolean;
  login?: { username: string; password: string; formUrl?: string | null; formFields: [string, string][] } | null;
  userAgent: string;
  cookies: string;
}

export interface FoundFile {
  url: string;
  fileName: string;
  size?: number | null;
  mime?: string | null;
  referer: string;
  depth: number;
}

export type GrabberEvent =
  | { type: "pageVisited"; url: string; depth: number; pages: number; queued: number }
  | { type: "fileFound"; file: FoundFile }
  | { type: "pageError"; url: string; error: string }
  | { type: "finished"; pages: number; files: number; cancelled: boolean };

export type Theme = "system" | "dark" | "light";
export type ConflictAction = "ask" | "rename" | "overwrite";

export interface Settings {
  general: {
    launchAtStartup: boolean;
    startMinimized: boolean;
    closeToTray: boolean;
    showCaptureDialog: boolean;
    showProgressWindow: boolean;
    showCompletionDialog: boolean;
    notifications: boolean;
    confirmDelete: boolean;
    useTrash: boolean;
    checkUpdates: boolean;
    /** UI language code ("en", "cs", …); "" = the system's language. */
    language: string;
    /** Finished downloads whose file was deleted from the computer leave the list. */
    removeDeletedFiles: boolean;
  };
  appearance: {
    theme: Theme;
    accent: string;
    density: "comfortable" | "compact";
    motion: "system" | "full" | "reduced";
    reduceTransparency: boolean;
    windowMaterial: boolean;
    visibleColumns: string[];
    /** Just the download list: no sidebar, details pane or tips. */
    compactView: boolean;
  };
  downloads: {
    defaultDir: string;
    useCategoryFolders: boolean;
    defaultConnections: number;
    maxConnections: number;
    conflict: ConflictAction;
    maxRetries: number;
    speedLimit: number;
    speedLimiterEnabled: boolean;
    verifyChecksums: boolean;
    autoStart: boolean;
  };
  connection: {
    userAgent: string;
    connectTimeoutSecs: number;
    readTimeoutSecs: number;
    httpVersion: "http1" | "auto" | "http3";
    maxConnectionsPerHost: number;
    dnsOverrides?: string[];
  };
  proxy: {
    mode: "none" | "system" | "http" | "socks5";
    address: string;
    username: string;
    password: string;
    bypass: string[];
  };
  capture: {
    enabled: boolean;
    extensions: string[];
    mimeTypes: string[];
    excludedSites: string[];
    minSizeKb: number;
    bypassKey: "alt" | "ctrl" | "shift" | "none";
    videoButton: boolean;
    videoMinDurationSecs: number;
  };
  sounds: { enabled: boolean; onComplete: boolean; onFailed: boolean; onQueueDone: boolean; volume: number; completeSound: string };
  clipboard: { monitor: boolean; ask: boolean };
  grabber: { defaultDepth: number; maxPages: number; concurrentRequests: number; respectRobots: boolean };
  antivirus: {
    enabled: boolean;
    kind: "auto" | "defender" | "clamav" | "custom";
    command: string;
    args: string;
    extensions: string[];
  };
  video: {
    ytdlpPath: string;
    ffmpegPath: string;
    autoUpdate: boolean;
    preferredHeight: number;
    container: string;
    audioFormat: string;
    embedSubtitles: boolean;
    subtitleLangs: string;
    embedThumbnail: boolean;
    embedMetadata: boolean;
    embedChapters: boolean;
    sponsorblock: "off" | "mark" | "remove";
    accelerate: boolean;
    cookiesFromBrowser: string;
    fileTemplate: string;
    /** Start playlist videos' file names with their position ("01 - Title"). */
    numberPlaylistVideos: boolean;
  };
  /** Models from Hugging Face. */
  huggingFace: {
    /** "" = HF_ENDPOINT if set, else huggingface.co; or a mirror's address. */
    endpoint: string;
    /** For gated and private models ("" = the one the Hugging Face tools saved, if any). */
    token: string;
    /** "" = Models in the download folder. */
    folder: string;
  };
  shortcuts: Record<string, string>;
  onboardingDone: boolean;
  /** The app version whose "What's new" was shown ("" before 1.1.0). */
  lastSeenVersion: string;
}

export type CaptureSource = "manual" | "browser" | "clipboard" | "batch" | "grabber";

export interface CaptureInfo {
  id: string;
  source: CaptureSource;
  kind: ItemKind;
  url: string;
  finalUrl?: string | null;
  fileName: string;
  size?: number | null;
  mime?: string | null;
  resumable?: boolean | null;
  category: string;
  saveDir: string;
  referer?: string | null;
  pageUrl?: string | null;
  description: string;
  probing: boolean;
  probeError?: string | null;
  exists: boolean;
  needsAuth: boolean;
}

export interface BatchEntry {
  url: string;
  fileName: string;
  size?: number | null;
  mime?: string | null;
  category: string;
  referer?: string | null;
  text?: string | null;
}

export interface BatchInfo {
  id: string;
  title: string;
  pageUrl?: string | null;
  entries: BatchEntry[];
}

export interface SniffedStream {
  url: string;
  kind: string;
  mime?: string | null;
  size?: number | null;
  label?: string | null;
}

export interface VideoCapture {
  id: string;
  pageUrl: string;
  title?: string | null;
  streams: SniffedStream[];
  preselected?: string | null;
}

export type StartMode = { type: "now" } | { type: "later" } | { type: "queue"; queueId: string };

export interface Bootstrap {
  version: string;
  items: DownloadItem[];
  categories: Category[];
  queues: Queue[];
  settings: Settings;
  grabberProjects: GrabberProject[];
  dataDir: string;
  captures: CaptureInfo[];
}

export type CoreEvent =
  | { type: "itemsAdded"; payload: DownloadItem[] }
  | { type: "itemUpdated"; payload: DownloadItem }
  | { type: "itemsRemoved"; payload: string[] }
  | { type: "progress"; payload: Progress[] }
  | { type: "queuesChanged"; payload: Queue[] }
  | { type: "categoriesChanged"; payload: Category[] }
  | { type: "settingsChanged"; payload: Settings }
  | { type: "captureRequested"; payload: CaptureInfo }
  | { type: "captureUpdated"; payload: CaptureInfo }
  | { type: "batchRequested"; payload: BatchInfo }
  | { type: "videoRequested"; payload: VideoCapture }
  | { type: "completed"; payload: DownloadItem }
  | { type: "failed"; payload: DownloadItem }
  | { type: "queueFinished"; payload: { queue_id: string; name: string } }
  | { type: "postActionPending"; payload: { action: PostAction; seconds: number } }
  | { type: "playSound"; payload: { kind: string } }
  | { type: "notify"; payload: { title: string; body: string } }
  | { type: "clipboardUrls"; payload: string[] }
  | { type: "grabber"; payload: { project_id: string; event: GrabberEvent } }
  | { type: "toolsProgress"; payload: { tool: string; downloaded: number; total?: number | null } }
  | { type: "showMain" }
  | { type: "exitRequested" };
