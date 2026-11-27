// End-to-end against the REAL app: Chromium + unpacked extension + real prism-native-host
// + a running Prism (release build started with WebView2 remote debugging).
//
// 1. Start Prism with remote debugging:  node scripts/app-cdp.mjs launch <prism.exe> <dataDir> 9333
// 2. Start the test server:              target/release/prism-testserver --port 18765
// 3. PRISM_APP_DATA=<dataDir> [PRISM_E2E_CHROME_PATH=...] node extension/e2e/real-app.mjs
//
// Verifies: hello reaches the app, clicking a download link in the browser opens Prism's
// "Download File" window, Start download completes and the file's SHA-256 matches the
// server's content. Windows only (registry + WebView2 CDP).
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream, existsSync, mkdtempSync } from 'node:fs';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '..', '..');
const extDir = resolve(here, '..', 'build', 'chromium');
const hostExe = join(repo, 'target', 'release', 'prism-native-host.exe');
const dataDir = process.env.PRISM_APP_DATA;
const appPort = Number(process.env.PRISM_APP_CDP ?? 9333);
const testServer = process.env.PRISM_TESTSERVER ?? 'http://127.0.0.1:18765';
const chromePath = process.env.PRISM_E2E_CHROME_PATH;
const REG = 'HKCU\\Software\\Chromium\\NativeMessagingHosts\\dev.prism.host';
assert.ok(dataDir, 'set PRISM_APP_DATA to the running app data dir');
assert.ok(existsSync(hostExe), 'build the release host first');

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function until(fn, ms, what) {
  const end = Date.now() + ms;
  for (;;) {
    const v = await fn().catch(() => null);
    if (v) return v;
    if (Date.now() > end) throw new Error(`timed out: ${what}`);
    await sleep(100);
  }
}
const sha256 = (path) =>
  new Promise((ok, fail) => {
    const h = createHash('sha256');
    createReadStream(path).on('data', (d) => h.update(d)).on('end', () => ok(h.digest('hex'))).on('error', fail);
  });

const size = 32 * 1024 * 1024;
const fileUrl = `${testServer}/file/browser-capture.zip?size=${size}&seed=77&throttle=6M`;
const site = createServer((req, res) => {
  res.writeHead(200, { 'Content-Type': 'text/html' });
  res.end(`<!doctype html><title>Prism real e2e</title><a id="dl" href="${fileUrl}">Download browser-capture.zip</a>`);
});
await new Promise((r) => site.listen(0, '127.0.0.1', r));
const origin = `http://127.0.0.1:${site.address().port}`;

let previous = null;
try {
  previous = /REG_SZ\s+(.*)/.exec(execFileSync('reg', ['query', REG, '/ve'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }))?.[1]?.trim() ?? null;
} catch {
  previous = null;
}
const tmp = mkdtempSync(join(tmpdir(), 'prism-real-'));
const app = await chromium.connectOverCDP(`http://127.0.0.1:${appPort}`);
const appPage = (part) => app.contexts().flatMap((c) => c.pages()).find((p) => p.url().includes(part));
const main = appPage('tauri.localhost/');
assert.ok(main, 'Prism main window not found over CDP');
const invoke = (cmd, args) => main.evaluate(([c, a]) => window.__TAURI_INTERNALS__.invoke(c, a), [cmd, args]);

let context;
try {
  execFileSync(hostExe, ['register', '--browsers', 'chromium', '--manifest-dir', join(tmp, 'nm')], { env: { ...process.env, PRISM_DATA_DIR: dataDir } });
  const before = (await invoke('integration_status')).lastSeen ?? 0;
  context = await chromium.launchPersistentContext(join(tmp, 'profile'), {
    ...(chromePath ? { executablePath: chromePath } : { channel: 'chromium' }),
    headless: true,
    // Texts are asserted in English, so the browser language is pinned (see e2e/language.mjs).
    args: [`--disable-extensions-except=${extDir}`, `--load-extension=${extDir}`, '--lang=en-US'],
    env: { ...process.env, PRISM_DATA_DIR: dataDir },
  });
  const page = await context.newPage();
  const cdp = await context.newCDPSession(page);
  await cdp.send('Browser.setDownloadBehavior', { behavior: 'default' });

  // 1. hello reaches the real app (integration status records the contact).
  await until(async () => ((await invoke('integration_status')).lastSeen ?? 0) > before, 15000, 'extension hello');
  console.log('  ✔ extension → native host → Prism: hello received');

  // 2. Click a download link in the browser → Prism opens its capture window.
  await page.goto(`${origin}/`);
  const t0 = Date.now();
  await page.click('#dl');
  const capture = await until(async () => appPage('#/capture/'), 15000, 'capture window');
  console.log(`  ✔ link click opened Prism's Download File window in ${Date.now() - t0} ms`);
  await capture.getByRole('button', { name: 'Start download' }).click({ timeout: 10000 });

  // 3. The download finishes in Prism and the bytes match the server.
  const item = await until(async () => {
    const boot = await invoke('bootstrap');
    return boot.items.find((i) => i.fileName.startsWith('browser-capture') && i.status === 'completed');
  }, 60000, 'download completion');
  const path = join(item.saveDir, item.fileName);
  const got = await sha256(path);
  const want = createHash('sha256').update(Buffer.from(await (await fetch(fileUrl.replace('&throttle=6M', ''))).arrayBuffer())).digest('hex');
  assert.equal(got, want, 'downloaded file differs from the server content');
  assert.equal(item.referer?.startsWith(origin), true, `referer not forwarded: ${item.referer}`);
  console.log(`  ✔ download completed and SHA-256 matches (${got.slice(0, 16)}…), referer forwarded`);
  console.log('REAL E2E PASSED');
} finally {
  await context?.close().catch(() => undefined);
  await app.close().catch(() => undefined);
  site.close();
  try {
    if (previous === null) execFileSync('reg', ['delete', REG, '/f'], { stdio: 'ignore' });
    else execFileSync('reg', ['add', REG, '/ve', '/d', previous, '/f'], { stdio: 'ignore' });
  } catch {
    /* ignore */
  }
}
