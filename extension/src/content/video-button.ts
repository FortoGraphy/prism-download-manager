/**
 * Floating "Download" pill shown over videos on hover (Shadow DOM isolated).
 * Clicking it lists qualities: app-resolved formats (probeVideo) on known
 * video sites, sniffed streams elsewhere.
 */
import { sendInternal } from '../shared/api';
import { t, tk, setLanguage } from '../shared/i18n';
import { icons, prismMark } from '../shared/icons';
import { type ElementSource, elementSources, linkedinCommentaryPath, linkedinPostPath, postUrl } from '../shared/media';
import { siteMatches } from '../shared/rules';
import type { ActionResult, VideoOption, VideoOptionsResult } from '../shared/messages';
import { FullscreenState } from './fullscreen';
import { VIDEO_BUTTON_CSS } from './styles';

setLanguage();

const MIN_W = 200;
const MIN_H = 120;
const HIDE_DELAY = 900;
const IS_YOUTUBE = /(^|\.)youtube(-nocookie)?\.com$/.test(location.hostname);

function esc(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);
}

/** The element whose box the pill attaches to (YouTube: the player, not the raw <video>). */
function targetOf(v: HTMLVideoElement): Element | null {
  if (IS_YOUTUBE) {
    // Only the main/Shorts players — not the muted hover previews on feeds.
    return v.closest('#movie_player, #shorts-player, ytd-reel-video-renderer .html5-video-player') ?? null;
  }
  return v;
}

function eligible(v: HTMLVideoElement, r: DOMRect): boolean {
  if (r.width < MIN_W || r.height < MIN_H) return false;
  // Small muted autoplay loops without controls are decorative/ads.
  if (v.muted && v.autoplay && v.loop && !v.controls && (r.width < 400 || r.height < 225)) return false;
  // A video the page hides (display, visibility or opacity) but keeps in its layout, like the
  // players Telegram Web keeps behind its chat (#66). Browsers without checkVisibility skip this.
  const visible = (v as { checkVisibility?: (o: object) => boolean }).checkVisibility;
  if (visible && !visible.call(v, { opacityProperty: true, visibilityProperty: true })) return false;
  return true;
}

/** `closest()` that also climbs out of (open) shadow roots. */
function composedClosest(el: Element, selector: string): Element | null {
  for (let n: Element | null = el; n; ) {
    const hit = n.closest(selector);
    if (hit) return hit;
    const root = n.getRootNode();
    n = root instanceof ShadowRoot ? root.host : null;
  }
  return null;
}

/**
 * On a feed, profile or listing (X, Instagram, Facebook, Reddit, TikTok…) the post the video
 * belongs to: the first single-post link of its <article>, so the app reads that post and not the
 * page. Undefined when the page itself is the post, or the video isn't inside a post.
 */
function postUrlOf(v: HTMLVideoElement, pageUrl: string): string | undefined {
  if (postUrl(pageUrl, pageUrl)) return undefined;
  // A grid tile that links to its post (Instagram Explore, profile grids).
  const tile = postUrl((composedClosest(v, 'a[href]') as HTMLAnchorElement | null)?.href, pageUrl);
  if (tile) return tile;
  // LinkedIn: feed posts carry their id in attributes (data-urn, data-id, composite ids), not
  // a link to themselves. The nearest container with one is the post (for a reshare: the
  // original); stop before a container that holds another video (that's the feed around it).
  if (/(^|\.)linkedin\.com$/.test(location.hostname)) {
    for (let n: Element | null = v, i = 0; n && i < 30; i++) {
      if (n !== v && n.querySelectorAll('video').length > 1) break;
      const path = linkedinPostPath([...n.attributes].map((a) => a.value));
      if (path) return postUrl(path, pageUrl);
      const root = n.getRootNode();
      n = n.parentElement ?? (root instanceof ShadowRoot ? root.host : null);
    }
    const path = linkedinCardPost(v);
    if (path) return postUrl(path, pageUrl);
  }
  const post = composedClosest(v, 'article, [role="article"], shreddit-post');
  if (!post) return undefined;
  const links = [post.getAttribute('permalink'), ...[...post.querySelectorAll<HTMLAnchorElement>('a[href]')].map((a) => a.href)];
  for (const href of links) {
    const u = postUrl(href, pageUrl);
    if (u) return u;
  }
  return undefined;
}

