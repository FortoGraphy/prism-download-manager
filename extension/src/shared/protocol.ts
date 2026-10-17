/**
 * Extension ⇄ native host ⇄ app protocol (version 1).
 *
 * Every message the extension sends is `{ id, type, noLaunch?, ...payload }`.
 * The host forwards it verbatim as the JSON body of `POST /v1/message` and
 * returns the app's JSON reply with the same `id`.
 *
 * App replies: `{ ok: true, ... }` or `{ ok: false, error: { code, message } }`.
 * See extension/README.md for the authoritative description.
 */

export const PROTOCOL_VERSION = 1;
export const NATIVE_HOST_NAME = 'dev.prism.host';

export type BrowserName = 'chrome' | 'edge' | 'brave' | 'opera' | 'vivaldi' | 'chromium' | 'firefox';
export type BypassKey = 'alt' | 'shift' | 'ctrl' | 'meta' | 'none';

/** Capture settings: sent by the app in the `hello` reply, cached and editable in the extension. */
export interface CaptureSettings {
  /** Master switch for automatic download capture. */
  captureEnabled: boolean;
  /** File extensions without the dot, upper-case; `*` and `?` wildcards allowed (e.g. `R0*`). */
  extensions: string[];
  /** MIME types; entries ending with `/`, `-` or `*` are prefixes (e.g. `video/`, `application/x-`). */
  mimeTypes: string[];
  /** Minimum size in bytes; downloads with a known smaller size stay in the browser. 0 = no limit. */
  minSize: number;
  /** Sites never captured: `example.com` (incl. subdomains), `*.example.com`, or a URL glob `https://x.com/dl/*`. */
  excludedSites: string[];
  /** Holding this key while clicking a link lets the browser handle the download. */
  bypassKey: BypassKey;
  /** Show the floating "Download this video" button. */
  showVideoButton: boolean;
  /** Hosts whose pages are resolved by the app (`probeVideo`, yt-dlp) instead of sniffed streams. */
  videoSites: string[];
}

export interface ProtocolError {
  code: string;
  message: string;
}

export interface OkReply {
  ok: true;
}

export interface ErrorReply {
  ok: false;
  error: ProtocolError;
}

// ---- hello ---------------------------------------------------------------

export interface HelloRequest {
  type: 'hello';
  protocol: number;
  /** Extension version. */
  version: string;
  browser: BrowserName;
  userAgent: string;
  /**
   * How the browser installed the extension (`management.getSelf().installType`): "development"
   * = loaded unpacked (the browser keeps old files after an update until it is reloaded),
   * "normal" = from a store, "admin" = by policy (both updated by the browser).
   */
  installType?: string;
}

export interface AppInfo {
  name: string;
  version: string;
}

export interface HelloReply extends OkReply {
  app: AppInfo;
  protocol?: number;
  /** Partial settings are merged over the defaults. */
  settings?: Partial<CaptureSettings>;
}

// ---- download ------------------------------------------------------------

export type DownloadSource = 'capture' | 'contextMenu' | 'popup' | 'videoButton';

export interface DownloadPayload {
  url: string;
  /** URL after redirects, when known. */
  finalUrl?: string;
  /** Referer the browser used (usually the page URL). */
  referrer?: string;
  /** Top-level page URL the download was started from. */
  pageUrl?: string;
  /** Suggested file name (no directories). */
  filename?: string;
  /** Size in bytes when known. */
  fileSize?: number;
  mime?: string;
  /** Always GET for handed-off downloads (POST downloads stay in the browser). */
  method?: 'GET';
  /** Extra request headers to replay (e.g. Origin). Cookie/Referer/User-Agent have their own fields. */
  headers?: Record<string, string>;
  /** `Cookie` header value for `url` (`a=1; b=2`), may be empty. */
  cookies?: string;
  /** `Cookie` header for `finalUrl`; only present when `finalUrl` is on another origin than `url`. */
  finalUrlCookies?: string;
}

export interface DownloadRequest extends DownloadPayload {
  type: 'download';
  userAgent: string;
  incognito: boolean;
  source: DownloadSource;
}

