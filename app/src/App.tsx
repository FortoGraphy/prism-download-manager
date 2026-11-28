import { MotionConfig } from "motion/react";
import { lazy, Suspense, useEffect, useState } from "react";
import { MenuHost } from "./components/MenuHost";
import { ToastHost } from "./components/ToastHost";
import { TooltipHost } from "./components/TooltipHost";
import { setLanguage, t } from "./lib/i18n";
import { applyPreviewParams } from "./lib/previewParams";
import { type Route, parseRoute, routeKey } from "./lib/route";
import { actions } from "./lib/actions";
import { api, onCoreEvent, webMode } from "./lib/api";
import { useApp } from "./lib/store";
import { attempt, toast } from "./lib/ui";
import { MainWindow } from "./views/MainWindow";

// Secondary windows are separate webviews; each only loads its own chunk.
const CaptureWindow = lazy(() => import("./windows/CaptureWindow"));
const AddUrlWindow = lazy(() => import("./windows/AddUrlWindow"));
const BatchWindow = lazy(() => import("./windows/BatchWindow"));
const VideoWindow = lazy(() => import("./windows/VideoWindow"));
const TorrentWindow = lazy(() => import("./windows/TorrentWindow"));
const ModelWindow = lazy(() => import("./windows/ModelWindow"));
const ToolsWindow = lazy(() => import("./windows/ToolsWindow"));
const CompleteWindow = lazy(() => import("./windows/CompleteWindow"));
const DetailsWindow = lazy(() => import("./windows/DetailsWindow"));
const MiniWindow = lazy(() => import("./windows/MiniWindow"));
const DropzoneWindow = lazy(() => import("./windows/DropzoneWindow"));

function View({ route }: { route: Route }) {
  switch (route.name) {
    case "main":
      return <MainWindow />;
    case "capture":
      return <CaptureWindow id={route.id} />;
    case "addurl":
      return <AddUrlWindow />;
    case "batch":
      return <BatchWindow id={route.id} />;
    case "video":
      return <VideoWindow id={route.id} />;
    case "torrent":
      return <TorrentWindow source={route.source} />;
    case "model":
      return <ModelWindow source={route.source} />;
    case "tools":
      return <ToolsWindow request={route.request} />;
    case "complete":
      return <CompleteWindow id={route.id} />;
    case "details":
      return <DetailsWindow id={route.id} />;
    case "progress":
      return <DetailsWindow id={route.id} simple />;
    case "mini":
      return <MiniWindow />;
    case "dropzone":
      return <DropzoneWindow />;
  }
}

export function App() {
  const [route, setRoute] = useState(() => parseRoute(location.hash));
  const ready = useApp((s) => s.ready);
  const reduced = useApp((s) => s.settings?.appearance.motion);
  // Set before the views render; changing the language remounts them in the new one.
  const lang = setLanguage(useApp((s) => s.settings?.general.language));
  useCoreNotifications();

  useEffect(() => {
    const on = () => setRoute(parseRoute(location.hash));
    window.addEventListener("hashchange", on);
    void attempt(useApp.getState().init(), t("Prism could not start")).then(applyPreviewParams);
    return () => window.removeEventListener("hashchange", on);
  }, []);

  useEffect(() => {
    document.documentElement.dataset.window = route.name;
  }, [route.name]);

  // The tray menu and notifications follow the language the main window uses.
  useEffect(() => {
    if (ready && route.name === "main") void api.setUiLanguage(lang).catch(() => {});
  }, [ready, route.name, lang]);

  return (
    <MotionConfig reducedMotion={reduced === "reduced" ? "always" : reduced === "full" ? "never" : "user"}>
      {ready && (
        <Suspense fallback={null}>
          <View key={`${lang}:${routeKey(route)}`} route={route} />
        </Suspense>
      )}
      <TooltipHost />
      <MenuHost />
      <ToastHost />
    </MotionConfig>
  );
}

/** Core toasts live above the routed views so dialog routes don't unmount them. */
function useCoreNotifications() {
  useEffect(() => {
    // Desktop toasts are handled by MainWindow; only web routes need an App-level listener.
    if (!webMode) return;
    let off: (() => void) | undefined;
    void onCoreEvent((e) => {
      if (e.type === "failed") toast(t("Download failed"), { kind: "error", body: `${e.payload.fileName}${e.payload.error ? ` — ${e.payload.error}` : ""}` });
      else if (e.type === "queueFinished") toast(t("{name} finished", { name: e.payload.name }), { kind: "success" });
      else if (e.type === "notify") toast(e.payload.title, { body: e.payload.body });
      else if (e.type === "clipboardUrls" && e.payload.length)
        toast(e.payload.length > 1 ? t("{count} links copied", { count: e.payload.length }) : t("Link copied"), {
          body: e.payload[0],
          // The core opens the Download Links window for the batch itself (#37).
          action: { label: t("Download"), run: () => (e.payload.length > 1 ? void attempt(api.batchFromText(e.payload.join("\n")), t("Could not add the links")) : void actions.addUrl(e.payload[0])) },
        });
    }).then((u) => (off = u));
    return () => off?.();
  }, []);
}
