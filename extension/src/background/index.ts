/**
 * Background entry (Chromium service worker / Firefox event page).
 * All listeners are registered synchronously at top level.
 */
import { IS_FIREFOX, api } from '../shared/api';
import type { NativeError } from '../shared/correlator';
import type { ActionResult, InternalMessage, PopupState } from '../shared/messages';
import { visibleMedia } from '../shared/media';
import { PROTOCOL_VERSION } from '../shared/protocol';
import { normalizeSettings } from '../shared/settings';
import { reflectConnection, setTabCount } from './badge';
import { bypass, initChromiumCapture, initFirefoxCapture, lastCapture } from './capture';
import { collectAndSend } from './collect';
import { EXTENSION_VERSION, actionTimeout, detectBrowser, tabInfo, userAgent } from './context';
import { createMenus, handleMenuClick, initMenus, sendSingleLink } from './menus';
import { native } from './native';
import { initRequestTracking } from './requests';
import { getTabMedia, initSniffer } from './sniffer';
import { applyAppSettings, getConnection, getState, markConnected, markFailure, onConnectionChange, updateState } from './state';
import { downloadMedia, sendVideo, videoOptions } from './video';

// ---- wiring -------------------------------------------------------------------

native.onResult((ok, err, reply) => {
  if (ok) {
    const app = reply?.app as { version?: string } | undefined;
    markConnected(app?.version);
  } else if (err) {
    markFailure(err);
  }
});
onConnectionChange(reflectConnection);

initRequestTracking();
initSniffer();
if (IS_FIREFOX) initFirefoxCapture();
else initChromiumCapture();
initMenus();

// ---- hello / watchdog -----------------------------------------------------------

let installTypeOnce: Promise<string | undefined> | undefined;
/** `management.getSelf()` needs no permission (Chromium and Firefox). */
function installType(): Promise<string | undefined> {
  installTypeOnce ??= Promise.resolve()
    .then(() => api.management?.getSelf?.())
    .then((self) => self?.installType, () => undefined);
  return installTypeOnce;
}

async function hello(noLaunch = true, forceSettings = false): Promise<void> {
  const kind = await installType();
  const reply = await native.request(
    { type: 'hello', protocol: PROTOCOL_VERSION, version: EXTENSION_VERSION, browser: detectBrowser(), userAgent: userAgent(), ...(kind ? { installType: kind } : {}) },
    { noLaunch, timeoutMs: noLaunch ? 4000 : 12000 },
  );
  await applyAppSettings(reply.settings, reply.app?.version, forceSettings);
}

function helloQuietly(): void {
  hello().catch((e: NativeError) => console.debug('[prism] hello failed:', e.code, e.message));
}

api.runtime.onInstalled.addListener(() => {
  void createMenus();
  helloQuietly();
});
api.runtime.onStartup.addListener(() => {
  void createMenus();
  helloQuietly();
});
api.alarms?.create('prism-watchdog', { periodInMinutes: 1 });
api.alarms?.onAlarm.addListener((a) => {
  if (a.name === 'prism-watchdog') {
    native.ensureConnected();
    helloQuietly();
  }
});
// Worker (re)start: open the port right away so the first capture is warm.
native.ensureConnected();
helloQuietly();

// ---- internal messages ------------------------------------------------------------

async function handle(msg: InternalMessage, sender: chrome.runtime.MessageSender): Promise<unknown> {
  switch (msg.kind) {
    case 'bypass':
      bypass.record(msg.url, Date.now());
      return { ok: true };
    case 'videoOptions':
      return videoOptions(msg, sender.tab);
    case 'videoDownload':
      await sendVideo(msg.pageUrl, msg.title, msg.option, sender.tab);
      return { ok: true } satisfies ActionResult;
    case 'popupState': {
      const [state, media, tab] = await Promise.all([getState(), getTabMedia(msg.tabId), tabInfo(msg.tabId)]);
      const visible = visibleMedia(media);
      setTabCount(msg.tabId, visible.length);
      return { connection: getConnection(), media: visible, state, ...(tab?.url ? { tabUrl: tab.url } : {}) } satisfies PopupState;
    }
    case 'refreshStatus':
      try {
        const r = await native.request({ type: 'status' }, { noLaunch: true, timeoutMs: 2500 });
        markConnected(r.app?.version);
        // Also refresh settings in the background.
        helloQuietly();
      } catch {
        /* state already updated by onResult */
      }
      return getConnection();
    case 'downloadMedia':
      await downloadMedia(msg.tabId, msg.key);
      return { ok: true } satisfies ActionResult;
    case 'collect': {
      const count = await collectAndSend(msg.tabId, msg.mode, msg.frameId);
      return { ok: true, count } satisfies ActionResult;
    }
    case 'setPaused':
      await updateState({ paused: msg.paused });
      return { ok: true };
    case 'openApp':
      await native.request({ type: 'open', page: msg.page ?? 'main' }, { timeoutMs: 12000 });
      return { ok: true };
    case 'syncSettings':
      await hello(false, true);
      return { ok: true, state: await getState() };
    case 'saveSettings':
      await updateState({ settings: normalizeSettings(msg.settings), settingsSource: 'local' });
      return { ok: true, state: await getState() };
    default:
      return { ok: false, error: 'unknown message' };
  }
}

api.runtime.onMessage.addListener((msg: InternalMessage, sender, sendResponse) => {
  if (!msg || typeof msg !== 'object' || !('kind' in msg)) return false;
  handle(msg, sender).then(
    (r) => sendResponse(r),
    (e: Error & { code?: string }) => sendResponse({ ok: false, error: e.message, code: e.code } satisfies ActionResult & { code?: string }),
  );
  return true;
});

declare const __DEV__: boolean;

// ---- debugging / e2e hooks (service worker global in dev/test only) ------------------
if (typeof __DEV__ !== 'undefined' && __DEV__) {
  Object.assign(globalThis, {
    __prism: {
      native,
      lastCapture,
      bypass,
      getConnection,
      collectAndSend,
      sendSingleLink,
      handleMenuClick,
      hello,
      actionTimeout,
    },
  });
}
