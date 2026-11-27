import { afterEach, describe, expect, it, vi } from "vitest";

const LINUX = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120 Safari/537.36";

describe("NATIVE_SOUNDS", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("uses browser audio in prism-web, even on Linux", async () => {
    vi.resetModules();
    vi.stubGlobal("navigator", { userAgent: LINUX });
    vi.stubGlobal("window", { __PRISM_WEB__: {} });

    const { NATIVE_SOUNDS } = await import("./sounds");

    expect(NATIVE_SOUNDS).toBe(false);
  });

  it("keeps native Linux on the system player", async () => {
    vi.resetModules();
    vi.stubGlobal("navigator", { userAgent: LINUX });
    vi.stubGlobal("window", {});

    const { NATIVE_SOUNDS } = await import("./sounds");

    expect(NATIVE_SOUNDS).toBe(true);
  });
});