/**
 * LinkedIn's newer UI: no link or id around the video; the post's text block next to it (not
 * above it) carries the id in its `componentkey`. Only within the post's card (`role=listitem`):
 * the list around it holds the neighbouring posts. Several ids in the card (a repost with its
 * own text): the one closest to the video. Without a card, the nearest container with exactly
 * one id, and not past one that holds another video.
 */
function linkedinCardPost(v: HTMLVideoElement): string | undefined {
  const keyed = (scope: Element) =>
    [...scope.querySelectorAll('[componentkey*="Urn("]')]
      .map((el) => ({ el, path: linkedinCommentaryPath(el.getAttribute('componentkey')) }))
      .filter((k): k is { el: Element; path: string } => !!k.path);
  const card = composedClosest(v, '[role="listitem"]');
  if (card) {
    const keys = keyed(card);
    if (keys.length < 2) return keys[0]?.path;
    const up: Element[] = [];
    for (let n: Element | null = v; n && n !== card.parentElement; n = n.parentElement) up.push(n);
    const depth = (el: Element) => {
      for (let n: Element | null = el; n; n = n.parentElement) {
        const i = up.indexOf(n);
        if (i >= 0) return i;
      }
      return up.length;
    };
    return keys.reduce((best, k) => (depth(k.el) < depth(best.el) ? k : best)).path;
  }
  for (let n = v.parentElement, i = 0; n && i < 30; n = n.parentElement, i++) {
    if (n.querySelectorAll('video').length > 1) return undefined;
    const keys = keyed(n);
    if (keys.length) return keys.length === 1 ? keys[0]?.path : undefined;
  }
  return undefined;
}

function inside(r: DOMRect, x: number, y: number, pad = 0): boolean {
  return x >= r.left - pad && x <= r.right + pad && y >= r.top - pad && y <= r.bottom + pad;
}

function videoIdentity(v: HTMLVideoElement): string | undefined {
  const page = location.href;
  const post = postUrlOf(v, page) ?? postUrl(page, page);
  if (post) return `post:${post}`;
  const source = v.getAttribute('src') ? v.src : v.currentSrc;
  return source ? `source:${source}` : undefined;
}

export class VideoButton {
  private host: HTMLDivElement | null = null;
  private wrap!: HTMLDivElement;
  private controls!: HTMLDivElement;
  private pill!: HTMLButtonElement;
  private menu!: HTMLDivElement;
  private videos = new Set<HTMLVideoElement>();
  /** Videos currently intersecting the viewport (only these are hit-tested). */
  private visible = new Set<HTMLVideoElement>();
  private io: IntersectionObserver | null = null;
  private current: HTMLVideoElement | null = null;
  private currentIdentity: string | undefined;
  private readonly dismissedVideos = new Set<string>();
  private readonly dismissedPlayers = new WeakSet<HTMLVideoElement>();
  private readonly fullscreen = new FullscreenState(() => {
    if (this.fullscreen.active) this.hide(true);
  });
  private hideTimer: ReturnType<typeof setTimeout> | undefined;
  private rafPending = false;
  private lastX = -1;
  private lastY = -1;
  private overUi = false;
  private menuOpen = false;
  private menuSession = 0;
  private contextOnly = false;
  private enabled = false;
  private videoSites: string[] = [];
  private prefetchTimer: ReturnType<typeof setTimeout> | undefined;
  private restPrefetch: Promise<void> | undefined;
  private observer: MutationObserver | null = null;
  private cache = new Map<string, { at: number; res: Promise<VideoOptionsResult | undefined> }>();
  private scanQueued = false;
  /** The video under the last right-click (for the context menu's "Download with Prism"). */
  private contextVideo: { v: HTMLVideoElement; at: number } | null = null;

  setVideoSites(sites: string[]): void {
    this.videoSites = sites;
  }

  setEnabled(on: boolean): void {
    if (on === this.enabled) return;
    this.enabled = on;
    if (on) this.start();
    else this.stop();
  }

