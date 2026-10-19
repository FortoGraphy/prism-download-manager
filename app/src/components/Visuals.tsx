// Small data visuals: progress bar, file tile/thumbnail, speed chart, sparkline,
// connection lanes and file map.

import { memo, useId } from "react";
import { Icon } from "../design/icons";
import { formatBytes, formatClock, formatSpeed } from "../lib/format";
import { fileVisual, type ProgressState } from "../lib/present";
import { smoothPath } from "../lib/ring";
import type { DownloadItem, SegmentInfo } from "../lib/types";

export function ProgressBar({ value, state, size, label }: { value: number | null; state: ProgressState; size?: "thin" | "thick" | "xl"; label?: string }) {
  const indeterminate = value == null && state === "active";
  return (
    <div
      className={`progress${size ? ` progress--${size}` : ""}`}
      data-state={indeterminate ? "indeterminate" : state}
      style={{ ["--value" as string]: String(value ?? 0) }}
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={value == null ? undefined : Math.round(value * 100)}
    >
      <div className="progress__fill" />
    </div>
  );
}

/** 28 px type tile, or a 44×28 thumbnail for media with artwork. */
export const FileTile = memo(function FileTile({ item, size = 28, thumbWidth = 44 }: { item: Pick<DownloadItem, "fileName" | "kind" | "category" | "media">; size?: number; thumbWidth?: number }) {
  const thumb = item.media?.thumbnail;
  if (thumb) {
    return (
      <span className="thumb" style={{ width: thumbWidth, height: size, backgroundImage: `url("${thumb}")` }}>
        {item.media?.duration ? <span className="thumb__dur">{formatClock(item.media.duration)}</span> : null}
      </span>
    );
  }
  const v = fileVisual(item);
  return (
    <span className="file-tile" style={{ ["--tint" as string]: `var(${v.tint})`, width: size, height: size, borderRadius: Math.round(size * 0.29) }}>
      <Icon name={v.icon} size={Math.round(size * 0.58)} />
    </span>
  );
});

/** Area chart for speed history. Horizontal gridlines with labels in MB/s. */
export function SpeedChart({ samples, height = 160, labels = true }: { samples: number[]; height?: number; labels?: boolean }) {
  const id = useId().replace(/:/g, "");
  const W = 300;
  const H = 100;
  const max = niceMax(Math.max(...samples, 1));
  const { line, area } = smoothPath(samples.length > 1 ? samples : [0, 0], W, H, max / 1.1, 3);
  const last = samples[samples.length - 1] ?? 0;
  const lastY = H - 3 - (last / max) * (H - 6);
  const ticks = [0.25, 0.5, 0.75, 1];
  return (
    <div className="chart" style={{ height }}>
      {labels &&
        ticks.map((t) => (
          <span key={t} className="chart__ylab" style={{ top: `${(1 - t) * 100}%` }}>
            {formatBytes(max * t, 0).replace(" ", " ")}
          </span>
        ))}
      <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" aria-hidden="true">
        <defs>
          <linearGradient id={`${id}a`} x1="0" x2="0" y1="0" y2="1">
            <stop offset="0" stopColor="var(--accent)" stopOpacity=".38" />
            <stop offset="1" stopColor="var(--accent)" stopOpacity="0" />
          </linearGradient>
        </defs>
        {ticks.map((t) => (
          <path key={t} d={`M0 ${H - t * H}H${W}`} stroke="var(--divider)" vectorEffect="non-scaling-stroke" />
        ))}
        <path d={area} fill={`url(#${id}a)`} />
        <path d={line} fill="none" stroke="var(--accent)" strokeWidth="1.75" strokeLinejoin="round" vectorEffect="non-scaling-stroke" />
      </svg>
      {samples.length > 1 && <span className="chart__dot" style={{ top: `${(lastY / H) * 100}%` }} />}
      <span className="sr-only">Current speed {formatSpeed(last)}</span>
    </div>
  );
}

