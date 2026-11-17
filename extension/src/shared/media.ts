import type { StreamKind } from './protocol';
import { fileExtension, filenameFromUrl } from './rules';

export type MediaKind = 'hls' | 'dash' | 'video' | 'audio' | 'segment';

export interface MediaClassification {
  kind: MediaKind;
  streamKind: StreamKind;
}

const HLS_MIMES = ['application/vnd.apple.mpegurl', 'application/x-mpegurl', 'audio/mpegurl', 'audio/x-mpegurl'];
const DASH_MIMES = ['application/dash+xml'];
const SEGMENT_MIMES = ['video/mp2t', 'video/iso.segment', 'audio/iso.segment'];
const GENERIC_MIMES = ['', 'application/octet-stream', 'binary/octet-stream', 'text/plain', 'application/binary', 'application/x-www-form-urlencoded'];

const VIDEO_EXTS = ['MP4', 'M4V', 'WEBM', 'MKV', 'MOV', 'FLV', 'AVI', 'OGV', '3GP', 'WMV'];
const AUDIO_EXTS = ['MP3', 'M4A', 'AAC', 'OGG', 'OGA', 'OPUS', 'FLAC', 'WAV', 'WMA'];
const SEGMENT_EXTS = ['TS', 'M4S', 'M4F', 'CMFV', 'CMFA'];
const IMAGE_EXTS = ['PNG', 'JPG', 'JPEG', 'GIF', 'WEBP', 'BMP', 'SVG', 'ICO', 'AVIF'];

/** Media we don't list because the page is resolved by the app instead (YouTube's chunked streams). */
export function isIgnoredMediaUrl(url: string): boolean {
  try {
    const u = new URL(url);
    return u.hostname.endsWith('googlevideo.com') || u.protocol === 'blob:' || u.protocol === 'data:';
  } catch {
    return true;
  }
}

/** Classifies a response as media (or not) from its URL, Content-Type and resource type. */
export function classifyMedia(input: { url: string; contentType?: string; resourceType?: string }): MediaClassification | null {
  const mime = (input.contentType ?? '').split(';')[0]!.trim().toLowerCase();
  const ext = fileExtension(input.url);
  if (HLS_MIMES.includes(mime) || (ext === 'M3U8' && (GENERIC_MIMES.includes(mime) || mime.includes('mpegurl')))) {
    return { kind: 'hls', streamKind: 'hls' };
  }
  if (DASH_MIMES.includes(mime) || (ext === 'MPD' && (GENERIC_MIMES.includes(mime) || mime.endsWith('xml')))) {
    return { kind: 'dash', streamKind: 'dash' };
  }
  if (SEGMENT_MIMES.includes(mime) || SEGMENT_EXTS.includes(ext)) {
    if (mime.startsWith('text/html') || mime.startsWith('image/')) return null;
    return { kind: 'segment', streamKind: 'direct' };
  }
  if (mime.startsWith('video/')) return { kind: 'video', streamKind: 'direct' };
  if (mime.startsWith('audio/')) return { kind: 'audio', streamKind: 'direct' };
  if (GENERIC_MIMES.includes(mime) || (input.resourceType === 'media' && !mime.startsWith('text/'))) {
    if (VIDEO_EXTS.includes(ext)) return { kind: 'video', streamKind: 'direct' };
    if (AUDIO_EXTS.includes(ext)) return { kind: 'audio', streamKind: 'direct' };
  }
  return null;
}

/**
 * A response URL/MIME classification can't place, worth a click-time manifest check: some
 * streaming embeds serve their master playlist as a fake image (`.png` URL, `image/*`
 * Content-Type) specifically to dodge extension/MIME sniffers. Scoped to the one case real
 * sites use it for — a player's own XHR/media fetch, not a page's actual `<img>` or a frame.
 */
export function disguiseCandidate(input: { url: string; contentType?: string; resourceType?: string }): boolean {
  if (input.resourceType !== 'xmlhttprequest' && input.resourceType !== 'media') return false;
  const mime = (input.contentType ?? '').split(';')[0]!.trim().toLowerCase();
  return mime.startsWith('image/') || IMAGE_EXTS.includes(fileExtension(input.url));
}

/** What the start of a response body says a disguised manifest is: HLS playlists start with
 *  the literal `#EXTM3U`; DASH is XML, so `<MPD` follows a possible prolog. */
