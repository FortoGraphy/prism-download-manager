/**
 * Media sniffing: observes responses (non-blocking `webRequest`) and keeps a
 * per-tab list of media (HLS/DASH manifests, progressive video/audio) in
 * `storage.session` so it survives service-worker restarts. Cleared when the
 * tab navigates.
 */
import { api } from '../shared/api';
import { GENERIC_MIMES, type MediaClassification, type MediaItem, addMedia, classifyMedia, isIgnoredMediaUrl, manifestKind, mediaKey, mediaName, qualityFromUrl, totalSize, visibleMedia } from '../shared/media';
import { setTabCount } from './badge';
import { requestById, requestByUrl } from './requests';

const SNIFF_TYPES = ['media', 'xmlhttprequest', 'other', 'object', 'main_frame', 'sub_frame'] as chrome.webRequest.ResourceType[];
/** Responses worth a body probe when URL/MIME classification says nothing (the disguise
 *  signature: a player fetching bytes via JS, never the browser's own image loader or a page). */
const PROBE_TYPES: chrome.webRequest.OnHeadersReceivedDetails['type'][] = ['media', 'xmlhttprequest', 'other'];
/** Playlists are tiny text files; a bigger Content-Length is never worth probing. */
const MAX_PROBE_BYTES = 2 * 1024 * 1024;

const tabs = new Map<number, MediaItem[]>();
const loaded = new Map<number, Promise<MediaItem[]>>();
const persistTimers = new Map<number, ReturnType<typeof setTimeout>>();
/** URLs already body-probed for a tab: one probe per URL per page, also breaking any loop. */
const probed = new Map<number, Set<string>>();
const session = (api.storage as typeof api.storage & { session?: chrome.storage.StorageArea }).session;

const keyFor = (tabId: number) => `media:${tabId}`;

/** The tab's media list (loads from session storage after a worker restart). */
export function getTabMedia(tabId: number): Promise<MediaItem[]> {
  const mem = tabs.get(tabId);
  if (mem) return Promise.resolve(mem);
  let p = loaded.get(tabId);
  if (!p) {
    p = (session ? session.get(keyFor(tabId)) : Promise.resolve({} as Record<string, unknown>))
      .then((r) => {
        const list = (r[keyFor(tabId)] as MediaItem[] | undefined) ?? [];
        // Items sniffed while loading take precedence.
        const cur = tabs.get(tabId);
        const merged = cur ? cur.reduce((acc, m) => addMedia(acc, m).list, list) : list;
        tabs.set(tabId, merged);
        return merged;
      })
      .catch(() => tabs.get(tabId) ?? [])
      .finally(() => loaded.delete(tabId));
    loaded.set(tabId, p);
  }
  return p;
}

function persist(tabId: number): void {
  clearTimeout(persistTimers.get(tabId));
  persistTimers.set(
    tabId,
    setTimeout(() => {
      persistTimers.delete(tabId);
      const list = tabs.get(tabId) ?? [];
      if (!session) return;
      (list.length ? session.set({ [keyFor(tabId)]: list }) : session.remove(keyFor(tabId))).catch(() => {});
    }, 250),
  );
}

export function clearTab(tabId: number, keepUrl?: string): void {
  const cur = tabs.get(tabId) ?? [];
  const kept = keepUrl ? cur.filter((m) => m.url === keepUrl) : [];
  tabs.set(tabId, kept);
  probed.set(tabId, new Set());
  persist(tabId);
  setTabCount(tabId, visibleMedia(kept).length);
}

async function record(tabId: number, item: MediaItem): Promise<void> {
  const cur = await getTabMedia(tabId);
  const { list, changed } = addMedia(cur, item);
  if (!changed) return;
  tabs.set(tabId, list);
  persist(tabId);
  setTabCount(tabId, visibleMedia(list).length);
}

function header(headers: chrome.webRequest.HttpHeader[] | undefined, name: string): string | undefined {
  return headers?.find((h) => h.name.toLowerCase() === name)?.value;
}

