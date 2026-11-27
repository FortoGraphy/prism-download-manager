import { beforeEach, describe, expect, it, vi } from 'vitest';

const setBadgeText = vi.fn(async (_p: object) => {});
const setBadgeBackgroundColor = vi.fn(async (_p: object) => {});
const setTitle = vi.fn(async (_p: object) => {});
(globalThis as unknown as { chrome: unknown }).chrome = { action: { setBadgeText, setBadgeBackgroundColor, setTitle } };

const { reflectConnection } = await import('../src/background/badge');

beforeEach(() => vi.clearAllMocks());

describe('global badge', () => {
  it('flags a failed server exchange as unauthorized', () => {
    reflectConnection({ state: 'app_down', errorCode: 'unauthorized' });
    expect(setBadgeText).toHaveBeenCalledWith({ text: '!' });
    expect(setTitle).toHaveBeenCalledWith({ title: 'Prism — check Prism web token in Options' });
  });

  it('uses a different title when the server or app is unreachable', () => {
    reflectConnection({ state: 'app_down', errorCode: 'app_unreachable' });
    expect(setTitle).toHaveBeenCalledWith({ title: 'Prism — server or app not reachable' });
  });

  it('clears the badge once connected', () => {
    reflectConnection({ state: 'connected' });
    expect(setBadgeText).toHaveBeenCalledWith({ text: '' });
    expect(setTitle).toHaveBeenCalledWith({ title: 'Prism' });
  });
});
