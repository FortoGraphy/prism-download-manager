/**
 * Media sniffing: observes responses (non-blocking `webRequest`) and keeps a
 * per-tab list of media (HLS/DASH manifests, progressive video/audio) in
 * `storage.session` so it survives service-worker restarts. Cleared when the
 * tab navigates.
 */
import { api } from '../shared/api';
import { type MediaItem, addMedia, classifyMedia, isIgnoredMediaUrl, mediaKey, mediaName, qualityFromUrl, totalSize, visibleMedia } from '../shared/media';
import { setTabCount } from './badge';
import { requestById } from './requests';

const SNIFF_TYPES = ['media', 'xmlhttprequest', 'other', 'object', 'main_frame', 'sub_frame'] as chrome.webRequest.ResourceType[];

const tabs = new Map<number, MediaItem[]>();
const loaded = new Map<number, Promise<MediaItem[]>>();
const persistTimers = new Map<number, ReturnType<typeof setTimeout>>();
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

/**
 * A manifest URL player-hook.js read straight from an hls.js instance in the page — for the
 * cases a disguised or inline-only manifest never reaches our own network listener at all (see
 * player-hook.ts). No fetch of our own: by the time this arrives, hls.js already confirmed the
 * URL parses as a real manifest, so it's recorded exactly like any other sniffed stream.
 */
export function recordPlayerManifest(tabId: number, pageUrl: string, manifestUrl: string): void {
  if (isIgnoredMediaUrl(manifestUrl)) return;
  const item: MediaItem = {
    key: mediaKey(manifestUrl),
    url: manifestUrl,
    kind: 'hls',
    streamKind: 'hls',
    name: mediaName(manifestUrl, 'hls'),
    ts: Date.now(),
    frameUrl: pageUrl,
  };
  void record(tabId, item);
}

export function initSniffer(): void {
  api.webRequest.onHeadersReceived.addListener(
    (d) => {
      if (d.tabId < 0 || d.statusCode < 200 || d.statusCode >= 300) return;
      if (isIgnoredMediaUrl(d.url)) return;
      const contentType = header(d.responseHeaders, 'content-type');
      const cls = classifyMedia({ url: d.url, contentType, resourceType: d.type });
      if (!cls) return;
      // A top-level HTML-ish navigation is never media; a direct .mp4 navigation is.
      const size = totalSize(d.statusCode, header(d.responseHeaders, 'content-length'), header(d.responseHeaders, 'content-range'));
      const req = requestById(d.requestId);
      const headers: Record<string, string> = {};
      if (req?.referer) headers.Referer = req.referer;
      if (req?.origin) headers.Origin = req.origin;
      const item: MediaItem = {
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
      void record(d.tabId, item);
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
    session?.remove(keyFor(tabId)).catch(() => {});
  });
}
