// Curated popular themes model, CSS token injection, and theme lookups.

export interface CustomThemeColors {
  bgCanvas: string;
  surfaceBase: string;
  surfaceRaised: string;
  surfaceSunken: string;
  surfaceOverlay: string;
  surfaceInverse?: string;
  textPrimary: string;
  textSecondary: string;
  textTertiary: string;
  borderDefault: string;
  borderSubtle?: string;
  borderStrong?: string;
  controlBg?: string;
  inputBg?: string;
  accent?: string;
  accentRgb?: string;
  success?: string;
  warning?: string;
  danger?: string;
  info?: string;
}

export interface CustomTheme {
  id: string;
  name: string;
  description: string;
  base: "dark" | "dimmed" | "light";
  builtIn?: boolean;
  author?: string;
  version?: number;
  colors: CustomThemeColors;
}

export const BASE_DARK_COLORS: CustomThemeColors = {
  bgCanvas: "#0d0e13",
  surfaceBase: "#141722",
  surfaceSunken: "#090a0e",
  surfaceRaised: "#1c202d",
  surfaceOverlay: "#252b3b",
  surfaceInverse: "#ffffff",
  textPrimary: "#f0f2f8",
  textSecondary: "#a2a8b8",
  textTertiary: "#6e7587",
  borderDefault: "rgba(255, 255, 255, 0.14)",
  borderSubtle: "rgba(255, 255, 255, 0.07)",
  borderStrong: "rgba(255, 255, 255, 0.25)",
  controlBg: "rgba(255, 255, 255, 0.07)",
  inputBg: "rgba(0, 0, 0, 0.28)",
  accent: "#8e8cff",
  accentRgb: "142 140 255",
  success: "#3dd68c",
  warning: "#f5b544",
  danger: "#ff6b6b",
  info: "#5cc8f0",
};

export const BASE_DIMMED_COLORS: CustomThemeColors = {
  bgCanvas: "#161b22",
  surfaceBase: "#1a1f27",
  surfaceSunken: "#12161c",
  surfaceRaised: "#22272e",
  surfaceOverlay: "#2d333b",
  surfaceInverse: "#ffffff",
  textPrimary: "#adbac7",
  textSecondary: "#768390",
  textTertiary: "#636e7b",
  borderDefault: "#373e47",
  borderSubtle: "rgba(205, 217, 229, 0.10)",
  borderStrong: "#444c56",
  controlBg: "rgba(205, 217, 229, 0.08)",
  inputBg: "#12161c",
  accent: "#539bf5",
  accentRgb: "83 155 245",
  success: "#57ab5a",
  warning: "#c69026",
  danger: "#e5534b",
  info: "#539bf5",
};

export const BASE_LIGHT_COLORS: CustomThemeColors = {
  bgCanvas: "#f0f3f8",
  surfaceBase: "#fbfbfd",
  surfaceSunken: "#e8ecf2",
  surfaceRaised: "#ffffff",
  surfaceOverlay: "#ffffff",
  surfaceInverse: "#0d0e13",
  textPrimary: "#181a20",
  textSecondary: "#525866",
  textTertiary: "#88909e",
  borderDefault: "rgba(0, 0, 0, 0.14)",
  borderSubtle: "rgba(0, 0, 0, 0.06)",
  borderStrong: "rgba(0, 0, 0, 0.24)",
  controlBg: "rgba(0, 0, 0, 0.05)",
  inputBg: "#ffffff",
  accent: "#4f4be0",
  accentRgb: "79 75 224",
  success: "#1f874f",
  warning: "#b56900",
  danger: "#d03a45",
  info: "#1b74a3",
};

/**
 * Curated preset themes built into Prism.
 * Filtered to popular, modern developer and community favorites.
 */
