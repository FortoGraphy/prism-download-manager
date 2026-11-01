// Typed bridge to the Rust side. When the UI runs outside Tauri (plain browser
// during development, design reviews, marketing screenshots) calls are served by
// an in-memory demo backend instead.

import type { MediaFile, MediaJob, MediaRequest, ShrinkPlan, ToolsRequest } from "./media";
import type {
  BatchInfo,
  Bootstrap,
  CaptureInfo,
  Category,
  CoreEvent,
  DownloadItem,
  GrabberConfig,
  GrabberProject,
  ItemDetails,
  Queue,
  Settings,
  SniffedStream,
  StartMode,
  VideoCapture,
  ZipFile,
  VideoSection,
} from "./types";
import type { VideoChapter } from "./videoPart";

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

type InvokeFn = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;

let invokeImpl: InvokeFn | null = null;

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!invokeImpl) {
    if (inTauri) {
      const core = await import("@tauri-apps/api/core");
      invokeImpl = core.invoke as InvokeFn;
    } else {
      const mock = await import("./mock");
      invokeImpl = mock.invoke as InvokeFn;
    }
  }
  return invokeImpl<T>(cmd, args);
}

/** Subscribe to core events. Returns an unsubscribe function. */
export async function onCoreEvent(handler: (e: CoreEvent) => void): Promise<() => void> {
  if (inTauri) {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<CoreEvent>("core", (e) => handler(e.payload));
  }
  const mock = await import("./mock");
  return mock.subscribe(handler);
}

/** Subscribe to a named app event emitted by the shell (e.g. `update-progress`). */
export async function onAppEvent<T>(name: string, handler: (payload: T) => void): Promise<() => void> {
  if (inTauri) {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<T>(name, (e) => handler(e.payload));
  }
  const mock = await import("./mock");
  return mock.subscribeApp(name, handler as (p: unknown) => void);
}

export interface CaptureDecision {
  id: string;
  fileName?: string;
  saveDir?: string;
  category?: string;
  description?: string;
  mode: StartMode;
  username?: string;
  password?: string;
  rememberDir?: boolean;
  conflict?: "rename" | "overwrite";
}

export interface ItemPatch {
  fileName?: string;
  saveDir?: string;
  description?: string;
  category?: string;
  queueId?: string | null;
  url?: string;
  connections?: number;
  speedLimit?: number;
  checksum?: string | null;
  username?: string;
  password?: string;
}

export interface VideoDecision {
  captureId: string;
  pageUrl: string;
  choice: VideoChoice;
  options: VideoOutputOptions;
  saveDir?: string;
  title?: string;
  thumbnail?: string;
  duration?: number;
  /** Playlist entries to download (each with its title and 1-based position). */
  entries: VideoEntry[];
  mode: StartMode;
  /** Playlists: one video after another (the "One at a time" queue). */
  oneAtATime?: boolean;
  /** A single video: only this part of it. */
  section?: VideoSection;
}

export interface VideoEntry {
  url: string;
  title?: string | null;
  index?: number;
}

export interface TorrentMeta {
  infoHash: string;
  name: string;
  totalSize: number;
  files: { index: number; path: string; size: number }[];
}

export type TorrentSource = { type: "url"; value: string } | { type: "file"; value: string };

/** A model (or dataset) on Hugging Face and its files (#29). */
export type HfKind = "model" | "dataset";
export interface HfFile {
  path: string;
  size: number;
  sha256?: string | null;
}
export interface HfVariant {
  label: string;
  files: string[];
  size: number;
}
export interface HfRepo {
  kind: HfKind;
  id: string;
  sha: string;
  revision: string;
  gated: boolean;
  private: boolean;
  downloads?: number | null;
  likes?: number | null;
  pipelineTag?: string | null;
  library?: string | null;
  gguf: boolean;
  files: HfFile[];
  path?: string | null;
  pageUrl: string;
  folder: string;
  suggestion: { variants: HfVariant[]; variant?: string | null; extras: string[]; files: string[] };
}
export interface HfHit {
  id: string;
  downloads?: number | null;
  likes?: number | null;
  pipelineTag?: string | null;
  gguf: boolean;
  gated: boolean;
}
export interface HfContext {
  endpoint: string;
  token?: "settings" | "environment" | "login" | null;
  folder: string;
  lmStudio?: string | null;
}
export interface HfPresent {
  onDisk: string[];
  listed: string[];
}

