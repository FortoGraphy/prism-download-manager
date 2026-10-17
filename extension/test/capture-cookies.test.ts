import { describe, expect, it } from 'vitest';
import { storeCookiesToAdd } from '../src/shared/cookies';

// What cookies.getAll returns for https://bank.example/statement.pdf.
const stored = [
  { name: 'session', value: 's1', path: '/', sameSite: 'strict' },
  { name: 'prefs', value: 'p1', path: '/', sameSite: 'lax' },
];
(globalThis as unknown as { chrome: unknown }).chrome = {
  runtime: { getManifest: () => ({ version: '0.0.0' }) },
  storage: { local: { get: async () => ({}), set: async () => {} }, onChanged: { addListener: () => {} } },
  cookies: { getAll: async () => stored, getAllCookieStores: async () => [] },
};

const { cookieHeaderFor } = await import('../src/background/context');

const url = 'https://bank.example/statement.pdf';

// Reported privately by @pepopepopepo: a page on another site could start a download that Prism
// then fetched with the target's SameSite=Strict cookies, which the browser had withheld.
describe('cookies for a captured download', () => {
  it('adds no SameSite=Strict cookie the browser withheld from a cross-site request', async () => {
    // Another site started the download, so the browser sent only the Lax cookie.
    expect(await cookieHeaderFor(url, false, 'prefs=p1', { initiator: 'https://evil.example/' })).toBe('prefs=p1');
  });
  it('adds no Strict cookie when the starting page is another site or unknown', async () => {
    expect(await cookieHeaderFor(url, false, undefined, { initiator: 'https://evil.example/' })).toBe('prefs=p1');
    expect(await cookieHeaderFor(url, false, undefined, { initiator: 'http://bank.example/' })).toBe('prefs=p1');
    expect(await cookieHeaderFor(url, false, undefined, { initiator: 'https://www.bank.example.evil.example/' })).toBe('prefs=p1');
    expect(await cookieHeaderFor(url, false, undefined, { initiator: undefined })).toBe('prefs=p1');
  });
  it('keeps Strict cookies for a download the site itself started', async () => {
    expect(await cookieHeaderFor(url, false, undefined, { initiator: 'https://bank.example/account' })).toBe('session=s1; prefs=p1');
    expect(await cookieHeaderFor(url, false, 'session=s1; prefs=p1', { initiator: 'https://bank.example/account' })).toBe('session=s1; prefs=p1');
  });
  it('leaves what the user starts (menus, video button, popup) unchanged', async () => {
    expect(await cookieHeaderFor(url, false)).toBe('session=s1; prefs=p1');
  });
});

describe('storeCookiesToAdd', () => {
  it('drops only Strict cookies unless the site itself started the download', () => {
    const all = [...stored, { name: 'x', value: '1', sameSite: 'no_restriction' }, { name: 'y', value: '2', sameSite: 'unspecified' }, { name: 'z', value: '3' }];
    expect(storeCookiesToAdd(all, url, undefined, 'https://evil.example/').map((c) => c.name)).toEqual(['prefs', 'x', 'y', 'z']);
    expect(storeCookiesToAdd(all, url, undefined, 'https://bank.example/').map((c) => c.name)).toEqual(['session', 'prefs', 'x', 'y', 'z']);
    // A known Cookie header is the browser's own decision: no Strict cookie is added to it.
    expect(storeCookiesToAdd(all, url, '', 'https://bank.example/').map((c) => c.name)).toEqual(['prefs', 'x', 'y', 'z']);
    expect(storeCookiesToAdd(all, url, undefined, 'not a url').map((c) => c.name)).toEqual(['prefs', 'x', 'y', 'z']);
  });
});
