import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hexEncode } from "./route";
import type { CoreEvent } from "./types";

describe("prism-web UI", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.stubGlobal("window", { __PRISM_WEB__: {} });
    vi.stubGlobal("location", { hash: "" });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("routes dialog events to their hash routes", async () => {
    const { handleEvent } = await import("./store");
    handleEvent({ type: "captureRequested", payload: { id: "cap1" } } as CoreEvent);
    expect(location.hash).toBe("#/capture/cap1");
    handleEvent({ type: "batchRequested", payload: { id: "batch1" } } as CoreEvent);
    expect(location.hash).toBe("#/batch/batch1");
    handleEvent({ type: "videoRequested", payload: { id: "video1" } } as CoreEvent);
    expect(location.hash).toBe("#/video/video1");
    handleEvent({ type: "openModel", payload: { source: "org/model" } });
    expect(location.hash).toBe(`#/model/${hexEncode("org/model")}`);
    handleEvent({ type: "openTorrent", payload: { source: "magnet:?xt=abc" } });
    expect(location.hash).toBe(`#/torrent/${hexEncode("magnet:?xt=abc")}`);
    handleEvent({ type: "showMain" });
    expect(location.hash).toBe("");
  });

  it("opens hash-routed windows without calling the no-op server command", async () => {
    const fetchMock = vi.fn(() => {
      throw new Error("open_window must not call HTTP in web mode");
    });
    vi.stubGlobal("fetch", fetchMock);
    const { api } = await import("./api");
    await api.openWindow("addurl");
    expect(location.hash).toBe("#/addurl");
    await api.openWindow("capture", "cap1");
    expect(location.hash).toBe("#/capture/cap1");
    await api.openDetails("item1");
    expect(location.hash).toBe("#/details/item1");
    await api.openTools({ tab: "convert" });
    expect(location.hash).toBe("#/tools/convert");
    await api.openTools({ tab: "shrink", paths: ["/srv/video.mp4"], preset: "email" });
    expect(location.hash.startsWith("#/tools/shrink/")).toBe(true);
    await api.toggleMini();
    expect(location.hash).toBe("#/mini");
    await api.toggleDropzone();
    expect(location.hash).toBe("#/dropzone");
    await api.showMain();
    expect(location.hash).toBe("");
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("leaves other core events on the current route", async () => {
    const { handleEvent } = await import("./store");
    location.hash = "#/capture/cap1";
    handleEvent({ type: "notify", payload: { title: "done", body: "ok" } });
    handleEvent({ type: "captureUpdated", payload: { id: "cap1" } } as CoreEvent);
    expect(location.hash).toBe("#/capture/cap1");
  });

  it("closes a routed dialog back to the main route", async () => {
    const { win } = await import("./win");
    location.hash = "#/batch/batch1";
    await win.close();
    expect(location.hash).toBe("");
  });

  it("does not route core events outside web mode", async () => {
    vi.stubGlobal("window", {});
    vi.resetModules();
    const { handleEvent } = await import("./store");
    handleEvent({ type: "captureRequested", payload: { id: "cap1" } } as CoreEvent);
    expect(location.hash).toBe("");
  });
});
