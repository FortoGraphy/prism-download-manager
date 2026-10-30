/** Floating video button + popup media actions. */
import type { NativeError } from '../shared/correlator';
import { type ElementSource, type MediaItem, elementMedia, mediaDetail, mediaLabel, visibleMedia } from '../shared/media';
import type { VideoOption, VideoOptionsResult } from '../shared/messages';
import type { VideoRequest, VideoStream } from '../shared/protocol';
import { siteMatches } from '../shared/rules';
import { variantOptions } from '../shared/variants';
import { actionTimeout, cookieHeaderFor, tabInfo, userAgent } from './context';
import { native } from './native';
import { confirmCandidate, getTabCandidates, getTabMedia } from './sniffer';
import { getState } from './state';

export function isVideoSite(url: string | undefined, sites: string[]): boolean {
  return !!url && sites.some((s) => siteMatches(url, s));
}

export function streamOf(m: MediaItem): VideoStream {
  return {
    url: m.url,
    kind: m.streamKind,
    ...(m.mime ? { mime: m.mime } : {}),
    ...(m.size ? { size: m.size } : {}),
    ...(m.quality ? { quality: m.quality } : {}),
    ...(m.headers ? { headers: m.headers } : {}),
  };
}

const rank = (m: MediaItem) => (m.kind === 'hls' || m.kind === 'dash' ? 0 : m.kind === 'video' ? 1 : 2);

const option = (m: MediaItem, group: VideoOption['group']): VideoOption => ({ id: `s:${m.key}`, label: mediaLabel(m), detail: mediaDetail(m), group, stream: streamOf(m) });

/** At most this many manifests are read for their qualities per menu, each for at most this long:
 *  the menu waits for them, and a slow or failing one keeps its single entry. */
const EXPAND_MAX = 4;
const EXPAND_TIMEOUT_MS = 3000;

/**
 * A sniffed HLS/DASH manifest is one request, but it can declare several qualities, of which the
 * player fetches one. The app reads the manifest and the menu lists them all; on any failure the
 * stream keeps its single entry.
 */
async function expandStream(m: MediaItem, group: VideoOption['group'], pageUrl: string, incognito: boolean): Promise<VideoOption[]> {
  const base = option(m, group);
  if (m.kind !== 'hls' && m.kind !== 'dash') return [base];
  try {
    const reply = await native.request(
      {
        type: 'probeStream',
        url: m.url,
        kind: m.kind,
        ...(m.headers ? { headers: m.headers } : {}),
        // Opening the menu is not picking this stream: as for a captured download, no SameSite=Strict
        // cookie unless the page that loaded the manifest is on the manifest's own host.
        cookies: await cookieHeaderFor(m.url, incognito, undefined, { initiator: m.frameUrl ?? m.headers?.Referer }),
        pageUrl,
        userAgent: userAgent(),
      },
      { timeoutMs: EXPAND_TIMEOUT_MS },
    );
    return variantOptions(base, m.kind, reply.variants ?? []) ?? [base];
  } catch {
    // Timeout, an older app without probeStream, or a manifest the app declined (live, DRM).
    return [base];
  }
}

/**
 * A candidate that looked disguised (image/* on an XHR/media fetch): checked only now, as the
 * user opens the menu. On confirmation it's treated exactly like any other sniffed manifest
 * (same expandStream, same cookie rules); on failure or timeout it contributes nothing — there
 * was never a single-entry fallback for it since it was never real media to begin with.
 */
async function confirmAndExpand(tabId: number, url: string, group: VideoOption['group'], pageUrl: string, incognito: boolean): Promise<VideoOption[]> {
  const m = await confirmCandidate(tabId, url);
  return m ? expandStream(m, group, pageUrl, incognito) : [];
}

function noticeFor(err: NativeError): string {
  switch (err.code) {
    case 'host_missing':
    case 'host_forbidden':
      return 'Browser integration is not installed — open Prism › Settings › Browser integration.';
    case 'app_not_running':
    case 'app_unreachable':
      return "Prism isn't running — showing streams found on this page.";
    case 'timeout':
      return 'Prism took too long to read this page — showing streams found on this page.';
    case 'not_a_video_page':
      return err.message;
    default:
      // yt-dlp's own text is long and technical ("please report this issue…"): console only.
      console.debug('[prism] probeVideo failed:', err.message);
      if (/couldn't read the cookies/i.test(err.message)) {
        return "Prism couldn't read your browser's cookies (Settings › Video & streams › Cookies from browser) — showing streams found on this page.";
      }
      if (/sign in|log in|logged-in|login required|private/i.test(err.message)) {
        return 'This video needs you to be signed in: sign in to the site in this browser and try again.';
      }
      // Prism 1.0.6+ already answers in plain words; older apps pass yt-dlp's text through.
      if (err.code === 'app_error' && !/yt-dlp/i.test(err.message) && err.message.length < 200) return err.message;
      return "Prism couldn't get this video's formats — showing streams found on this page (some may be video or audio only).";
  }
}

