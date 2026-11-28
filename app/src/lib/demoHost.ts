// The website's live demo (prismdl.site/demo/): the main window and every dialog run in their own
// frame on one page, and all of them use the main frame's demo backend, so a download added in
// the Add URL frame shows up in the main window's list. Never active inside Tauri.

import { inTauri } from "./api";

type Backend = typeof import("./mock");

/** What the demo page (`site/demo/index.html`) puts on its window as `__prismDemoHost`. */
export interface DemoHost {
  /** Opens a window: `hash` is the route, e.g. `addurl` or `details/demo3`. */
  open(hash: string): void;
  /** Closes the frame this window runs in (the main window stays). */
  close(frame: Window): void;
  /** The demo backend, set by the first frame that loads it (the main window). */
  backend?: Backend;
}

export function demoHost(): DemoHost | null {
  if (inTauri || typeof window === "undefined" || window.parent === window) return null;
  try {
    return ((window.parent as unknown as { __prismDemoHost?: DemoHost }).__prismDemoHost) ?? null;
  } catch {
    return null; // another site framed the page
  }
}

/** Objects from the main frame belong to its realm: copy them into this one. */
function local<T>(value: T): T {
  if (value === undefined || value === null || typeof value !== "object") return value;
  try {
    return structuredClone(value);
  } catch {
    return JSON.parse(JSON.stringify(value)) as T;
  }
}

/** True while this frame is still on the page (a closed dialog's listeners stay with the backend). */
const alive = () => {
  try {
    return !!window.frameElement?.isConnected;
  } catch {
    return false;
  }
};

const VIDEO_SITE = /^https?:\/\/([\w-]+\.)*(youtube\.com|youtu\.be|vimeo\.com|twitch\.tv|tiktok\.com|dailymotion\.com)\//i;

/** The demo backend for this frame, shared through the demo page when there is one. */
export async function demoBackend(): Promise<Backend> {
  const host = demoHost();
  if (!host) return import("./mock");
  if (!host.backend) {
    host.backend = await import("./mock");
    // The site is in English, so is the demo (not the visitor's system language).
    await host.backend.invoke("update_settings", { patch: { general: { language: "en" } } });
    return withWindows(host.backend, host);
  }
  const shared = host.backend;
  return withWindows(
    {
      ...shared,
      invoke: (async (cmd: string, args?: Record<string, unknown>) => local(await shared.invoke(cmd, local(args)))) as Backend["invoke"],
      subscribe: (handler) => shared.subscribe((e) => void (alive() && handler(local(e)))),
      subscribeApp: (name, handler) => shared.subscribeApp(name, (p) => void (alive() && handler(local(p)))),
    },
    host,
  );
}

/** In the app, Rust opens these windows; on the demo page, the page opens a frame. */
function withWindows(backend: Backend, host: DemoHost): Backend {
  const invoke = (async (cmd: string, args: Record<string, unknown> = {}) => {
    switch (cmd) {
      // the demo is always the newest Prism: no update banner, no "What's new"
      case "check_update":
        return null;
      case "open_window":
        host.open(args.id ? `${String(args.kind)}/${String(args.id)}` : String(args.kind));
        return undefined;
      case "open_details":
        host.open(`details/${String(args.id)}`);
        return undefined;
      case "open_tools": {
        const r = (args.request ?? {}) as { tab?: string };
        const tab = r.tab === "shrink" ? "shrink" : "convert";
        const hex = Array.from(new TextEncoder().encode(JSON.stringify(r)), (b) => b.toString(16).padStart(2, "0")).join("");
        host.open(`tools/${tab}/${hex}`);
        return undefined;
      }
      case "add_url": {
        const url = String((args.args as { url?: string } | undefined)?.url ?? "");
        if (VIDEO_SITE.test(url)) {
          const v = (await backend.invoke("open_video_url", { url })) as { id: string };
          host.open(`video/${encodeURIComponent(v.id)}`);
          return v;
        }
        const c = (await backend.invoke("add_url", args)) as { id: string };
        host.open(`capture/${encodeURIComponent(c.id)}`);
        return c;
      }
      case "batch_from_text":
      case "batch_from_pattern": {
        const b = (await backend.invoke(cmd, args)) as { id: string };
        host.open(`batch/${encodeURIComponent(b.id)}`);
        return b;
      }
    }
    return backend.invoke(cmd, args);
  }) as Backend["invoke"];
  return { ...backend, invoke };
}