export const api = {
  bootstrap: () => invoke<Bootstrap>("bootstrap"),
  updateSettings: (patch: unknown) => invoke<Settings>("update_settings", { patch }),
  toggleSpeedLimiter: () => invoke<boolean>("toggle_speed_limiter"),

  addUrl: (args: { url: string; referer?: string; username?: string; password?: string; fileName?: string }) =>
    invoke<CaptureInfo>("add_url", { args }),
  getCapture: (id: string) => invoke<CaptureInfo | null>("get_capture", { id }),
  confirmCapture: (decision: CaptureDecision) => invoke<DownloadItem>("confirm_capture", { decision }),
  cancelCapture: (id: string) => invoke<void>("cancel_capture", { id }),
  reprobeCapture: (id: string, username?: string, password?: string) =>
    invoke<void>("reprobe_capture", { id, username, password }),
  /** The files in a captured ZIP, read from its end. Rejects with a code: `zip:noRanges`, `zip:notZip`, `zip:split`, `zip:tooBig`, `zip:protected`, `zip:changed`, `zip:network`, `zip:damaged`, `zip:server`. */
  zipContents: (id: string) => invoke<ZipFile[]>("zip_contents", { id }),
  /** Downloads one file of it into `dir` and returns its path (progress: `zipProgress` events). Also rejects with `zip:disk`, `zip:threat`, `zip:cancelled`. */
  extractZipFile: (id: string, index: number, dir: string) => invoke<string>("extract_zip_file", { id, index, dir }),
  cancelZipFile: (id: string, index: number) => invoke<void>("cancel_zip_file", { id, index }),

  batchFromPattern: (pattern: string) => invoke<BatchInfo>("batch_from_pattern", { pattern }),
  batchFromText: (text: string) => invoke<BatchInfo>("batch_from_text", { text }),
  getBatch: (id: string) => invoke<BatchInfo | null>("get_batch", { id }),
  confirmBatch: (decision: { id: string; urls: string[]; saveDir?: string; mode: StartMode }) =>
    invoke<number>("confirm_batch", { decision }),
  cancelBatch: (id: string) => invoke<void>("cancel_batch", { id }),

  getVideo: (id: string) => invoke<VideoCapture | null>("get_video", { id }),
  openVideoUrl: (url: string) => invoke<VideoCapture>("open_video_url", { url }),
  probeVideo: (url: string) => invoke<VideoProbe>("probe_video", { url }),
  confirmVideo: (decision: VideoDecision) => invoke<number>("confirm_video", { decision }),
  cancelVideo: (id: string) => invoke<void>("cancel_video", { id }),
  addStream: (stream: SniffedStream, pageUrl?: string, title?: string) =>
    invoke<DownloadItem>("add_stream", { args: { stream, pageUrl, title } }),

  inspectTorrent: (source: TorrentSource) => invoke<TorrentMeta>("inspect_torrent", { source }),
  readTorrentFile: (path: string) => invoke<TorrentSource>("read_torrent_file", { path }),
  hfContext: () => invoke<HfContext>("hf_context"),
  /** Rejects with a code: `hf:notFound`, `hf:needsToken`, `hf:badToken`, `hf:gated`, `hf:rateLimited`, `hf:server`, `hf:network`. */
  hfRepo: (input: string) => invoke<HfRepo>("hf_repo", { input }),
  hfSearch: (query: string) => invoke<HfHit[]>("hf_search", { query }),
  hfPresent: (args: { kind: HfKind; id: string; sha: string; folder: string; files: HfFile[] }) => invoke<HfPresent>("hf_present", { args }),
  /** `mode` null: the "Hugging Face" queue, started. */
  hfAdd: (args: { kind: HfKind; id: string; sha: string; files: HfFile[]; folder: string; mode: StartMode | null }) => invoke<number>("hf_add", { args }),
  /** Tools: read files (ffmpeg is downloaded first if there's none; `toolsProgress` events). */
  mediaInspect: (paths: string[]) => invoke<MediaFile[]>("media_inspect", { paths }),
  /** Tools: the "Make smaller" estimate for each file (no work done). */
  mediaPlan: (files: MediaFile[], targetBytes: number) => invoke<ShrinkPlan[]>("media_plan", { files, targetBytes }),
  /** Tools: start jobs (one per file, or one ZIP); they report as `mediaJob` events. */
  mediaStart: (request: MediaRequest) => invoke<MediaJob[]>("media_start", { request }),
  mediaCancel: (id: string) => invoke<void>("media_cancel", { id }),
  mediaJobs: () => invoke<MediaJob[]>("media_jobs"),
  /** Removes finished jobs; returns the ones left. */
  mediaClear: () => invoke<MediaJob[]>("media_clear"),
  /** Opens (or brings forward) the Tools window for a tab, files and a size. */
  openTools: (request: ToolsRequest) => invoke<void>("open_tools", { request }),
  openFile: (path: string) => invoke<void>("open_file", { path }),
  revealPath: (path: string) => invoke<void>("reveal_path", { path }),
  addTorrent: (args: {
    source: TorrentSource;
    name: string;
    size?: number;
    onlyFiles?: number[] | null;
    saveDir?: string;
    category?: string;
    mode?: StartMode;
  }) => invoke<DownloadItem>("add_torrent", { args }),

  startItems: (ids: string[]) => invoke<void>("start_items", { ids }),
  pauseItems: (ids: string[]) => invoke<void>("pause_items", { ids }),
  pauseAll: () => invoke<void>("pause_all"),
  resumeAll: () => invoke<void>("resume_all"),
  deleteItems: (ids: string[], deleteFiles: boolean, permanent: boolean) =>
    invoke<void>("delete_items", { ids, deleteFiles, permanent }),
  restoreItems: (ids: string[]) => invoke<void>("restore_items", { ids }),
  emptyTrash: () => invoke<void>("empty_trash"),
  removeCompleted: () => invoke<void>("remove_completed"),
  redownload: (id: string) => invoke<void>("redownload", { id }),
  updateItem: (id: string, patch: ItemPatch) => invoke<DownloadItem>("update_item", { id, patch }),
  itemDetails: (id: string) => invoke<ItemDetails | null>("item_details", { id }),
  openItem: (id: string) => invoke<void>("open_item", { id }),
  revealItem: (id: string) => invoke<void>("reveal_item", { id }),
  /** Shows a file selected in its folder. */
  openPath: (path: string) => invoke<void>("open_path", { path }),
  /** A newer Prism release, or null when up to date. */
  checkUpdate: () => invoke<UpdateInfo | null>("check_update"),
  /** Downloads, installs and restarts into the update (progress: `update-progress` events). */
  installUpdate: () => invoke<void>("install_update"),
  /** The folder to pick in the browser's "Load unpacked" dialog. */
  extensionFolder: (firefox: boolean) => invoke<string | null>("extension_folder", { firefox }),
  /** Starts (or focuses) the browser. It can't be sent to `chrome://extensions` directly. */
  openBrowser: (browser: string) => invoke<void>("open_browser", { browser }),
  /** Prism's Chrome Web Store page in that browser (the default browser if it can't start). */
  openExtensionStore: (browser: string) => invoke<void>("open_extension_store", { browser }),
  /** Linux: a sound through the system's player (see NATIVE_SOUNDS). */
  playSound: (kind: string, completeSound?: string) => invoke<void>("play_sound", { kind, completeSound: completeSound ?? null }),

  assignToQueue: (ids: string[], queueId: string | null) => invoke<void>("assign_to_queue", { ids, queueId }),
  moveInQueue: (ids: string[], delta: number) => invoke<void>("move_in_queue", { ids, delta }),
  upsertQueue: (queue: Queue) => invoke<Queue[]>("upsert_queue", { queue }),
  deleteQueue: (id: string) => invoke<Queue[]>("delete_queue", { id }),
  startQueue: (id: string) => invoke<void>("start_queue", { id }),
  stopQueue: (id: string) => invoke<void>("stop_queue", { id }),
  nextScheduleRuns: () => invoke<[string, string | null][]>("next_schedule_runs"),
  cancelPostAction: () => invoke<void>("cancel_post_action"),
  upsertCategory: (category: Category) => invoke<Category[]>("upsert_category", { category }),
  deleteCategory: (id: string) => invoke<Category[]>("delete_category", { id }),

  grabberProjects: () => invoke<GrabberProject[]>("grabber_projects"),
  startGrabber: (projectId: string | null, name: string, config: GrabberConfig) =>
    invoke<GrabberProject>("start_grabber", { projectId, name, config }),
  stopGrabber: (projectId: string) => invoke<void>("stop_grabber", { projectId }),
  deleteGrabberProject: (projectId: string) => invoke<void>("delete_grabber_project", { projectId }),
  downloadGrabbed: (projectId: string, urls: string[], saveDir: string | null, keepStructure: boolean) =>
    invoke<number>("download_grabbed", { projectId, urls, saveDir, keepStructure }),

  exportList: (path: string) => invoke<void>("export_list", { path }),
  importList: (path: string) => invoke<number>("import_list", { path }),
  exportSettings: (path: string) => invoke<void>("export_settings", { path }),
  importSettings: (path: string) => invoke<Settings>("import_settings", { path }),
  exportDiagnostics: (path: string) => invoke<void>("export_diagnostics", { path }),

  toolStatus: () => invoke<ToolStatus>("tool_status"),
  ensureTools: () => invoke<ToolStatus>("ensure_tools"),
  updateYtdlp: () => invoke<unknown>("update_ytdlp"),
  integrationStatus: () => invoke<IntegrationStatus>("integration_status"),
  repairIntegration: () => invoke<IntegrationStatus>("repair_integration"),
  extensionDir: () => invoke<string | null>("extension_dir"),

  openDetails: (id: string) => invoke<void>("open_details", { id }),
  openWindow: (kind: string, id?: string) => invoke<void>("open_window", { kind, id }),
  toggleMini: () => invoke<boolean>("toggle_mini"),
  toggleDropzone: () => invoke<boolean>("toggle_dropzone"),
  showMain: () => invoke<void>("show_main"),
  mainWindowStartsVisible: () => invoke<boolean>("main_window_starts_visible"),
  appRunId: () => invoke<number>("app_run_id"),
  /** Whether the OS puts a material (Mica / vibrancy) behind the window. */
  nativeMaterial: () => invoke<boolean>("native_material"),
  setUiLanguage: (language: string) => invoke<void>("set_ui_language", { language }),
  removeDeletedFiles: () => invoke<number>("remove_deleted_files"),
  setAutostart: (enabled: boolean) => invoke<boolean>("set_autostart", { enabled }),
  quit: () => invoke<void>("quit_app"),
  freeSpace: (path: string) => invoke<number | null>("free_space", { path }),
};

