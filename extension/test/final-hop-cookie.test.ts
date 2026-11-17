import { describe, expect, it, vi } from 'vitest';

// The cookie store per host, as cookies.getAll returns it.
const store: Record<string, { name: string; value: string; path: string; sameSite: string }[]> = {
  'a.example': [{ name: 'a_session', value: 'A', path: '/', sameSite: 'lax' }],
  'b.example': [{ name: 'b_prefs', value: 'B', path: '/', sameSite: 'lax' }],
};
(globalThis as unknown as { chrome: unknown }).chrome = {
  runtime: { getManifest: () => ({ version: '0.0.0' }), id: 'prism' },
  storage: { local: { get: async () => ({}), set: async () => {} }, onChanged: { addListener: () => {} } },
  cookies: {
    getAll: async ({ url }: { url: string }) => store[new URL(url).hostname] ?? [],
    getAllCookieStores: async () => [],
  },
};

const sent: Record<string, unknown>[] = [];
vi.mock('../src/background/native', () => ({
  native: {
    request: async (msg: Record<string, unknown>) => {
      sent.push(msg);
      return { accepted: true };
    },
  },
}));
vi.mock('../src/background/state', () => ({
  getState: async () => ({ settings: {}, paused: false }),
  getConnection: () => ({ state: 'connected' }),
}));
vi.mock('../src/background/badge', () => ({ showUnreachable: () => {} }));
vi.mock('../src/shared/rules', async (orig) => ({
  ...(await orig<typeof import('../src/shared/rules')>()),
  decideCapture: () => ({ capture: true }),
}));

const { maybeHandOff } = await import('../src/background/capture');

describe('a captured download redirected to another host', () => {
  it("sends the final host none of the first host's Cookie header", async () => {
    // The request map held only the first hop (requestByUrl(finalUrl) missed), so req is a.example's.
    const req = {
      requestId: '1',
      url: 'https://a.example/get',
      method: 'GET',
      tabId: 1,
      frameId: 0,
      type: 'main_frame',
      cookie: 'a_session=A',
      ts: Date.now(),
    };
    await maybeHandOff({
      candidate: { url: 'https://a.example/get', finalUrl: 'https://b.example/file.zip' },
      req,
      incognito: false,
    });
    const msg = sent.pop()!;
    expect(msg.cookies).toBe('a_session=A');
    expect(String(msg.finalUrlCookies)).not.toContain('a_session');
  });
});
