import { useVirtualizer } from "@tanstack/react-virtual";
import { type RefObject, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Icon } from "../design/icons";
import { api, inTauri, onCoreEvent, webMode } from "../lib/api";
import { formatBytes } from "../lib/format";
import { t } from "../lib/i18n";
import { fileVisual, formatPct } from "../lib/present";
import { buildTree, type TreeNode } from "../lib/tree";
import type { ZipFile } from "../lib/types";
import { attempt } from "../lib/ui";
import { ProgressBar } from "./Visuals";

/** A file downloaded on its own: on its way, saved, or not saved (with the reason's code). */
type Save = { state: "saving"; done: number } | { state: "saved"; path: string } | { state: "failed"; problem: string };

/** Files from this size on show their progress; smaller ones are done in a moment. */
const SHOW_PROGRESS = 4 * 1024 * 1024;
const ROW_HEIGHT = 28;

const codeOf = (e: unknown) => String(e instanceof Error ? e.message : e);

function commonProblem(code: string): string {
  switch (code) {
    case "zip:protected":
      return t("The server didn’t let Prism in. The download may need the browser’s sign-in.");
    case "zip:changed":
      return t("The file on the server has changed. Close this window and add the download again.");
    case "zip:network":
      return t("Couldn’t reach the server. Check the connection and try again.");
    default:
      return t("The server didn’t answer as expected. Try again later.");
  }
}

/** Why the list can't be shown (the download itself still works). */
function listProblem(code: string): string {
  switch (code) {
    case "zip:noRanges":
      return t("The server sends this file only as a whole, so its contents can’t be listed before downloading.");
    case "zip:notZip":
      return t("This doesn’t look like a ZIP archive, so there’s no list of files to show.");
    case "zip:split":
      return t("This archive is split into parts, and its list of files isn’t in this one.");
    case "zip:tooBig":
      return t("This archive holds too many files to list here.");
    case "zip:damaged":
      return t("The archive’s list of files is damaged.");
    default:
      return commonProblem(code);
  }
}

/** Why a file wasn't saved. */
function fileProblem(code: string): string {
  switch (code) {
    case "zip:damaged":
      return t("The file came out damaged, so it wasn’t kept.");
    case "zip:disk":
      return t("Couldn’t save the file in this folder.");
    case "zip:threat":
      return t("Your antivirus found a threat in this file, so it wasn’t kept.");
    default:
      return commonProblem(code);
  }
}

/** The tree's rows as shown: open folders' contents after them. */
function visibleRows(root: TreeNode, open: Set<string>): { node: TreeNode; depth: number }[] {
  const rows: { node: TreeNode; depth: number }[] = [];
  const walk = (node: TreeNode, depth: number) => {
    for (const child of node.children) {
      rows.push({ node: child, depth });
      if (child.children.length && open.has(child.path)) walk(child, depth + 1);
    }
  };
  walk(root, 0);
  return rows;
}

/** An archive that keeps everything in one folder (`project-1.2/…`) opens on its contents. */
function openedAtFirst(root: TreeNode): Set<string> {
  const open = new Set<string>();
  let node = root;
  while (node.children.length === 1 && node.children[0].children.length) {
    node = node.children[0];
    open.add(node.path);
  }
  return open;
}

/**
 * The Download File window has a fixed size: while the list is open it grows by what its body no
 * longer fits (moving up where the screen ends), and goes back to its size once it fits again,
 * so only the list scrolls. The browser preview's staged window does the same.
 */
