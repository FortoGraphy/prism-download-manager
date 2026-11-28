import { AnimatePresence, motion } from "motion/react";
import { useEffect } from "react";
import { handleDrop } from "../lib/drop";
import { win } from "../lib/win";
import { showAfterPaint } from "../lib/show-window";
import { Icon } from "../design/icons";
import { actions } from "../lib/actions";
import { api } from "../lib/api";
import { t } from "../lib/i18n";
import { useApp } from "../lib/store";
import { attempt, toast, useUi } from "../lib/ui";
import { BottomPane } from "./BottomPane";
import { useWhatsNew } from "./sheets/WhatsNewSheet";
import { ConnectBanner } from "./BrowserConnect";
import { Dialogs } from "./dialogs/Dialogs";
import { DownloadTable } from "./DownloadTable";
import { Onboarding } from "./Onboarding";
import { PostActionBanner } from "./PostActionBanner";
import { Sheets } from "./Sheets";
import { Sidebar } from "./Sidebar";
import { StatusBar } from "./StatusBar";
import { TitleRow } from "./TitleRow";
import { UpdateBanner } from "./UpdateBanner";
import { Toolbar } from "./Toolbar";
import { listIds } from "./useListing";

const isTyping = (target: EventTarget | null) => {
  const el = target as HTMLElement | null;
  return !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable);
};

