import { MotionConfig } from "motion/react";
import { lazy, Suspense, useEffect, useState } from "react";
import { StartupFrame } from "./components/StartupFrame";
import { MenuHost } from "./components/MenuHost";
import { ToastHost } from "./components/ToastHost";
import { TooltipHost } from "./components/TooltipHost";
import { setLanguage, t } from "./lib/i18n";
import { applyPreviewParams } from "./lib/previewParams";
import { type Route, parseRoute } from "./lib/route";
import { api } from "./lib/api";
import { useApp } from "./lib/store";
import { attempt } from "./lib/ui";
// Share the import promise between preloading and React.lazy (including failures).
function preloadable<T extends React.ComponentType<any>>(load: () => Promise<{ default: T }>) {
  let pending: ReturnType<typeof load> | undefined;
  const preload = () => pending ??= load();
  return Object.assign(lazy(preload), { preload });
}

const MainWindow = preloadable(() => import("./views/MainWindow").then((m) => ({ default: m.MainWindow })));

// Secondary windows are separate webviews; each only loads its own chunk.
const CaptureWindow = preloadable(() => import("./windows/CaptureWindow"));
const AddUrlWindow = preloadable(() => import("./windows/AddUrlWindow"));
const BatchWindow = preloadable(() => import("./windows/BatchWindow"));
const VideoWindow = preloadable(() => import("./windows/VideoWindow"));
const TorrentWindow = preloadable(() => import("./windows/TorrentWindow"));
const ModelWindow = preloadable(() => import("./windows/ModelWindow"));
const ToolsWindow = preloadable(() => import("./windows/ToolsWindow"));
const CompleteWindow = preloadable(() => import("./windows/CompleteWindow"));
const DetailsWindow = preloadable(() => import("./windows/DetailsWindow"));
const MiniWindow = preloadable(() => import("./windows/MiniWindow"));
const DropzoneWindow = preloadable(() => import("./windows/DropzoneWindow"));

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

  useEffect(() => {
    const selected = { main: MainWindow, capture: CaptureWindow, addurl: AddUrlWindow,
      batch: BatchWindow, video: VideoWindow, torrent: TorrentWindow, model: ModelWindow,
      tools: ToolsWindow, complete: CompleteWindow, details: DetailsWindow,
      progress: DetailsWindow, mini: MiniWindow, dropzone: DropzoneWindow }[route.name];
    // React's lazy boundary handles import failures when the view is rendered.
    void selected.preload().catch(() => {});
  }, [route.name]);
  const ready = useApp((s) => s.ready);
  const reduced = useApp((s) => s.settings?.appearance.motion);
  // Set before the views render; changing the language remounts them in the new one.
  const lang = setLanguage(useApp((s) => s.settings?.general.language));

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
      {ready ? (
        <Suspense fallback={<StartupFrame />}>
          <View key={lang} route={route} />
        </Suspense>
      ) : <StartupFrame />}
      <TooltipHost />
      <MenuHost />
      <ToastHost />
    </MotionConfig>
  );
}
