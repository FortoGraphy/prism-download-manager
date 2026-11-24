import { useState } from "react";
import { Checkbox } from "../components/Controls";
import { FileTile } from "../components/Visuals";
import { WindowFrame } from "../components/WindowFrame";
import { Icon } from "../design/icons";
import { patchSettings } from "../lib/actions";
import { api } from "../lib/api";
import { formatBytes, formatDuration, formatSpeed } from "../lib/format";
import { t } from "../lib/i18n";
import { fullPath } from "../lib/paths";
import { useApp } from "../lib/store";
import { mediaPath } from "../lib/tools";
import { attempt, toast } from "../lib/ui";
import { copyText, win } from "../lib/win";

export default function CompleteWindow({ id }: { id: string }) {
  const item = useApp((s) => s.items[id]);
  const [dontShow, setDontShow] = useState(false);

  const close = async () => {
    if (dontShow) await patchSettings({ general: { showCompletionDialog: false } });
    await win.close();
  };
  const run = async (p: Promise<void>, fail: string) => {
    await attempt(p, fail);
    await close();
  };

  const took = item?.finishedAt && item.lastTryAt ? (item.finishedAt - item.lastTryAt) / 1000 : null;
  const size = item?.size ?? item?.downloaded ?? 0;
  const path = item ? fullPath(item.saveDir, item.fileName) : "";
  const media = mediaPath(item);

  // Native drag-out: the webview starts an HTML drag, we cancel it and hand the
  // still-pressed mouse to the OS with the real file, so it drops into WhatsApp,
  // a mail, Explorer… exactly like dragging it from a folder.
  // Windows and macOS: Linux has no drag-out yet (the command says so), so nothing to grab there.
  const canDrag = !!item && item.status === "completed" && !/Linux/i.test(navigator.userAgent);
  const onDragStart = (e: React.DragEvent) => {
    e.preventDefault();
    if (canDrag) void attempt(api.startFileDrag(id), t("Could not drag the file"));
  };
  const dragProps = canDrag ? { draggable: true, onDragStart, "data-tip": t("Drag the file into a chat, an e-mail or a folder") } : {};

  return (
    <WindowFrame kind="complete" title={t("Download complete")} onClose={() => void close()} className="complete">
      <div className="complete__top">
        <div className={canDrag ? "burst is-draggable" : "burst"} {...dragProps}>
          <span>
            <Icon name="check" size={28} strokeWidth={2} />
          </span>
        </div>
        <div className="complete__txt">
          <h2 className="ellipsis" title={item?.fileName}>
            {item?.fileName ?? t("Download complete")}
          </h2>
          <div className="meta tnum">
            {formatBytes(size)}
            {took && took > 0 ? ` · ${t("finished in {duration} · average {speed}", { duration: formatDuration(took), speed: formatSpeed(size / took) })}` : ""}
          </div>
        </div>
      </div>
      {item && (
        <div className={canDrag ? "complete__path is-draggable" : "complete__path"} {...dragProps}>
          <FileTile item={item} size={24} thumbWidth={36} />
          <span className="t-mono ellipsis" title={path}>
            {path}
          </span>
          {canDrag && <Icon name="drag-handle" size={15} className="complete__grip" />}
        </div>
      )}
      {media && (
        <div className="complete__tools">
          <button type="button" className="btn btn--ghost btn--sm" onClick={() => void run(api.openTools({ tab: "convert", paths: [media] }), t("Could not open Tools"))}>
            <Icon name="convert" size={14} /> {t("Convert")}
          </button>
          <button type="button" className="btn btn--ghost btn--sm" onClick={() => void run(api.openTools({ tab: "shrink", paths: [media] }), t("Could not open Tools"))}>
            <Icon name="shrink" size={14} /> {t("Make smaller")}
          </button>
        </div>
      )}
      <footer className="dialog__footer">
        <Checkbox checked={dontShow} onChange={setDontShow}>
          {t("Don’t show again")}
        </Checkbox>
        <span className="spacer" />
        <button
          type="button"
          className="btn btn--icon"
          aria-label={t("Copy path")}
          data-tip={t("Copy path")}
          disabled={!item}
          onClick={async () => {
            if (await copyText(path)) toast(t("Path copied"), { kind: "success", ms: 1500 });
          }}
        >
          <Icon name="copy" size={15} />
        </button>
        <button type="button" className="btn" disabled={!item} onClick={() => void run(api.revealItem(id), t("Could not open the folder"))}>
          <Icon name="folder-open" size={15} /> {t("Open folder")}
        </button>
        <button type="button" className="btn btn--primary" autoFocus disabled={!item} onClick={() => void run(api.openItem(id), t("Could not open the file"))}>
          <Icon name="open-file" size={15} /> {t("Open")}
        </button>
      </footer>
    </WindowFrame>
  );
}
