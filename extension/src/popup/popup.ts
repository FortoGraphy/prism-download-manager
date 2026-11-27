import { api, sendInternal } from '../shared/api';
import { t, setLanguage } from '../shared/i18n';
import { icons, prismMark } from '../shared/icons';
import { type MediaItem, humanSize } from '../shared/media';
import type { ActionResult, ConnectionInfo, PopupState } from '../shared/messages';
import type { BatchKind } from '../shared/protocol';

setLanguage();

const app = document.getElementById('app')!;
let tabId = -1;
let state: PopupState | null = null;
let busy = false;

function esc(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);
}

function toast(text: string, bad = false): void {
  let node = document.querySelector<HTMLDivElement>('.toast');
  if (!node) {
    node = document.createElement('div');
    node.className = 'toast glass';
    node.setAttribute('role', 'status');
    document.body.append(node);
  }
  node.textContent = text;
  node.classList.toggle('bad', bad);
  node.classList.add('show');
  clearTimeout(Number(node.dataset.timer));
  node.dataset.timer = String(setTimeout(() => node!.classList.remove('show'), 2200));
}

function chip(c: ConnectionInfo): string {
  if (c.state === 'connected') return `<span class="chip ok" id="conn" data-state="connected"><span class="dot"></span>${t('Connected')}${c.appVersion ? ` · v${esc(c.appVersion)}` : ''}</span>`;
  if (c.state === 'unknown') return `<span class="chip unknown" id="conn" data-state="unknown"><span class="dot"></span>${t('Checking…')}</span>`;
  return `<span class="chip bad" id="conn" data-state="${c.state}"><span class="dot"></span>${t('Not connected')}</span>`;
}

function alertCard(c: ConnectionInfo): string {
  if (c.state === 'connected' || c.state === 'unknown') return '';
  const missing = c.state === 'host_missing';
  const title = missing ? t('Browser integration isn’t installed') : t('Prism isn’t reachable');
  const hint = missing
    ? t('Fix: open Prism › Settings › Browser integration and click Repair.')
    : t('Start Prism, or open Prism › Settings › Browser integration › Repair. Downloads stay in the browser meanwhile.');
  return `<section class="alert glass" id="fix">
    <span class="ic">${icons.alert(18)}</span><b>${esc(title)}</b>
    <p>${esc(hint)}${c.error ? `<br><span class="small">${esc(t('Details: {error}', { error: c.error }))}</span>` : ''}</p>
    <div class="actions">
      ${missing ? '' : `<button class="btn primary" data-act="launch">${icons.external(14)}${esc(t('Start Prism'))}</button>`}
      <button class="btn" data-act="retry">${icons.refresh(14)}${esc(t('Retry'))}</button>
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
  const capturing = st.settings.captureEnabled && !st.paused;
  app.innerHTML = `
    <header class="top">
      <span class="tile">${prismMark(24)}</span>
      <div class="brand"><b>Prism</b><span class="muted">${esc(t('Download manager'))}</span></div>
      ${chip(connection)}
    </header>
    ${alertCard(connection)}
    <section class="row glass">
      <div class="tx"><b>${esc(t('Capture downloads'))}</b><span>${esc(
        !st.settings.captureEnabled ? t('Disabled in settings') : st.paused ? t('Paused — the browser downloads files itself') : t('Matching files open in Prism')
      )}</span></div>
      <label class="switch" title="${esc(t('Pause capture'))}"><input type="checkbox" id="capture" ${capturing ? 'checked' : ''} ${
        st.settings.captureEnabled ? '' : 'disabled'
      } aria-label="${esc(t('Capture downloads'))}"><span></span></label>
    </section>
    <section class="section glass">
      <h2>${esc(t('Media on this page'))} <span class="count" id="media-count">${media.length}</span></h2>
      <div class="list" id="media">${
        media.length
          ? media
              .map(
                (m) => `<div class="media" data-key="${esc(m.key)}">
                  <span class="kind">${kindIcon(m)}</span>
                  <span class="tx"><span class="name" title="${esc(m.url)}">${esc(m.name)}</span><span class="meta">${esc(mediaMeta(m))}</span></span>
                  <button class="btn" data-act="media" data-key="${esc(m.key)}">${icons.download(14)}${esc(t('Download'))}</button>
                </div>`,
              )
              .join('')
          : `<div class="empty"><b>${esc(t('No media detected yet'))}</b>${esc(t('Play a video or audio on the page and it will show up here.'))}</div>`
      }</div>
    </section>
    <section class="grab">
      <button class="btn" data-act="collect" data-mode="links">${icons.link(18)}${esc(t('All links'))}</button>
      <button class="btn" data-act="collect" data-mode="images">${icons.image(18)}${esc(t('All images'))}</button>
      <button class="btn" data-act="collect" data-mode="videos">${icons.film(18)}${esc(t('All videos'))}</button>
    </section>
    <footer class="foot">
      <button class="btn ghost" data-act="open">${prismMark(16)}${esc(t('Open Prism'))}</button>
      <span class="spacer"></span>
      <button class="btn ghost icon" data-act="options" title="${esc(t('Options'))}" aria-label="${esc(t('Options'))}">${icons.gear(16)}</button>
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
        if (r?.ok) toast(t('Sent to Prism'));
        break;
      case 'collect':
        r = await sendInternal<ActionResult>({ kind: 'collect', tabId, mode: el.dataset.mode as BatchKind });
        if (r?.ok) toast(r.count ? t('Sent {count} item(s) to Prism', { count: r.count }) : t('Nothing found on this page'));
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
    if (r && !r.ok) toast(r.error ? t(r.error) : t('Something went wrong'), true);
    if (!r) toast(t('Prism extension is restarting — try again'), true);
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
