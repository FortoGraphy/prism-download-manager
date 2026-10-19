import { type ReactNode, useEffect, useRef, useState } from "react";
import { IconButton } from "../components/IconButton";
import { useItemDetails } from "../components/useItemDetails";
import { ConnectionLanes, FileMap, SpeedChart } from "../components/Visuals";
import { Icon } from "../design/icons";
import { actions } from "../lib/actions";
import { formatBytes, formatDate, formatDuration, formatSpeed } from "../lib/format";
import { fullPath } from "../lib/paths";
import { progressOf, statusView } from "../lib/present";
import { Ring } from "../lib/ring";
import { useApp } from "../lib/store";
import type { ItemDetails } from "../lib/types";
import { useUi } from "../lib/ui";
import { copyText } from "../lib/win";

const TABS = [
  { id: "connections", label: "Connections" },
  { id: "speed", label: "Speed" },
  { id: "info", label: "Info" },
] as const;

/** Collapsible pane under the list showing the focused download (C-style engineering view). */
export function BottomPane() {
  const open = useUi((s) => s.bottomOpen);
  const tab = useUi((s) => s.bottomTab);
  const height = useUi((s) => s.bottomHeight);
  const focus = useApp((s) => s.selection[s.selection.length - 1] ?? null);
  const item = useApp((s) => (focus ? s.items[focus] : undefined));
  const details = useItemDetails(item && item.status !== "completed" ? focus : null, 500, open);

  const startResize = (e: React.PointerEvent) => {
    e.preventDefault();
    const y0 = e.clientY;
    const h0 = height;
    const max = Math.max(180, window.innerHeight - 320);
    const move = (ev: PointerEvent) => useUi.getState().setBottom({ bottomHeight: Math.min(max, Math.max(150, h0 - (ev.clientY - y0))) });
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      document.body.classList.remove("is-resizing-v");
    };
    document.body.classList.add("is-resizing-v");
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  // The pane only appears for a selection; with nothing selected the list gets the room.
  if (!open || !item) return null;
  return (
    <section className="pane" aria-label="Selected download">
      <div className="pane__resize" onPointerDown={startResize} aria-hidden="true" />
      <div className="pane__tabs" role="tablist">
        {TABS.map((t) => (
          <button key={t.id} type="button" role="tab" aria-selected={tab === t.id} className={`pane__tab${tab === t.id ? " on" : ""}`} onClick={() => useUi.getState().setBottom({ bottomTab: t.id })}>
            {t.label}
          </button>
        ))}
        <span className="spacer" />
        <span className="pane__file">
          <span className="ellipsis">{item.fileName}</span>
          {details?.segments.length ? <span className="badge badge--accent">{details.segments.filter((s) => s.state === "active").length} conn</span> : null}
        </span>
        <IconButton icon="external" size={15} label="Open details window" onClick={() => actions.details(item.id)} />
        <IconButton icon="chevron-down" size={15} label="Hide pane" onClick={() => useUi.getState().setBottom({ bottomOpen: false })} />
      </div>
      <div className="pane__body">
        {tab === "connections" ? (
          <ConnectionsTab details={details} done={item.status === "completed"} />
        ) : tab === "speed" ? (
          <SpeedTab details={details} id={item.id} />
        ) : (
          <InfoTab id={item.id} />
        )}
      </div>
    </section>
  );
}

function ConnectionsTab({ details, done }: { details: ItemDetails | null; done: boolean }) {
  if (done) return <p className="pane__hint">Download complete — no open connections.</p>;
  if (!details) return <p className="pane__hint">Loading…</p>;
  const size = details.item.size ?? 0;
  const unit = details.item.kind === "hls" || details.item.kind === "dash" ? "segments" : "bytes";
  return (
    <div className="pane__conn">
      <div className="pane__map">
        <span>File map</span>
        <FileMap segments={details.segments} size={size} unit={unit} />
        <span className="tnum">{details.segments.length} segments</span>
      </div>
      <ConnectionLanes segments={details.segments} unit={unit} />
    </div>
  );
}

function SpeedTab({ details, id }: { details: ItemDetails | null; id: string }) {
  const progress = useApp((s) => s.progress[id]);
  const ring = useRef(new Ring(120));
  const [, force] = useState(0);
  // Seed with the core's history, then append live progress samples.
  useEffect(() => {
    ring.current = new Ring(120);
  }, [id]);
  useEffect(() => {
    if (details && ring.current.size === 0) details.speedHistory.slice(-120).forEach((v) => ring.current.push(v));
  }, [details]);
  useEffect(() => {
    if (progress) {
      ring.current.push(progress.speed);
      force((n) => n + 1);
    }
  }, [progress]);
  const samples = ring.current.values();
  const avg = samples.length ? samples.reduce((a, b) => a + b, 0) / samples.length : 0;
  return (
    <div className="pane__speed">
      <SpeedChart samples={samples} height={140} />
      <div className="sstats">
        <Stat k="Now" v={formatSpeed(progress?.speed)} />
        <Stat k="Average" v={formatSpeed(avg)} />
        <Stat k="Peak" v={formatSpeed(ring.current.max())} />
        <Stat k="Time left" v={formatDuration(progress?.etaSecs)} />
      </div>
    </div>
  );
}

const Stat = ({ k, v }: { k: string; v: ReactNode }) => (
  <div className="sstat">
    <div className="k">{k}</div>
    <div className="v tnum">{v}</div>
  </div>
);

function InfoTab({ id }: { id: string }) {
  const item = useApp((s) => s.items[id]);
  const progress = useApp((s) => s.progress[id]);
  const cat = useApp((s) => s.categories.find((c) => c.id === item?.category)?.name);
  const queue = useApp((s) => s.queues.find((q) => q.id === item?.queueId)?.name);
  if (!item) return null;
  const p = progressOf(item, progress);
  const rows: [string, ReactNode, string?][] = [
    ["Address", item.url, item.url],
    ...(item.finalUrl && item.finalUrl !== item.url ? ([["Final address", item.finalUrl, item.finalUrl]] as [string, ReactNode, string][]) : []),
    ["Saved to", fullPath(item.saveDir, item.fileName), fullPath(item.saveDir, item.fileName)],
    ["Status", statusView(item, progress).detail],
    ["Size", `${formatBytes(p.done)} of ${formatBytes(p.total)}`],
    ["Resume", item.resumable === false ? "Not supported — restarts from zero" : item.resumable ? "Supported" : "Unknown"],
    ["Category", cat ?? item.category],
    ["Queue", queue ?? "—"],
    ["Connections", String(item.connections)],
    ["Speed limit", item.speedLimit ? `${formatBytes(item.speedLimit)}/s` : "None"],
    ["Added", formatDate(item.addedAt)],
    ["Finished", formatDate(item.finishedAt)],
    ["Checksum", item.checksum || "—"],
    ...(item.description ? ([["Description", item.description]] as [string, ReactNode][]) : []),
  ];
  return (
    <dl className="kv">
      {rows.map(([k, v, copy]) => (
        <div key={k} className="kv__row">
          <dt>{k}</dt>
          <dd className={copy ? "t-mono selectable" : "selectable"}>
            {v}
            {copy && (
              <button type="button" className="kv__copy" aria-label={`Copy ${k}`} data-tip="Copy" onClick={() => void copyText(copy)}>
                <Icon name="copy" size={13} />
              </button>
            )}
          </dd>
        </div>
      ))}
    </dl>
  );
}