  private start(): void {
    this.io = new IntersectionObserver((entries) => {
      for (const e of entries) {
        const v = e.target as HTMLVideoElement;
        if (e.isIntersecting) this.visible.add(v);
        else this.visible.delete(v);
      }
    });
    this.scan(document);
    this.observer = new MutationObserver((records) => {
      for (const r of records) {
        for (const n of r.addedNodes) {
          if (n.nodeType === 1 && ((n as Element).tagName === 'VIDEO' || (n as Element).firstElementChild)) {
            this.queueScan();
            return;
          }
        }
      }
    });
    this.observer.observe(document.documentElement, { childList: true, subtree: true });
    window.addEventListener('pointermove', this.onMove, { capture: true, passive: true });
    window.addEventListener('scroll', this.onScroll, { capture: true, passive: true });
    window.addEventListener('resize', this.onScroll, { passive: true });
    this.fullscreen.start();
    document.addEventListener('pointerdown', this.onDocDown, true);
    document.addEventListener('keydown', this.onKey, true);
    document.addEventListener('contextmenu', this.onContextMenu, true);
    // SPA navigation (YouTube fires yt-navigate-finish; others: history/navigation API).
    document.addEventListener('yt-navigate-finish', this.onNavigate);
    window.addEventListener('popstate', this.onNavigate);
    const nav = (window as unknown as { navigation?: EventTarget }).navigation;
    nav?.addEventListener('navigatesuccess', this.onNavigate);
  }

  private stop(): void {
    this.observer?.disconnect();
    this.observer = null;
    this.io?.disconnect();
    this.io = null;
    this.visible.clear();
    window.removeEventListener('pointermove', this.onMove, true);
    window.removeEventListener('scroll', this.onScroll, true);
    window.removeEventListener('resize', this.onScroll);
    this.fullscreen.stop();
    document.removeEventListener('pointerdown', this.onDocDown, true);
    document.removeEventListener('keydown', this.onKey, true);
    document.removeEventListener('contextmenu', this.onContextMenu, true);
    document.removeEventListener('yt-navigate-finish', this.onNavigate);
    window.removeEventListener('popstate', this.onNavigate);
    const nav = (window as unknown as { navigation?: EventTarget }).navigation;
    nav?.removeEventListener('navigatesuccess', this.onNavigate);
    this.hide(true);
    this.host?.remove();
    this.host = null;
    this.videos.clear();
  }

  private queueScan(): void {
    if (this.scanQueued) return;
    this.scanQueued = true;
    requestAnimationFrame(() => {
      this.scanQueued = false;
      if (this.enabled) this.scan(document);
    });
  }

  private scan(root: ParentNode): void {
    root.querySelectorAll('video').forEach((v) => {
      if (this.videos.has(v)) return;
      this.videos.add(v);
      this.io?.observe(v);
    });
    for (const v of this.videos) {
      if (v.isConnected) continue;
      this.videos.delete(v);
      this.visible.delete(v);
      this.io?.unobserve(v);
    }
  }

  // ---- DOM -----------------------------------------------------------------