export interface DownloadReply extends OkReply {
  /** false → the app declined; the extension lets the browser download it. */
  accepted: boolean;
  downloadId?: string;
}

// ---- batch ---------------------------------------------------------------

export type BatchKind = 'links' | 'images' | 'videos' | 'selection' | 'media';

export interface BatchItem {
  url: string;
  referrer?: string;
  filename?: string;
  mime?: string;
  /** Link text / alt text. */
  text?: string;
  kind: 'link' | 'image' | 'video' | 'audio';
  cookies?: string;
}

export interface BatchRequest {
  type: 'batch';
  pageUrl: string;
  title?: string;
  kind: BatchKind;
  items: BatchItem[];
  userAgent: string;
  incognito: boolean;
}

export interface BatchReply extends OkReply {
  accepted: boolean;
  count?: number;
}

// ---- video ---------------------------------------------------------------

export type StreamKind = 'hls' | 'dash' | 'direct';

export interface VideoStream {
  url: string;
  kind: StreamKind;
  mime?: string;
  size?: number;
  quality?: string;
  /** Request headers the player used (Referer, Origin). */
  headers?: Record<string, string>;
  /** One `probeStream` variant picked from the quality list, instead of the engine default. */
  variantIndex?: number;
}

export interface VideoRequest {
  type: 'video';
  pageUrl: string;
  title?: string;
  /** Set when the user picked a format from a `probeVideo` reply. */
  formatId?: string;
  formatLabel?: string;
  /** Sniffed streams (empty when `formatId` is set). */
  streams: VideoStream[];
  /** Cookie header for the stream URL (or the page URL when `formatId` is set). */
  cookies?: string;
  userAgent: string;
  incognito: boolean;
}

export interface VideoReply extends OkReply {
  accepted: boolean;
}

export interface ProbeVideoRequest {
  type: 'probeVideo';
  pageUrl: string;
  cookies?: string;
  userAgent: string;
}

export interface VideoFormat {
  /** yt-dlp format selector the app understands when sent back in `video.formatId`. */
  id: string;
  /** Human label, e.g. "1080p60 · MP4 · 145 MB" or "Audio · M4A · 4 MB". */
  label: string;
  kind: 'video' | 'audio' | 'muxed';
  ext?: string;
  height?: number;
  fps?: number;
  size?: number;
  note?: string;
}

export interface ProbeVideoReply extends OkReply {
  title?: string;
  thumbnail?: string;
  duration?: number;
  formats: VideoFormat[];
}

/** A sniffed HLS/DASH manifest: list its declared quality variants (no download). */
export interface ProbeStreamRequest {
  type: 'probeStream';
  url: string;
  kind: 'hls' | 'dash';
  headers?: Record<string, string>;
  cookies?: string;
  pageUrl?: string;
  userAgent: string;
}

/** One manifest-declared variant. `index` is what `VideoStream.variantIndex` must carry
 *  back to `video` for the app to download this exact one. */
export interface StreamVariant {
  index: number;
  url: string;
  bandwidth?: number;
  width?: number;
  height?: number;
}

export interface ProbeStreamReply extends OkReply {
  variants: StreamVariant[];
}

// ---- status / open -------------------------------------------------------

export interface StatusRequest {
  type: 'status';
}

export interface StatusReply extends OkReply {
  app: AppInfo;
  captureEnabled?: boolean;
}

export interface OpenRequest {
  type: 'open';
  page?: 'main' | 'browserIntegration' | 'settings';
}

export interface HostPingReply extends OkReply {
  host: { version: string; dataDir?: string };
}

export type AppRequest =
  | HelloRequest
  | DownloadRequest
  | BatchRequest
  | VideoRequest
  | ProbeVideoRequest
  | ProbeStreamRequest
  | StatusRequest
  | OpenRequest
  | { type: 'hostPing' };

export interface ReplyMap {
  hello: HelloReply;
  download: DownloadReply;
  batch: BatchReply;
  video: VideoReply;
  probeVideo: ProbeVideoReply;
  probeStream: ProbeStreamReply;
  status: StatusReply;
  open: OkReply;
  hostPing: HostPingReply;
}
