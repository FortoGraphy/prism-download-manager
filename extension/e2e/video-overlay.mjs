import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const extDir = fileURLToPath(new URL('../build/chromium', import.meta.url));
const html = (body) => `<!doctype html><meta name="viewport" content="width=device-width">
  <style>body { margin: 0; } main { padding: 64px 8px 0; } article { margin-bottom: 52px; }
  video { display: block; width: min(360px, 100%); height: 150px; background: #202735; }
  iframe { width: 100%; height: 440px; border: 0; }</style>${body}`;

const players = html(`<main id="player">
  <article><a href="/posts/first">First post</a><video id="first" src="/first.mp4" preload="none"></video></article>
  <article><a href="/posts/second">Second post</a><video id="second" src="/second.mp4" preload="none"></video></article>
  </main><script>document.addEventListener('keydown', (event) => {
    if (event.key === 'f') document.querySelector('#player').requestFullscreen();
    if (event.key === 'v') document.querySelector('#first').requestFullscreen();
  });</script>`);

async function listen(serve) {
  const server = createServer((req, res) => {
    const body = serve(req.url);
    res.writeHead(body === undefined ? 404 : 200, { 'Content-Type': 'text/html' });
    res.end(body);
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  return { origin: `http://127.0.0.1:${server.address().port}`, close: () => server.close() };
}

// Different ports make the iframe cross-origin, with no external sites required.
const child = await listen((path) => path === '/feed' ? players : undefined);
const site = await listen((path) => {
  if (path === '/feed') return players;
  if (path === '/embed') {
    return html(`<section><button onclick="this.parentElement.requestFullscreen()">Fullscreen</button>
      <iframe src="${child.origin}/feed" allowfullscreen></iframe></section>`);
  }
  if (path === '/standalone') {
    return html('<div style="height:100vh;overflow:clip"><main style="padding-top:205px"><video id="first" preload="none"></video></main></div>');
  }
});
const profile = await mkdtemp(join(tmpdir(), 'prism-video-overlay-'));
const chromePath = process.env.PRISM_E2E_CHROME_PATH;
let context;

const frames = (page) => page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
const overlay = (scope) => scope.locator('[data-prism-video-button] .wrap.show');
const menu = (scope) => scope.locator('[data-prism-video-button] .menu.open');
const show = async (scope, id) => {
  await scope.locator(id).hover();
  await overlay(scope).waitFor();
};

try {
  context = await chromium.launchPersistentContext(profile, {
    ...(chromePath ? { executablePath: chromePath } : { channel: 'chromium' }),
    headless: process.env.PRISM_E2E_HEADED !== '1',
    viewport: { width: 800, height: 650 },
    args: [`--disable-extensions-except=${extDir}`, `--load-extension=${extDir}`],
  });
  const sw = context.serviceWorkers()[0] ?? await context.waitForEvent('serviceworker', { timeout: 10000 });
  const page = await context.newPage();
  page.setDefaultTimeout(5000);
  const feed = `${site.origin}/feed`;
  await page.goto(feed);
  await show(page, '#first');
  await overlay(page).getByRole('button', { name: 'Hide Prism for this video' }).press('Enter');
  await overlay(page).waitFor({ state: 'hidden' });
  assert.equal(await page.locator('[data-prism-video-button] .wrap[inert]').count(), 1);

  // Replace the post and change its video URL, as feeds do when quality changes.
  await page.locator('#first').evaluate((video) => {
    const post = video.closest('article');
    const replacement = post.cloneNode(true);
    replacement.querySelector('video').src = '/first-hd.mp4';
    post.replaceWith(replacement);
  });
  await page.setViewportSize({ width: 760, height: 620 });
  await page.locator('#first').hover();
  await frames(page);
  assert.equal(await overlay(page).count(), 0);
  await show(page, '#second');
  await overlay(page).getByRole('button', { name: 'Download' }).click();
  await menu(page).waitFor();

  // Reopen a dismissed player's quality menu through the extension's context-menu handler.
  await page.keyboard.press('Escape');
  await page.locator('#first').click({ button: 'right' });
  await sw.evaluate(async (url) => {
    const [tab] = await chrome.tabs.query({ url });
    await globalThis.__prism.handleMenuClick({ menuItemId: 'prism-link', mediaType: 'video', srcUrl: 'blob:fixture', pageUrl: url }, tab);
  }, feed);
  await menu(page).waitFor();
  await page.keyboard.press('Escape');
  await overlay(page).waitFor({ state: 'hidden' });
  await page.locator('#first').hover();
  await frames(page);
  assert.equal(await overlay(page).count(), 0);
  await page.reload();
  await show(page, '#first');
  console.log('Per-post dismissal, keyboard activation, context menu and reload passed');

  // Fullscreen closes an open quality menu and requires a fresh hover on exit.
  for (const key of ['f', 'v']) {
    await overlay(page).getByRole('button', { name: 'Download' }).click();
    await menu(page).waitFor();
    await page.keyboard.press(key);
    await page.waitForFunction(() => document.fullscreenElement);
    await overlay(page).waitFor({ state: 'hidden' });
    assert.equal(await menu(page).count(), 0);
    await page.mouse.move(80, 110);
    await frames(page);
    assert.equal(await overlay(page).count(), 0);
    await page.evaluate(() => document.exitFullscreen());
    await frames(page);
    assert.equal(await overlay(page).count(), 0);
    await show(page, '#first');
  }
  await page.goto(`${site.origin}/embed`);
  const frame = page.frameLocator('iframe');
  await show(frame, '#first');
  await overlay(frame).getByRole('button', { name: 'Download' }).click();
  await page.getByRole('button', { name: 'Fullscreen' }).click();
  await page.waitForFunction(() => document.fullscreenElement);
  await overlay(frame).waitFor({ state: 'hidden' });
  await frame.locator('#first').hover();
  await frames(page);
  assert.equal(await overlay(frame).count(), 0);
  await page.evaluate(() => document.exitFullscreen());
  await frames(page);
  await show(frame, '#first');
  console.log('Fullscreen for a player, a video and an ancestor of a cross-origin frame passed');

  await page.setViewportSize({ width: 260, height: 300 });
  await page.goto(`${site.origin}/standalone`);
  await page.locator('#first').hover({ position: { x: 90, y: 35 } });
  await overlay(page).waitFor();
  await overlay(page).getByRole('button', { name: 'Download' }).click();
  await menu(page).waitFor();
  await menu(page).getByText('No downloadable stream found yet', { exact: false }).waitFor();
  const bounds = await menu(page).boundingBox();
  assert.ok(bounds.x >= 7 && bounds.x + bounds.width <= 253 && bounds.y >= 7);
  assert.ok(bounds.y + bounds.height <= (await overlay(page).boundingBox()).y);
  await overlay(page).getByRole('button', { name: 'Hide Prism for this video' }).click();
  await overlay(page).waitFor({ state: 'hidden' });
  await page.mouse.move(90, 242);
  await frames(page);
  assert.equal(await overlay(page).count(), 0);
  await page.locator('#first').evaluate((video) => { video.src = '/new-source.mp4'; });
  await page.mouse.move(100, 245);
  await overlay(page).waitFor();
  console.log('Narrow viewport positioning and source-less player reuse passed');

  // Keep one real content-script request pending while opening the second video's menu.
  await sw.evaluate(async () => {
    const key = 'prism.state';
    const state = (await chrome.storage.local.get(key))[key];
    await chrome.storage.local.set({ [key]: { ...state, settings: { ...state?.settings, videoSites: ['127.0.0.1'] } } });
    const native = globalThis.__prism.native;
    const request = native.request.bind(native);
    globalThis.pendingPrismProbes = new Map();
    native.request = (msg, opts) => msg.type === 'probeVideo'
      ? new Promise((resolve) => globalThis.pendingPrismProbes.set(new URL(msg.pageUrl).pathname, resolve))
      : request(msg, opts);
  });
  const finishProbe = (path, title) => sw.evaluate(async ({ path, title }) => {
    const deadline = Date.now() + 5000;
    while (!globalThis.pendingPrismProbes.has(path) && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
    const finish = globalThis.pendingPrismProbes.get(path);
    if (!finish) throw new Error(`Prism did not request ${path}`);
    finish({ title, formats: [{ id: '720p', label: title, kind: 'video' }] });
  }, { path, title });

  await page.setViewportSize({ width: 800, height: 650 });
  await page.goto(feed);
  await show(page, '#first');
  await overlay(page).getByRole('button', { name: 'Download' }).click();
  await menu(page).getByText('Finding qualities…').waitFor();
  await overlay(page).getByRole('button', { name: 'Hide Prism for this video' }).click();
  await show(page, '#second');
  await overlay(page).getByRole('button', { name: 'Download' }).click();
  await finishProbe('/posts/second', 'Second post result');
  await menu(page).getByRole('menuitem', { name: 'Second post result' }).waitFor();
  await finishProbe('/posts/first', 'Stale first post result');
  await frames(page);
  assert.equal(await menu(page).getByRole('menuitem', { name: 'Second post result' }).count(), 1);
  assert.equal(await menu(page).getByRole('menuitem', { name: 'Stale first post result' }).count(), 0);
  console.log('Late quality response stays attached to its original menu');
} finally {
  await context?.close();
  site.close();
  child.close();
  await rm(profile, { recursive: true, force: true });
}