  private ensureHost(): void {
    if (this.host) return;
    const host = document.createElement('div');
    host.setAttribute('data-prism-video-button', '');
    const st = host.style;
    st.setProperty('all', 'initial', 'important');
    st.setProperty('position', 'fixed', 'important');
    st.setProperty('top', '0', 'important');
    st.setProperty('left', '0', 'important');
    st.setProperty('width', '0', 'important');
    st.setProperty('height', '0', 'important');
    st.setProperty('z-index', '2147483647', 'important');
    st.setProperty('pointer-events', 'none', 'important');
    const root = host.attachShadow({ mode: 'open' });
    try {
      const sheet = new CSSStyleSheet();
      sheet.replaceSync(VIDEO_BUTTON_CSS);
      root.adoptedStyleSheets = [sheet];
    } catch {
      const style = document.createElement('style');
      style.textContent = VIDEO_BUTTON_CSS;
      root.append(style);
    }
    this.wrap = document.createElement('div');
    this.wrap.className = 'wrap';
    this.wrap.inert = true;
    this.wrap.innerHTML = `
      <div class="controls">
        <button class="pill glass" type="button" aria-haspopup="menu" aria-expanded="false" title="${esc(t('Download this video with Prism'))}">
          ${prismMark(18)}<span class="label">${esc(t('Download'))}</span><span class="chev">${icons.chevronDown(14)}</span>
        </button>
        <button class="dismiss glass" type="button" aria-label="${esc(t('Hide Prism for this video'))}" title="${esc(t('Hide Prism for this video until the page is reloaded'))}">
          ${icons.close(16)}
        </button>
      </div>
      <div class="menu glass" role="menu"></div>`;
    root.append(this.wrap);
    this.controls = this.wrap.querySelector('.controls')!;
    this.pill = this.wrap.querySelector('.pill')!;
    this.menu = this.wrap.querySelector('.menu')!;
    // Only the user's own clicks (isTrusted): the shadow root is open, and a page script must not
    // open the menu or start a download with the page's cookies through synthetic events.
    this.pill.addEventListener('click', (e) => {
      e.preventDefault();
      e.stopPropagation();
      if (!e.isTrusted) return;
      void this.toggleMenu();
    });
    this.pill.addEventListener('pointerenter', (e) => {
      if (e.isTrusted) this.prefetch();
    });
    this.wrap.querySelector('.dismiss')!.addEventListener('click', (e) => {
      e.preventDefault();
      e.stopPropagation();
      if (!e.isTrusted || !this.current) return;
      if (this.currentIdentity) this.dismissedVideos.add(this.currentIdentity);
      else this.dismissedPlayers.add(this.current);
      this.hide(true);
    });
    for (const el of [this.controls, this.menu]) {
      el.addEventListener('pointerenter', () => {
        this.overUi = true;
        clearTimeout(this.hideTimer);
        this.hideTimer = undefined;
      });
      el.addEventListener('pointerleave', () => {
        this.overUi = false;
        this.scheduleHide();
      });
    }
    this.host = host;
    document.documentElement.append(host);
  }

  // ---- show / hide / position -------------------------------------------------

  private onMove = (e: PointerEvent): void => {
    if (e.pointerType === 'touch') return;
    this.lastX = e.clientX;
    this.lastY = e.clientY;
    if (this.rafPending) return;
    this.rafPending = true;
    requestAnimationFrame(() => {
      this.rafPending = false;
      this.hitTest();
    });
  };

  private onScroll = (): void => {
    if (this.current) requestAnimationFrame(() => this.position());
  };

  private hitTest(): void {
    if (!this.enabled || this.fullscreen.active || this.menuOpen || this.overUi) return;
    const x = this.lastX;
    const y = this.lastY;
    let found: HTMLVideoElement | null = null;
    let identity: string | undefined;
    // Prefer the current video while the pointer is still on it.
    const ordered = this.current ? [this.current, ...this.visible] : this.visible;
    for (const v of ordered) {
      if (!v.isConnected) continue;
      const t = targetOf(v);
      if (!t) continue;
      const r = t.getBoundingClientRect();
      if (!inside(r, x, y)) continue;
      if (!eligible(v, v.getBoundingClientRect())) continue;
      const key = videoIdentity(v);
      if (this.isDismissed(v, key)) continue;
      found = v;
      identity = key;
      break;
    }
    if (found) {
      clearTimeout(this.hideTimer);
      this.hideTimer = undefined;
      if (found !== this.current || identity !== this.currentIdentity) this.show(found, identity);
    } else if (this.current) {
      this.scheduleHide();
    }
  }

  private isDismissed(v: HTMLVideoElement, key: string | undefined): boolean {
    return key ? this.dismissedVideos.has(key) : this.dismissedPlayers.has(v);
  }

  private show(v: HTMLVideoElement, identity: string | undefined): void {
    this.ensureHost();
    this.current = v;
    this.currentIdentity = identity;
    // Start reading the qualities while the pointer rests on the video, so the menu is ready
    // (or nearly) when it opens. A short wait skips videos merely crossed on the way; one at a
    // time, as scrolling a feed under a resting pointer would start one per video.
    clearTimeout(this.prefetchTimer);
    this.prefetchTimer = setTimeout(() => {
      if (this.current !== v || this.currentIdentity !== identity || this.contextOnly || this.restPrefetch) return;
      const done = () => {
        this.restPrefetch = undefined;
      };
      this.restPrefetch = this.options().then(done, done);
    }, 400);
    this.resetPill();
    this.position();
  }

