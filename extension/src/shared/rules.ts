import type { CaptureSettings } from './protocol';

/** Only these schemes can be handed to the app (blob:, data:, filesystem: stay in the browser). */
export function isHandoffUrl(url: string | undefined): boolean {
  return !!url && /^(https?|ftp):\/\//i.test(url);
}

function safeUrl(u: string | undefined): URL | null {
  if (!u) return null;
  try {
    return new URL(u);
  } catch {
    return null;
  }
}

/** Last path segment of a URL, percent-decoded. */
export function filenameFromUrl(url: string): string {
  const u = safeUrl(url);
  const path = u ? u.pathname : url.split(/[?#]/)[0] ?? '';
  const last = path.split('/').filter(Boolean).pop() ?? '';
  try {
    return decodeURIComponent(last);
  } catch {
    return last;
  }
}

/** Upper-case extension without the dot (`archive.tar.gz` → `GZ`), '' if none. Accepts names, paths or URLs. */
export function fileExtension(nameOrUrl: string | undefined): string {
  if (!nameOrUrl) return '';
  let name = nameOrUrl;
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(name)) name = filenameFromUrl(name);
  name = name.split(/[\\/]/).pop() ?? '';
  const dot = name.lastIndexOf('.');
  if (dot <= 0 || dot === name.length - 1) return '';
  const ext = name.slice(dot + 1);
  return /^[a-z0-9]{1,10}$/i.test(ext) ? ext.toUpperCase() : '';
}

/** Glob (`*`, `?`) → anchored, case-insensitive RegExp. */
export function globToRegExp(glob: string): RegExp {
  const re = glob.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*').replace(/\?/g, '.');
  return new RegExp(`^${re}$`, 'i');
}

export function matchesExtension(ext: string, patterns: string[]): boolean {
  if (!ext) return false;
  return patterns.some((p) => (/[*?]/.test(p) ? globToRegExp(p).test(ext) : p.toUpperCase() === ext));
}

/** Entries ending in `/`, `-` or `*` are prefixes; others match exactly (parameters ignored). */
export function matchesMime(mime: string | undefined, list: string[]): boolean {
  if (!mime) return false;
  const m = mime.split(';')[0]!.trim().toLowerCase();
  if (!m) return false;
  return list.some((entry) => {
    const e = entry.toLowerCase();
    if (e.endsWith('*')) return m.startsWith(e.slice(0, -1));
    if (e.endsWith('/') || e.endsWith('-')) return m.startsWith(e);
    return m === e;
  });
}

/**
 * Site pattern matching.
 * - `example.com` → that host and its subdomains
 * - `*.example.com` → subdomains only (and the bare domain)
 * - anything with `://` or `/` → glob against the full URL (`https://example.com/files/*`)
 */
export function siteMatches(url: string, pattern: string): boolean {
  const p = pattern.trim().toLowerCase();
  if (!p) return false;
  if (p.includes('://') || p.includes('/')) {
    const glob = p.includes('://') ? p : `*://${p}`;
    return globToRegExp(glob.endsWith('*') ? glob : `${glob}*`).test(url);
  }
  const u = safeUrl(url);
  if (!u) return false;
  const host = u.hostname.toLowerCase();
  const bare = p.replace(/^\*\./, '');
  if (/[*?]/.test(bare)) return globToRegExp(bare).test(host);
  return host === bare || host.endsWith(`.${bare}`);
}

export function isExcluded(urls: Array<string | undefined>, patterns: string[]): boolean {
  if (!patterns.length) return false;
  return urls.some((u) => !!u && patterns.some((p) => siteMatches(u, p)));
}

export interface CaptureCandidate {
  url: string;
  finalUrl?: string;
  filename?: string;
  mime?: string;
  /** Bytes; <= 0 or undefined = unknown. */
  fileSize?: number;
  pageUrl?: string;
  referrer?: string;
  /** HTTP method of the request when known. */
  method?: string;
}

export interface CaptureContext {
  paused: boolean;
  bypass: boolean;
}

export interface CaptureDecision {
  capture: boolean;
  reason: string;
}

/** MIME types servers use for "some file", whatever it is. */
const GENERIC_MIMES = new Set([
  'application/octet-stream',
  'binary/octet-stream',
  'application/binary',
  'application/force-download',
  'application/x-download',
  'application/download',
  'application/unknown',
]);

/** Extensions of scripts that serve files (`get.php`): not the type of the file they send. */
const SCRIPT_EXTS = new Set(['PHP', 'ASP', 'ASPX', 'JSP', 'CGI', 'PL', 'DO', 'ACTION', 'CFM', 'PY']);

/** Pure capture decision for a browser download. */
export function decideCapture(c: CaptureCandidate, s: CaptureSettings, ctx: CaptureContext): CaptureDecision {
  const no = (reason: string): CaptureDecision => ({ capture: false, reason });
  if (!s.captureEnabled) return no('capture disabled');
  if (ctx.paused) return no('capture paused');
  if (ctx.bypass) return no('bypass key held');
  const target = c.finalUrl || c.url;
  if (!isHandoffUrl(c.url) || !isHandoffUrl(target)) return no('unsupported URL scheme');
  if (c.method && c.method.toUpperCase() !== 'GET') return no(`${c.method.toUpperCase()} request`);
  if (isExcluded([c.pageUrl, c.referrer, c.url, c.finalUrl], s.excludedSites)) return no('excluded site');

  const ext = fileExtension(c.filename) || fileExtension(c.finalUrl) || fileExtension(c.url);
  const byExt = matchesExtension(ext, s.extensions);
  // A generic type (application/octet-stream…) says nothing about the file: it decides only
  // for a name without a real extension. A .blend that a server sends as octet-stream is not
  // in the list, so the browser keeps it (#137); "get.php?id=1" or "/download" still count.
  const mime = (c.mime ?? '').split(';')[0]!.trim().toLowerCase();
  const generic = GENERIC_MIMES.has(mime) && !!ext && !SCRIPT_EXTS.has(ext);
  const byMime = !generic && matchesMime(c.mime, s.mimeTypes);
  if (!byExt && !byMime) return no(`type not in capture list (${ext || '-'} / ${c.mime || '-'})`);
  if (s.minSize > 0 && c.fileSize !== undefined && c.fileSize > 0 && c.fileSize < s.minSize) {
    return no(`smaller than ${s.minSize} bytes`);
  }
  return { capture: true, reason: byExt ? `extension ${ext}` : `mime ${c.mime}` };
}

/** Filename from a Content-Disposition header (RFC 6266, incl. filename*=UTF-8''...). */
export function filenameFromContentDisposition(header: string | undefined): string | undefined {
  if (!header) return undefined;
  const star = /filename\*\s*=\s*([^;]+)/i.exec(header);
  if (star) {
    const v = star[1]!.trim().replace(/^"(.*)"$/, '$1');
    const m = /^([\w-]+)'[^']*'(.*)$/.exec(v);
    try {
      return decodeURIComponent(m ? m[2]! : v);
    } catch {
      /* fall through */
    }
  }
  const plain = /filename\s*=\s*("((?:[^"\\]|\\.)*)"|[^;]+)/i.exec(header);
  if (plain) return (plain[2] ?? plain[1]!).replace(/\\(.)/g, '$1').trim();
  return undefined;
}

export function isAttachment(header: string | undefined): boolean {
  return !!header && /^\s*attachment/i.test(header);
}
