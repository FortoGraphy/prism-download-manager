import { api, sendInternal } from '../shared/api';
import { icons, prismMark } from '../shared/icons';
import { type MediaItem, humanSize } from '../shared/media';
import type { ActionResult, ConnectionInfo, PopupState } from '../shared/messages';
import type { BatchKind } from '../shared/protocol';

const app = document.getElementById('app')!;
let tabId = -1;
let state: PopupState | null = null;
let busy = false;

function esc(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);
}

function toast(text: string, bad = false): void {
  let t = document.querySelector<HTMLDivElement>('.toast');
  if (!t) {
    t = document.createElement('div');
    t.className = 'toast glass';
    t.setAttribute('role', 'status');
    document.body.append(t);
  }
  t.textContent = text;
  t.classList.toggle('bad', bad);
  t.classList.add('show');
  clearTimeout(Number(t.dataset.timer));
  t.dataset.timer = String(setTimeout(() => t!.classList.remove('show'), 2200));
}

function chip(c: ConnectionInfo): string {
  if (c.state === 'connected') return `<span class="chip ok" id="conn" data-state="connected"><span class="dot"></span>Connected${c.appVersion ? ` · v${esc(c.appVersion)}` : ''}</span>`;
  if (c.state === 'unknown') return `<span class="chip unknown" id="conn" data-state="unknown"><span class="dot"></span>Checking…</span>`;
  return `<span class="chip bad" id="conn" data-state="${c.state}"><span class="dot"></span>Not connected</span>`;
}

function alertCard(c: ConnectionInfo, remote = false): string {
  if (c.state === 'connected' || c.state === 'unknown') return '';
  const missing = c.state === 'host_missing';
  const title = missing ? 'Browser integration isn’t installed' : remote ? 'Prism web isn’t reachable' : 'Prism isn’t reachable';
  const hint = missing
    ? 'Fix: open Prism › Settings › Browser integration and click Repair.'
    : remote
      ? 'Check the server URL and bearer token in Options, and that the server is running. Downloads stay in the browser meanwhile.'
      : 'Start Prism, or open Prism › Settings › Browser integration › Repair. Downloads stay in the browser meanwhile.';
  return `<section class="alert glass" id="fix">
    <span class="ic">${icons.alert(18)}</span><b>${title}</b>
    <p>${hint}${c.error ? `<br><span class="small">Details: ${esc(c.error)}</span>` : ''}</p>
    <div class="actions">
      ${missing ? '' : remote ? `<button class="btn primary" data-act="options">${icons.gear(14)}Options</button>` : `<button class="btn primary" data-act="launch">${icons.external(14)}Start Prism</button>`}
      <button class="btn" data-act="retry">${icons.refresh(14)}Retry</button>
    </div>
  </section>`;
}

function kindIcon(m: MediaItem): string {
  if (m.kind === 'audio') return icons.audio(16);
  if (m.kind === 'hls' || m.kind === 'dash') return icons.stream(16);
  return icons.film(16);
}

function mediaMeta(m: MediaItem): string {
  const kind = m.kind === 'hls' ? 'HLS' : m.kind === 'dash' ? 'DASH' : m.mime || m.kind;
  return [kind, m.quality, humanSize(m.size)].filter(Boolean).join(' · ');
}

