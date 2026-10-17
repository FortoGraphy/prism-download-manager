// Human-readable formatting helpers (sizes, speeds, durations, dates).

const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function formatBytes(n?: number | null, digits = 1): string {
  if (n == null || !Number.isFinite(n)) return "—";
  if (n < 1024) return `${Math.round(n)} B`;
  let v = n;
  let i = 0;
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 100 ? v.toFixed(0) : v.toFixed(digits)} ${UNITS[i]}`;
}

export function formatSpeed(bps?: number | null): string {
  if (!bps) return "—";
  return `${formatBytes(bps)}/s`;
}

export function formatDuration(secs?: number | null): string {
  if (secs == null || !Number.isFinite(secs)) return "—";
  const s = Math.max(0, Math.round(secs));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, "0")}s`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ${String(m % 60).padStart(2, "0")}m`;
  return `${Math.floor(h / 24)}d ${h % 24}h`;
}

/** Media duration as h:mm:ss / m:ss. */
export function formatClock(secs?: number | null): string {
  if (secs == null) return "";
  const s = Math.round(secs);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

export function formatDate(ms?: number | null): string {
  if (!ms) return "—";
  const d = new Date(ms);
  const now = new Date();
  const sameDay = d.toDateString() === now.toDateString();
  const yesterday = new Date(now.getTime() - 86_400_000).toDateString() === d.toDateString();
  const time = d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  if (sameDay) return `Today ${time}`;
  if (yesterday) return `Yesterday ${time}`;
  return d.toLocaleDateString(undefined, { day: "numeric", month: "short", year: d.getFullYear() === now.getFullYear() ? undefined : "numeric" });
}

export function percent(done: number, total?: number | null): number | null {
  if (!total || total <= 0) return null;
  return Math.min(100, (done / total) * 100);
}

/** Parse "5M", "800k", "2.5 MB" etc. into bytes/s (0 on failure). */
export function parseRate(input: string): number {
  const m = input.trim().match(/^(\d+(?:\.\d+)?)\s*([kmg]?)(?:i?b)?(?:\/s)?$/i);
  if (!m) return 0;
  const mult = { "": 1, k: 1024, m: 1024 ** 2, g: 1024 ** 3 }[m[2].toLowerCase() as "" | "k" | "m" | "g"];
  return Math.round(parseFloat(m[1]) * mult);
}

export function fileExtension(name: string): string {
  const i = name.lastIndexOf(".");
  return i > 0 ? name.slice(i + 1).toLowerCase() : "";
}

export function hostOf(url?: string | null): string {
  if (!url) return "";
  try {
    return new URL(url).host;
  } catch {
    return "";
  }
}