export async function videoOptions(
  msg: { pageUrl: string; title: string; sources?: ElementSource[]; postUrl?: string },
  tab: chrome.tabs.Tab | undefined,
): Promise<VideoOptionsResult> {
  const st = await getState();
  let notice: string | undefined;
  if (isVideoSite(msg.pageUrl, st.settings.videoSites) || isVideoSite(tab?.url, st.settings.videoSites)) {
    // On a feed / profile / listing the hovered post, not the page (which yt-dlp can't read or
    // reads as some other video).
    const pageUrl =
      msg.postUrl && isVideoSite(msg.postUrl, st.settings.videoSites) ? msg.postUrl : isVideoSite(msg.pageUrl, st.settings.videoSites) ? msg.pageUrl : tab!.url!;
    // Feeds and profiles of sites that read single posts only (LinkedIn, Instagram) are refused by
    // the app at once (`not_a_video_page`), without running yt-dlp.
    try {
      const reply = await native.request(
        { type: 'probeVideo', pageUrl, cookies: await cookieHeaderFor(pageUrl, !!tab?.incognito), userAgent: userAgent() },
        { timeoutMs: 60_000 },
      );
      const options: VideoOption[] = (reply.formats ?? []).map((f) => ({
        id: `fmt:${f.id}`,
        label: f.label,
        group: f.kind === 'audio' ? 'audio' : 'video',
        formatId: f.id,
        ...(f.note ? { detail: f.note } : {}),
      }));
      if (options.length) return { source: 'probe', pageUrl, ...(reply.title ? { title: reply.title } : {}), options };
      notice = 'Prism found no downloadable formats on this page.';
    } catch (e) {
      notice = noticeFor(e as NativeError);
    }
  }
  const media = tab?.id !== undefined ? visibleMedia(await getTabMedia(tab.id)) : [];
  // The hovered video first: on a feed the tab also holds other posts' and autoplaying ads'
  // streams, and the largest of those is rarely the one the user pointed at.
  const own = elementMedia(msg.sources ?? [], media, msg.pageUrl);
  const ownKeys = new Set(own.map((m) => m.key));
  const rest = media.filter((m) => !ownKeys.has(m.key)).sort((a, b) => rank(a) - rank(b) || (b.size ?? 0) - (a.size ?? 0));
  const candidateUrls = tab?.id !== undefined ? getTabCandidates(tab.id) : [];
  const incognito = !!tab?.incognito;
  let budget = EXPAND_MAX;
  const take = () => budget-- > 0;
  const entries = (m: MediaItem, group: VideoOption['group']) =>
    (m.kind === 'hls' || m.kind === 'dash') && take() ? expandStream(m, group, msg.pageUrl, incognito) : Promise.resolve([option(m, group)]);
  const candidateEntries = (c: { url: string }) => (take() ? confirmAndExpand(tab!.id!, c.url, 'stream', msg.pageUrl, incognito) : Promise.resolve([]));
  const expanded = await Promise.all([...own.map((m) => entries(m, 'element')), ...rest.map((m) => entries(m, 'stream')), ...candidateUrls.map(candidateEntries)]);
  const options = expanded.flat();
  return { source: 'sniffed', options, ...(notice ? { notice } : {}) };
}

export async function sendVideo(pageUrl: string, title: string, option: VideoOption, tab: chrome.tabs.Tab | undefined): Promise<void> {
  const incognito = !!tab?.incognito;
  const msg: VideoRequest = option.formatId
    ? {
        type: 'video',
        pageUrl,
        title,
        formatId: option.formatId,
        formatLabel: option.label,
        streams: [],
        cookies: await cookieHeaderFor(pageUrl, incognito),
        userAgent: userAgent(),
        incognito,
      }
    : {
        type: 'video',
        pageUrl,
        title,
        streams: option.stream ? [option.stream] : [],
        cookies: option.stream ? await cookieHeaderFor(option.stream.url, incognito) : '',
        userAgent: userAgent(),
        incognito,
      };
  const reply = await native.request(msg, { timeoutMs: actionTimeout() });
  if (reply.accepted === false) throw new Error('Prism declined the video');
}

/** Popup "Download" on a sniffed media item. */
export async function downloadMedia(tabId: number, key: string): Promise<void> {
  const tab = await tabInfo(tabId);
  const m = (await getTabMedia(tabId)).find((x) => x.key === key);
  if (!m) throw new Error('media item is gone (page navigated?)');
  await sendVideo(tab?.url ?? m.frameUrl ?? '', tab?.title ?? m.name, { id: `s:${m.key}`, label: mediaLabel(m), group: 'stream', stream: streamOf(m) }, tab);
}
