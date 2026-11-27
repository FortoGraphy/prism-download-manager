import { beforeEach, describe, expect, it, vi } from 'vitest';
import { STATE_KEY, normalizeRemoteToken, normalizeRemoteUrl } from '../src/shared/settings';

type Listener = (m: unknown) => void;

function fakePort() {
  const listeners: Listener[] = [];
  return {
    onMessage: { addListener: (l: Listener) => listeners.push(l) },
    onDisconnect: { addListener: (_l: Listener) => {} },
    postMessage: (m: Record<string, unknown>) => queueMicrotask(() => listeners.forEach((l) => l({ id: m.id, ok: true }))),
    disconnect: () => {},
  };
}

let stored: Record<string, unknown> = {};
const chromeMock = {
  runtime: {
    lastError: undefined as undefined | { message: string },
    connectNative: vi.fn(() => fakePort()),
    sendNativeMessage: vi.fn(async (_h: string, m: Record<string, unknown>) => ({ id: m.id, ok: true })),
  },
  storage: {
    local: { get: vi.fn(async () => ({ [STATE_KEY]: stored })), set: vi.fn(async () => {}) },
    onChanged: { addListener: vi.fn() },
  },
};
vi.stubGlobal('chrome', chromeMock);

const fetchMock = vi.fn((_url: string, _init?: RequestInit): Promise<Response> => Promise.reject(new Error('unexpected fetch')));
vi.stubGlobal('fetch', fetchMock);

const { NativeClient } = await import('../src/background/native');

beforeEach(() => {
  stored = {};
  fetchMock.mockReset();
  fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => ({ ok: true, ping: true }) } as Response);
  chromeMock.runtime.connectNative.mockClear();
});

describe('remote transport', () => {
  it('POSTs the envelope with the bearer token and echoes the reply', async () => {
    stored = { remoteUrl: 'http://127.0.0.1:8787/base/', remoteToken: 's3cr3t' };
    const c = new NativeClient();
    const reply = await c.request({ type: 'status' }, { noLaunch: true });

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0]!;
    expect(url).toBe('http://127.0.0.1:8787/base/v1/message');
    expect(init?.method).toBe('POST');
    expect(init?.headers).toEqual({ 'Content-Type': 'application/json', Authorization: 'Bearer s3cr3t' });
    const body = JSON.parse(String(init?.body)) as Record<string, unknown>;
    expect(body).toMatchObject({ type: 'status', noLaunch: true });
    expect(typeof body.id).toBe('string');
    expect(reply).toMatchObject({ ok: true, ping: true });
    expect(chromeMock.runtime.connectNative).not.toHaveBeenCalled();
  });

  it('maps 401 to unauthorized and reports it to the connection indicator', async () => {
    stored = { remoteUrl: 'http://127.0.0.1:8787', remoteToken: 'wrong' };
    fetchMock.mockResolvedValue({
      ok: false,
      status: 401,
      json: async () => ({ ok: false, error: { code: 'unauthorized', message: 'bad token' } }),
    } as Response);
    const c = new NativeClient();
    const seen: string[] = [];
    c.onResult((ok, err) => seen.push(ok ? 'ok' : err!.code));

    await expect(c.request({ type: 'status' })).rejects.toMatchObject({ code: 'unauthorized' });
    expect(seen).toEqual(['unauthorized']);
  });

  it('maps an aborted request to timeout', async () => {
    stored = { remoteUrl: 'http://127.0.0.1:8787', remoteToken: 's3cr3t' };
    fetchMock.mockImplementation((_url, init) => new Promise<Response>((_resolve, reject) => {
      init?.signal?.addEventListener('abort', () => reject(new Error('aborted')));
    }));

    const c = new NativeClient();
    await expect(c.request({ type: 'status' }, { timeoutMs: 10 })).rejects.toMatchObject({ code: 'timeout' });
  });

  it('keeps native messaging when no remote URL is configured', async () => {
    const reply = await new NativeClient().request({ type: 'status' });
    expect(reply).toMatchObject({ ok: true });
    expect(chromeMock.runtime.connectNative).toHaveBeenCalledTimes(1);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('normalizes remote URL and token before storing them', () => {
    expect(normalizeRemoteUrl(' http://127.0.0.1:8787/base/ ')).toBe('http://127.0.0.1:8787/base');
    expect(normalizeRemoteUrl('javascript:alert(1)')).toBe('');
    expect(normalizeRemoteUrl('http://user:pw@host')).toBe('');
    expect(normalizeRemoteToken(' s3cr3t ')).toBe('s3cr3t');
  });
});