function useWindowFitsBody(ref: RefObject<HTMLElement | null>, open: boolean) {
  const opened = useRef(open);
  useLayoutEffect(() => {
    opened.current = open;
  }, [open]);
  useEffect(() => {
    const body = ref.current?.closest<HTMLElement>(".wbody");
    if (!ref.current || !body) return;
    let base: number | null = null;
    const fit = async () => {
      const last = body.lastElementChild;
      if (!last) return;
      // How much taller the body's content is than the body (negative: room to spare).
      const content = last.getBoundingClientRect().bottom - body.getBoundingClientRect().top + body.scrollTop;
      const need = Math.ceil(content + parseFloat(getComputedStyle(body).paddingBottom) - body.clientHeight);
      const fitted = (height: number) => (opened.current ? Math.max(base ?? height, height + need) : (base ?? height));
      if (!inTauri) {
        const staged = body.closest<HTMLElement>(".preview-window");
        if (staged) {
          base ??= staged.offsetHeight;
          staged.style.height = `${fitted(staged.offsetHeight)}px`;
        }
        return;
      }
      const { currentMonitor, getCurrentWindow, LogicalSize, PhysicalPosition } = await import("@tauri-apps/api/window");
      const w = getCurrentWindow();
      const [scale, size, pos, monitor] = await Promise.all([w.scaleFactor(), w.innerSize(), w.outerPosition(), currentMonitor()]);
      const height = size.height / scale;
      base ??= height;
      let target = fitted(height);
      let y = pos.y;
      if (monitor) {
        const { position: top, size: area } = monitor.workArea;
        target = Math.min(target, area.height / scale);
        const bottom = top.y + area.height;
        if (y + target * scale > bottom) y = Math.max(top.y, Math.round(bottom - target * scale));
      }
      if (Math.abs(target - height) < 1) return;
      if (y !== pos.y) await w.setPosition(new PhysicalPosition(pos.x, y));
      await w.setSize(new LogicalSize(size.width / scale, target));
    };
    // One fit at a time, each with fresh measurements.
    let queue = Promise.resolve();
    const observer = new ResizeObserver(() => {
      queue = queue.then(fit).catch(() => undefined);
    });
    observer.observe(ref.current);
    return () => observer.disconnect();
  }, [ref]);
}

/**
 * "What’s inside" a ZIP in the Download File window: nothing is read until it's opened, then
 * only the archive's list of files (from its end). Any file can be downloaded on its own into
 * the folder the window shows.
 */
