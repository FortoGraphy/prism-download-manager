// Tools from the main window and the completion window: opening the Tools window, and the
// quick conversions of the list's menu (they run right away and report in toasts).

import { api, onCoreEvent, webMode } from "./api";
import { t } from "./i18n";
import { baseName, kindByName, type MediaTarget, problemText, TARGET_NAMES, type ToolsRequest } from "./media";
import { fullPath } from "./paths";
import { useApp } from "./store";
import type { DownloadItem } from "./types";
import { attempt, toast } from "./ui";

export const openTools = (request: ToolsRequest) => void attempt(api.openTools(request), t("Could not open Tools"));

/** The file of a finished download, if it's a picture, music or a video. */
export function mediaPath(item: DownloadItem | undefined | null): string | null {
  if (!item || item.status !== "completed" || item.trashed || kindByName(item.fileName) === "other") return null;
  return fullPath(item.saveDir, item.fileName);
}

/** The selected finished pictures, music and videos (all of the selection, or none). */
export function selectedMediaPaths(): string[] {
  const st = useApp.getState();
  const paths = st.selection.map((id) => mediaPath(st.items[id]));
  return paths.length && paths.every(Boolean) ? (paths as string[]) : [];
}

/** Jobs started from the menu, reported when they end. */
const watching = new Map<string, MediaTarget>();
let listening = false;

function listen() {
  if (listening) return;
  listening = true;
  void onCoreEvent((e) => {
    if (e.type !== "mediaJob" || !watching.has(e.payload.id)) return;
    const j = e.payload;
    const target = watching.get(j.id)!;
    if (j.state === "done" && j.output) {
      watching.delete(j.id);
      const out = j.output;
      toast(t("Converted to {format}", { format: TARGET_NAMES[target] }), {
        kind: "success",
        body: baseName(out),
        ...(webMode ? {} : { action: { label: t("Show in folder"), run: () => void attempt(api.revealPath(out), t("Could not open the folder")) } }),
        ms: 6000,
      });
    } else if (j.state === "failed") {
      watching.delete(j.id);
      toast(t("Couldn’t convert {name}", { name: j.name }), { kind: "error", body: problemText(j.problem ?? "failed", { target }) });
    } else if (j.state === "cancelled") {
      watching.delete(j.id);
    }
  });
}

/** Convert a finished download right away (the list's "Convert to ▸ MP3"). */
export async function quickConvert(item: DownloadItem, target: MediaTarget) {
  const path = mediaPath(item);
  if (!path) return;
  listen();
  const jobs = await attempt(api.mediaStart({ op: "convert", paths: [path], target }), t("Could not start the conversion"));
  if (!jobs?.length) return;
  watching.set(jobs[0].id, target);
  toast(t("Converting to {format}…", { format: TARGET_NAMES[target] }), {
    body: item.fileName,
    action: { label: t("Show progress"), run: () => openTools({ tab: "convert" }) },
  });
}
