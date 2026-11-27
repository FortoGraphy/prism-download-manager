import { afterEach, describe, expect, it, vi } from "vitest";

const MAC = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)";
const WINDOWS = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko)";

/** `platform` decides once, at import, which system it runs on. */
async function load(userAgent: string) {
  vi.resetModules();
  vi.stubGlobal("navigator", { userAgent, language: "en", languages: ["en"] });
  return import("./platform");
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("platform wording", () => {
  it("keeps Windows and Linux texts and shortcuts as they are", async () => {
    const p = await load(WINDOWS);
    expect(p.IS_MAC).toBe(false);
    expect(p.keyLabel("Ctrl Shift M")).toBe("Ctrl Shift M");
    expect(p.keyLabel("Del")).toBe("Del");
    expect(p.showInFolderLabel()).toBe("Show in folder");
    expect(p.openFolderLabel()).toBe("Open folder");
    expect(p.propertiesLabel()).toBe("Properties…");
    expect(p.propertiesTitle()).toBe("Properties");
  });

  it("uses Finder, Get Info and Mac key glyphs on macOS", async () => {
    const p = await load(MAC);
    expect(p.IS_MAC).toBe(true);
    expect(p.showInFolderLabel()).toBe("Show in Finder");
    expect(p.openFolderLabel()).toBe("Open in Finder");
    expect(p.propertiesLabel()).toBe("Get Info");
    expect(p.propertiesTitle()).toBe("Info");
  });

  it("writes shortcuts the Mac way, modifiers in Apple's order", async () => {
    const { keyLabel } = await load(MAC);
    expect(keyLabel("Ctrl N")).toBe("⌘ N");
    expect(keyLabel("Ctrl ,")).toBe("⌘ ,");
    expect(keyLabel("Ctrl Shift M")).toBe("⇧ ⌘ M");
    expect(keyLabel("Ctrl K / Ctrl F")).toBe("⌘ K / ⌘ F");
    expect(keyLabel("Alt ↑")).toBe("⌥ ↑");
    // Keys a Mac keyboard doesn't have map to the shortcuts the app adds there.
    expect(keyLabel("Del")).toBe("⌘ ⌫");
    expect(keyLabel("Delete")).toBe("⌘ ⌫");
    expect(keyLabel("F2")).toBe("⌘ I");
    expect(keyLabel("Enter")).toBe("Return");
    expect(keyLabel("Space")).toBe("Space");
    expect(keyLabel("Esc")).toBe("Esc");
    expect(keyLabel("↑ ↓ Home End")).toBe("↑ ↓ Home End");
  });
});
