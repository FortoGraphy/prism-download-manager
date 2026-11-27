import { useEffect, useMemo, useState } from "react";
import { Checkbox, TextField } from "../components/Controls";
import { openMenuAt } from "../components/MenuHost";
import { FileTile } from "../components/Visuals";
import { WindowFrame } from "../components/WindowFrame";
import { Icon } from "../design/icons";
import { api, webMode } from "../lib/api";
import { fileExtension, formatBytes, hostOf } from "../lib/format";
import { t, tk, tr } from "../lib/i18n";
import { categoryIcon, displayName } from "../lib/present";
import { useApp } from "../lib/store";
import type { BatchInfo, StartMode } from "../lib/types";
import { attempt } from "../lib/ui";
import { pickFolder, win } from "../lib/win";

/** The title the backend gives a batch of links from the clipboard; other titles (page titles) are shown as is. */
const CLIPBOARD_TITLE = tk("Links from clipboard");

export default function BatchWindow({ id }: { id: string }) {
  const [batch, setBatch] = useState<BatchInfo | null>(null);
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [cat, setCat] = useState("all");
  const [ext, setExt] = useState("");
  const [dir, setDir] = useState("");
  const [busy, setBusy] = useState(false);
  const categories = useApp((s) => s.categories);
  const queues = useApp((s) => s.queues);

  useEffect(() => {
    void api.getBatch(id).then((b) => {
      setBatch(b);
      if (b) setChecked(new Set(b.entries.map((e) => e.url)));
    });
  }, [id]);

  const entries = useMemo(() => batch?.entries ?? [], [batch]);
  const byCat = useMemo(() => {
    const m: Record<string, number> = {};
    for (const e of entries) m[e.category || "other"] = (m[e.category || "other"] ?? 0) + 1;
    return m;
  }, [entries]);
  const exts = ext
    .split(/[\s,;]+/)
    .map((x) => x.replace(/^\./, "").toLowerCase())
    .filter(Boolean);
  const shown = entries.filter((e) => (cat === "all" || (e.category || "other") === cat) && (!exts.length || exts.includes(fileExtension(e.fileName || e.url))));
  const sel = entries.filter((e) => checked.has(e.url));
  const allShown = shown.length > 0 && shown.every((e) => checked.has(e.url));
  const setAll = (on: boolean) => {
    const n = new Set(checked);
    shown.forEach((e) => (on ? n.add(e.url) : n.delete(e.url)));
    setChecked(n);
  };
  const confirm = async (mode: StartMode) => {
    setBusy(true);
    const n = await attempt(api.confirmBatch({ id, urls: sel.map((e) => e.url), saveDir: dir || undefined, mode }), t("Could not add the downloads"));
    setBusy(false);
    if (n !== undefined) void win.close();
  };
  const cancel = () => {
    void api.cancelBatch(id);
    void win.close();
  };

  return (
    <WindowFrame kind="batch" title={batch?.title === CLIPBOARD_TITLE ? t(CLIPBOARD_TITLE) : (batch?.title ?? t("Download links"))} icon={<Icon name="batch" size={17} className="text-accent" />} onClose={cancel} minimizable className="batch">
      <div className="batch__head">
        <div className="chips">
          <button type="button" className={`chip${cat === "all" ? " on" : ""}`} onClick={() => setCat("all")}>
            {t("All")} <em>{entries.length}</em>
          </button>
          {categories
            .filter((c) => byCat[c.id])
            .map((c) => (
              <button key={c.id} type="button" className={`chip${cat === c.id ? " on" : ""}`} onClick={() => setCat(c.id)}>
                <Icon name={categoryIcon(c)} size={13} strokeWidth={2} /> {displayName(c.name)} <em>{byCat[c.id]}</em>
              </button>
            ))}
        </div>
        <TextField value={ext} onChange={setExt} label={t("Filter by extension")} icon="filter" placeholder={t("Extensions: zip pdf")} />
      </div>
      <div className="gtable batch__table">
        <div className="grow2 h">
          <Checkbox checked={allShown} indeterminate={!allShown && shown.some((e) => checked.has(e.url))} label={t("Select all shown")} onChange={(v) => setAll(v)} />
          <span />
          <span>{t("File")}</span>
          <span>{t("Category")}</span>
          <span className="r">{t("Size")}</span>
        </div>
        <div className="gtable__rows">
          {shown.map((e) => (
            <div
              key={e.url}
              className={`grow2${checked.has(e.url) ? "" : " is-off"}`}
              onClick={() => {
                const n = new Set(checked);
                if (n.has(e.url)) n.delete(e.url);
                else n.add(e.url);
                setChecked(n);
              }}
            >
              <Checkbox checked={checked.has(e.url)} label={t("Select {name}", { name: e.fileName })} onChange={() => undefined} />
              <FileTile item={{ fileName: e.fileName || e.url, kind: "http", category: e.category, media: null }} size={26} />
              <span className="nm">
                <b>{e.fileName || e.url.split("/").pop()}</b>
                <small>{e.text ? `${e.text} · ` : ""}{hostOf(e.url)}</small>
              </span>
              <span className="dim">{categories.find((c) => c.id === e.category)?.name ?? "—"}</span>
              <span className="r tnum">{formatBytes(e.size)}</span>
            </div>
          ))}
          {!shown.length && <p className="wempty">{batch ? t("No links match the filter.") : t("Loading…")}</p>}
        </div>
      </div>
      <div className="batch__save">
        <span className="field__label">{t("Save to")}</span>
        <div className="withbtn">
          <TextField value={dir} onChange={setDir} icon="folder" label={t("Save folder")} placeholder={t("Category folders (default)")} />
          {!webMode && (
            <button type="button" className="btn" onClick={async () => setDir((await pickFolder(dir)) ?? dir)}>
              {t("Browse…")}
            </button>
          )}
        </div>
      </div>
      <footer className="dialog__footer">
        <button type="button" className="btn btn--ghost btn--sm" onClick={() => setAll(true)}>
          {t("Select all")}
        </button>
        <button type="button" className="btn btn--ghost btn--sm" onClick={() => setAll(false)}>
          {t("Select none")}
        </button>
        <span className="spacer" />
        {webMode && (
          <button type="button" className="btn btn--ghost btn--sm" onClick={cancel}>
            {t("Cancel")}
          </button>
        )}
        <span className="sumtext">
          {tr("{selected} of {total} · {size}", { selected: <b>{sel.length}</b>, total: entries.length, size: <b>{formatBytes(sel.reduce((s, e) => s + (e.size ?? 0), 0))}</b> })}
        </span>
        <div className="split">
          <button type="button" className="btn" disabled={!sel.length || busy} onClick={() => confirm({ type: "later" })}>
            {t("Later")}
          </button>
          <button
            type="button"
            className="btn"
            aria-label={t("Add to queue")}
            disabled={!sel.length || busy}
            onClick={(ev) => openMenuAt(ev.currentTarget, [{ heading: t("Add to queue") }, ...queues.map((q) => ({ label: displayName(q.name), icon: "queue" as const, onSelect: () => void confirm({ type: "queue", queueId: q.id }) }))])}
          >
            <Icon name="chevron-down" size={13} strokeWidth={2.2} />
          </button>
        </div>
        <button type="button" className="btn btn--primary" disabled={!sel.length || busy} onClick={() => confirm({ type: "now" })}>
          <Icon name="download" size={15} /> {t("Download {number}", { number: sel.length })}
        </button>
      </footer>
    </WindowFrame>
  );
}
