// UI translations. The keys are the English texts themselves, so `t()` falls back to the
// English text for anything a language doesn't have (and texts that come from the core can
// be passed through `t()` too). Catalogs live in `src/locales/<code>.json`:
//
//   "Pause all": "Vše pozastavit"
//   "{count} files": { "one": "{count} soubor", "few": "{count} soubory", "other": "{count} souborů" }
//
// Plural entries are picked with Intl.PluralRules by the `count` variable; `en.json` holds the
// English plural forms. `i18n.test.ts` checks that every catalog covers every key the code uses.

import { createElement, Fragment, type ReactNode } from "react";
import ar from "../locales/ar.json";
import cs from "../locales/cs.json";
import de from "../locales/de.json";
import en from "../locales/en.json";
import es from "../locales/es.json";
import fr from "../locales/fr.json";
import id from "../locales/id.json";
import it from "../locales/it.json";
import turkish from "../locales/tr.json";
import zh from "../locales/zh.json";

export type PluralEntry = Partial<Record<Intl.LDMLPluralRule, string>>;
export type Catalog = Record<string, string | PluralEntry>;

/** Languages in the Language setting, each named in itself. */
export const LANGUAGES = [
  { code: "en", name: "English" },
  { code: "cs", name: "Čeština" },
  { code: "de", name: "Deutsch" },
  { code: "es", name: "Español" },
  { code: "fr", name: "Français" },
  { code: "id", name: "Bahasa Indonesia" },
  { code: "it", name: "Italiano" },
  { code: "tr", name: "Türkçe" },
  { code: "zh", name: "简体中文" },
  { code: "ar", name: "العربية" },
] as const;

export type Language = (typeof LANGUAGES)[number]["code"];

export const CATALOGS: Record<Language, Catalog> = { en, cs, de, es, fr, id, it, tr: turkish, zh, ar };

/** Languages written right to left: the whole layout mirrors (`<html dir="rtl">`). */
const RTL: readonly Language[] = ["ar"];

let current: Language = "en";
let plurals = new Intl.PluralRules("en");

/** The language to use for a setting value ("" = the system's language, else English). */
export function resolveLanguage(setting: string | undefined, preferred: readonly string[] = navigatorLanguages()): Language {
  const known = (code: string) => LANGUAGES.find((l) => l.code === code.toLowerCase().split(/[-_]/)[0])?.code;
  if (setting) return known(setting) ?? "en";
  for (const p of preferred) {
    // Prism has Simplified Chinese only: a system in Traditional Chinese (Taiwan, Hong Kong,
    // Macau) goes on to its next language rather than switch script; Chinese can be picked in
    // Settings.
    if (/^zh[-_](hant|tw|hk|mo)\b/i.test(p)) continue;
    const k = known(p);
    if (k) return k;
  }
  return "en";
}

function navigatorLanguages(): readonly string[] {
  if (typeof navigator === "undefined") return [];
  return navigator.languages?.length ? navigator.languages : [navigator.language ?? "en"];
}

/** Switches the UI language (from the setting); returns the language now in use. */
export function setLanguage(setting: string | undefined): Language {
  const lang = resolveLanguage(setting);
  if (lang !== current) {
    current = lang;
    plurals = new Intl.PluralRules(lang);
  }
  if (typeof document !== "undefined") {
    document.documentElement.lang = lang;
    document.documentElement.dir = RTL.includes(lang) ? "rtl" : "ltr";
  }
  return lang;
}

export function getLanguage(): Language {
  return current;
}

/** Locale for dates and times: the UI language, with Western digits like the rest of the UI. */
export function getLocale(): string {
  return current === "ar" ? "ar-u-nu-latn" : current;
}

type Vars = Record<string, string | number>;

function lookup(key: string, vars?: Vars): string {
  const entry = CATALOGS[current][key] ?? CATALOGS.en[key] ?? key;
  if (typeof entry === "string") return entry;
  const n = Number(vars?.count ?? 0);
  const form = entry[plurals.select(n)] ?? entry.other;
  // A language without this plural entry falls back to the English forms.
  return form ?? (CATALOGS.en[key] as PluralEntry | undefined)?.[n === 1 ? "one" : "other"] ?? key;
}

/** The text for `key` in the current language, with `{name}` placeholders filled in. */
export function t(key: string, vars?: Vars): string {
  const text = lookup(key, vars);
  return vars ? text.replace(/\{(\w+)\}/g, (m, k: string) => (k in vars ? String(vars[k]) : m)) : text;
}

/** Marks a text for translation where it's defined (module-level tables); show it with `t()`. */
export const tk = (key: string): string => key;

/** Like `t`, but placeholders can be React nodes (links, bold text, buttons). */
export function tr(key: string, vars: Record<string, ReactNode>): ReactNode {
  const count = vars.count;
  const text = lookup(key, typeof count === "number" || typeof count === "string" ? { count } : undefined);
  return text.split(/(\{\w+\})/).map((part, i) => {
    const name = /^\{(\w+)\}$/.exec(part)?.[1];
    return createElement(Fragment, { key: i }, name && name in vars ? vars[name] : part);
  });
}
