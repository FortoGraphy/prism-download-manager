/** Context menu items. Created on install/startup, handled at top level. */
import { api } from '../shared/api';
import { t, tk } from '../shared/i18n';
import type { DownloadRequest, VideoRequest } from '../shared/protocol';
import { filenameFromUrl, isHandoffUrl } from '../shared/rules';
import { showUnreachable } from './badge';
import { collectAndSend } from './collect';
import { actionTimeout, cookieHeaderFor, userAgent } from './context';
import { native } from './native';

type Ctx = chrome.contextMenus.ContextType;

// Keys (marked with `tk`), translated when the menus are created.
const ITEMS: Array<{ id: string; title: string; contexts: Ctx[] }> = [
  { id: 'prism-link', title: tk('Download with Prism'), contexts: ['link', 'image', 'video', 'audio'] as Ctx[] },
  { id: 'prism-selection', title: tk('Download selection links with Prism'), contexts: ['selection'] as Ctx[] },
  { id: 'prism-all-links', title: tk('Download all links with Prism'), contexts: ['page', 'selection'] as Ctx[] },
  { id: 'prism-all-images', title: tk('Download all images with Prism'), contexts: ['page', 'image'] as Ctx[] },
  { id: 'prism-all-videos', title: tk('Download all videos with Prism'), contexts: ['page', 'video', 'audio'] as Ctx[] },
];

export async function createMenus(): Promise<void> {
  const menus = api.contextMenus;
  if (!menus) return;
  await menus.removeAll();
  for (const it of ITEMS) {
    menus.create({ id: it.id, title: t(it.title), contexts: it.contexts as [Ctx, ...Ctx[]] }, () => void api.runtime.lastError);
  }
}

export async function sendSingleLink(url: string, referrer: string | undefined, tab: chrome.tabs.Tab | undefined, source: DownloadRequest['source']): Promise<void> {
  const incognito = !!tab?.incognito;
  const filename = filenameFromUrl(url);
  const msg: DownloadRequest = {
    type: 'download',
    url,
    ...(referrer ? { referrer } : {}),
    ...(tab?.url ? { pageUrl: tab.url } : {}),
    ...(filename ? { filename } : {}),
    method: 'GET',
    cookies: await cookieHeaderFor(url, incognito),
    userAgent: userAgent(),
    incognito,
    source,
  };
  await native.request(msg, { timeoutMs: actionTimeout() });
}

/**
 * "Download with Prism" on a video/audio element that plays a `blob:` (MSE) stream, which has
 * no URL to hand over: open the page's Prism video menu for that element (it knows the post on
 * a feed and the streams it loaded); without the in-page button, let the app read the page.
 */
async function downloadElementVideo(info: chrome.contextMenus.OnClickData, tab: chrome.tabs.Tab | undefined): Promise<void> {
  const tabId = tab?.id;
  if (tabId === undefined) return;
  const shown = await api.tabs
    .sendMessage(tabId, { kind: 'showVideoMenu' }, { frameId: info.frameId ?? 0 })
    .then((r: { ok?: boolean } | undefined) => !!r?.ok)
    .catch(() => false);
  if (shown) return;
  const pageUrl = info.frameUrl ?? info.pageUrl ?? tab?.url;
  if (!pageUrl || !isHandoffUrl(pageUrl)) return;
  const incognito = !!tab?.incognito;
  const msg: VideoRequest = {
    type: 'video',
    pageUrl,
    title: tab?.title ?? '',
    streams: [],
    cookies: await cookieHeaderFor(pageUrl, incognito),
    userAgent: userAgent(),
    incognito,
  };
  await native.request(msg, { timeoutMs: actionTimeout() });
}

export async function handleMenuClick(info: chrome.contextMenus.OnClickData, tab: chrome.tabs.Tab | undefined): Promise<void> {
  const tabId = tab?.id;
  switch (info.menuItemId) {
    case 'prism-link': {
      const url = info.linkUrl ?? info.srcUrl;
      if (url && isHandoffUrl(url)) await sendSingleLink(url, info.frameUrl ?? info.pageUrl, tab, 'contextMenu');
      else if (info.mediaType === 'video' || info.mediaType === 'audio') await downloadElementVideo(info, tab);
      break;
    }
    case 'prism-selection':
      if (tabId !== undefined) await collectAndSend(tabId, 'selection', info.frameId);
      break;
    case 'prism-all-links':
      if (tabId !== undefined) await collectAndSend(tabId, 'links');
      break;
    case 'prism-all-images':
      if (tabId !== undefined) await collectAndSend(tabId, 'images');
      break;
    case 'prism-all-videos':
      if (tabId !== undefined) await collectAndSend(tabId, 'videos');
      break;
  }
}

export function initMenus(): void {
  api.contextMenus?.onClicked.addListener((info, tab) => {
    void (async () => {
      try {
        await handleMenuClick(info, tab);
      } catch (e) {
        console.warn('[prism] context menu action failed', e);
        showUnreachable(true, (e as Error).message);
      }
    })();
  });
}
