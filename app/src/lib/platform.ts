// Platform wording: macOS has Finder, Get Info, the menu bar and ⌘ where Windows and Linux have
// folders, Properties, the tray and Ctrl. Every window can use this (`dataset.os` is set only by
// the main window).

import { t } from "./i18n";

export const IS_MAC = typeof navigator !== "undefined" && /Macintosh|Mac OS X/i.test(navigator.userAgent);

/** Mac modifier glyphs, in Apple's order (⌃ ⌥ ⇧ ⌘ before the key). */
const MAC_MODIFIERS: Record<string, string> = { Alt: "⌥", Shift: "⇧", Ctrl: "⌘" };
const MAC_ORDER = ["⌃", "⌥", "⇧", "⌘"];
/** Keys a Mac keyboard doesn't have, or names differently. Delete and F2 map to the shortcuts
 * `useGlobalShortcuts` adds on macOS (⌘⌫ like Move to Trash, ⌘I like Get Info). */
const MAC_KEYS: Record<string, string[]> = { Del: ["⌘", "⌫"], Delete: ["⌘", "⌫"], F2: ["⌘", "I"], Enter: ["Return"] };

/**
 * A shortcut as the platform writes it: "Ctrl Shift M" → "⇧ ⌘ M" on a Mac, unchanged elsewhere.
 * Alternatives are separated by " / " ("Ctrl K / Ctrl F"), keys by spaces.
 */
export function keyLabel(combo: string): string {
  if (!IS_MAC) return combo;
  return combo
    .split(" / ")
    .map((alt) => {
      const keys = alt.split(" ").flatMap((k) => MAC_KEYS[k] ?? [MAC_MODIFIERS[k] ?? k]);
      const mods = [...new Set(keys.filter((k) => MAC_ORDER.includes(k)))].sort((a, b) => MAC_ORDER.indexOf(a) - MAC_ORDER.indexOf(b));
      return [...mods, ...keys.filter((k) => !MAC_ORDER.includes(k))].join(" ");
    })
    .join(" / ");
}

/** Reveals a file in the file manager. */
export const showInFolderLabel = () => (IS_MAC ? t("Show in Finder") : t("Show in folder"));
/** Opens a folder in the file manager. */
export const openFolderLabel = () => (IS_MAC ? t("Open in Finder") : t("Open folder"));
/** The menu command for a download's properties dialog. */
export const propertiesLabel = () => (IS_MAC ? t("Get Info") : t("Properties…"));
/** The properties dialog's title. */
export const propertiesTitle = () => (IS_MAC ? t("Info") : t("Properties"));
