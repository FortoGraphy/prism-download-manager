import { isValidElement, type ReactNode } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { CATALOGS, LANGUAGES, type PluralEntry, resolveLanguage, setLanguage, t, tr } from "./i18n";

// The app's own code (no tests, no demo data), as text.
const SOURCES = import.meta.glob(["../**/*.{ts,tsx}", "!../**/*.test.{ts,tsx}", "!../lib/mock.ts"], {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

/** Every literal key passed to t(), tr() or tk() in the app's code. */
function usedKeys(): Map<string, string> {
  const keys = new Map<string, string>();
  const call = /\b(?:t|tr|tk)\(\s*(?:"((?:\\.|[^"\\])*)"|'((?:\\.|[^'\\])*)'|`((?:\\.|[^`\\$])*)`)/g;
  for (const [file, code] of Object.entries(SOURCES)) {
    for (const m of code.matchAll(call)) {
      const raw = m[1] ?? m[2] ?? m[3] ?? "";
      const key = m[1] !== undefined ? (JSON.parse(`"${raw}"`) as string) : raw.replace(/\\(.)/g, "$1");
      keys.set(key, file);
    }
  }
  return keys;
}

const placeholders = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
const forms = (entry: string | PluralEntry) => (typeof entry === "string" ? [entry] : Object.values(entry));

describe("translations", () => {
  const keys = usedKeys();

  it("finds the texts the code translates", () => {
    expect(keys.size).toBeGreaterThan(300);
  });

  for (const { code } of LANGUAGES) {
    if (code === "en") continue;
    it(`${code} has every text, with the same placeholders`, () => {
      const catalog = CATALOGS[code];
      const missing = [...keys.keys()].filter((k) => !(k in catalog));
      expect(missing, `missing in ${code}.json`).toEqual([]);
      for (const [key, entry] of Object.entries(catalog)) {
        for (const form of forms(entry)) {
          expect(placeholders(form), `${code}: ${JSON.stringify(key)}`).toEqual(placeholders(key));
          expect(form.trim(), `${code}: empty translation of ${JSON.stringify(key)}`).not.toBe("");
        }
        if (typeof entry !== "string") expect(entry.other, `${code}: ${key} needs "other"`).toBeTruthy();
      }
    });
    it(`${code} has no texts the code no longer uses`, () => {
      const unused = Object.keys(CATALOGS[code]).filter((k) => !keys.has(k));
      expect(unused, `unused in ${code}.json`).toEqual([]);
    });
  }

  it("plural texts have English forms and plural forms everywhere", () => {
    const pluralKeys = [...keys.keys()].filter((k) => LANGUAGES.some(({ code }) => typeof CATALOGS[code][k] === "object"));
    for (const k of pluralKeys) {
      const en = CATALOGS.en[k] as PluralEntry | undefined;
      expect(en?.one && en?.other, `en.json needs one/other for ${k}`).toBeTruthy();
      expect(placeholders(k), `${k} must use {count}`).toContain("count");
      for (const { code } of LANGUAGES) expect(typeof CATALOGS[code][k], `${code}: ${k} must be plural`).toBe("object");
    }
    for (const k of Object.keys(CATALOGS.en)) expect(keys.has(k), `en.json: unused ${k}`).toBe(true);
  });
});

describe("t()", () => {
  afterEach(() => setLanguage("en"));

  it("picks the setting, else the system language, else English", () => {
    expect(resolveLanguage("de", ["cs-CZ"])).toBe("de");
    expect(resolveLanguage("", ["pt-BR", "it-IT", "en"])).toBe("it");
    expect(resolveLanguage("", ["cs"])).toBe("cs");
    expect(resolveLanguage("", ["ja-JP"])).toBe("en");
    expect(resolveLanguage("xx", ["cs"])).toBe("en");
    // Simplified Chinese for a Simplified (or plain) Chinese system, not for Traditional.
    expect(resolveLanguage("", ["zh-CN"])).toBe("zh");
    expect(resolveLanguage("", ["zh"])).toBe("zh");
    expect(resolveLanguage("", ["zh-Hans-HK"])).toBe("zh");
    expect(resolveLanguage("", ["zh-TW", "en"])).toBe("en");
    expect(resolveLanguage("", ["zh-Hant-HK", "ja"])).toBe("en");
    expect(resolveLanguage("", ["zh_HK"])).toBe("en");
    expect(resolveLanguage("zh", ["zh-TW"])).toBe("zh");
  });

  it("falls back to the English text and fills placeholders", () => {
    setLanguage("cs");
    expect(t("A text nobody translated {x}", { x: 5 })).toBe("A text nobody translated 5");
  });

  it("chooses plural forms by language", () => {
    CATALOGS.en["{count} test files"] = { one: "{count} test file", other: "{count} test files" };
    CATALOGS.cs["{count} test files"] = { one: "{count} soubor", few: "{count} soubory", other: "{count} souborů" };
    try {
      expect(t("{count} test files", { count: 1 })).toBe("1 test file");
      expect(t("{count} test files", { count: 3 })).toBe("3 test files");
      setLanguage("cs");
      expect([1, 3, 5].map((count) => t("{count} test files", { count }))).toEqual(["1 soubor", "3 soubory", "5 souborů"]);
      setLanguage("de");
      expect(t("{count} test files", { count: 1 })).toBe("1 test file");
    } finally {
      delete CATALOGS.en["{count} test files"];
      delete CATALOGS.cs["{count} test files"];
    }
  });

  it("tr() puts React nodes into the text", () => {
    const parts = tr("Click {button} to go on", { button: <b>Next</b> }) as ReactNode[];
    expect(parts).toHaveLength(3);
    const inner = (n: ReactNode) => (isValidElement<{ children: ReactNode }>(n) ? n.props.children : n);
    expect(inner(parts[0])).toBe("Click ");
    expect(isValidElement(inner(parts[1]))).toBe(true);
    expect(inner(parts[2])).toBe(" to go on");
  });
});