  private scheduleHide(): void {
    if (!this.current || this.hideTimer || this.menuOpen) return;
    this.hideTimer = setTimeout(() => {
      this.hideTimer = undefined;
      if (!this.overUi && !this.menuOpen) this.hide();
    }, HIDE_DELAY);
  }

  private hide(force = false): void {
    if (!this.host) return;
    if (!force && (this.menuOpen || this.overUi)) return;
    clearTimeout(this.prefetchTimer);
    clearTimeout(this.hideTimer);
    this.hideTimer = undefined;
    this.resetMenu();
    this.wrap.classList.remove('show');
    this.wrap.inert = true;
    this.current = null;
    this.currentIdentity = undefined;
    this.contextOnly = false;
    this.overUi = false;
  }

  private position(): void {
    const v = this.current;
    if (!v || !this.host) return;
    const t = targetOf(v);
    if (this.fullscreen.active || !v.isConnected || !t || videoIdentity(v) !== this.currentIdentity) return this.hide(true);
    const r = t.getBoundingClientRect();
    const vw = document.documentElement.clientWidth || innerWidth;
    const vh = document.documentElement.clientHeight || innerHeight;
    if (r.bottom < 40 || r.top > vh - 40 || r.right < 60 || r.left > vw - 60) {
      return this.hide(true);
    }
    const pw = this.controls.offsetWidth;
    const ph = this.controls.offsetHeight;
    const x = Math.max(8, Math.min(r.right, vw) - pw - 10);
    // Above the player's top-right edge when there is room, else just inside it.
    const y = r.top >= ph + 14 ? r.top - ph - 6 : Math.max(r.top, 0) + 10;
    this.wrap.style.setProperty('--x', `${x}px`);
    this.wrap.style.setProperty('--y', `${y}px`);
    const width = Math.max(0, Math.min(300, vw - 16));
    const left = Math.max(8, Math.min(x + pw - width, vw - width - 8));
    const above = y - 14;
    const below = vh - y - ph - 14;
    const openUp = below < 340 && above > below;
    this.menu.style.setProperty('--menu-x', `${left - x}px`);
    this.menu.style.setProperty('--menu-width', `${width}px`);
    this.menu.style.setProperty('--menu-height', `${Math.max(0, Math.min(340, openUp ? above : below))}px`);
    this.menu.classList.toggle('up', openUp);
    this.wrap.classList.add('show');
    this.wrap.inert = false;
  }

  private onNavigate = (): void => {
    this.cache.clear();
    this.hide(true);
    this.queueScan();
  };

  // Remember the right-clicked video: the element itself, or (players put overlays and
  // controls on top) the visible video under the pointer.
  private onContextMenu = (e: MouseEvent): void => {
    const own = e.composedPath().find((n): n is HTMLVideoElement => n instanceof HTMLVideoElement);
    const under = [...this.visible].find((v) => v.isConnected && targetOf(v) && inside(targetOf(v)!.getBoundingClientRect(), e.clientX, e.clientY));
    const v = own ?? under;
    this.contextVideo = v ? { v, at: Date.now() } : null;
  };

  /** Opens the Prism menu over the right-clicked video; false when there is none (any more). */
  openForContextMenu(): boolean {
    const c = this.contextVideo;
    this.contextVideo = null;
    if (!this.enabled || !c || Date.now() - c.at > 60_000 || !c.v.isConnected || !targetOf(c.v)) return false;
    if (this.fullscreen.active) return false;
    this.hide(true);
    const identity = videoIdentity(c.v);
    this.contextOnly = this.isDismissed(c.v, identity);
    this.show(c.v, identity);
    if (!this.current) return false;
    void this.toggleMenu();
    return true;
  }

  private onDocDown = (e: Event): void => {
    if (!this.menuOpen || !this.host) return;
    if (e.composedPath().includes(this.host)) return;
    this.closeMenu();
    this.scheduleHide();
  };

