import { api, sendInternal } from '../shared/api';
import { prismMark } from '../shared/icons';
import type { CaptureSettings } from '../shared/protocol';
import { DEFAULT_SETTINGS, STATE_KEY, type StoredState, normalizeState } from '../shared/settings';

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const form = $<HTMLFormElement>('form');
const status = $<HTMLSpanElement>('status');

$('mark').innerHTML = prismMark(40);

function setStatus(text: string, kind: '' | 'ok' | 'bad' = ''): void {
  status.textContent = text;
  status.className = `muted small ${kind}`;
}

function fill(s: CaptureSettings, st?: StoredState): void {
  $<HTMLInputElement>('captureEnabled').checked = s.captureEnabled;
  $<HTMLInputElement>('showVideoButton').checked = s.showVideoButton;
  $<HTMLSelectElement>('bypassKey').value = s.bypassKey;
  $<HTMLTextAreaElement>('extensions').value = s.extensions.join(' ');
  $<HTMLTextAreaElement>('mimeTypes').value = s.mimeTypes.join('\n');
  $<HTMLInputElement>('minSize').value = String(Math.round(s.minSize / 1024));
  $<HTMLTextAreaElement>('excludedSites').value = s.excludedSites.join('\n');
  $<HTMLTextAreaElement>('videoSites').value = s.videoSites.join('\n');
  if (st) {
    $<HTMLInputElement>('remoteUrl').value = st.remoteUrl;
    $<HTMLInputElement>('remoteToken').value = st.remoteToken;
    $('source-line').textContent =
      st.settingsSource === 'app'
        ? `Settings follow the Prism app${st.appVersion ? ` (v${st.appVersion})` : ''}. Saving here overrides them locally.`
        : 'Using local settings. “Sync from Prism” goes back to the app’s settings.';
  }
}

function read(): Partial<CaptureSettings> {
  const lines = (id: string) => $<HTMLTextAreaElement>(id).value.split(/\n+/).map((s) => s.trim()).filter(Boolean);
  return {
    captureEnabled: $<HTMLInputElement>('captureEnabled').checked,
    showVideoButton: $<HTMLInputElement>('showVideoButton').checked,
    bypassKey: $<HTMLSelectElement>('bypassKey').value as CaptureSettings['bypassKey'],
    extensions: $<HTMLTextAreaElement>('extensions').value.split(/[\s,;]+/).filter(Boolean),
    mimeTypes: lines('mimeTypes'),
    minSize: Math.max(0, Number($<HTMLInputElement>('minSize').value) || 0) * 1024,
    excludedSites: lines('excludedSites'),
    videoSites: lines('videoSites'),
  };
}

async function load(): Promise<void> {
  const r = await api.storage.local.get(STATE_KEY);
  const st = normalizeState(r[STATE_KEY]);
  fill(st.settings, st);
}

form.addEventListener('submit', async (e) => {
  e.preventDefault();
  const r = await sendInternal<{ ok: boolean; state?: StoredState; error?: string }>({
    kind: 'saveSettings',
    settings: read(),
    remoteUrl: $<HTMLInputElement>('remoteUrl').value,
    remoteToken: $<HTMLInputElement>('remoteToken').value,
  });
  if (r?.ok && r.state) {
    fill(r.state.settings, r.state);
    setStatus('Saved.', 'ok');
  } else setStatus(r?.error ?? 'Could not save.', 'bad');
});

$('reset').addEventListener('click', () => {
  fill(DEFAULT_SETTINGS);
  setStatus('Defaults restored — click Save to keep them.');
});

$('sync').addEventListener('click', async () => {
  setStatus('Asking Prism…');
  const r = await sendInternal<{ ok: boolean; state?: StoredState; error?: string }>({ kind: 'syncSettings' });
  if (r?.ok && r.state) {
    fill(r.state.settings, r.state);
    setStatus('Settings synced from Prism.', 'ok');
  } else setStatus(`Couldn’t reach Prism: ${r?.error ?? 'no answer'}. Is the app running?`, 'bad');
});

void load();
