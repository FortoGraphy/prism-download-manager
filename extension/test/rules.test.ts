import { describe, expect, it } from 'vitest';
import {
  decideCapture,
  fileExtension,
  filenameFromContentDisposition,
  filenameFromUrl,
  isAttachment,
  isHandoffUrl,
  matchesExtension,
  matchesMime,
  siteMatches,
} from '../src/shared/rules';
import { DEFAULT_SETTINGS, normalizeSettings, normalizeState } from '../src/shared/settings';

const S = DEFAULT_SETTINGS;
const ctx = { paused: false, bypass: false };

describe('file names and extensions', () => {
  it('extracts extensions from names, paths and URLs', () => {
    expect(fileExtension('setup.EXE')).toBe('EXE');
    expect(fileExtension('archive.tar.gz')).toBe('GZ');
    expect(fileExtension('C:\\Users\\me\\Downloads\\a.zip')).toBe('ZIP');
    expect(fileExtension('https://x.com/dl/file.iso?token=1#frag')).toBe('ISO');
    expect(fileExtension('https://x.com/download?id=5')).toBe('');
    expect(fileExtension('.bashrc')).toBe('');
    expect(fileExtension('noext')).toBe('');
    expect(fileExtension(undefined)).toBe('');
  });
  it('decodes URL file names', () => {
    expect(filenameFromUrl('https://x.com/a/My%20File.zip?x=1')).toBe('My File.zip');
    expect(filenameFromUrl('https://x.com/')).toBe('');
  });
  it('matches extension wildcards', () => {
    expect(matchesExtension('R01', S.extensions)).toBe(true);
    expect(matchesExtension('R15', S.extensions)).toBe(true);
    expect(matchesExtension('R2', S.extensions)).toBe(false);
    expect(matchesExtension('APPIMAGE', S.extensions)).toBe(true);
    expect(matchesExtension('HTML', S.extensions)).toBe(false);
    expect(matchesExtension('', S.extensions)).toBe(false);
  });
  it('parses Content-Disposition', () => {
    expect(filenameFromContentDisposition('attachment; filename="a b.zip"')).toBe('a b.zip');
    expect(filenameFromContentDisposition("attachment; filename*=UTF-8''%C5%BElu%C5%A5ou%C4%8Dk%C3%BD.zip")).toBe('žluťoučký.zip');
    expect(filenameFromContentDisposition('attachment; filename=plain.iso')).toBe('plain.iso');
    expect(filenameFromContentDisposition('inline')).toBeUndefined();
    expect(isAttachment('Attachment; filename=x')).toBe(true);
    expect(isAttachment('inline; filename=x')).toBe(false);
  });
});

describe('MIME matching', () => {
  it('handles prefixes and parameters', () => {
    expect(matchesMime('video/mp4', S.mimeTypes)).toBe(true);
    expect(matchesMime('audio/mpeg; charset=binary', S.mimeTypes)).toBe(true);
    expect(matchesMime('application/zip', S.mimeTypes)).toBe(true);
    expect(matchesMime('text/html', S.mimeTypes)).toBe(false);
    expect(matchesMime('application/x-foo', ['application/x-'])).toBe(true);
    expect(matchesMime('application/xml', ['application/x-'])).toBe(false);
    expect(matchesMime('image/png', ['image/*'])).toBe(true);
    expect(matchesMime(undefined, S.mimeTypes)).toBe(false);
  });
});

describe('site patterns', () => {
  it('matches hosts, subdomains, wildcards and URL globs', () => {
    expect(siteMatches('https://example.com/a', 'example.com')).toBe(true);
    expect(siteMatches('https://cdn.example.com/a', 'example.com')).toBe(true);
    expect(siteMatches('https://notexample.com/a', 'example.com')).toBe(false);
    expect(siteMatches('https://a.example.com/', '*.example.com')).toBe(true);
    expect(siteMatches('https://example.com/files/x.zip', 'https://example.com/files/*')).toBe(true);
    expect(siteMatches('https://example.com/other/x.zip', 'https://example.com/files/*')).toBe(false);
    expect(siteMatches('http://example.com/files/x.zip', 'example.com/files/')).toBe(true);
    expect(siteMatches('https://mirror3.dl.net/', 'mirror?.dl.net')).toBe(true);
  });
});

