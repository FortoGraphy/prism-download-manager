// Hash routing: every window loads the same SPA and `location.hash` selects the view.
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
  | { name: "torrent"; source: string };

const WITH_ID = ["capture", "batch", "video", "complete", "details", "progress"] as const;

export function parseRoute(hash: string): Route {
  const path = hash.replace(/^#/, "").replace(/^\/+/, "");
  const [kind = "", ...rest] = path.split("/");
  const arg = decodeURIComponent(rest.join("/"));
  switch (kind) {
    case "addurl":
    case "mini":
    case "dropzone":
      return { name: kind };
    case "torrent":
      return arg ? { name: "torrent", source: hexDecode(arg) } : { name: "main" };
    default:
      if ((WITH_ID as readonly string[]).includes(kind) && arg) {
        return { name: kind as (typeof WITH_ID)[number], id: arg };
      }
      return { name: "main" };
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
