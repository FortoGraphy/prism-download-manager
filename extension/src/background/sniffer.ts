/**
 * Media sniffing: observes responses (non-blocking `webRequest`) and keeps a
 * per-tab list of media (HLS/DASH manifests, progressive video/audio) in
 * `storage.session` so it survives service-worker restarts. Cleared when the
 * tab navigates.
 */
import { api } from '../shared/api';
import { type MediaItem, addMedia, classifyMedia, disguiseCandidate, isIgnoredMediaUrl, manifestKind, mediaKey, mediaName, qualityFromUrl, totalSize, visibleMedia } from '../shared/media';
import { setTabCount } from './badge';
import { requestById } from './requests';

const SNIFF_TYPES = ['media', 'xmlhttprequest', 'other', 'object', 'main_frame', 'sub_frame'] as chrome.webRequest.ResourceType[];
/** A real playlist is a tiny text file; a bigger disguised candidate is never worth a check. */
const MAX_CANDIDATE_BYTES = 2 * 1024 * 1024;
/** How long the click-time manifest check waits before giving up on a candidate. */
const CONFIRM_TIMEOUT_MS = 3000;
/** Candidates kept per tab: a handful of distinct disguised-looking URLs is plenty. */
const MAX_CANDIDATES = 20;

export interface Candidate {
  url: string;
  headers?: Record<string, string>;
  frameUrl?: string;
}

const tabs = new Map<number, MediaItem[]>();
const loaded = new Map<number, Promise<MediaItem[]>>();
const persistTimers = new Map<number, ReturnType<typeof setTimeout>>();
/** URLs that looked disguised but were never confirmed: checked only when the user opens the
 *  video menu (confirmCandidate below) — never fetched automatically. */
const candidates = new Map<number, Map<string, Candidate>>();
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
  candidates.delete(tabId);
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

function refererOrigin(d: chrome.webRequest.OnHeadersReceivedDetails): Record<string, string> {
  const req = requestById(d.requestId);
  const headers: Record<string, string> = {};
  if (req?.referer) headers.Referer = req.referer;
  if (req?.origin) headers.Origin = req.origin;
  return headers;
}

function frameUrlOf(d: chrome.webRequest.OnHeadersReceivedDetails): string | undefined {
  return (d as { documentUrl?: string }).documentUrl ?? (d as { initiator?: string }).initiator;
}

/** Candidate URLs for `tabId` not yet confirmed — what the menu checks when it opens. */
export function getTabCandidates(tabId: number): Candidate[] {
  return [...(candidates.get(tabId)?.values() ?? [])];
}

/**
 * Confirms one candidate is really a disguised HLS/DASH manifest: a small, unauthenticated
 * (`credentials: 'omit'`) GET for its body prefix, checked for the HLS/DASH magic bytes. Only
 * ever called from the user opening the video menu (video.ts) — never automatically, and never
 * with the user's cookies, so it can't be used to replay a credentialed request to another site.
 */
export async function confirmCandidate(tabId: number, url: string): Promise<MediaItem | null> {
  const c = candidates.get(tabId)?.get(url);
  if (!c) return null;
  candidates.get(tabId)?.delete(url);
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), CONFIRM_TIMEOUT_MS);
  try {
    const res = await fetch(url, { headers: c.headers, credentials: 'omit', signal: ctrl.signal });
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
    const kind = manifestKind(new TextDecoder().decode(buf));
    if (!kind) return null;
    const item: MediaItem = {
      key: mediaKey(url),
      url,
      kind,
      streamKind: kind,
      name: mediaName(url, kind),
      ts: Date.now(),
      ...(c.headers && Object.keys(c.headers).length ? { headers: c.headers } : {}),
      ...(c.frameUrl ? { frameUrl: c.frameUrl } : {}),
    };
    void record(tabId, item);
    return item;
  } catch {
    return null; // network error, aborted, rejected — the menu just keeps the single entry
  } finally {
    clearTimeout(timer);
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
        if (!disguiseCandidate({ url: d.url, contentType, resourceType: d.type })) return;
        const len = header(d.responseHeaders, 'content-length');
        if (len && /^\d+$/.test(len.trim()) && Number(len) > MAX_CANDIDATE_BYTES) return;
        let perTab = candidates.get(d.tabId);
        if (!perTab) candidates.set(d.tabId, (perTab = new Map()));
        if (perTab.size >= MAX_CANDIDATES && !perTab.has(d.url)) {
          const oldest = perTab.keys().next().value;
          if (oldest !== undefined) perTab.delete(oldest);
        }
        const headers = refererOrigin(d);
        const frameUrl = frameUrlOf(d);
        perTab.set(d.url, { url: d.url, ...(Object.keys(headers).length ? { headers } : {}), ...(frameUrl ? { frameUrl } : {}) });
        return;
      }
      // A top-level HTML-ish navigation is never media; a direct .mp4 navigation is.
      const size = totalSize(d.statusCode, header(d.responseHeaders, 'content-length'), header(d.responseHeaders, 'content-range'));
      const headers = refererOrigin(d);
      const frameUrl = frameUrlOf(d);
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
        ...(frameUrl ? { frameUrl } : {}),
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
    candidates.delete(tabId);
    session?.remove(keyFor(tabId)).catch(() => {});
  });
}
