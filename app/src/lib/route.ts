// Hash routing: every window loads the same SPA and `location.hash` selects the view.

import type { ToolsRequest } from "./media";
// Windows are created by Rust (`src-tauri/src/windows.rs`) with `index.html#/<kind>/<id>`.

export type Route =
  | { name: "main" }
  | { name: "addurl" }
  | { name: "mini" }
  | { name: "dropzone" }
  | { name: "capture"; id: string }
  | { name: "batch"; id: string }
  | { name: "video"; id: string }
  | { name: "complete"; id: string }
  | { name: "details"; id: string }
  | { name: "progress"; id: string }
  | { name: "torrent"; source: string }
  | { name: "model"; source: string }
  | { name: "tools"; request: ToolsRequest };

const WITH_ID = ["capture", "batch", "video", "complete", "details", "progress"] as const;

export function parseRoute(hash: string): Route {
  const path = hash.replace(/^#/, "").replace(/^\/+/, "");
  const [kind = "", ...rest] = path.split("/");
  let arg = "";
  try {
    arg = decodeURIComponent(rest.join("/"));
  } catch {
    return { name: "main" };
  }
  switch (kind) {
    case "addurl":
    case "mini":
    case "dropzone":
      return { name: kind };
    case "torrent":
      return arg ? { name: "torrent", source: hexDecode(arg) } : { name: "main" };
    case "model":
      return { name: "model", source: arg ? hexDecode(arg) : "" };
    case "tools":
      return { name: "tools", request: toolsRequest(rest[0], rest[1]) };
    default:
      if ((WITH_ID as readonly string[]).includes(kind) && arg) {
        return { name: kind as (typeof WITH_ID)[number], id: arg };
      }
      return { name: "main" };
  }
}

/** `#/tools/<tab>[/<hex of the request's JSON>]` (see `open_tools` in windows.rs). */
function toolsRequest(tab = "", hex = ""): ToolsRequest {
  const fallback: ToolsRequest = { tab: tab === "shrink" ? "shrink" : "convert" };
  if (!hex) return fallback;
  try {
    const r = JSON.parse(hexDecode(decodeURIComponent(hex))) as ToolsRequest;
    return { ...fallback, ...r, tab: r.tab === "shrink" ? "shrink" : "convert" };
  } catch {
    return fallback;
  }
}

/** Inverse of Rust's `encode_path`: hex → UTF-8 string. Non-hex input is returned unchanged. */
export function hexDecode(hex: string): string {
  if (!/^(?:[0-9a-f]{2})+$/i.test(hex)) return hex;
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) bytes[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return new TextDecoder().decode(bytes);
}

export function hexEncode(text: string): string {
  return Array.from(new TextEncoder().encode(text), (b) => b.toString(16).padStart(2, "0")).join("");
}
