import { describe, expect, it, vi } from 'vitest';
import type { ConnectionInfo, PopupState } from '../src/shared/messages';
import { defaultState } from '../src/shared/settings';

const app = { innerHTML: '', addEventListener: vi.fn() };
(globalThis as unknown as { document: unknown }).document = {
  getElementById: (id: string) => (id === 'app' ? app : null),
  querySelector: () => null,
};
(globalThis as unknown as { location: unknown }).location = { search: '?tabId=1' };

const connection: ConnectionInfo = {
  state: 'app_down',
  errorCode: 'unauthorized',
  error: 'Prism web server rejected the token',
};
const popupState: PopupState = {
  connection,
  media: [],
  state: { ...defaultState(), remoteUrl: 'http://127.0.0.1:18186', remoteToken: 'fake-test-token' },
};
const sendMessage = vi.fn(async (msg: { kind: string }) => (msg.kind === 'popupState' ? popupState : connection));
(globalThis as unknown as { chrome: unknown }).chrome = {
  runtime: { sendMessage, openOptionsPage: vi.fn() },
  tabs: { query: vi.fn(async () => []) },
};

await import('../src/popup/popup');

describe('popup in remote mode', () => {
  it('points failed connections at Options and relabels the footer', async () => {
    await vi.waitFor(() => expect(app.innerHTML).toContain('Open Prism Web'));
    expect(app.innerHTML).toContain('Prism web isn’t reachable');
    expect(app.innerHTML).toContain('Check the server URL and bearer token in Options');
    expect(app.innerHTML).toContain('data-act="options"');
    expect(app.innerHTML).toContain('Retry');
    expect(app.innerHTML).not.toContain('Start Prism');
    expect(app.innerHTML).not.toContain('fake-test-token');
  });
});
