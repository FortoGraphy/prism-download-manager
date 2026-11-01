/** Content script (all frames, document_start): bypass modifier + video button. */
import { api, sendInternal } from '../shared/api';
import { STATE_KEY, normalizeState } from '../shared/settings';
import { initBypass, setBypassKey } from './bypass';
import { VideoButton } from './video-button';

declare global {
  interface Window {
    __prismContentLoaded?: boolean;
  }
}

/** Runs player-hook.js in the page's own JS context (not our isolated world) so it can see
 *  `window.Hls` — the only way to read a manifest URL the page itself never exposed to a
 *  network listener (see player-hook.ts). */
function injectPlayerHook(): void {
  try {
    const script = document.createElement('script');
    script.src = api.runtime.getURL('player-hook.js');
    script.addEventListener('load', () => script.remove());
    (document.head || document.documentElement).appendChild(script);
  } catch {
    /* best effort — the regular network sniffer still covers the common case */
  }
}

/** player-hook.js found an hls.js manifest URL the network sniffer may never have seen. */
function listenForPlayerManifest(): void {
  window.addEventListener('message', (e) => {
    if (e.source !== window) return;
    const data = e.data as { source?: string; manifestUrl?: string } | undefined;
    if (data?.source !== 'prism-player-hook' || typeof data.manifestUrl !== 'string') return;
    void sendInternal({ kind: 'playerManifest', pageUrl: location.href, manifestUrl: data.manifestUrl });
  });
}

if (!window.__prismContentLoaded) {
  window.__prismContentLoaded = true;
  injectPlayerHook();
  listenForPlayerManifest();
  initBypass();
  const button = new VideoButton();

  const apply = (raw: unknown) => {
    const st = normalizeState(raw);
    setBypassKey(st.settings.bypassKey);
    button.setVideoSites(st.settings.videoSites);
    const enable = () => button.setEnabled(st.settings.showVideoButton);
    if (document.documentElement) enable();
    else document.addEventListener('DOMContentLoaded', enable, { once: true });
  };

  api.storage.local
    .get(STATE_KEY)
    .then((r) => apply(r[STATE_KEY]))
    .catch(() => apply(undefined));
  api.storage.onChanged.addListener((changes, area) => {
    if (area === 'local' && changes[STATE_KEY]) apply(changes[STATE_KEY].newValue);
  });
  // Context menu "Download with Prism" on a video that has no downloadable URL (blob:).
  api.runtime.onMessage.addListener((msg: { kind?: string } | undefined, _sender, sendResponse) => {
    if (msg?.kind !== 'showVideoMenu') return false;
    sendResponse({ ok: button.openForContextMenu() });
    return false;
  });
}