export function MainWindow() {
  const bottomOpen = useUi((s) => s.bottomOpen);
  const bottomHeight = useUi((s) => s.bottomHeight);
  const dragOver = useUi((s) => s.dragOver);
  // Settings › Appearance › "Compact view": just the list (no sidebar, details pane or tips).
  const compact = useApp((s) => s.settings?.appearance.compactView ?? false);
  const detailsOpen = bottomOpen && !compact;

  useEffect(() => {
    // Shown after the first paint, unless Prism started in the background (autostart) or
    // "Start minimized" is on: then it waits in the tray.
    let cancelled = false;
    let cancelShow: (() => void) | undefined;
    const show = () => {
      if (!cancelled) cancelShow = showAfterPaint(() => void win.show());
    };
    void api.mainWindowStartsVisible().then((visible) => visible !== false && show(), show);
    return () => {
      cancelled = true;
      cancelShow?.();
    };
  }, []);

  // Settings › General › "Remove downloads whose files were deleted": checked again whenever the
  // window comes to the front (files are usually deleted while Prism is in the background).
  useEffect(() => {
    let last = 0;
    const check = () => {
      if (!useApp.getState().settings?.general.removeDeletedFiles || Date.now() - last < 10_000) return;
      last = Date.now();
      void api.removeDeletedFiles().catch(() => {});
    };
    window.addEventListener("focus", check);
    return () => window.removeEventListener("focus", check);
  }, []);

  // Square corners while maximized / fullscreen, rounded otherwise (see shell.css).
  useEffect(() => {
    const root = document.documentElement;
    root.dataset.os = /Mac/i.test(navigator.userAgent) ? "mac" : /Windows/i.test(navigator.userAgent) ? "windows" : "linux";
    const sync = () => void win.isMaximized().then((m) => root.toggleAttribute("data-maximized", m));
    sync();
    let off: (() => void) | undefined;
    void win.onResized(sync).then((u) => (off = u));
    return () => off?.();
  }, []);

  useGlobalShortcuts();
  useCoreNotifications();
  useWhatsNew();

  // ---- drag & drop of links / .torrent files anywhere on the window -------------------
  const onDragOver = (e: React.DragEvent) => {
    const types = Array.from(e.dataTransfer.types);
    if (types.includes("text/uri-list") || types.includes("text/plain") || types.includes("Files")) {
      e.preventDefault();
      e.dataTransfer.dropEffect = "copy";
      if (!useUi.getState().dragOver) useUi.getState().setDragOver(true);
    }
  };
  const onDragLeave = (e: React.DragEvent) => {
    if (e.currentTarget === e.target || !(e.relatedTarget && e.currentTarget.contains(e.relatedTarget as Node))) useUi.getState().setDragOver(false);
  };
  const onDrop = async (e: React.DragEvent) => {
    e.preventDefault();
    useUi.getState().setDragOver(false);
    await handleDrop(e.dataTransfer);
  };

  return (
    <div className={compact ? "win canvas win--compact" : "win canvas"} onDragOver={onDragOver} onDragLeave={onDragLeave} onDrop={onDrop}>
      {!compact && <Sidebar />}
      <main className="sheet" style={{ ["--bottom-h" as string]: detailsOpen ? `${bottomHeight}px` : "0px" }}>
        <Toolbar />
        <TitleRow />
        <div className="banner-stack">
          <UpdateBanner />
          {!compact && <ConnectBanner onOpen={() => useUi.getState().openSheet({ kind: "settings", section: "browser" })} />}
        </div>
        <DownloadTable />
        {!compact && <BottomPane />}
        <StatusBar />
      </main>
      <PostActionBanner />
      <AnimatePresence>
        {dragOver && (
          <motion.div className="drop-overlay" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} transition={{ duration: 0.14 }}>
            <motion.div className="drop-overlay__card glass-strong" initial={{ scale: 0.96, y: 6 }} animate={{ scale: 1, y: 0 }} transition={{ duration: 0.22, ease: [0.16, 1, 0.3, 1] }}>
              <span className="drop-overlay__icon">
                <Icon name="download" size={30} strokeWidth={1.5} />
              </span>
              <b>{t("Drop to download")}</b>
              <span>{t("Links, magnet links and .torrent files")}</span>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>
      <Sheets />
      <Dialogs />
      <Onboarding />
    </div>
  );
}

function useGlobalShortcuts() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const ui = useUi.getState();
      if (ui.sheet || ui.dialog || ui.menu || document.querySelector(".modal-scrim")) return;
      const ctrl = e.ctrlKey || e.metaKey;
      const k = e.key.toLowerCase();
      const sel = useApp.getState().selection;
      let handled = true;
      if (ctrl && e.shiftKey && k === "m") actions.toggleMini();
      else if (ctrl && k === "n") actions.openAddUrl();
      else if (ctrl && (k === "k" || k === "f")) ui.focusSearch();
      else if (ctrl && k === ",") ui.openSheet({ kind: "settings" });
      else if (ctrl && k === "l") void actions.toggleLimiter();
      else if (e.key === "F2" && sel.length) ui.openDialog({ kind: "properties", id: sel[sel.length - 1] });
      else if (isTyping(e.target)) handled = false;
      else if (ctrl && k === "a") useApp.getState().select(listIds(useApp.getState(), ui.filter));
      else if (e.key === "Delete" && sel.length) actions.requestDelete(sel, e.shiftKey);
      else handled = false;
      if (handled) e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}

/** Toasts for failures / notifications coming from the core while the main window is open. */
function useCoreNotifications() {
  useEffect(() => {
    let off: (() => void) | undefined;
    void import("../lib/api").then(({ onCoreEvent }) =>
      onCoreEvent((e) => {
        if (e.type === "failed") toast(t("Download failed"), { kind: "error", body: `${e.payload.fileName}${e.payload.error ? ` — ${e.payload.error}` : ""}` });
        else if (e.type === "queueFinished") toast(t("{name} finished", { name: e.payload.name }), { kind: "success" });
        else if (e.type === "notify") toast(e.payload.title, { body: e.payload.body });
        else if (e.type === "clipboardUrls" && e.payload.length)
          toast(e.payload.length > 1 ? t("{count} links copied", { count: e.payload.length }) : t("Link copied"), {
            body: e.payload[0],
            // The core opens the Download Links window for the batch itself (#37).
            action: { label: t("Download"), run: () => (e.payload.length > 1 ? void attempt(api.batchFromText(e.payload.join("\n")), t("Could not add the links")) : void actions.addUrl(e.payload[0])) },
          });
      }).then((u) => (off = u)),
    );
    return () => off?.();
  }, []);
}
