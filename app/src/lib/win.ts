// Current-window controls. Outside Tauri they are no-ops, except web-mode close returning to main.

import { inTauri, webMode } from "./api";

async function current() {
  if (!inTauri) return null;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  return getCurrentWindow();
}

export const win = {
  show: async () => {
    const w = await current();
    if (w) {
      await w.show();
      await w.setFocus();
    }
  },
  /** Closes this window for good (dialogs are destroyed, not kept hidden); the main window
   * goes through the normal close request (to the tray, if enabled). */
  close: async () => {
    if (webMode) {
      location.hash = "";
      return;
    }
    const w = await current();
    if (!w) return;
    if (w.label === "main") await w.close();
    else await w.destroy();
  },
  minimize: async () => (await current())?.minimize(),
  toggleMaximize: async () => (await current())?.toggleMaximize(),
  isMaximized: async () => (await (await current())?.isMaximized()) ?? false,
  onResized: async (cb: () => void) => (await current())?.onResized(cb),
  setAlwaysOnTop: async (on: boolean) => (await current())?.setAlwaysOnTop(on),
};

/** Folder picker (plugin-dialog). Returns null when cancelled or outside Tauri. */
export async function pickFolder(defaultPath?: string): Promise<string | null> {
  if (!inTauri) return null;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const r = await open({ directory: true, multiple: false, defaultPath });
  return typeof r === "string" ? r : null;
}

export async function pickFile(filters: { name: string; extensions: string[] }[]): Promise<string | null> {
  if (!inTauri) return null;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const r = await open({ directory: false, multiple: false, filters });
  return typeof r === "string" ? r : null;
}

export async function pickSavePath(defaultPath: string, filters: { name: string; extensions: string[] }[]): Promise<string | null> {
  if (!inTauri) return null;
  const { save } = await import("@tauri-apps/plugin-dialog");
  return save({ defaultPath, filters });
}

// The clipboard goes through Rust in the app: WebView2 would show a permission prompt
// for navigator.clipboard.readText().
export async function copyText(text: string) {
  try {
    if (inTauri) {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("clipboard_write", { text });
    } else {
      await navigator.clipboard.writeText(text);
    }
    return true;
  } catch {
    return false;
  }
}

export async function readClipboardText(): Promise<string> {
  try {
    if (inTauri) {
      const { invoke } = await import("@tauri-apps/api/core");
      return await invoke<string>("clipboard_read");
    }
    return await navigator.clipboard.readText();
  } catch {
    return "";
  }
}
