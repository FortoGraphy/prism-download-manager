// Real-browser regressions for per-video dismissal. No native host or desktop app needed.
// npm run e2e:dismiss
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { build } from 'esbuild';
import { chromium } from 'playwright';
import { fileURLToPath } from 'node:url';

const { outputFiles } = await build({
  banner: { js: `window.__requests = []; window.__pending = [];
    window.chrome = { runtime: { sendMessage: (msg) => {
      window.__requests.push(msg);
      return new Promise(resolve => window.__pending.push(resolve));
    } } };` },
  stdin: {
    contents: `import { VideoButton } from './src/content/video-button';
      const start = () => {
        window.__testButton = new VideoButton();
        window.__testButton.setEnabled(true);
      };
      if (document.documentElement) start();
      else document.addEventListener('DOMContentLoaded', start, { once: true });`,
    resolveDir: fileURLToPath(new URL('..', import.meta.url)),
  },
  bundle: true,
  write: false,
  format: 'iife',
});

const server = createServer((req, res) => {
  if (req.url.endsWith('.mp4')) { res.writeHead(404); res.end(); return; }
  res.setHeader('Content-Type', 'text/html');
  res.end(`<!doctype html><title>Dismiss regression</title>
    <style>body{margin:50px} article{width:400px;margin-bottom:70px}
    video{display:block;width:400px;height:225px;background:#223}</style>
    <article id="first" permalink="/posts/one"><video controls></video><button>More</button></article>
    <article id="second" permalink="/posts/two"><video controls></video><button>More</button></article>
    <div style="height:500px"></div>
    <script>window.siteClicks = 0; window.sitePointerDowns = 0;
      document.body.addEventListener('click', () => window.siteClicks++);
      document.body.addEventListener('pointerdown', () => window.sitePointerDowns++);
    </script>`);
});
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
let browser;
try {
  browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({ viewport: { width: 1000, height: 900 } });
  await context.addInitScript({ content: outputFiles[0].text });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(e.message));
  const ui = page.locator('[data-prism-video-button]');
  const shown = () => page.evaluate(() => !!document.querySelector('[data-prism-video-button]')?.shadowRoot?.querySelector('.wrap.show'));
  const settle = () => page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const hover = async (id = 'first') => {
    await page.locator(`#${id} video`).hover({ position: { x: 180, y: 120 } });
    await settle();
  };
  const expectShown = async () => {
    await page.waitForFunction(() => !!document.querySelector('[data-prism-video-button]')?.shadowRoot?.querySelector('.wrap.show'));
  };
  const dismiss = async () => {
    await ui.getByRole('button', { name: 'Hide download button for this video', exact: true }).click();
    assert.equal(await shown(), false, 'X must immediately hide the overlay');
    assert.equal(await ui.locator('.wrap').evaluate((el) => el.inert), true, 'hidden controls must leave the keyboard focus order');
  };
  const fresh = async () => {
    await page.goto(`http://127.0.0.1:${server.address().port}/feed`);
    await hover();
    await expectShown();
  };

  await fresh();
  await ui.locator('.dismiss').evaluate((el) => el.click());
  assert.equal(await shown(), true, 'page scripts must not dismiss the button with a synthetic click');
  await dismiss();
  for (let i = 0; i < 3; i++) {
    await page.mouse.move(900, 800);
    await hover();
    assert.equal(await shown(), false, 'pointer movement must not undo dismissal');
  }
  await page.setViewportSize({ width: 950, height: 880 });
  await page.mouse.wheel(0, 20);
  await page.waitForFunction(() => window.scrollY > 0);
  await hover();
  assert.equal(await shown(), false, 'scroll/resize must not undo dismissal');
  assert.deepEqual(await page.evaluate(() => [window.siteClicks, window.sitePointerDowns]), [0, 0], 'X must not activate underlying page controls');
  assert.equal(await page.evaluate(() => window.__requests.some((m) => m.kind !== 'videoOptions')), false, 'dismissal must not request a download');
  await hover('second');
  await expectShown();
  const bounds = await ui.locator('.controls').boundingBox();
  assert.ok(bounds.x >= 0 && bounds.x + bounds.width <= 950, 'both controls must fit within the viewport');
  console.log('PASS: trusted X hides only the selected post and stays hidden through hover, scroll and resize');

  await page.evaluate(() => {
    const old = document.querySelector('#first video');
    old.replaceWith(old.cloneNode(true));
  });
  await hover();
  // Moving from another video can retain its old overlay briefly; the dismissed post must
  // never replace it, and the old overlay must go away after its normal leave delay.
  await page.waitForFunction(() => !document.querySelector('[data-prism-video-button]')?.shadowRoot?.querySelector('.wrap.show'));
  await hover();
  assert.equal(await shown(), false, 'rerendering the same post must preserve dismissal');
  await page.evaluate(() => { document.querySelector('#first video').src = '/new-quality.mp4'; });
  await hover();
  assert.equal(await shown(), false, 'a quality/source change within the same post must preserve dismissal');
  await page.evaluate(() => document.querySelector('#first').setAttribute('permalink', '/posts/three'));
  await hover();
  await expectShown();
  console.log('PASS: dismissal survives post rerenders but a recycled element in a new post remains available');

  await fresh();
  await ui.locator('.dismiss').focus();
  await page.keyboard.press('Enter');
  assert.equal(await shown(), false, 'keyboard activation must dismiss');
  await page.reload();
  await hover();
  await expectShown();
  console.log('PASS: X is keyboard accessible and reload restores the button');

  await page.goto(`http://127.0.0.1:${server.address().port}/posts/one`);
  await hover();
  await expectShown();
  await dismiss();
  await hover('second');
  await expectShown();
  console.log('PASS: related posts on a single-post page retain their own download buttons');

  await fresh();

  await ui.locator('.pill').click();
  await page.waitForFunction(() => window.__pending.length > 0);
  await dismiss();
  await hover('second');
  await expectShown();
  await ui.locator('.pill').click();
  await page.evaluate(() => window.__pending[0]({ title: 'Stale first post', source: 'detected', options: [] }));
  await settle();
  assert.equal(await ui.locator('.menu .loading').count(), 1, 'old options must not replace the next post menu');
  await page.evaluate(() => {
    for (const resolve of window.__pending.slice(1)) resolve({ title: 'Current second post', source: 'detected', options: [] });
  });
  await ui.locator('.menu .t').filter({ hasText: 'Current second post' }).waitFor();
  await dismiss();
  await hover();
  assert.equal(await shown(), false, 'late option responses must not reopen a dismissed overlay');
  console.log('PASS: dismissal closes an open menu and discards its stale asynchronous results');

  await page.locator('#first video').click({ button: 'right', position: { x: 180, y: 120 } });
  assert.equal(await page.evaluate(() => window.__testButton.openForContextMenu()), true, 'explicit context-menu action must still work');
  assert.equal(await shown(), true);
  await page.mouse.click(900, 800);
  await hover();
  await page.waitForFunction(() => !document.querySelector('[data-prism-video-button]')?.shadowRoot?.querySelector('.wrap.show'));
  await hover();
  assert.equal(await shown(), false, 'manual context-menu use must not reset the hover dismissal');
  console.log('PASS: explicit context-menu download remains available without undoing hover dismissal');

  await fresh();
  await page.evaluate(() => {
    for (const article of document.querySelectorAll('article')) article.removeAttribute('permalink');
    document.querySelector('#first video').src = '/first.mp4';
    document.querySelector('#second video').src = '/second.mp4';
  });
  await dismiss();
  await hover();
  assert.equal(await shown(), false);
  await page.evaluate(() => history.pushState({}, '', '/feed?comment=42#expanded'));
  await hover();
  assert.equal(await shown(), false, 'same-document URL changes must not undo a media dismissal');
  await page.evaluate(() => { document.querySelector('#first video').src = '/replacement.mp4'; });
  await hover();
  await expectShown();
  await dismiss();
  await hover('second');
  await expectShown();
  console.log('PASS: videos without post links use their media identity and recycled sources remain available');

  await fresh();
  await page.evaluate(() => {
    for (const article of document.querySelectorAll('article')) article.removeAttribute('permalink');
  });
  await dismiss();
  await hover();
  assert.equal(await shown(), false);
  await hover('second');
  await expectShown();
  console.log('PASS: source-less players can be dismissed without suppressing unrelated players');
  assert.deepEqual(errors, [], 'content UI must not throw page errors');
} finally {
  await browser?.close();
  await new Promise((resolve) => server.close(resolve));
}