/** The media item a classified response becomes (shared by the confirmed and body-probed paths). */
function buildItem(
  d: chrome.webRequest.OnHeadersReceivedDetails,
  cls: MediaClassification,
  contentType: string | undefined,
  size: number | undefined,
): MediaItem {
  const req = requestById(d.requestId);
  const headers: Record<string, string> = {};
  if (req?.referer) headers.Referer = req.referer;
  if (req?.origin) headers.Origin = req.origin;
  return {
    key: mediaKey(d.url),
    url: d.url,
    kind: cls.kind,
    streamKind: cls.streamKind,
    name: mediaName(d.url, cls.kind),
    ts: Date.now(),
    ...(contentType ? { mime: contentType.split(';')[0]!.trim() } : {}),
    ...(size !== undefined && cls.kind !== 'hls' && cls.kind !== 'dash' ? { size } : {}),
    ...(qualityFromUrl(d.url) ? { quality: qualityFromUrl(d.url)! } : {}),
    ...(Object.keys(headers).length ? { headers } : {}),
    ...((d as { documentUrl?: string }).documentUrl || (d as { initiator?: string }).initiator
      ? { frameUrl: (d as { documentUrl?: string }).documentUrl ?? (d as { initiator?: string }).initiator! }
      : {}),
  };
}

/**
 * Fallback for responses URL/MIME classification can't place: some streaming embeds
 * disguise their playlist as a non-media file (`.png` URL, `image/*` MIME), which real
 * players parse as text regardless. Probes a small body prefix for the HLS/DASH magic
 * strings and records a hit through the same path as confirmed media. Fire-and-forget,
 * so `onHeadersReceived` never waits on it.
 */
function maybeSniffBody(d: chrome.webRequest.OnHeadersReceivedDetails, contentType: string | undefined): void {
  if (!PROBE_TYPES.includes(d.type)) return;
  const mime = (contentType ?? '').split(';')[0]!.trim().toLowerCase();
  if (!GENERIC_MIMES.includes(mime) && !mime.startsWith('image/')) return;
  const len = header(d.responseHeaders, 'content-length');
  if (len && /^\d+$/.test(len.trim()) && Number(len) > MAX_PROBE_BYTES) return;
  let seen = probed.get(d.tabId);
  if (!seen) probed.set(d.tabId, (seen = new Set()));
  if (seen.has(d.url)) return;
  seen.add(d.url);
  void sniffBody(d.url).then((kind) => {
    if (!kind) return;
    void record(d.tabId, buildItem(d, { kind, streamKind: kind }, contentType, undefined));
  });
}

/** Reads a small prefix of `url` (with the request's Referer/Origin) and reports which
 *  manifest magic it starts with. Any failure is a plain "not a manifest". */
async function sniffBody(url: string): Promise<'hls' | 'dash' | null> {
  try {
    const info = requestByUrl(url);
    const headers: Record<string, string> = {};
    if (info?.referer) headers.Referer = info.referer;
    if (info?.origin) headers.Origin = info.origin;
    const res = await fetch(url, { headers, credentials: 'include' });
    if (!res.ok || !res.body) return null;
    const reader = res.body.getReader();
    let buf = new Uint8Array(0);
    while (buf.length < 512) {
      const { done, value } = await reader.read();
      if (done || !value) break;
      const next = new Uint8Array(buf.length + value.length);
      next.set(buf);
      next.set(value, buf.length);
      buf = next;
    }
    reader.cancel().catch(() => {});
    return manifestKind(new TextDecoder().decode(buf));
  } catch {
    return null; // network error, rejected, aborted — skip, never throw into the listener
  }
}

export function initSniffer(): void {
  api.webRequest.onHeadersReceived.addListener(
    (d) => {
      if (d.tabId < 0 || d.statusCode < 200 || d.statusCode >= 300) return;
      if (isIgnoredMediaUrl(d.url)) return;
      const contentType = header(d.responseHeaders, 'content-type');
      const cls = classifyMedia({ url: d.url, contentType, resourceType: d.type });
      if (!cls) {
        maybeSniffBody(d, contentType);
        return;
      }
      // A top-level HTML-ish navigation is never media; a direct .mp4 navigation is.
      const size = totalSize(d.statusCode, header(d.responseHeaders, 'content-length'), header(d.responseHeaders, 'content-range'));
      void record(d.tabId, buildItem(d, cls, contentType, size));
    },
    { urls: ['<all_urls>'], types: SNIFF_TYPES },
    ['responseHeaders'],
  );

  // New document in the tab → forget the previous page's media.
  api.webNavigation?.onCommitted.addListener((d) => {
    if (d.frameId === 0) clearTab(d.tabId, d.url);
  });
  api.tabs.onRemoved.addListener((tabId) => {
    tabs.delete(tabId);
    probed.delete(tabId);
    session?.remove(keyFor(tabId)).catch(() => {});
  });
}