export function manifestKind(prefix: string): 'hls' | 'dash' | null {
  if (/^\s*#EXTM3U/.test(prefix)) return 'hls';
  if (/<MPD[\s>]/i.test(prefix)) return 'dash';
  return null;
}

/** Identity of a media resource: URL without fragment and byte-range style query params. */
export function mediaKey(url: string): string {
  try {
    const u = new URL(url);
    u.hash = '';
    for (const p of ['range', 'bytes', 'rn', 'rbuf', 'bytestart', 'byteend', '_']) u.searchParams.delete(p);
    return u.toString();
  } catch {
    return url;
  }
}

/** Total resource size from a response: Content-Range total for 206, else Content-Length. */
export function totalSize(status: number, contentLength?: string, contentRange?: string): number | undefined {
  if (status === 206 && contentRange) {
    const m = /\/(\d+)\s*$/.exec(contentRange);
    if (m) return Number(m[1]);
  }
  if (contentLength && /^\d+$/.test(contentLength.trim())) {
    const n = Number(contentLength);
    return n > 0 ? n : undefined;
  }
  return undefined;
}

/** Quality hint from a URL (`1080p`, `720`, `4k`; any height written as `640p`, e.g. LinkedIn's `mp4-640p-30fp`). */
export function qualityFromUrl(url: string): string | undefined {
  const s = decodeSafe(url);
  const p = /(?:^|[^0-9])(2160|1440|1080|720|540|480|360|240|144)p?(?:[^0-9]|$)/i.exec(s) ?? /(?:^|[^0-9a-z])([1-9][0-9]{2,3})p(?:[^0-9a-z]|$)/i.exec(s);
  if (p) return `${p[1]}p`;
  if (/(?:^|[^a-z])(4k|uhd)(?:[^a-z]|$)/i.test(s)) return '2160p';
  return undefined;
}

/**
 * Paths of a single post / video on the video sites (`m[0]` is the canonical part). On a feed,
 * listing or profile the page URL names no single video, so yt-dlp needs the hovered post's.
 */
const POST_PATHS: RegExp[] = [
  /^\/(?:[^/]+|i(?:\/web)?)\/status(?:es)?\/\d+/, // x.com, twitter.com
  /^\/(?:r|u|user)\/[^/]+\/comments\/[a-z0-9]+/i, // reddit.com
  /^\/(?:[\w.]+\/)?(?:p|reels?|tv)\/[\w-]+/, // instagram.com
  /^\/@[^/]+\/video\/\d+/, // tiktok.com
  /^\/(?:[^/]+\/)?(?:videos|reel|posts)\/[\w.:-]+/, // facebook.com, twitch.tv/videos/<id>
  /^\/share\/[rv]\/[\w-]+/, // facebook.com share links
  /^\/shorts\/[\w-]+/, // youtube.com
  /^\/video\/[\w-]+/, // dailymotion.com, bilibili.com, ok.ru
  /^\/[^/]+\/clip\/[\w-]+/, // twitch.tv clips
  /^\/\d{5,}(?:\/[0-9a-f]{6,})?\/?$/, // vimeo.com
  /^\/watch\/[a-z]{2}\d+/, // nicovideo.jp
  /^\/v[a-z0-9]+-[\w-]+\.html/, // rumble.com
  /^\/(?:video|clip)-?\d+_\d+/, // vk.com
  /^\/talks\/[\w-]+/, // ted.com
  /^\/@[^/]+\/[^/]+:[0-9a-z]+/i, // odysee.com
  /^\/track\/[\w-]+/, // bandcamp.com
  /^\/feed\/update\/urn:li:(?:activity|ugcPost|share):\d+/, // linkedin.com (its /posts/<slug> match the pattern above)
];

const siteKey = (host: string): string => {
  const h = host.toLowerCase().replace(/^(?:www|m|mobile|old|new|web)\./, '');
  return ({ 'twitter.com': 'x.com', 'fb.watch': 'facebook.com', 'youtu.be': 'youtube.com' } as Record<string, string>)[h] ?? h;
};

/**
 * `href` (absolute or relative to `pageUrl`) as the canonical URL of a single post / video on
 * the same site as `pageUrl` (`https://x.com/nasa/status/123`), or undefined when it is
 * something else (profile, hashtag, another site, …).
 */
export function postUrl(href: string | null | undefined, pageUrl: string): string | undefined {
  if (!href) return undefined;
  let u: URL;
  let page: URL;
  try {
    page = new URL(pageUrl);
    u = new URL(href, page);
  } catch {
    return undefined;
  }
  if (!/^https?:$/.test(u.protocol) || siteKey(u.hostname) !== siteKey(page.hostname)) return undefined;
  if (/^\/watch\/?$/.test(u.pathname) && u.searchParams.get('v')) return `${u.origin}${u.pathname}?v=${encodeURIComponent(u.searchParams.get('v')!)}`;
  for (const re of POST_PATHS) {
    const m = re.exec(u.pathname);
    if (m) return `${u.origin}${m[0].replace(/\/$/, '')}`;
  }
  return undefined;
}

/**
 * LinkedIn post of a video from the attribute values of its containers, nearest first: the
 * post's `urn:li:activity:<id>` or `urn:li:ugcPost:<id>`, also inside composite ids such as
 * `urn:li:fsd_update:(urn:li:activity:<id>,MAIN_FEED,...)`. Returns `/feed/update/<urn>/`,
 * which LinkedIn serves as the public post page yt-dlp reads.
 */
export function linkedinPostPath(values: Array<string | null | undefined>): string | undefined {
  for (const v of values) {
    const m = v ? /urn:li:(activity|ugcPost):(\d{6,})/.exec(v) : null;
    if (m) return `/feed/update/urn:li:${m[1]}:${m[2]}/`;
  }
  return undefined;
}

/**
 * LinkedIn's newer feed / search / company pages: the post id sits only in the `componentkey`
 * of the post's text block, e.g. `translatable-commentary-FeTranslationUrn(contentUrnCommentUrn=null,
 * ..., contentUrnUgcPostUrn=ContentUrnUgcPostUrn(userGeneratedContentPostUrn=...(userGeneratedContentId=751...)), ...)`.
 * The variant that isn't `null` says the kind: ugcPost or share (others are skipped).
 */
export function linkedinCommentaryPath(key: string | null | undefined): string | undefined {
  if (!key) return undefined;
  const kinds = new Map([['UgcPost', 'ugcPost'], ['Share', 'share']]);
  for (const part of key.split(/(?=contentUrn\w+?Urn=)/)) {
    const m = /^contentUrn(\w+?)Urn=(?!null)([\s\S]*)$/.exec(part);
    const kind = m?.[1] ? kinds.get(m[1]) : undefined;
    const id = kind ? /\d{15,20}/.exec(m?.[2] ?? '')?.[0] : undefined;
    if (kind && id) return `/feed/update/urn:li:${kind}:${id}/`;
  }
  return undefined;
}

/** Height of a quality label (`720p` → 720), 0 when unknown. */
export function qualityHeight(q: string | undefined): number {
  return q ? Number(/^(\d+)p$/.exec(q)?.[1] ?? 0) : 0;
}

/** A URL a media element can play, with the MIME type the page declared for it. */
export interface ElementSource {
  url: string;
  type?: string;
}

/**
 * What a `<video>` / `<audio>` element itself plays: its current source, `src`, `<source>`
 * children and a `data-sources` JSON list (`[{"src": …, "type": …}]`, LinkedIn's video.js
 * players list every quality there). Absolute http(s) URLs only (no `blob:` MSE players), in
 * that order, without duplicates. Lets the in-page button offer the hovered video instead of
 * whatever else the tab loaded (ads, other posts).
 */
export function elementSources(
  el: { currentSrc?: string; src?: string; sources?: ElementSource[]; dataSources?: string | null },
  baseUrl: string,
): ElementSource[] {
  const out: ElementSource[] = [];
  const add = (raw: unknown, type?: unknown) => {
    if (typeof raw !== 'string' || !raw.trim()) return;
    let url: string;
    try {
      url = new URL(raw.trim(), baseUrl).toString();
    } catch {
      return;
    }
    if (!/^https?:$/.test(new URL(url).protocol)) return;
    const prev = out.find((s) => s.url === url);
    if (prev) {
      if (!prev.type && typeof type === 'string' && type) prev.type = type;
      return;
    }
    out.push(typeof type === 'string' && type ? { url, type } : { url });
  };
  add(el.currentSrc);
  add(el.src);
  for (const s of el.sources ?? []) add(s.url, s.type);
  if (el.dataSources) {
    try {
      const list: unknown = JSON.parse(el.dataSources);
      if (Array.isArray(list)) for (const s of list) if (s && typeof s === 'object') add((s as { src?: unknown }).src, (s as { type?: unknown }).type);
    } catch {
      /* not a JSON source list */
    }
  }
  return out;
}

function decodeSafe(s: string): string {
  try {
    return decodeURIComponent(s);
  } catch {
    return s;
  }
}

export function humanSize(n: number | undefined): string {
  if (!n || n <= 0) return '';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 100 || i === 0 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
}

/** Display name for a media URL. */
export function mediaName(url: string, kind: MediaKind): string {
  const name = filenameFromUrl(url);
  if (name) return name.length > 80 ? `${name.slice(0, 77)}…` : name;
  return kind === 'hls' ? 'HLS stream' : kind === 'dash' ? 'DASH stream' : 'media';
}

const MIME_CONTAINERS: Record<string, string> = {
  'video/mp4': 'MP4',
  'video/webm': 'WEBM',
  'video/ogg': 'OGV',
  'video/quicktime': 'MOV',
  'video/x-matroska': 'MKV',
  'audio/mp4': 'M4A',
  'audio/mpeg': 'MP3',
  'audio/webm': 'WEBA',
  'audio/ogg': 'OGG',
  'audio/aac': 'AAC',
  'audio/flac': 'FLAC',
};

/** Second menu line: the file name when the URL has a real one, else the server (CDN URLs end in IDs). */
export function mediaDetail(m: MediaItem): string {
  if (fileExtension(m.url)) return m.name;
  try {
    return new URL(m.url).hostname;
  } catch {
    return m.name;
  }
}

/** Menu label: `720p · MP4 · 5.1 MB`. */
export function mediaLabel(m: MediaItem): string {
  // Container from the extension, else from the MIME type (CDN URLs often have no extension).
  const container = fileExtension(m.url) || MIME_CONTAINERS[m.mime?.toLowerCase() ?? ''];
  const kind = m.kind === 'hls' ? 'HLS' : m.kind === 'dash' ? 'DASH' : container || (m.kind === 'audio' ? 'Audio' : 'Video');
  const parts = [m.quality ?? (m.kind === 'audio' ? 'Audio' : m.kind === 'hls' || m.kind === 'dash' ? 'Stream' : 'Video'), kind];
  const size = humanSize(m.size);
  if (size) parts.push(size);
  return parts.join(' · ');
}

/**
 * The hovered element's own sources as media items, best quality first. A source the tab has
 * already loaded keeps what the sniffer saw (size, Referer/Origin actually sent); the others
 * (e.g. qualities the player didn't pick) borrow those headers, or get the page as Referer.
 */
export function elementMedia(sources: ElementSource[], sniffed: MediaItem[], pageUrl: string): MediaItem[] {
  const byKey = new Map(sniffed.map((m) => [m.key, m]));
  const seen = sources.map((s) => byKey.get(mediaKey(s.url)));
  const headers = seen.find((m) => m?.headers)?.headers ?? (pageUrl ? { Referer: pageUrl } : undefined);
  const items = sources.map((s, i): MediaItem => {
    const hit = seen[i];
    if (hit) return hit;
    const cls = classifyMedia({ url: s.url, contentType: s.type, resourceType: 'media' }) ?? { kind: 'video', streamKind: 'direct' };
    const kind = cls.kind === 'segment' ? 'video' : cls.kind;
    const quality = qualityFromUrl(s.url);
    return {
      key: mediaKey(s.url),
      url: s.url,
      kind,
      streamKind: cls.streamKind,
      name: mediaName(s.url, kind),
      ts: Date.now(),
      ...(s.type ? { mime: s.type.split(';')[0]!.trim() } : {}),
      ...(quality ? { quality } : {}),
      ...(headers ? { headers } : {}),
    };
  });
  const unique = items.filter((m, i) => items.findIndex((x) => x.key === m.key) === i);
  // Stable: equal / unknown qualities keep the element's order (what it plays comes first).
  return unique.map((m, i) => ({ m, i })).sort((a, b) => qualityHeight(b.m.quality) - qualityHeight(a.m.quality) || a.i - b.i).map((x) => x.m);
}

/** Minimum size for progressive media to be listed (filters tracking pixels, tiny previews). */
export const MIN_DIRECT_MEDIA_BYTES = 64 * 1024;

export interface MediaItem {
  key: string;
  url: string;
  kind: MediaKind;
  streamKind: StreamKind;
  mime?: string;
  size?: number;
  name: string;
  quality?: string;
  /** Referer / Origin used by the player. */
  headers?: Record<string, string>;
  frameUrl?: string;
  ts: number;
}

/**
 * Adds a media item to a tab's list with de-duplication and segment pruning:
 * segments are dropped once a manifest is known (and not added afterwards).
 * Returns the new list and whether it changed.
 */
export function addMedia(list: MediaItem[], item: MediaItem, maxItems = 100): { list: MediaItem[]; changed: boolean } {
  const hasManifest = list.some((m) => m.kind === 'hls' || m.kind === 'dash');
  if (item.kind === 'segment' && hasManifest) return { list, changed: false };
  if (item.kind !== 'hls' && item.kind !== 'dash' && item.size !== undefined && item.size < MIN_DIRECT_MEDIA_BYTES) {
    return { list, changed: false };
  }
  const idx = list.findIndex((m) => m.key === item.key);
  if (idx >= 0) {
    const prev = list[idx]!;
    if ((item.size ?? 0) > (prev.size ?? 0)) {
      const next = list.slice();
      next[idx] = { ...prev, size: item.size };
      return { list: next, changed: true };
    }
    return { list, changed: false };
  }
  let next = list.concat(item);
  if (item.kind === 'hls' || item.kind === 'dash') next = next.filter((m) => m.kind !== 'segment');
  if (next.length > maxItems) next = next.slice(next.length - maxItems);
  return { list: next, changed: true };
}

/** Number of user-meaningful items (segments excluded). */
export function visibleMedia(list: MediaItem[]): MediaItem[] {
  return list.filter((m) => m.kind !== 'segment');
}
