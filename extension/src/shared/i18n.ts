/**
 * UI translations for the extension. The keys are the English texts themselves, so `t()` falls
 * back to the English text for anything a language doesn't have. Catalogs live in
 * `src/locales/<code>.json`:
 *
 *   "Sent {count} item(s) to Prism": { "one": "...", "few": "...", "many": "...", "other": "..." }
 *
 * Plural entries are picked with Intl.PluralRules by the `count` variable; `en.json` holds the
 * English forms. Add a language by dropping a catalog here and listing it in `CATALOGS` — keys it
 * doesn't cover stay English. `test/i18n.test.ts` checks every catalog covers every key the code
 * uses, including the `data-i18n` texts of `options.html`.
 *
 * The language comes from the browser: `navigator.languages` in the pages, `i18n.getUILanguage()`
 * as the fallback (available in all four contexts, including the service worker).
 */
import en from '../locales/en.json';
import ru from '../locales/ru.json';
import { api } from './api';

export type PluralEntry = Partial<Record<Intl.LDMLPluralRule, string>>;
export type Catalog = Record<string, string | PluralEntry>;

/** English is the source of truth: keys are English texts and every fallback ends here. */
export const EN: Catalog = en;

/** Languages the extension ships; every other language falls back to English. */
export const CATALOGS: Record<string, Catalog> = { en: EN, ru };

let current = 'en';
let plurals = new Intl.PluralRules('en');

function preferredLanguages(): string[] {
  const langs = typeof navigator !== 'undefined' ? navigator.languages : undefined;
  if (langs?.length) return [...langs];
  try {
    return [api?.i18n?.getUILanguage?.() ?? 'en'];
  } catch {
    return ['en'];
  }
}

/**
 * Switches to the browser's language (or to `lang`, for tests) and marks the document with it.
 * Returns the language now in use.
 */
export function setLanguage(lang?: string): string {
  const base = (p: string) => p.toLowerCase().split(/[-_]/)[0] ?? '';
  const wanted = lang ? [lang] : preferredLanguages();
  const next = wanted.map(base).find((b) => b in CATALOGS) ?? 'en';
  if (next !== current) {
    current = next;
    plurals = new Intl.PluralRules(next);
  }
  if (typeof document !== 'undefined' && document.documentElement) document.documentElement.lang = current;
  return current;
}

export function getLanguage(): string {
  return current;
}

type Vars = Record<string, string | number>;

function lookup(key: string, vars?: Vars): string {
  const entry = (CATALOGS[current] ?? EN)[key] ?? EN[key] ?? key;
  if (typeof entry === 'string') return entry;
  const n = Number(vars?.count ?? 0);
  const enForms = EN[key] as PluralEntry;
  // A language that doesn't have this plural text (or this form of it) uses the English one.
  return entry[plurals.select(n)] ?? entry.other ?? enForms[plurals.select(n)] ?? enForms.other ?? enForms.one ?? key;
}

/** The text for `key` in the current language, with `{name}` placeholders filled in. */
export function t(key: string, vars?: Vars): string {
  const text = lookup(key, vars);
  return vars ? text.replace(/\{(\w+)\}/g, (m, k: string) => (k in vars ? String(vars[k]) : m)) : text;
}

/** Marks a text as a translation key where it is defined (module-level tables); show it with `t()`. */
export const tk = (key: string): string => key;