export const THEME_PRESETS: CustomTheme[] = [
  {
    id: "catppuccin-mocha",
    name: "Catppuccin Mocha",
    description: "Universally loved pastel dark palette with soothing mauve accents and soft contrast.",
    base: "dark",
    builtIn: true,
    colors: {
      bgCanvas: "#1e1e2e",
      surfaceBase: "#181825",
      surfaceSunken: "#11111b",
      surfaceRaised: "#313244",
      surfaceOverlay: "#45475a",
      surfaceInverse: "#cdd6f4",
      textPrimary: "#cdd6f4",
      textSecondary: "#bac2de",
      textTertiary: "#a6adc8",
      borderDefault: "rgba(205, 214, 244, 0.16)",
      borderSubtle: "rgba(205, 214, 244, 0.08)",
      borderStrong: "rgba(205, 214, 244, 0.28)",
      controlBg: "rgba(205, 214, 244, 0.08)",
      inputBg: "#11111b",
      accent: "#cba6f7",
      accentRgb: "203 166 247",
      success: "#a6e3a1",
      warning: "#f9e2af",
      danger: "#f38ba8",
      info: "#89b4fa",
    },
  },
  {
    id: "tokyo-night",
    name: "Tokyo Night",
    description: "Iconic Japanese cyberpunk dusk theme with deep indigo tones and electric blue accents.",
    base: "dark",
    builtIn: true,
    colors: {
      bgCanvas: "#16161e",
      surfaceBase: "#1a1b26",
      surfaceSunken: "#13141c",
      surfaceRaised: "#24283b",
      surfaceOverlay: "#2f3549",
      surfaceInverse: "#c0caf5",
      textPrimary: "#c0caf5",
      textSecondary: "#9aa5ce",
      textTertiary: "#565f89",
      borderDefault: "rgba(122, 162, 247, 0.18)",
      borderSubtle: "rgba(255, 255, 255, 0.08)",
      borderStrong: "rgba(122, 162, 247, 0.32)",
      controlBg: "rgba(255, 255, 255, 0.07)",
      inputBg: "#13141c",
      accent: "#7aa2f7",
      accentRgb: "122 162 247",
      success: "#9ece6a",
      warning: "#e0af68",
      danger: "#f7768e",
      info: "#7dcfff",
    },
  },
  {
    id: "dracula",
    name: "Dracula",
    description: "The classic high-contrast vampire dark theme with famous purple and pink highlights.",
    base: "dark",
    builtIn: true,
    colors: {
      bgCanvas: "#1e1f29",
      surfaceBase: "#282a36",
      surfaceSunken: "#191a21",
      surfaceRaised: "#343746",
      surfaceOverlay: "#44475a",
      surfaceInverse: "#f8f8f2",
      textPrimary: "#f8f8f2",
      textSecondary: "#bfbfd6",
      textTertiary: "#6272a4",
      borderDefault: "rgba(189, 147, 249, 0.20)",
      borderSubtle: "rgba(255, 255, 255, 0.08)",
      borderStrong: "rgba(189, 147, 249, 0.35)",
      controlBg: "rgba(255, 255, 255, 0.07)",
      inputBg: "#191a21",
      accent: "#bd93f9",
      accentRgb: "189 147 249",
      success: "#50fa7b",
      warning: "#ffb86c",
      danger: "#ff5555",
      info: "#8be9fd",
    },
  },
  {
    id: "one-dark",
    name: "One Dark Pro",
    description: "The iconic, beautifully balanced dark theme popularized by Atom and VS Code.",
    base: "dark",
    builtIn: true,
    colors: {
      bgCanvas: "#1e2227",
      surfaceBase: "#21252b",
      surfaceSunken: "#181a1f",
      surfaceRaised: "#282c34",
      surfaceOverlay: "#353b45",
      surfaceInverse: "#abb2bf",
      textPrimary: "#abb2bf",
      textSecondary: "#828997",
      textTertiary: "#5c6370",
      borderDefault: "rgba(171, 178, 191, 0.16)",
      borderSubtle: "rgba(255, 255, 255, 0.08)",
      borderStrong: "rgba(171, 178, 191, 0.28)",
      controlBg: "rgba(255, 255, 255, 0.07)",
      inputBg: "#181a1f",
      accent: "#61afef",
      accentRgb: "97 175 239",
      success: "#98c379",
      warning: "#e5c07b",
      danger: "#e06c75",
      info: "#56b6c2",
    },
  },
  {
    id: "nord-frost",
    name: "Nordic Frost",
    description: "Arctic, cool north-bluish clean palette inspired by the iconic Nord color scheme.",
    base: "dark",
    builtIn: true,
    colors: {
      bgCanvas: "#242933",
      surfaceBase: "#2e3440",
      surfaceSunken: "#20242c",
      surfaceRaised: "#3b4252",
      surfaceOverlay: "#434c5e",
      surfaceInverse: "#eceff4",
      textPrimary: "#eceff4",
      textSecondary: "#d8dee9",
      textTertiary: "#81a1c1",
      borderDefault: "rgba(216, 222, 233, 0.16)",
      borderSubtle: "rgba(216, 222, 233, 0.09)",
      borderStrong: "rgba(216, 222, 233, 0.26)",
      controlBg: "rgba(216, 222, 233, 0.08)",
      inputBg: "#20242c",
      accent: "#88c0d0",
      accentRgb: "136 192 208",
      success: "#a3be8c",
      warning: "#ebcb8b",
      danger: "#bf616a",
      info: "#81a1c1",
    },
  },
  {
    id: "oled-black",
    name: "OLED Midnight",
    description: "Pure #000000 black canvas for maximum contrast and battery efficiency on OLED displays.",
    base: "dark",
    builtIn: true,
    colors: {
      bgCanvas: "#000000",
      surfaceBase: "#000000",
      surfaceSunken: "#080808",
      surfaceRaised: "#121212",
      surfaceOverlay: "#1a1a1a",
      surfaceInverse: "#ffffff",
      textPrimary: "#ffffff",
      textSecondary: "#a8a8a8",
      textTertiary: "#707070",
      borderDefault: "rgba(255, 255, 255, 0.16)",
      borderSubtle: "rgba(255, 255, 255, 0.08)",
      borderStrong: "rgba(255, 255, 255, 0.28)",
      controlBg: "rgba(255, 255, 255, 0.08)",
      inputBg: "#080808",
      accent: "#8c89ff",
      accentRgb: "140 137 255",
      success: "#38db8d",
      warning: "#f5b742",
      danger: "#ff6363",
      info: "#4ebdff",
    },
  },
  {
    id: "rose-pine",
    name: "Rosé Pine",
    description: "Minimalist vintage dark theme with soothing pine surfaces, love, and gold highlights.",
    base: "dark",
    builtIn: true,
    colors: {
      bgCanvas: "#191724",
      surfaceBase: "#1f1d2e",
      surfaceSunken: "#14121d",
      surfaceRaised: "#26233a",
      surfaceOverlay: "#312f44",
      surfaceInverse: "#e0def4",
      textPrimary: "#e0def4",
      textSecondary: "#908caa",
      textTertiary: "#6e6a86",
      borderDefault: "rgba(224, 222, 244, 0.15)",
      borderSubtle: "rgba(224, 222, 244, 0.07)",
      borderStrong: "rgba(224, 222, 244, 0.26)",
      controlBg: "rgba(224, 222, 244, 0.07)",
      inputBg: "#14121d",
      accent: "#eb6f92",
      accentRgb: "235 111 146",
      success: "#9ccfd8",
      warning: "#f6c177",
      danger: "#eb6f92",
      info: "#c4a7e7",
    },
  },
  {
    id: "catppuccin-latte",
    name: "Catppuccin Latte",
    description: "Delightful warm pastel light theme with soothing lavender accents and gentle contrast.",
    base: "light",
    builtIn: true,
    colors: {
      bgCanvas: "#eff1f5",
      surfaceBase: "#e6e9ef",
      surfaceSunken: "#dce0e8",
      surfaceRaised: "#ffffff",
      surfaceOverlay: "#ffffff",
      surfaceInverse: "#4c4f69",
      textPrimary: "#4c4f69",
      textSecondary: "#5c5f77",
      textTertiary: "#6c6f85",
      borderDefault: "rgba(76, 79, 105, 0.16)",
      borderSubtle: "rgba(76, 79, 105, 0.08)",
      borderStrong: "rgba(76, 79, 105, 0.28)",
      controlBg: "rgba(76, 79, 105, 0.06)",
      inputBg: "#ffffff",
      accent: "#8839ef",
      accentRgb: "136 57 239",
      success: "#40a02b",
      warning: "#df8e1d",
      danger: "#d20f39",
      info: "#1e66f5",
    },
  },
];