function niceMax(v: number): number {
  const mb = 1024 * 1024;
  const steps = [256 * 1024, 512 * 1024, mb, 2 * mb, 4 * mb, 5 * mb, 10 * mb, 20 * mb, 25 * mb, 40 * mb, 50 * mb, 80 * mb, 100 * mb, 200 * mb, 400 * mb, 1000 * mb];
  return steps.find((s) => s >= v * 1.05) ?? v * 1.2;
}

export function Sparkline({ samples, width = 196, height = 38 }: { samples: number[]; width?: number; height?: number }) {
  const id = useId().replace(/:/g, "");
  const { line, area } = smoothPath(samples.length > 1 ? samples : [0, 0], width, height, undefined, 2);
  return (
    <svg className="spark" viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="none" aria-hidden="true">
      <defs>
        <linearGradient id={`${id}s`} x1="0" x2="0" y1="0" y2="1">
          <stop offset="0" stopColor="var(--accent)" stopOpacity=".35" />
          <stop offset="1" stopColor="var(--accent)" stopOpacity="0" />
        </linearGradient>
      </defs>
      <path d={area} fill={`url(#${id}s)`} />
      <path d={line} stroke="var(--accent)" strokeWidth="1.5" fill="none" vectorEffect="non-scaling-stroke" />
    </svg>
  );
}

const spectrum = (i: number) => `var(--spectrum-${(i % 8) + 1})`;

/** Whole-file map: each segment occupies its range (bytes, or — for HLS/DASH — a position
 *  among the media segments); the filled part is coloured per connection. */
export function FileMap({ segments, size, unit = "bytes" }: { segments: SegmentInfo[]; size: number; unit?: "bytes" | "segments" }) {
  const total = unit === "segments" ? segments.length : size;
  if (!total || !segments.length) return <div className="segbar" />;
  return (
    <div className="segbar" role="img" aria-label={`${segments.length} segments`}>
      {segments.map((s, i) => {
        const span = Math.max(1, s.end - s.start);
        return (
          <span
            key={i}
            className="segbar__seg"
            data-active={s.state === "active" && s.pos < s.end}
            style={{
              ["--a" as string]: s.start / total,
              ["--b" as string]: s.end / total,
              ["--p" as string]: Math.min(1, (s.pos - s.start) / span),
              ["--c" as string]: spectrum(i),
            }}
          />
        );
      })}
    </div>
  );
}

/** One lane per connection — the segments actually being transferred right now, never the
 *  whole (for HLS/DASH, hundreds-strong) segment list. */
export function ConnectionLanes({ segments, max, unit = "bytes" }: { segments: SegmentInfo[]; max?: number; unit?: "bytes" | "segments" }) {
  const active = segments.filter((s) => s.state === "active");
  const shown = max ? active.slice(0, max) : active;
  if (!active.length) return <p className="lanes__empty">No active connections.</p>;
  return (
    <div className="lanes">
      {shown.map((s, i) => {
        const span = Math.max(1, s.end - s.start);
        const f = Math.min(1, (s.pos - s.start) / span);
        return (
          <div key={i} className="lane" style={{ ["--c" as string]: spectrum(i) }}>
            <span className="lane__id">{String(i + 1).padStart(2, "0")}</span>
            <span className="lane__track">
              <span className="lane__fill" style={{ transform: `scaleX(${f})` }} />
              <span className="lane__head" style={{ left: `calc(${f * 100}% - 1px)` }} />
            </span>
            <span className="lane__range">{unit === "segments" ? `Segment ${s.start + 1}` : `${formatBytes(s.start)}–${formatBytes(s.end)}`}</span>
            <span className="lane__speed">{formatSpeed(s.speed)}</span>
          </div>
        );
      })}
      {max && active.length > max && <p className="lanes__more">+ {active.length - max} more connections</p>}
    </div>
  );
}