export interface ToolInfo {
  tool?: string;
  path?: string | null;
  version?: string | null;
  source: "managed" | "system" | "custom" | "missing" | string;
  message?: string | null;
}

export interface ToolStatus {
  ytDlp?: ToolInfo;
  ffmpeg?: ToolInfo;
  ffprobe?: ToolInfo;
  deno?: ToolInfo;
}

export interface BrowserIntegration {
  browser: string;
  /** Display name ("Microsoft Edge"); `browser` is the id. */
  name?: string;
  supported?: boolean;
  note?: string | null;
  installed: boolean;
  registered: boolean;
  manifestPath?: string | null;
  pointsToThisHost: boolean;
}

export interface UpdateInfo {
  version: string;
  current: string;
  notes?: string | null;
  /** false for deb/rpm/portable installs: point to the website instead. */
  canInstall: boolean;
}

export interface IntegrationStatus {
  hostPath?: string | null;
  hostExists?: boolean;
  extensionId?: string;
  firefoxExtensionId?: string;
  browsers: BrowserIntegration[];
  extensionConnected?: boolean;
  lastSeen?: number | null;
  /** Version the extension reported; older than `appVersion` = the browser runs old files. */
  extensionVersion?: string | null;
  /** How the browser installed it (`management.getSelf().installType`; extensions before 1.0.7 don't say). */
  extensionInstallType?: string | null;
  extensionOutdated?: boolean;
  appVersion?: string;
}

