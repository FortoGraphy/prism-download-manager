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
