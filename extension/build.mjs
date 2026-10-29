// Builds build/chromium and build/firefox (+ zips) from one source.
//   node build.mjs           production (minified)
//   node build.mjs --dev     unminified + inline source maps
import { build } from 'esbuild';
import { copyFileSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { crc32, deflateRawSync } from 'node:zlib';

const root = dirname(fileURLToPath(import.meta.url));
const dev = process.argv.includes('--dev');
const pkg = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
const PUBLIC_KEY = readFileSync(join(root, 'keys', 'public-key.txt'), 'utf8').trim();

export const CHROME_EXTENSION_ID = 'mmcdeiglkbfbicdjboocmhehodogilfl';
export const GECKO_ID = 'prism@prismdl.site';

const description = 'Hand downloads, videos and links to the Prism download manager.';
const icons = { 16: 'icons/icon-16.png', 32: 'icons/icon-32.png', 48: 'icons/icon-48.png', 128: 'icons/icon-128.png' };
const common = {
  manifest_version: 3,
  name: 'Prism Download Manager',
  short_name: 'Prism',
  version: pkg.version,
  description,
  icons,
  action: { default_popup: 'popup.html', default_title: 'Prism', default_icon: { 16: icons[16], 32: icons[32] } },
  options_ui: { page: 'options.html', open_in_tab: true },
  host_permissions: ['<all_urls>'],
  content_scripts: [{ matches: ['<all_urls>'], js: ['content.js'], run_at: 'document_start', all_frames: true }],
};
const basePermissions = ['downloads', 'nativeMessaging', 'cookies', 'storage', 'contextMenus', 'scripting', 'webRequest', 'webNavigation', 'alarms'];

const manifests = {
  chromium: {
    ...common,
    key: PUBLIC_KEY,
    minimum_chrome_version: '116',
    background: { service_worker: 'background.js' },
    permissions: basePermissions,
  },
  firefox: {
    ...common,
    background: { scripts: ['background.js'] },
    permissions: [...basePermissions, 'webRequestBlocking'],
    browser_specific_settings: {
      gecko: { id: GECKO_ID, strict_min_version: '140.0', data_collection_permissions: { required: ['none'] } },
    },
  },
};

const entries = {
  background: 'src/background/index.ts',
  content: 'src/content/index.ts',
  popup: 'src/popup/popup.ts',
  options: 'src/options/options.ts',
};
const statics = {
  'popup.html': 'src/popup/popup.html',
  'popup.css': 'src/popup/popup.css',
  'options.html': 'src/options/options.html',
  'options.css': 'src/options/options.css',
  'ui.css': 'src/shared/ui.css',
};

function copyDir(from, to) {
  mkdirSync(to, { recursive: true });
  for (const f of readdirSync(from)) {
    const s = join(from, f);
    if (statSync(s).isDirectory()) copyDir(s, join(to, f));
    else copyFileSync(s, join(to, f));
  }
}

// ---- minimal zip writer (deflate) ------------------------------------------------
function listFiles(dir, base = dir) {
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    return statSync(p).isDirectory() ? listFiles(p, base) : [relative(base, p).split('\\').join('/')];
  });
}

function zipDir(dir, outFile) {
  const locals = [];
  const centrals = [];
  let offset = 0;
  const dosTime = 0x0000; // 00:00:00
  const dosDate = ((2026 - 1980) << 9) | (1 << 5) | 1; // fixed date → reproducible zips
  for (const name of listFiles(dir).sort()) {
    const data = readFileSync(join(dir, name));
    const comp = deflateRawSync(data, { level: 9 });
    const crc = crc32(data) >>> 0;
    const nameBuf = Buffer.from(name, 'utf8');
    const lh = Buffer.alloc(30);
    lh.writeUInt32LE(0x04034b50, 0);
    lh.writeUInt16LE(20, 4);
    lh.writeUInt16LE(0x0800, 6); // UTF-8 names
    lh.writeUInt16LE(8, 8);
    lh.writeUInt16LE(dosTime, 10);
    lh.writeUInt16LE(dosDate, 12);
    lh.writeUInt32LE(crc, 14);
    lh.writeUInt32LE(comp.length, 18);
    lh.writeUInt32LE(data.length, 22);
    lh.writeUInt16LE(nameBuf.length, 26);
    lh.writeUInt16LE(0, 28);
    locals.push(lh, nameBuf, comp);
    const ch = Buffer.alloc(46);
    ch.writeUInt32LE(0x02014b50, 0);
    ch.writeUInt16LE(20, 4);
    ch.writeUInt16LE(20, 6);
    ch.writeUInt16LE(0x0800, 8);
    ch.writeUInt16LE(8, 10);
    ch.writeUInt16LE(dosTime, 12);
    ch.writeUInt16LE(dosDate, 14);
    ch.writeUInt32LE(crc, 16);
    ch.writeUInt32LE(comp.length, 20);
    ch.writeUInt32LE(data.length, 24);
    ch.writeUInt16LE(nameBuf.length, 28);
    ch.writeUInt32LE(offset, 42);
    centrals.push(ch, nameBuf);
    offset += lh.length + nameBuf.length + comp.length;
  }
  const central = Buffer.concat(centrals);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(centrals.length / 2, 8);
  end.writeUInt16LE(centrals.length / 2, 10);
  end.writeUInt32LE(central.length, 12);
  end.writeUInt32LE(offset, 16);
  writeFileSync(outFile, Buffer.concat([...locals, central, end]));
}

// ---- build ---------------------------------------------------------------------
const outRoot = join(root, 'build');
for (const target of ['chromium', 'firefox']) {
  const out = join(outRoot, target);
  rmSync(out, { recursive: true, force: true });
  mkdirSync(out, { recursive: true });
  await build({
    entryPoints: entries,
    outdir: out,
    bundle: true,
    format: 'iife',
    target: target === 'chromium' ? ['chrome116'] : ['firefox128'],
    minify: !dev,
    sourcemap: dev ? 'inline' : false,
    legalComments: 'none',
    define: {
      __TARGET__: JSON.stringify(target),
      __DEV__: JSON.stringify(dev || process.env.PRISM_E2E === '1'),
    },
    logLevel: 'warning',
    charset: 'utf8',
  });
  for (const [dst, src] of Object.entries(statics)) copyFileSync(join(root, src), join(out, dst));
  copyDir(join(root, 'static', 'icons'), join(out, 'icons'));
  writeFileSync(join(out, 'manifest.json'), JSON.stringify(manifests[target], null, 2) + '\n');
  zipDir(out, join(outRoot, `prism-${target}.zip`));
  console.log(`built ${relative(root, out)} + build/prism-${target}.zip`);
}
