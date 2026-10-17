/** Floating video button + popup media actions. */
import type { NativeError } from '../shared/correlator';
import { type ElementSource, type MediaItem, elementMedia, mediaDetail, mediaLabel, visibleMedia } from '../shared/media';
import type { VideoOption, VideoOptionsResult } from '../shared/messages';
import type { StreamVariant, VideoRequest, VideoStream } from '../shared/protocol';
import { siteMatches } from '../shared/rules';
import { actionTimeout, cookieHeaderFor, tabInfo, userAgent } from './context';
import { native } from './native';
import { getTabMedia } from './sniffer';
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

function variantLabel(v: StreamVariant): string {
  return v.height ? `${v.height}p` : v.bandwidth ? `${Math.round(v.bandwidth / 1000)} kbps` : 'Stream';
}

function variantDetail(v: StreamVariant, kind: 'hls' | 'dash'): string {
  const parts = [kind.toUpperCase()];
  if (v.bandwidth) parts.push(`${Math.round(v.bandwidth / 1000)} kbps`);
  return parts.join(' · ');
}

/**
 * A sniffed HLS/DASH manifest is one network request, but it can declare several quality
 * variants inside its own content — the player only fetches whichever one it picks, so the
 * sniffer alone never sees the others. Probes the manifest for its full variant list; falls
 * back to the single raw entry (today's behavior) on any failure, timeout, or a manifest with
 * only one variant worth naming.
 */
async function expandStream(
  m: MediaItem,
  group: VideoOption['group'],
  pageUrl: string,
  incognito: boolean,
): Promise<VideoOption[]> {
  const base = (): VideoOption => ({ id: `s:${m.key}`, label: mediaLabel(m), detail: mediaDetail(m), group, stream: streamOf(m) });
  if (m.kind !== 'hls' && m.kind !== 'dash') return [base()];
  try {
    const reply = await native.request(
      {
        type: 'probeStream',
        url: m.url,
        kind: m.kind,
        ...(m.headers ? { headers: m.headers } : {}),
        cookies: await cookieHeaderFor(m.url, incognito),
        pageUrl,
        userAgent: userAgent(),
      },
      { timeoutMs: 8000 },
    );
    if (reply.variants.length < 2) return [base()];
    return reply.variants
      .slice()
      .sort((a, b) => (b.height ?? 0) - (a.height ?? 0) || (b.bandwidth ?? 0) - (a.bandwidth ?? 0))
      .map((v) => ({
        id: `s:${m.key}:${v.index}`,
        label: variantLabel(v),
        detail: variantDetail(v, m.kind as 'hls' | 'dash'),
        group,
        stream: { ...streamOf(m), variantIndex: v.index },
      }));
  } catch {
    // Network error, timeout, or the app declined (e.g. a live/DRM manifest) — the one raw
    // entry still downloads correctly today, it just won't offer a quality choice.
    return [base()];
  }
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
  const incognito = !!tab?.incognito;
  const expanded = await Promise.all([
    ...own.map((m) => expandStream(m, 'element', msg.pageUrl, incognito)),
    ...rest.map((m) => expandStream(m, 'stream', msg.pageUrl, incognito)),
  ]);
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
