import { MotionConfig } from "motion/react";
import { lazy, Suspense, useEffect, useState } from "react";
import { MenuHost } from "./components/MenuHost";
import { ToastHost } from "./components/ToastHost";
import { TooltipHost } from "./components/TooltipHost";
import { applyPreviewParams } from "./lib/previewParams";
import { type Route, parseRoute } from "./lib/route";
import { useApp } from "./lib/store";
import { attempt } from "./lib/ui";
import { MainWindow } from "./views/MainWindow";

// Secondary windows are separate webviews; each only loads its own chunk.
const CaptureWindow = lazy(() => import("./windows/CaptureWindow"));
const AddUrlWindow = lazy(() => import("./windows/AddUrlWindow"));
const BatchWindow = lazy(() => import("./windows/BatchWindow"));
const VideoWindow = lazy(() => import("./windows/VideoWindow"));
const TorrentWindow = lazy(() => import("./windows/TorrentWindow"));
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

  useEffect(() => {
    const on = () => setRoute(parseRoute(location.hash));
    window.addEventListener("hashchange", on);
    void attempt(useApp.getState().init(), "Prism could not start").then(applyPreviewParams);
    return () => window.removeEventListener("hashchange", on);
  }, []);

  useEffect(() => {
    document.documentElement.dataset.window = route.name;
  }, [route.name]);

  return (
    <MotionConfig reducedMotion={reduced === "reduced" ? "always" : reduced === "full" ? "never" : "user"}>
      {ready && (
        <Suspense fallback={null}>
          <View route={route} />
        </Suspense>
      )}
      <TooltipHost />
      <MenuHost />
      <ToastHost />
    </MotionConfig>
  );
}
