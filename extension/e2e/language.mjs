// Language e2e: the extension renders in the browser's language.
//
//   npm run build && npm run e2e:language
//
// The other e2e runs pin --lang=en-US so their text assertions stay English; this one pins
// --lang=ru and checks that Russian actually shows up in all four contexts (service worker,
// popup, options page, content script). No native host and no external sites are needed.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFileSync } from 'node:fs';
import { mkdtemp, rm } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const here = dirname(fileURLToPath(import.meta.url));
const extDir = join(here, '..', 'build', 'chromium');
const ruCatalog = JSON.parse(readFileSync(join(here, '..', 'src', 'locales', 'ru.json'), 'utf8'));
const ruMessages = JSON.parse(readFileSync(join(here, '..', 'src', '_locales', 'ru', 'messages.json'), 'utf8'));
const enCatalog = JSON.parse(readFileSync(join(here, '..', 'src', 'locales', 'en.json'), 'utf8'));
const enMessages = JSON.parse(readFileSync(join(here, '..', 'src', '_locales', 'en', 'messages.json'), 'utf8'));

const html = `<!doctype html><meta charset="utf-8"><title>Prism language e2e</title>
  <video id="first" src="/first.mp4" preload="none" style="width:320px;height:180px;background:#202735"></video>`;

async function listen(serve) {
  const server = createServer((req, res) => {
    const body = serve(req.url);
    res.writeHead(body === undefined ? 404 : 200, { 'Content-Type': 'text/html; charset=utf-8' });
    res.end(body);
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  return { origin: `http://127.0.0.1:${server.address().port}`, close: () => server.close() };
}

const site = await listen((path) => (path === '/' ? html : undefined));
const profile = await mkdtemp(join(tmpdir(), 'prism-language-'));
const chromePath = process.env.PRISM_E2E_CHROME_PATH;
let context;

const text = (locator) => locator.textContent().then((s) => s.trim());

try {
  context = await chromium.launchPersistentContext(profile, {
    ...(chromePath ? { executablePath: chromePath } : { channel: 'chromium' }),
    headless: process.env.PRISM_E2E_HEADED !== '1',
    viewport: { width: 800, height: 650 },
    args: [`--disable-extensions-except=${extDir}`, `--load-extension=${extDir}`, '--lang=ru'],
  });
  const sw = context.serviceWorkers()[0] ?? (await context.waitForEvent('serviceworker', { timeout: 10000 }));
  const extId = new URL(sw.url()).host;

  // 1. The service worker (badge, context menus, video notices) resolves the browser language,
  //    and the manifest texts come from _locales/ru.
  const worker = await sw.evaluate(() => ({
    lang: chrome.i18n.getUILanguage(),
    description: chrome.i18n.getMessage('extDescription'),
    title: chrome.i18n.getMessage('actionTitle'),
  }));
  assert.equal(worker.lang, 'ru', `service worker language: ${worker.lang}`);
  assert.equal(worker.description, ruMessages.extDescription.message, 'manifest description is not Russian');
  assert.notEqual(enMessages.extDescription.message, ruMessages.extDescription.message, 'the Russian message equals the English one');
  assert.ok(worker.title, 'manifest action title is empty');
  console.log(`  ✔ service worker: ${worker.lang}, manifest description in Russian`);

  // 2. Popup.
  const popup = await context.newPage();
  await popup.goto(`chrome-extension://${extId}/popup.html`);
  await popup.locator('[data-act="collect"][data-mode="links"]').waitFor({ timeout: 5000 });
  assert.equal(await popup.evaluate(() => document.documentElement.lang), 'ru');
  const linksButton = await text(popup.locator('[data-act="collect"][data-mode="links"]'));
  assert.equal(linksButton, ruCatalog['All links'], `popup button: ${linksButton}`);
  const chip = await text(popup.locator('#conn'));
  assert.ok(
    chip === ruCatalog['Checking…'] || chip === ruCatalog['Not connected'] || chip.includes(ruCatalog['Connected']),
    `popup chip is not Russian: ${chip}`,
  );
  console.log(`  ✔ popup: "${linksButton}" · ${chip}`);

  // 3. Options page (data-i18n pass runs at load).
  const options = await context.newPage();
  await options.goto(`chrome-extension://${extId}/options.html`);
  await options.locator('h1').waitFor({ timeout: 5000 });
  assert.equal(await options.evaluate(() => document.documentElement.lang), 'ru');
  const heading = await text(options.locator('h1'));
  assert.equal(heading, ruCatalog['Prism browser integration'], `options heading: ${heading}`);
  console.log(`  ✔ options: "${heading}"`);

  // 4. Content script: the video button.
  const page = await context.newPage();
  await page.goto(`${site.origin}/`);
  await page.locator('#first').hover();
  await page.locator('[data-prism-video-button] .wrap.show').waitFor({ timeout: 5000 });
  const label = await text(page.locator('[data-prism-video-button] .label'));
  assert.equal(label, ruCatalog.Download, `video button: ${label}`);
  assert.notEqual(enCatalog.Download, label, 'the button is still English');
  console.log(`  ✔ video button: "${label}"`);

  console.log('Russian rendering passed in the service worker, popup, options page and content script');
} finally {
  await context?.close();
  site.close();
  await rm(profile, { recursive: true, force: true });
}
