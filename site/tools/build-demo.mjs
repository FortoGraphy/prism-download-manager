// Builds the live demo's app (site/demo/app/) from app/: the real Prism UI with its in-memory
// demo backend (app/src/lib/mock.ts), opened in frames by site/demo/index.html.
// Run after a release, from the repository root:  node site/tools/build-demo.mjs
import { execSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const out = resolve(root, "site/demo/app");
execSync(`npx vite build --base ./ --outDir "${out}" --emptyOutDir`, { cwd: resolve(root, "app"), stdio: "inherit" });

// The frames are part of /demo/, not pages of their own.
const page = resolve(out, "index.html");
writeFileSync(page, readFileSync(page, "utf8").replace("<head>", '<head>\n    <meta name="robots" content="noindex">'));
console.log(`demo app written to ${out}`);
