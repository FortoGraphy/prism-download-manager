/**
 * Injected into the page's own JS context (not the extension's isolated world) at
 * document_start. Some streaming embeds never expose their HLS manifest to a normal network
 * listener — served inline, or fetched a way our webRequest listener can't see — but every
 * hls.js player still calls `loadSource(url)` with it. This makes no network request of its
 * own: it only reads the URL the page's own script already fetched and parsed, the same way
 * the page's own code could read it. Fails open everywhere: a site without hls.js, or one that
 * never exposes it on `window`, is simply never hooked.
 */
interface HlsInstance {
  on(event: string, cb: () => void): void;
  once?(event: string, cb: () => void): void;
}

interface HlsCtor {
  new (...args: unknown[]): HlsInstance;
  prototype: HlsInstance & { loadSource(this: HlsInstance, url: string): void };
  Events: { MANIFEST_PARSED: string };
  __prismWrapped?: boolean;
}

function post(manifestUrl: string): void {
  let absolute: string;
  try {
    absolute = new URL(manifestUrl, document.baseURI).href;
  } catch {
    return;
  }
  if (!/^https?:$/.test(new URL(absolute).protocol)) return;
  window.postMessage({ source: 'prism-player-hook', manifestUrl: absolute }, window.location.origin);
}

function wrap(Hls: HlsCtor | undefined): HlsCtor | undefined {
  if (!Hls?.prototype?.loadSource || Hls.__prismWrapped) return Hls;
  const original = Hls.prototype.loadSource;
  Hls.prototype.loadSource = function (this: HlsInstance, url: string) {
    try {
      const report = () => post(url);
      if (this.once) this.once(Hls.Events.MANIFEST_PARSED, report);
      else this.on(Hls.Events.MANIFEST_PARSED, report);
    } catch {
      /* best effort — never break the page's own playback */
    }
    return original.call(this, url);
  };
  Hls.__prismWrapped = true;
  return Hls;
}

(function hookHls() {
  const w = window as unknown as { Hls?: HlsCtor };
  let current = wrap(w.Hls);
  try {
    Object.defineProperty(w, 'Hls', {
      configurable: true,
      get: () => current,
      set: (v: HlsCtor) => {
        current = wrap(v);
      },
    });
  } catch {
    /* the page made window.Hls non-configurable first — nothing more we can do */
  }
})();