  private onKey = (e: KeyboardEvent): void => {
    if (e.key === 'Escape' && this.menuOpen) {
      this.closeMenu();
      if (this.current) this.pill.focus();
    }
  };

  // ---- menu ----------------------------------------------------------------------

  private context(): { pageUrl: string; title: string; sources: ElementSource[]; postUrl?: string; key: string } {
    const v = this.current;
    const src = v?.currentSrc || v?.src || '';
    const title = (IS_YOUTUBE ? document.title.replace(/ - YouTube$/, '') : document.title) || location.hostname;
    const pageUrl = location.href;
    // What this very <video> plays, so the menu can put it before the tab's other streams.
    const sources = v
      ? elementSources(
          {
            currentSrc: v.currentSrc,
            src: v.getAttribute('src') ?? '',
            sources: [...v.querySelectorAll('source')].map((s) => ({ url: s.getAttribute('src') ?? '', type: s.type })),
            dataSources: v.getAttribute('data-sources'),
          },
          document.baseURI,
        )
      : [];
    const post = v ? postUrlOf(v, pageUrl) : undefined;
    return { pageUrl, title, sources, ...(post ? { postUrl: post } : {}), key: `${pageUrl}|${post ?? ''}|${src}` };
  }

  private options(): Promise<VideoOptionsResult | undefined> {
    const c = this.context();
    const hit = this.cache.get(c.key);
    if (hit && Date.now() - hit.at < 600_000) return hit.res;
    const res = sendInternal<VideoOptionsResult>({ kind: 'videoOptions', pageUrl: c.pageUrl, title: c.title, sources: c.sources, ...(c.postUrl ? { postUrl: c.postUrl } : {}) });
    this.cache.set(c.key, { at: Date.now(), res });
    // Don't cache failures/empties for long.
    void res.then((r) => {
      if ((!r || !r.options.length) && this.cache.get(c.key)?.res === res) this.cache.delete(c.key);
    });
    return res;
  }

  private prefetch(): void {
    if (this.current && !this.contextOnly && !this.fullscreen.active) void this.options();
  }

  private closeMenu(): void {
    if (this.contextOnly) this.hide(true);
    else this.resetMenu();
  }

  private resetMenu(): void {
    if (!this.host) return;
    this.menuOpen = false;
    this.menuSession++;
    this.menu.classList.remove('open');
    this.pill.setAttribute('aria-expanded', 'false');
  }

  private async toggleMenu(): Promise<void> {
    if (this.menuOpen) {
      this.closeMenu();
      return;
    }
    if (!this.current || this.fullscreen.active) return;
    this.menuOpen = true;
    this.pill.setAttribute('aria-expanded', 'true');
    this.menu.classList.add('open');
    this.position();
    if (!this.menuOpen) return;
    const session = ++this.menuSession;
    const ctx = this.context();
    this.menu.innerHTML = `<div class="loading"><span class="spin"></span>${esc(t('Finding qualities…'))}</div>`;
    // On a video site's single post / video page "Best quality" works right away: the app picks
    // the best format (within the preferred quality) once it has read the page.
    const best = this.bestTarget(ctx);
    if (best) {
      this.menu.insertAdjacentHTML(
        'beforeend',
        `<button class="item" role="menuitem" data-id="best"><span class="ic">${icons.film(18)}</span><span class="tx"><span class="l">${esc(t('Best quality'))}</span><span class="d">${esc(t('Download now, without waiting for the list'))}</span></span><span class="go">${icons.download(16)}</span></button>`,
      );
      this.menu.querySelector<HTMLButtonElement>('.item[data-id="best"]')!.addEventListener('click', (e) => {
        e.stopPropagation();
        if (!e.isTrusted) return;
        void this.choose({ id: 'fmt:best', label: t('Best quality'), group: 'video', formatId: 'best' }, ctx.title, ctx.key, best);
      });
    }
    const res = await this.options();
    if (session !== this.menuSession) return;
    if (ctx.key !== this.context().key) return this.closeMenu();
    this.renderMenu(ctx.title, ctx.key, res);
  }

