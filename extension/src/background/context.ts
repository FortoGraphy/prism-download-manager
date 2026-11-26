import { IS_FIREFOX, api } from '../shared/api';
import { buildCookieHeader, mergeCookieHeaders, pickCookieStoreId, storeCookiesToAdd } from '../shared/cookies';
import type { BrowserName } from '../shared/protocol';
import { getConnection } from './state';

export const userAgent = (): string => navigator.userAgent;

export const EXTENSION_VERSION = api.runtime.getManifest().version;

export function detectBrowser(): BrowserName {
  if (IS_FIREFOX) return 'firefox';
  const nav = navigator as Navigator & { userAgentData?: { brands?: Array<{ brand: string }> }; brave?: unknown };
  const brands = (nav.userAgentData?.brands ?? []).map((b) => b.brand.toLowerCase());
  if (brands.some((b) => b.includes('edge'))) return 'edge';
  if (brands.some((b) => b.includes('opera'))) return 'opera';
  if (nav.brave) return 'brave';
  if (/vivaldi/i.test(navigator.userAgent)) return 'vivaldi';
  if (brands.some((b) => b === 'google chrome')) return 'chrome';
  return 'chromium';
}

/**
 * How long a captured download waits for the app (#130, #98): longer than the host takes to
 * start the app (8 s) and the app to try the link (2.5 s). The browser holds its own download
 * meanwhile; the request carries a deadline, after which the app opens no dialog, so a reply
 * that comes too late never leaves both the browser and Prism downloading the file.
 */
export const CAPTURE_TIMEOUT_MS = 20_000;

/** The app must answer a captured download this much before the extension stops waiting. */
export const CAPTURE_DEADLINE_MARGIN_MS = 2_000;

/**
 * Timeout for user-initiated requests: short when the app is known to be up,
 * long enough for the host to launch it (8 s) otherwise.
 */
export function actionTimeout(): number {
  return getConnection().state === 'connected' ? 5000 : 12000;
}

/**
 * Cookie header for `url` from the right cookie store, merged with a captured request Cookie header.
 * For a captured download (`capture`, which a page can start on its own), stored SameSite=Strict
 * cookies are added only where the browser itself would have sent them (`storeCookiesToAdd`). The
 * menus, the video button and the popup act on the user's own click and keep the full store.
 */
export async function cookieHeaderFor(url: string, incognito: boolean, sent?: string, capture?: { initiator: string | undefined }): Promise<string> {
  let fromStore = '';
  try {
    let storeId: string | undefined;
    if (incognito) storeId = pickCookieStoreId(await api.cookies.getAllCookieStores(), true);
    const details: chrome.cookies.GetAllDetails = { url };
    if (storeId) details.storeId = storeId;
    const stored = await api.cookies.getAll(details);
    fromStore = buildCookieHeader(capture ? storeCookiesToAdd(stored, url, sent, capture.initiator) : stored);
  } catch (e) {
    console.debug('[prism] cookies.getAll failed', e);
  }
  return mergeCookieHeaders(sent, fromStore);
}

/** Cookie headers per origin, cached for one batch. */
export function cookieCache(incognito: boolean): (url: string) => Promise<string> {
  const cache = new Map<string, Promise<string>>();
  return (url: string) => {
    let origin: string;
    try {
      const u = new URL(url);
      origin = u.origin + u.pathname.slice(0, u.pathname.lastIndexOf('/') + 1);
    } catch {
      return Promise.resolve('');
    }
    let p = cache.get(origin);
    if (!p) {
      p = cookieHeaderFor(url, incognito);
      cache.set(origin, p);
    }
    return p;
  };
}

export async function tabInfo(tabId: number | undefined): Promise<chrome.tabs.Tab | undefined> {
  if (tabId === undefined || tabId < 0) return undefined;
  try {
    return await api.tabs.get(tabId);
  } catch {
    return undefined;
  }
}