/** One entry of the quality picker (`prism_video::Choice`, camelCase). Echo it back unchanged in confirmVideo. */
export interface VideoChoice {
  id: string;
  kind: "video" | "audio";
  /** e.g. "2160p60 · HDR · AV1" */
  label: string;
  /** e.g. "MP4 · ~743 MB" */
  subLabel: string;
  height?: number | null;
  width?: number | null;
  fps?: number | null;
  hdr: boolean;
  dynamicRange?: string | null;
  vcodec?: string | null;
  acodec?: string | null;
  estSize?: number | null;
  videoFormatId?: string | null;
  audioFormatId?: string | null;
  formatId?: string | null;
  container: "mp4" | "mkv" | "webm";
  audioFormat?: "mp3" | "m4a" | "opus" | "flac" | "wav" | null;
  audioBitrateKbps?: number | null;
  accelerable: boolean;
}

export interface SubtitleTrack {
  name?: string | null;
  auto: boolean;
  [k: string]: unknown;
}

export interface VideoInfo {
  kind: "video";
  id: string;
  title: string;
  uploader?: string | null;
  channel?: string | null;
  duration?: number | null;
  uploadDate?: string | null;
  description?: string | null;
  thumbnail?: string | null;
  webpageUrl?: string | null;
  extractor?: string | null;
  isLive: boolean;
  liveStatus?: string | null;
  chapters: VideoChapter[];
  subtitles: Record<string, SubtitleTrack | SubtitleTrack[] | unknown>;
  formats: unknown[];
  choices: VideoChoice[];
  hasDrm: boolean;
}

export interface PlaylistEntry {
  id: string;
  url: string;
  title?: string | null;
  duration?: number | null;
  thumbnail?: string | null;
  uploader?: string | null;
}

export interface PlaylistInfo {
  kind: "playlist";
  id?: string | null;
  title?: string | null;
  uploader?: string | null;
  webpageUrl?: string | null;
  extractor?: string | null;
  entries: PlaylistEntry[];
}

/** Raw `prism_video::Probe` (internally tagged by `kind`). */
export type VideoProbe = VideoInfo | PlaylistInfo;

/** Partial `prism_video::OutputOptions`; missing fields use the video settings. */
export interface VideoOutputOptions {
  container?: "mp4" | "mkv" | "webm";
  audioFormat?: "mp3" | "m4a" | "opus" | "flac" | "wav";
  embedSubtitles?: boolean;
  subLangs?: string[];
  autoSubtitles?: boolean;
  writeSubtitles?: boolean;
  embedThumbnail?: boolean;
  writeThumbnail?: boolean;
  embedMetadata?: boolean;
  embedChapters?: boolean;
  sponsorblock?: { mode: "off" | "mark" | "remove"; categories: string[] };
}