export function ZipContents({ captureId, dir }: { captureId: string; dir: string }) {
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [files, setFiles] = useState<ZipFile[] | null>(null);
  const [problem, setProblem] = useState("");
  const [folders, setFolders] = useState<Set<string>>(new Set());
  const [saves, setSaves] = useState<Record<number, Save>>({});
  const rootRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  useWindowFitsBody(rootRef, open);

  useEffect(() => {
    let off: (() => void) | undefined;
    void onCoreEvent((e) => {
      if (e.type !== "zipProgress" || e.payload.captureId !== captureId) return;
      const { index, done } = e.payload;
      setSaves((s) => (s[index]?.state === "saving" ? { ...s, [index]: { state: "saving", done } } : s));
    }).then((u) => (off = u));
    return () => off?.();
  }, [captureId]);

  const tree = useMemo(() => (files ? buildTree(files) : null), [files]);
  const byIndex = useMemo(() => new Map((files ?? []).map((f) => [f.index, f])), [files]);
  const rows = useMemo(() => (tree ? visibleRows(tree, folders) : []), [tree, folders]);
  const unpacked = useMemo(() => (files ?? []).reduce((sum, f) => sum + f.size, 0), [files]);

  const virt = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 8,
    getItemKey: (i) => rows[i].node.path,
  });

  const load = async () => {
    setLoading(true);
    setProblem("");
    try {
      const list = await api.zipContents(captureId);
      setFolders(openedAtFirst(buildTree(list)));
      setFiles(list);
    } catch (e) {
      setProblem(codeOf(e));
    } finally {
      setLoading(false);
    }
  };

  // Opening it again after a problem tries again.
  const toggle = () => {
    if (!open && !files && !loading) void load();
    setOpen(!open);
  };

  const toggleFolder = (path: string) => {
    const next = new Set(folders);
    if (!next.delete(path)) next.add(path);
    setFolders(next);
  };

  const save = async (f: ZipFile) => {
    setSaves((s) => ({ ...s, [f.index]: { state: "saving", done: 0 } }));
    try {
      const path = await api.extractZipFile(captureId, f.index, dir);
      setSaves((s) => ({ ...s, [f.index]: { state: "saved", path } }));
    } catch (e) {
      const code = codeOf(e);
      setSaves((s) => {
        const next = { ...s };
        if (code === "zip:cancelled") delete next[f.index];
        else next[f.index] = { state: "failed", problem: code };
        return next;
      });
    }
  };

  const folderRow = (node: TreeNode) => {
    const isOpen = folders.has(node.path);
    return (
      <>
        <button type="button" className="zrow__name" aria-expanded={isOpen} onClick={() => toggleFolder(node.path)}>
          <span className="zrow__twist">
            <Icon name={isOpen ? "chevron-down" : "chevron-right"} size={12} strokeWidth={2.2} />
          </span>
          <Icon name="folder" size={15} className="text-tertiary" />
          <span className="ellipsis">{node.name}</span>
        </button>
        <span className="zrow__meta tnum">
          {t("{count} files", { count: node.files.length })} · {formatBytes(node.size)}
        </span>
        <span />
      </>
    );
  };

  const fileRow = (node: TreeNode, f: ZipFile) => {
    const s = saves[f.index];
    const big = f.size >= SHOW_PROGRESS;
    let meta: React.ReactNode = formatBytes(f.size);
    let action: React.ReactNode = null;
    if (s?.state === "saved") {
      if (!webMode)
        meta = (
          <button type="button" className="zrow__link" onClick={() => void attempt(api.revealPath(s.path), t("Could not open the folder"))}>
            {t("Show in folder")}
          </button>
        );
      action = <Icon name="check" size={15} strokeWidth={2.4} className="text-success" />;
    } else if (s?.state === "saving") {
      const fraction = f.size ? Math.min(1, s.done / f.size) : null;
      if (big) {
        meta = (
          <>
            <ProgressBar value={fraction} state="active" size="thin" label={t("{name} progress", { name: node.name })} />
            {formatPct(fraction == null ? null : fraction * 100)}
          </>
        );
      }
      action = big ? (
        <button type="button" className="zrow__act" aria-label={t("Stop")} data-tip={t("Stop")} onClick={() => void api.cancelZipFile(captureId, f.index)}>
          <Icon name="close" size={13} strokeWidth={2.2} />
        </button>
      ) : (
        <Icon name="retry" size={13} strokeWidth={2.2} className="anim-spin text-tertiary" />
      );
    } else if (f.extractable) {
      action = (
        <button type="button" className="zrow__act" aria-label={t("Download just this file")} data-tip={t("Download just this file")} disabled={!dir} onClick={() => void save(f)}>
          <Icon name="download" size={14} />
        </button>
      );
    } else if (f.encrypted) {
      action = (
        <span className="zrow__act" data-tip={t("Protected with a password")} aria-label={t("Protected with a password")}>
          <Icon name="lock-closed" size={13} className="text-tertiary" />
        </span>
      );
    }
    return (
      <>
        <span className="zrow__name">
          <span className="zrow__twist" />
          <Icon name={fileVisual({ fileName: node.name, kind: "http", category: "" }).icon} size={15} className="text-tertiary" />
          <span className="ellipsis" title={f.path}>
            {node.name}
          </span>
        </span>
        <span className="zrow__meta tnum">{meta}</span>
        <span className="zrow__cell">{action}</span>
        {s?.state === "failed" && <span className="zrow__err">{fileProblem(s.problem)}</span>}
      </>
    );
  };

  return (
    <div className="zipin" ref={rootRef}>
      <button type="button" className="disclose zipin__head" aria-expanded={open} onClick={toggle}>
        <Icon name={open ? "chevron-down" : "chevron-right"} size={12} strokeWidth={2.2} /> {t("What’s inside")}
        {files && (
          <span className="zipin__sum tnum">
            · {t("{count} files", { count: files.length })} · {t("{size} unpacked", { size: formatBytes(unpacked) })}
          </span>
        )}
      </button>
      {open &&
        (loading ? (
          <p className="zipin__note">
            <Icon name="retry" size={12} strokeWidth={2.2} className="anim-spin" /> {t("Reading the list of files…")}
          </p>
        ) : problem ? (
          <p className="zipin__note">
            <Icon name="info" size={13} /> {listProblem(problem)}
          </p>
        ) : files && !files.length ? (
          <p className="zipin__note">{t("This archive is empty.")}</p>
        ) : files ? (
          <div className="zipin__list" ref={scrollRef} role="tree" aria-label={t("What’s inside")}>
            <div className="zipin__rows" style={{ height: virt.getTotalSize() }}>
              {virt.getVirtualItems().map((v) => {
                const { node, depth } = rows[v.index];
                const folder = node.children.length > 0;
                const file = folder ? undefined : byIndex.get(node.files[0]);
                return (
                  <div
                    key={v.key}
                    ref={virt.measureElement}
                    data-index={v.index}
                    className="zrow"
                    role="treeitem"
                    aria-level={depth + 1}
                    style={{ transform: `translateY(${v.start}px)`, paddingInlineStart: 8 + depth * 16 }}
                  >
                    {folder ? folderRow(node) : file ? fileRow(node, file) : null}
                  </div>
                );
              })}
            </div>
          </div>
        ) : null)}
    </div>
  );
}