describe('decideCapture', () => {
  const base = { url: 'https://dl.example.com/file.zip', filename: 'file.zip', mime: 'application/zip', fileSize: 5_000_000 };

  it('captures listed types', () => {
    expect(decideCapture(base, S, ctx).capture).toBe(true);
    expect(decideCapture({ url: 'https://x.com/get?id=1', mime: 'video/mp4' }, S, ctx).capture).toBe(true);
    expect(decideCapture({ url: 'https://x.com/get?id=1', filename: 'a.r01' }, S, ctx).capture).toBe(true);
  });
  it('lets a generic MIME type decide only for a name without an extension (#137)', () => {
    const octet = 'application/octet-stream';
    // A Discord attachment of a type that is not in the list stays in the browser.
    expect(decideCapture({ url: 'https://cdn.discordapp.com/attachments/1/2/scene.blend', filename: 'scene.blend', mime: octet }, S, ctx).capture).toBe(false);
    expect(decideCapture({ url: 'https://x.com/file', filename: 'notes.psd', mime: 'binary/octet-stream; charset=x' }, S, ctx).capture).toBe(false);
    // A listed type is captured whatever the server calls it.
    expect(decideCapture({ url: 'https://x.com/a.zip', filename: 'a.zip', mime: octet }, S, ctx).capture).toBe(true);
    // No extension, or a script that serves the file: the type is all there is.
    expect(decideCapture({ url: 'https://x.com/download?id=1', mime: octet }, S, ctx).capture).toBe(true);
    expect(decideCapture({ url: 'https://x.com/get.php?id=1', mime: octet }, S, ctx).capture).toBe(true);
    // A specific MIME type in the list still decides on its own.
    expect(decideCapture({ url: 'https://x.com/a.weird', filename: 'a.weird', mime: 'application/zip' }, S, ctx).capture).toBe(true);
  });
  it('leaves other types, blob/data URLs and POSTs to the browser', () => {
    expect(decideCapture({ url: 'https://x.com/a.html', mime: 'text/html' }, S, ctx).capture).toBe(false);
    expect(decideCapture({ ...base, url: 'blob:https://x.com/123' }, S, ctx).capture).toBe(false);
    expect(decideCapture({ ...base, url: 'data:application/zip;base64,AAAA' }, S, ctx).capture).toBe(false);
    expect(decideCapture({ ...base, method: 'POST' }, S, ctx).reason).toMatch(/POST/);
    expect(isHandoffUrl('ftp://x/y.zip')).toBe(true);
  });
  it('honours pause, bypass, disable, exclusions and minimum size', () => {
    expect(decideCapture(base, S, { paused: true, bypass: false }).capture).toBe(false);
    expect(decideCapture(base, S, { paused: false, bypass: true }).capture).toBe(false);
    expect(decideCapture(base, { ...S, captureEnabled: false }, ctx).capture).toBe(false);
    expect(decideCapture({ ...base, pageUrl: 'https://intranet.corp/x' }, { ...S, excludedSites: ['intranet.corp'] }, ctx).capture).toBe(false);
    expect(decideCapture(base, { ...S, excludedSites: ['example.com'] }, ctx).capture).toBe(false);
    expect(decideCapture({ ...base, fileSize: 1000 }, { ...S, minSize: 1 << 20 }, ctx).capture).toBe(false);
    // unknown size is captured even with a minimum
    expect(decideCapture({ ...base, fileSize: undefined }, { ...S, minSize: 1 << 20 }, ctx).capture).toBe(true);
  });
  it('uses the final URL extension after redirects', () => {
    expect(decideCapture({ url: 'https://x.com/dl?id=9', finalUrl: 'https://cdn.x.com/f/setup.msi' }, S, ctx).capture).toBe(true);
  });
});

describe('settings normalization', () => {
  it('fills defaults and cleans values', () => {
    const s = normalizeSettings({ extensions: '.zip, *.iso rar zip', minSize: -5, bypassKey: 'hyper', mimeTypes: ['Video/'] });
    expect(s.extensions).toEqual(['ZIP', 'ISO', 'RAR']);
    expect(s.minSize).toBe(DEFAULT_SETTINGS.minSize);
    expect(s.bypassKey).toBe('alt');
    expect(s.mimeTypes).toEqual(['video/']);
    expect(s.showVideoButton).toBe(true);
    expect(normalizeState(null).settingsSource).toBe('app');
    expect(normalizeState({ settingsSource: 'local', paused: true }).paused).toBe(true);
  });
});
