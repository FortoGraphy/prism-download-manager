// Builds prism-native-host and places it where Tauri's `externalBin` expects it:
// app/src-tauri/binaries/prism-native-host-<target-triple>[.exe]
//
// Usage: node scripts/build-sidecar.mjs [--debug] [--target <triple>]
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

// Ensure the browser extension is built so Tauri resource packaging doesn't fail
const extDir = join(root, "extension");
const extBuildChromium = join(extDir, "build", "chromium");
const extBuildFirefox = join(extDir, "build", "firefox");
if (!existsSync(extBuildChromium) || !existsSync(extBuildFirefox)) {
  try {
    console.log("Building browser extension...");
    execFileSync("node", ["build.mjs"], { cwd: extDir, stdio: "inherit" });
  } catch (err) {
    console.warn("Could not build extension, ensuring directories exist:", err?.message || err);
    mkdirSync(extBuildChromium, { recursive: true });
    mkdirSync(extBuildFirefox, { recursive: true });
  }
}
const args = process.argv.slice(2);
const debug = args.includes("--debug");
const targetIdx = args.indexOf("--target");
const hostTriple = execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(/host: (\S+)/)[1];
const triple = targetIdx >= 0 ? args[targetIdx + 1] : process.env.TAURI_ENV_TARGET_TRIPLE || hostTriple;

const cargoArgs = ["build", "-p", "prism-native-host", "--target", triple];
if (!debug) cargoArgs.push("--release");
execFileSync("cargo", cargoArgs, { cwd: root, stdio: "inherit" });

const ext = triple.includes("windows") ? ".exe" : "";
const targetBase = process.env.CARGO_TARGET_DIR ? resolve(process.env.CARGO_TARGET_DIR) : join(root, "target");
const built = join(targetBase, triple, debug ? "debug" : "release", `prism-native-host${ext}`);
const outDir = join(root, "app", "src-tauri", "binaries");
mkdirSync(outDir, { recursive: true });
const out = join(outDir, `prism-native-host-${triple}${ext}`);
copyFileSync(built, out);
console.log(`sidecar: ${out}`);