  private renderMenu(title: string, key: string, res: VideoOptionsResult | undefined): void {
    const head = `<div class="head">${prismMark(16)}<span class="t">${esc(res?.title || title)}</span><span class="src">${res?.source === 'probe' ? 'Prism' : esc(t('Detected'))}</span></div>`;
    if (!res) {
      this.menu.innerHTML = `${head}<div class="note">${esc(t('Prism extension is restarting — try again in a moment.'))}</div>`;
      return;
    }
    const own = res.options.some((o) => o.group === 'element');
    const groups: Array<[VideoOption['group'], string]> = [
      ['element', tk('This video')],
      ['video', tk('Video')],
      ['audio', tk('Audio only')],
      ['stream', own ? tk('Other streams on this page') : tk('Streams on this page')],
    ];
    let html = head;
    if (res.notice) html += `<div class="note">${esc(t(res.notice))}</div>`;
    if (!res.options.length) {
      html += `<div class="empty">${esc(t('No downloadable stream found yet — start playback and try again.'))}</div>`;
    }
    for (const [g, label] of groups) {
      const items = res.options.filter((o) => o.group === g);
      if (!items.length) continue;
      if (res.source === 'probe' || g === 'stream' || g === 'element') html += `<div class="group">${esc(t(label))}</div>`;
      for (const o of items) {
        const ic = g === 'audio' ? icons.audio(18) : g === 'stream' ? icons.stream(18) : icons.film(18);
        html += `<button class="item" role="menuitem" data-id="${esc(o.id)}"><span class="ic">${ic}</span><span class="tx"><span class="l">${esc(o.label)}</span>${o.detail ? `<span class="d">${esc(o.detail)}</span>` : ''}</span><span class="go">${icons.download(16)}</span></button>`;
      }
    }
    this.menu.innerHTML = html;
    this.menu.querySelectorAll<HTMLButtonElement>('.item').forEach((btn) => {
      btn.addEventListener('click', (e) => {
        e.stopPropagation();
        if (!e.isTrusted) return;
        const opt = res.options.find((o) => o.id === btn.dataset.id);
        if (opt) void this.choose(opt, res.title || title, key, opt.formatId ? res.pageUrl : undefined);
      });
    });
  }

  /** Where "Best quality" can go before the list is in: a video site's single post or video page. */
  private bestTarget(c: { pageUrl: string; postUrl?: string }): string | undefined {
    const page = c.postUrl ?? (postUrl(c.pageUrl, c.pageUrl) ? c.pageUrl : undefined);
    return page && this.videoSites.some((s) => siteMatches(page, s)) ? page : undefined;
  }

  /** `pageUrl`: the page the app read this format from (the post on a feed), else this page. */
  private async choose(option: VideoOption, title: string, key: string, pageUrl?: string): Promise<void> {
    if (!this.menuOpen || this.fullscreen.active) return;
    const ctx = this.context();
    this.closeMenu();
    if (key !== ctx.key) return;
    const session = this.menuSession;
    const label = this.pill.querySelector('.label')!;
    if (this.current) label.textContent = t('Sending…');
    const r = await sendInternal<ActionResult>({ kind: 'videoDownload', pageUrl: pageUrl ?? ctx.pageUrl, title, option });
    if (!this.current || session !== this.menuSession || ctx.key !== this.context().key) return;
    if (r?.ok) {
      this.pill.classList.add('done');
      label.textContent = t('Sent to Prism');
      this.pill.querySelector('.chev')!.innerHTML = icons.check(14);
    } else {
      this.pill.classList.add('err');
      label.textContent = r?.error ? t('Prism unavailable') : t('Failed');
      this.pill.title = r?.error ? t(r.error) : t('Could not reach Prism');
    }
    setTimeout(() => {
      if (session === this.menuSession && ctx.key === this.context().key) this.resetPill();
    }, 2200);
  }

  private resetPill(): void {
    if (!this.host) return;
    this.pill.classList.remove('done', 'err');
    this.pill.title = t('Download this video with Prism');
    this.pill.querySelector('.label')!.textContent = t('Download');
    this.pill.querySelector('.chev')!.innerHTML = icons.chevronDown(14);
  }
}
