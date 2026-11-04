import { describe, expect, it, beforeEach } from "vitest";
import {
  THEME_PRESETS,
  applyCustomThemeTokens,
  clearInjectedThemeTokens,
  createNewCustomTheme,
  deleteUserTheme,
  findThemeById,
  getAllThemes,
  getUserThemes,
  hexToRgbTriplet,
  saveUserTheme,
} from "./customThemes";

const storage = new Map<string, string>();
if (typeof globalThis.localStorage === "undefined" || !globalThis.localStorage.clear) {
  (globalThis as any).localStorage = {
    getItem: (k: string) => storage.get(k) ?? null,
    setItem: (k: string, v: string) => storage.set(k, String(v)),
    removeItem: (k: string) => storage.delete(k),
    clear: () => storage.clear(),
  };
}

const docStyles = new Map<string, string>();
const elements = new Map<string, any>();
if (typeof globalThis.document === "undefined") {
  (globalThis as any).document = {};
}
const doc = globalThis.document as any;
doc.documentElement = doc.documentElement ?? {
  dataset: {} as Record<string, string>,
  style: {
    setProperty: (k: string, v: string) => docStyles.set(k, v),
    getPropertyValue: (k: string) => docStyles.get(k) ?? "",
    removeProperty: (k: string) => docStyles.delete(k),
  },
};
doc.getElementById = (id: string) => elements.get(id) ?? null;
doc.createElement = (_tag: string) => {
  const el = {
    id: "",
    textContent: "",
    remove: () => {
      if (el.id) elements.delete(el.id);
    },
  };
  return el;
};
doc.head = doc.head ?? {
  appendChild: (el: any) => {
    if (el.id) elements.set(el.id, el);
  },
};

describe("customThemes", () => {
  beforeEach(() => {
    clearInjectedThemeTokens();
    docStyles.clear();
  });

  it("converts hex to RGB triplet accurately", () => {
    expect(hexToRgbTriplet("#6461f2")).toBe("100 97 242");
    expect(hexToRgbTriplet("#ffffff")).toBe("255 255 255");
    expect(hexToRgbTriplet("#000000")).toBe("0 0 0");
    expect(hexToRgbTriplet("#fff")).toBe("255 255 255");
    expect(hexToRgbTriplet("invalid")).toBe("124 120 255");
  });

  it("loads built-in curated theme presets", () => {
    expect(THEME_PRESETS.length).toBeGreaterThanOrEqual(6);
    const all = getAllThemes();
    expect(all.length).toBeGreaterThanOrEqual(6);
    expect(all.some((t) => t.id === "catppuccin-mocha")).toBe(true);
    expect(all.some((t) => t.id === "tokyo-night")).toBe(true);
    expect(all.some((t) => t.id === "dracula")).toBe(true);
    expect(all.some((t) => t.id === "one-dark")).toBe(true);
    expect(all.some((t) => t.id === "nord-frost")).toBe(true);
    expect(all.some((t) => t.id === "oled-black")).toBe(true);
    expect(all.some((t) => t.id === "rose-pine")).toBe(true);
    expect(all.some((t) => t.id === "catppuccin-latte")).toBe(true);
  });

  it("finds a theme preset by its identifier", () => {
    const mocha = findThemeById("catppuccin-mocha");
    expect(mocha).toBeDefined();
    expect(mocha?.name).toBe("Catppuccin Mocha");
    expect(mocha?.base).toBe("dark");

    const oled = findThemeById("oled-black");
    expect(oled).toBeDefined();
    expect(oled?.colors.bgCanvas).toBe("#000000");

    expect(findThemeById("non-existent-theme")).toBeUndefined();
  });

  it("creates, saves, retrieves, and deletes user custom themes", () => {
    localStorage.clear();
    expect(getUserThemes()).toEqual([]);

    const created = createNewCustomTheme("dimmed", "Midnight Teal", {
      accent: "#14b8a6",
      bgCanvas: "#111827",
    });

    expect(created.name).toBe("Midnight Teal");
    expect(created.base).toBe("dimmed");
    expect(created.colors.accent).toBe("#14b8a6");
    expect(created.colors.accentRgb).toBe("20 184 166");

    saveUserTheme(created);
    const userThemes = getUserThemes();
    expect(userThemes.length).toBe(1);
    expect(userThemes[0].id).toBe(created.id);

    const found = findThemeById(created.id);
    expect(found).toBeDefined();
    expect(found?.name).toBe("Midnight Teal");

    const deleted = deleteUserTheme(created.id);
    expect(deleted).toBe(true);
    expect(getUserThemes()).toEqual([]);
    expect(findThemeById(created.id)).toBeUndefined();
  });

  it("applies custom theme tokens into DOM document head and dataset", () => {
    const theme = createNewCustomTheme("dark", "Neon Matrix", {
      accent: "#00ff66",
      bgCanvas: "#050a05",
    });
    applyCustomThemeTokens(theme);
    expect(doc.documentElement.dataset.theme).toBe("dark");
    expect(elements.has("prism-custom-theme-styles")).toBe(true);
  });
});
