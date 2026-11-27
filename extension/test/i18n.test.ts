import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, expect, it } from 'vitest';
import { CATALOGS, EN, setLanguage, t } from '../src/shared/i18n';

const extRoot = join(dirname(fileURLToPath(import.meta.url)), '..');
const appRu = join(extRoot, '..', 'app', 'src', 'locales', 'ru.json');

/** The extension's own code and markup (no tests), as text. */
function sources(): Array<[string, string]> {
  const src = join(extRoot, 'src');
  const walk = (dir: string): Array<[string, string]> =>
    readdirSync(dir).flatMap((f) => {
      const p = join(dir, f);
      if (statSync(p).isDirectory()) return walk(p);
      return /\.(ts|html)$/.test(p) ? [[p, readFileSync(p, 'utf8')]] : [];
    });
  return walk(src);
}

/** Every literal key passed to t() or tk(), plus the data-i18n texts of the HTML pages. */
function usedKeys(): Map<string, string> {
  const keys = new Map<string, string>();
  const bt = String.fromCharCode(96);
  const patterns = [
    { re: /\b(?:t|tk)\(\s*"((?:\\.|[^"\\])*)"/g, quoted: true },
    { re: /\b(?:t|tk)\(\s*'((?:\\.|[^'\\])*)'/g, quoted: true },
    { re: new RegExp('\\b(?:t|tk)\\(\\s*' + bt + '((?:\\\\.|[^' + bt + '\\\\$])*)' + bt, 'g'), quoted: true },
    { re: /\bdata-i18n=(?:\"((?:\\.|[^"\\])*)\"|'((?:\\.|[^'\\])*)')/g, quoted: false },
  ];
  for (const [file, code] of sources()) {
    for (const { re, quoted } of patterns) {
      for (const m of code.matchAll(re)) {
        const raw = [...m.slice(1)].find((g) => g !== undefined) ?? '';
        const key = quoted ? (JSON.parse('"' + raw + '"') as string) : raw.replace(/\\(.)/g, '$1');
        if (key) keys.set(key, file);
      }
    }
  }
  return keys;
}

const placeholders = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
const forms = (entry: string | Record<string, string | undefined>): string[] =>
  typeof entry === 'string' ? [entry] : Object.values(entry).filter((f) => f !== undefined);

describe('translations', () => {
  const keys = usedKeys();

  it('finds the texts the code translates', () => {
    expect(keys.size).toBeGreaterThan(70);
  });

  it('en.json is the source of truth: it has every key the code uses', () => {
    const missing = [...keys.keys()].filter((k) => !(k in EN));
    expect(missing, 'missing in en.json').toEqual([]);
  });

  for (const [code, catalog] of Object.entries(CATALOGS)) {
    it(`${code} covers every text the code uses, with the same placeholders`, () => {
      const missing = [...keys.keys()].filter((k) => !(k in catalog));
      expect(missing, 'missing in ' + code + '.json').toEqual([]);
      for (const [key, entry] of Object.entries(catalog)) {
        for (const form of forms(entry)) {
          expect(placeholders(form), code + ': ' + JSON.stringify(key)).toEqual(placeholders(key));
          expect(form.trim(), code + ': empty translation of ' + JSON.stringify(key)).not.toBe('');
        }
      }
    });

    it(`${code} has no texts the code no longer uses`, () => {
      const unused = Object.keys(catalog).filter((k) => !keys.has(k));
      expect(unused, 'unused in ' + code + '.json').toEqual([]);
    });
  }

  it('plural texts have English forms and plural forms everywhere', () => {
    const pluralKeys = [...keys.keys()].filter((k) => Object.values(CATALOGS).some((c) => typeof c[k] === 'object'));
    expect(pluralKeys.length).toBeGreaterThan(0);
    for (const k of pluralKeys) {
      const en = EN[k] as Record<string, string | undefined>;
      expect(en.one && en.other, 'en.json needs one/other for ' + k).toBeTruthy();
      expect(placeholders(k), k + ' must use {count}').toContain('count');
      for (const [code, catalog] of Object.entries(CATALOGS)) {
        const entry = catalog[k];
        expect(typeof entry, code + ': ' + k + ' must be a plural entry').toBe('object');
        expect((entry as Record<string, string | undefined>).other, code + ': ' + k + ' needs "other"').toBeTruthy();
      }
    }
  });

  it('Russian uses the same wording as the app for the texts they share', () => {
    const app = JSON.parse(readFileSync(appRu, 'utf8')) as Record<string, string>;
    const ru = CATALOGS.ru ?? {};
    const shared = Object.keys(ru).filter((k) => k in app && typeof ru[k] === 'string');
    expect(shared.length, 'the extension should share wording with the app').toBeGreaterThan(10);
    const different = shared.filter((k) => ru[k] !== app[k]);
    expect(different, 'wording differs from the app: ' + different.map((k) => JSON.stringify(k)).join(', ')).toEqual([]);
  });
});

describe('t()', () => {
  afterEach(() => setLanguage('en'));

  it('uses English by default', () => {
    expect(t('Download')).toBe('Download');
    expect(t('Sent {count} item(s) to Prism', { count: 3 })).toBe('Sent 3 items to Prism');
  });

  it('translates when the language is Russian', () => {
    setLanguage('ru');
    expect(t('Download')).toBe('Скачать');
    expect(t('Sent {count} item(s) to Prism', { count: 1 })).toBe('Отправлен 1 файл в Prism');
    expect(t('Sent {count} item(s) to Prism', { count: 2 })).toBe('Отправлено 2 файла в Prism');
    expect(t('Sent {count} item(s) to Prism', { count: 5 })).toBe('Отправлено 5 файлов в Prism');
  });

  it('falls back to English for a language it does not have', () => {
    setLanguage('de');
    expect(t('Download')).toBe('Download');
    expect(t('Sent {count} item(s) to Prism', { count: 2 })).toBe('Sent 2 items to Prism');
  });

  it('falls back to the English text for a key a catalog does not have', () => {
    setLanguage('ru');
    expect(t('Text from the app')).toBe('Text from the app');
  });

  it('keeps a placeholder it cannot fill', () => {
    expect(t('Details: {error}')).toBe('Details: {error}');
  });
});