/** Convert hex (e.g. #7c78ff or #fff) to RGB triplet string ("124 120 255") */
export function hexToRgbTriplet(hex: string): string {
  let clean = hex.replace("#", "").trim();
  if (clean.length === 3) {
    clean = clean.split("").map((c) => c + c).join("");
  }
  if (clean.length !== 6) return "124 120 255";
  const num = parseInt(clean, 16);
  if (isNaN(num)) return "124 120 255";
  const r = (num >> 16) & 255;
  const g = (num >> 8) & 255;
  const b = num & 255;
  return `${r} ${g} ${b}`;
}

export const USER_THEMES_STORAGE_KEY = "prism_user_custom_themes";

/** Get user-created custom themes from localStorage */
export function getUserThemes(): CustomTheme[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const raw = localStorage.getItem(USER_THEMES_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

/** Save or update a user custom theme in localStorage */
export function saveUserTheme(theme: CustomTheme): void {
  if (typeof localStorage === "undefined") return;
  const current = getUserThemes().filter((t) => t.id !== theme.id);
  const updatedTheme: CustomTheme = {
    ...theme,
    builtIn: false,
    version: 1,
  };
  current.push(updatedTheme);
  try {
    localStorage.setItem(USER_THEMES_STORAGE_KEY, JSON.stringify(current));
    if (typeof window !== "undefined" && typeof window.dispatchEvent === "function") {
      window.dispatchEvent(new CustomEvent("prism:themes-changed"));
    }
  } catch (err) {
    console.warn("Failed to persist custom theme:", err);
  }
}

/** Delete a user custom theme */
export function deleteUserTheme(id: string): boolean {
  if (typeof localStorage === "undefined") return false;
  const current = getUserThemes();
  const filtered = current.filter((t) => t.id !== id);
  if (filtered.length === current.length) return false;
  try {
    localStorage.setItem(USER_THEMES_STORAGE_KEY, JSON.stringify(filtered));
    if (typeof window !== "undefined" && typeof window.dispatchEvent === "function") {
      window.dispatchEvent(new CustomEvent("prism:themes-changed"));
    }
    return true;
  } catch {
    return false;
  }
}

/** Create a new custom theme scaffold based on base colors */
export function createNewCustomTheme(
  base: "dark" | "dimmed" | "light",
  name: string,
  overrides?: Partial<CustomThemeColors>
): CustomTheme {
  const baseDefaults =
    base === "light"
      ? BASE_LIGHT_COLORS
      : base === "dimmed"
      ? BASE_DIMMED_COLORS
      : BASE_DARK_COLORS;

  const id = `custom-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
  const colors: CustomThemeColors = {
    ...baseDefaults,
    ...overrides,
  };

  if (colors.accent) {
    colors.accentRgb = hexToRgbTriplet(colors.accent);
  }

  return {
    id,
    name: name.trim() || "Untitled Theme",
    description: "Custom user theme",
    base,
    builtIn: false,
    version: 1,
    colors,
  };
}

/** Get all curated and custom user themes */
export function getAllThemes(): CustomTheme[] {
  return [...THEME_PRESETS, ...getUserThemes()];
}

/** Find a theme by ID (checks presets and user custom themes) */
export function findThemeById(id: string): CustomTheme | undefined {
  if (id.startsWith("custom:")) {
    id = id.slice(7);
  }
  if (id === "dracula-night") id = "dracula";
  return getAllThemes().find((t) => t.id === id);
}

// --------------------------------------------------------------------------
// CSS Custom Properties Injection
// --------------------------------------------------------------------------

const INJECTED_STYLE_ID = "prism-custom-theme-styles";

export const THEME_CSS_PROPS = [
  "--bg-canvas",
  "--surface-base",
  "--surface-sunken",
  "--surface-raised",
  "--surface-overlay",
  "--surface-inverse",
  "--text-primary",
  "--text-secondary",
  "--text-tertiary",
  "--border-default",
  "--border-subtle",
  "--border-strong",
  "--control-bg",
  "--input-bg",
  "--accent",
  "--accent-solid",
  "--accent-rgb",
  "--accent-soft",
  "--accent-softer",
  "--accent-glow",
  "--success",
  "--warning",
  "--danger",
  "--info",
] as const;

/**
 * Apply a theme dynamically to the DOM document.
 * Injects CSS variables directly into documentElement.style and a dedicated <style> element.
 */
export function applyCustomThemeTokens(theme: CustomTheme): void {
  if (typeof document === "undefined" || !document.documentElement) return;

  const root = document.documentElement;
  root.dataset.theme = theme.base;

  const c = theme.colors;
  const accentRgb = c.accentRgb || (c.accent ? hexToRgbTriplet(c.accent) : "140 137 255");

  // Direct element styles
  if (root.style && typeof root.style.setProperty === "function") {
    root.style.setProperty("--bg-canvas", c.bgCanvas);
    root.style.setProperty("--surface-base", c.surfaceBase);
    root.style.setProperty("--surface-sunken", c.surfaceSunken);
    root.style.setProperty("--surface-raised", c.surfaceRaised);
    root.style.setProperty("--surface-overlay", c.surfaceOverlay);
    if (c.surfaceInverse) root.style.setProperty("--surface-inverse", c.surfaceInverse);
    root.style.setProperty("--text-primary", c.textPrimary);
    root.style.setProperty("--text-secondary", c.textSecondary);
    root.style.setProperty("--text-tertiary", c.textTertiary);
    root.style.setProperty("--border-default", c.borderDefault);
    if (c.borderSubtle) root.style.setProperty("--border-subtle", c.borderSubtle);
    if (c.borderStrong) root.style.setProperty("--border-strong", c.borderStrong);
    if (c.controlBg) root.style.setProperty("--control-bg", c.controlBg);
    if (c.inputBg) root.style.setProperty("--input-bg", c.inputBg);
    if (c.accent) {
      root.style.setProperty("--accent", c.accent);
      root.style.setProperty("--accent-solid", c.accent);
    }
    root.style.setProperty("--accent-rgb", accentRgb);
    root.style.setProperty("--accent-soft", `rgba(${accentRgb} / 0.22)`);
    root.style.setProperty("--accent-softer", `rgba(${accentRgb} / 0.12)`);
    root.style.setProperty("--accent-glow", `rgba(${accentRgb} / 0.45)`);
    if (c.success) root.style.setProperty("--success", c.success);
    if (c.warning) root.style.setProperty("--warning", c.warning);
    if (c.danger) root.style.setProperty("--danger", c.danger);
    if (c.info) root.style.setProperty("--info", c.info);
  }

  if (typeof document.getElementById === "function" && typeof document.createElement === "function") {
    const cssText = `
      :root, [data-theme="${theme.base}"] {
        --bg-canvas: ${c.bgCanvas} !important;
        --surface-base: ${c.surfaceBase} !important;
        --surface-sunken: ${c.surfaceSunken} !important;
        --surface-raised: ${c.surfaceRaised} !important;
        --surface-overlay: ${c.surfaceOverlay} !important;
        ${c.surfaceInverse ? `--surface-inverse: ${c.surfaceInverse} !important;` : ""}

        --text-primary: ${c.textPrimary} !important;
        --text-secondary: ${c.textSecondary} !important;
        --text-tertiary: ${c.textTertiary} !important;

        --border-default: ${c.borderDefault} !important;
        ${c.borderSubtle ? `--border-subtle: ${c.borderSubtle} !important;` : ""}
        ${c.borderStrong ? `--border-strong: ${c.borderStrong} !important;` : ""}

        ${c.controlBg ? `--control-bg: ${c.controlBg} !important;` : ""}
        ${c.inputBg ? `--input-bg: ${c.inputBg} !important;` : ""}

        ${c.accent ? `--accent: ${c.accent} !important;` : ""}
        ${c.accent ? `--accent-solid: ${c.accent} !important;` : ""}
        --accent-rgb: ${accentRgb} !important;
        --accent-soft: rgba(${accentRgb} / 0.22) !important;
        --accent-softer: rgba(${accentRgb} / 0.12) !important;
        --accent-glow: rgba(${accentRgb} / 0.45) !important;

        ${c.success ? `--success: ${c.success} !important;` : ""}
        ${c.warning ? `--warning: ${c.warning} !important;` : ""}
        ${c.danger ? `--danger: ${c.danger} !important;` : ""}
        ${c.info ? `--info: ${c.info} !important;` : ""}
      }
    `;

    let styleEl = document.getElementById(INJECTED_STYLE_ID) as HTMLStyleElement | null;
    if (!styleEl && document.head && typeof document.head.appendChild === "function") {
      styleEl = document.createElement("style");
      styleEl.id = INJECTED_STYLE_ID;
      document.head.appendChild(styleEl);
    }
    if (styleEl) {
      styleEl.textContent = cssText;
    }
  }
}

/** Clear any injected theme tokens and restore pure standard stylesheet tokens */
export function clearInjectedThemeTokens(): void {
  if (typeof document !== "undefined" && document.documentElement?.style && typeof document.documentElement.style.removeProperty === "function") {
    for (const v of THEME_CSS_PROPS) {
      document.documentElement.style.removeProperty(v);
    }
  }
  if (typeof document !== "undefined" && typeof document.getElementById === "function") {
    const styleEl = document.getElementById(INJECTED_STYLE_ID);
    if (styleEl && typeof styleEl.remove === "function") {
      styleEl.remove();
    }
  }
}
