/** Menu entries for the qualities an HLS/DASH manifest declares (`probeStream`). */
import { t } from './i18n';
import type { VideoOption } from './messages';
import type { StreamVariant } from './protocol';

/** The shorter side, as players name qualities: a 1080×1920 portrait stream is 1080p. */
function variantHeight(v: StreamVariant): number {
  return v.width && v.height ? Math.min(v.width, v.height) : (v.height ?? 0);
}

export function variantLabel(v: StreamVariant): string {
  const h = variantHeight(v);
  if (h) return `${h}p`;
  return v.bandwidth ? `${Math.round(v.bandwidth / 1000)} kbps` : t('Stream');
}

/**
 * One entry per variant, best first, each carrying the variant's index in the manifest for the app.
 * `null` when the manifest names fewer than two: the stream's single entry stays.
 */
export function variantOptions(base: VideoOption, kind: 'hls' | 'dash', variants: StreamVariant[]): VideoOption[] | null {
  const stream = base.stream;
  if (!stream || variants.length < 2) return null;
  return variants
    .slice()
    .sort((a, b) => variantHeight(b) - variantHeight(a) || (b.bandwidth ?? 0) - (a.bandwidth ?? 0))
    .map((v) => ({
      id: `${base.id}:${v.index}`,
      label: variantLabel(v),
      detail: [kind.toUpperCase(), ...(v.bandwidth ? [`${Math.round(v.bandwidth / 1000)} kbps`] : [])].join(' · '),
      group: base.group,
      stream: { ...stream, quality: variantLabel(v), variantIndex: v.index },
    }));
}
