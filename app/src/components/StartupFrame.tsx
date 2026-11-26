import { t } from "../lib/i18n";

/** Also present in index.html so the native window can paint before JS starts. */
export function StartupFrame() {
  return <div className="startup-frame" role="status" aria-live="polite" data-tauri-drag-region>
    <strong>Prism</strong><span>{t("Loading…")}</span>
  </div>;
}
