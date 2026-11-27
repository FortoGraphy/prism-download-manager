import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DownloadItem, Settings } from "./types";

const completed: DownloadItem = {
  id: "a",
  kind: "http",
  url: "https://example.com/file.zip",
  fileName: "file.zip",
  saveDir: "/downloads",
  size: 1000,
  downloaded: 1000,
  status: "completed",
  category: "compressed",
  description: "",
  addedAt: 0,
  connections: 8,
  speedLimit: 0,
  trashed: false,
  position: 0,
  mirrors: [],
};

describe("activate in prism-web", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.stubGlobal("window", { __PRISM_WEB__: {} });
    vi.stubGlobal("location", { hash: "" });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("shows details for a completed item instead of calling the no-op open_item", async () => {
    const fetchMock = vi.fn(() => {
      throw new Error("open_item must not be sent from prism-web");
    });
    vi.stubGlobal("fetch", fetchMock);
    const { actions } = await import("./actions");
    const { useApp } = await import("./store");
    useApp.setState({ items: { a: completed } });

    actions.activate("a");

    expect(location.hash).toBe("#/details/a");
    expect(fetchMock).not.toHaveBeenCalled();
  });
});

describe("prism-web settings recovery", () => {
  const base = {
    antivirus: { kind: "auto", command: "", args: "" },
    video: { ytdlpPath: "", ffmpegPath: "" },
  } as Settings;

  it("detects each desktop-only value hidden in prism-web", async () => {
    const { hasBlockedWebSettings } = await import("./actions");
    expect(hasBlockedWebSettings(base)).toBe(false);
    expect(hasBlockedWebSettings({ ...base, antivirus: { ...base.antivirus, kind: "custom" } })).toBe(true);
    expect(hasBlockedWebSettings({ ...base, antivirus: { ...base.antivirus, command: "scan.exe" } })).toBe(true);
    expect(hasBlockedWebSettings({ ...base, antivirus: { ...base.antivirus, args: '"[FILE]"' } })).toBe(true);
    expect(hasBlockedWebSettings({ ...base, video: { ...base.video, ytdlpPath: "/opt/yt-dlp" } })).toBe(true);
    expect(hasBlockedWebSettings({ ...base, video: { ...base.video, ffmpegPath: "/opt/ffmpeg" } })).toBe(true);
  });

  it("clears all executable settings in one patch", async () => {
    const { WEB_SETTINGS_RECOVERY_PATCH } = await import("./actions");
    expect(WEB_SETTINGS_RECOVERY_PATCH).toEqual({
      antivirus: { kind: "auto", command: "", args: "" },
      video: { ytdlpPath: "", ffmpegPath: "" },
    });
  });
});