function render(): void {
  if (!state) return;
  const { connection, media, state: st } = state;
  const remote = !!st.remoteUrl;
  const capturing = st.settings.captureEnabled && !st.paused;
  app.innerHTML = `
    <header class="top">
      <span class="tile">${prismMark(24)}</span>
      <div class="brand"><b>Prism</b><span class="muted">Download manager</span></div>
      ${chip(connection)}
    </header>
    ${alertCard(connection, remote)}
    <section class="row glass">
      <div class="tx"><b>Capture downloads</b><span>${
        !st.settings.captureEnabled ? 'Disabled in settings' : st.paused ? 'Paused — the browser downloads files itself' : 'Matching files open in Prism'
      }</span></div>
      <label class="switch" title="Pause capture"><input type="checkbox" id="capture" ${capturing ? 'checked' : ''} ${
        st.settings.captureEnabled ? '' : 'disabled'
      } aria-label="Capture downloads"><span></span></label>
    </section>
    <section class="section glass">
      <h2>Media on this page <span class="count" id="media-count">${media.length}</span></h2>
      <div class="list" id="media">${
        media.length
          ? media
              .map(
                (m) => `<div class="media" data-key="${esc(m.key)}">
                  <span class="kind">${kindIcon(m)}</span>
                  <span class="tx"><span class="name" title="${esc(m.url)}">${esc(m.name)}</span><span class="meta">${esc(mediaMeta(m))}</span></span>
                  <button class="btn" data-act="media" data-key="${esc(m.key)}">${icons.download(14)}Download</button>
                </div>`,
              )
              .join('')
          : `<div class="empty"><b>No media detected yet</b>Play a video or audio on the page and it will show up here.</div>`
      }</div>
    </section>
    <section class="grab">
      <button class="btn" data-act="collect" data-mode="links">${icons.link(18)}All links</button>
      <button class="btn" data-act="collect" data-mode="images">${icons.image(18)}All images</button>
      <button class="btn" data-act="collect" data-mode="videos">${icons.film(18)}All videos</button>
    </section>
    <footer class="foot">
      <button class="btn ghost" data-act="open">${prismMark(16)}Open Prism${remote ? ' Web' : ''}</button>
      <span class="spacer"></span>
      <button class="btn ghost icon" data-act="options" title="Options" aria-label="Options">${icons.gear(16)}</button>
    </footer>`;
}

async function load(): Promise<void> {
  const s = await sendInternal<PopupState>({ kind: 'popupState', tabId });
  if (s) {
    state = s;
    render();
  }
}

async function refreshStatus(): Promise<void> {
  const c = await sendInternal<ConnectionInfo>({ kind: 'refreshStatus' });
  if (c && state) {
    state.connection = c;
    render();
  }
}

async function act(el: HTMLElement): Promise<void> {
  if (busy) return;
  const a = el.dataset.act;
  if (a === 'options') {
    await api.runtime.openOptionsPage();
    return;
  }
  busy = true;
  (el as HTMLButtonElement).disabled = true;
  try {
    let r: ActionResult | undefined;
    switch (a) {
      case 'media':
        r = await sendInternal<ActionResult>({ kind: 'downloadMedia', tabId, key: el.dataset.key! });
        if (r?.ok) toast('Sent to Prism');
        break;
      case 'collect':
        r = await sendInternal<ActionResult>({ kind: 'collect', tabId, mode: el.dataset.mode as BatchKind });
        if (r?.ok) toast(r.count ? `Sent ${r.count} item${r.count === 1 ? '' : 's'} to Prism` : 'Nothing found on this page');
        break;
      case 'open':
      case 'launch':
        r = await sendInternal<ActionResult>({ kind: 'openApp', page: 'main' });
        if (r?.ok) {
          if (a === 'open') window.close();
          else await refreshStatus();
        }
        break;
      case 'retry':
        await refreshStatus();
        r = { ok: true };
        break;
    }
    if (r && !r.ok) toast(r.error ?? 'Something went wrong', true);
    if (!r) toast('Prism extension is restarting — try again', true);
  } finally {
    busy = false;
    (el as HTMLButtonElement).disabled = false;
    if (a !== 'retry' && a !== 'launch') void refreshStatus();
  }
}

app.addEventListener('click', (e) => {
  const el = (e.target as HTMLElement).closest<HTMLElement>('[data-act]');
  if (el) void act(el);
});
app.addEventListener('change', async (e) => {
  const t = e.target as HTMLInputElement;
  if (t.id === 'capture') {
    await sendInternal({ kind: 'setPaused', paused: !t.checked });
    await load();
  }
});

(async () => {
  const q = new URLSearchParams(location.search).get('tabId');
  if (q) tabId = Number(q);
  else {
    const [tab] = await api.tabs.query({ active: true, currentWindow: true });
    tabId = tab?.id ?? -1;
  }
  await load();
  await refreshStatus();
})();
