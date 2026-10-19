import { useState } from "react";
import { RateField, Switch } from "../components/Controls";
import { IconButton } from "../components/IconButton";
import { useItemDetails } from "../components/useItemDetails";
import { ConnectionLanes, FileMap, FileTile, ProgressBar, SpeedChart } from "../components/Visuals";
import { WindowFrame } from "../components/WindowFrame";
import { Icon } from "../design/icons";
import { api } from "../lib/api";
import { formatBytes, formatDuration, formatSpeed } from "../lib/format";
import { canPause, formatPct, progressOf, statusView } from "../lib/present";
import { useApp } from "../lib/store";
import { attempt } from "../lib/ui";
import { copyText, win } from "../lib/win";

export default function DetailsWindow({ id, simple }: { id: string; simple?: boolean }) {
  const item = useApp((s) => s.items[id]);
  const progress = useApp((s) => s.progress[id]);
  const settings = useApp((s) => s.settings);
  const details = useItemDetails(id, 500);
  const [pinned, setPinned] = useState(false);
  const [confirmCancel, setConfirmCancel] = useState(false);
  const [limitDraft, setLimitDraft] = useState<number | null>(null);

  if (!item)
    return (
      <WindowFrame kind="details" title="Download details">
        <div className="wempty">
          <Icon name="info" size={22} />
          <p>This download no longer exists.</p>
        </div>
      </WindowFrame>
    );

  const sv = statusView(item, progress);
  const p = progressOf(item, progress);
  const status = progress?.status ?? item.status;
  const segments = details?.segments ?? [];
  const active = segments.filter((s) => s.state === "active" && s.pos < s.end).length;
  const unit = item.kind === "hls" || item.kind === "dash" ? "segments" : "bytes";
  const history = details?.speedHistory.slice(-60) ?? [];
  const limit = limitDraft ?? item.speedLimit;
  const speed = progress?.speed ?? 0;
  const samples = history.length ? [...history.slice(0, -1), speed || history[history.length - 1]] : [];
  const avg = history.length ? history.reduce((a, b) => a + b, 0) / history.length : 0;

  const setLimit = (bps: number) => {
    setLimitDraft(bps);
    void attempt(api.updateItem(id, { speedLimit: bps }), "Could not set the limit");
  };

  return (
    <WindowFrame
      kind="details"
      minimizable
      className="details"
      icon={<Icon name={sv.icon} size={16} className={`text-${sv.tone === "neutral" ? "tertiary" : sv.tone}`} />}
      title={`${sv.label}${p.pct != null && status !== "completed" ? ` · ${Math.floor(p.pct)}%` : ""} · ${item.fileName}`}
      actions={
        <IconButton
          icon="pin"
          size={15}
          label={pinned ? "Unpin from top" : "Keep on top"}
          pressed={pinned}
          onClick={() => {
            setPinned(!pinned);
            void win.setAlwaysOnTop(!pinned);
          }}
        />
      }
    >
      <div className="hero">
        <FileTile item={item} size={44} thumbWidth={72} />
        <div className="grow">
          <div className="hero__name ellipsis" title={item.fileName}>
            {item.fileName}
          </div>
          <div className="hero__url">
            <Icon name="link" size={13} />
            <span className="ellipsis selectable">{item.finalUrl || item.url}</span>
            <button type="button" className="kv__copy" aria-label="Copy address" data-tip="Copy address" onClick={() => void copyText(item.url)}>
              <Icon name="copy" size={13} />
            </button>
          </div>
        </div>
        <span className={`badge badge--${sv.tone === "neutral" ? "" : sv.tone}`} data-tip={sv.detail}>
          {status === "downloading" ? <span className="status-dot status-dot--live" /> : <Icon name={sv.icon} size={12} strokeWidth={2} />}
          {sv.label}
        </span>
        {item.resumable !== false && (
          <span className="badge badge--success">
            <Icon name="shield" size={12} strokeWidth={2} /> Resumable
          </span>
        )}
      </div>

      <div className="meter">
        <div className="meter__row">
          <div className="pct tnum">
            {p.pct == null ? "—" : formatPct(p.pct).replace("%", "")}
            <small>%</small>
          </div>
          <div className="of tnum">
            <b>{formatBytes(p.done)}</b> of {formatBytes(p.total)}
          </div>
          <div className="kpis">
            <div>
              <div className="kpi__k">Speed</div>
              <div className="kpi__v tnum">{formatSpeed(speed)}</div>
            </div>
            <div>
              <div className="kpi__k">Time left</div>
              <div className="kpi__v tnum">{status === "downloading" ? formatDuration(progress?.etaSecs) : "—"}</div>
            </div>
            <div>
              <div className="kpi__k">Connections</div>
              <div className="kpi__v tnum">
                {active}
                <small> / {item.connections}</small>
              </div>
            </div>
          </div>
        </div>
        <ProgressBar value={p.fraction} state={sv.bar} size="thick" label="Progress" />
        <div className="map">
          <span>File map</span>
          <FileMap segments={segments} size={item.size ?? 0} unit={unit} />
          <span className="tnum">{segments.length} segments</span>
        </div>
        {status === "failed" && item.error && (
          <div className="banner banner--danger">
            <Icon name="status-error" size={16} />
            <span>{item.error}</span>
          </div>
        )}
      </div>

      <div className={`panel${simple ? " panel--single" : ""}`}>
        <div className="panel__conn">
          <div className="ph">Connections</div>
          {status === "completed" ? <p className="lanes__empty">Download complete.</p> : <ConnectionLanes segments={segments} max={12} unit={unit} />}
        </div>
        {!simple && (
          <div className="panel__speed">
            <div className="ph">
              Speed <span>last 60 s</span>
            </div>
            <SpeedChart samples={samples} height={150} />
            <div className="sstats">
              <div className="sstat">
                <div className="k">Now</div>
                <div className="v tnum">{formatSpeed(speed)}</div>
              </div>
              <div className="sstat">
                <div className="k">Average</div>
                <div className="v tnum">{formatSpeed(avg)}</div>
              </div>
              <div className="sstat">
                <div className="k">Peak</div>
                <div className="v tnum">{formatSpeed(Math.max(0, ...history))}</div>
              </div>
            </div>
          </div>
        )}
      </div>

      <footer className="dialog__footer">
        <label className="opt" data-tip={limit === 0 && settings?.downloads.speedLimiterEnabled ? `Also shares the global ${formatSpeed(settings.downloads.speedLimit)} limit (Settings → Speed & queues)` : undefined}>
          <Switch checked={limit > 0} label="Limit speed" onChange={(on) => setLimit(on ? limit || 2 * 1024 * 1024 : 0)} />
          Limit
          {limit === 0 && settings?.downloads.speedLimiterEnabled && <span className="text-tertiary"> (global {formatSpeed(settings.downloads.speedLimit)} applies)</span>}
        </label>
        {limit > 0 && <RateField value={limit} onCommit={setLimit} label="Speed limit" width={120} />}
        <span className="spacer" />
        {status !== "completed" &&
          (confirmCancel ? (
            <button
              type="button"
              className="btn btn--danger-solid"
              onClick={async () => {
                await attempt(api.deleteItems([id], true, false), "Could not cancel");
                void win.close();
              }}
            >
              Delete partial file
            </button>
          ) : (
            <button type="button" className="btn btn--ghost btn--danger" onClick={() => setConfirmCancel(true)}>
              Cancel
            </button>
          ))}
        <button type="button" className="btn" onClick={() => void attempt(api.revealItem(id), "Could not open the folder")}>
          <Icon name="folder-open" size={15} /> Folder
        </button>
        {status === "completed" ? (
          <button type="button" className="btn btn--primary" onClick={() => void attempt(api.openItem(id), "Could not open the file")}>
            <Icon name="open-file" size={15} /> Open
          </button>
        ) : canPause(status) ? (
          <button type="button" className="btn btn--primary" onClick={() => void attempt(api.pauseItems([id]), "Could not pause")}>
            <Icon name="pause" size={15} /> Pause
          </button>
        ) : (
          <button type="button" className="btn btn--primary" onClick={() => void attempt(api.startItems([id]), "Could not resume")}>
            <Icon name="resume" size={15} /> Resume
          </button>
        )}
      </footer>
    </WindowFrame>
  );
}
