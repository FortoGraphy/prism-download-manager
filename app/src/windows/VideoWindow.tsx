import { useEffect, useMemo, useRef, useState } from "react";
import { Checkbox, Segmented, Switch, TextField } from "../components/Controls";
import { openMenuAt } from "../components/MenuHost";
import { PartPicker } from "../components/PartPicker";
import { WindowFrame } from "../components/WindowFrame";
import { Icon } from "../design/icons";
import { api, type PlaylistEntry, type VideoChoice, type VideoProbe } from "../lib/api";
import { formatBytes, formatClock, hostOf } from "../lib/format";
import { t, tk } from "../lib/i18n";
import { useApp } from "../lib/store";
import { displayName } from "../lib/present";
import type { SniffedStream, StartMode, VideoCapture, VideoSection } from "../lib/types";
import { attempt, toast } from "../lib/ui";
import { MIN_PART, defaultRange, formatRange, isWhole, ltr, partSize, timelineLength } from "../lib/videoPart";
import { pickFolder, win } from "../lib/win";

type Sponsor = "off" | "mark" | "remove";

/** Quality presets applied to every entry of a playlist (the core resolves them per video). */
const preset = (id: string, kind: "video" | "audio", label: string, subLabel: string, extra: Partial<VideoChoice> = {}): VideoChoice => ({
  id,
  kind,
  label,
  subLabel,
  hdr: false,
  container: "mp4",
  accelerable: false,
  ...extra,
});

export const PLAYLIST_CHOICES: VideoChoice[] = [
  preset("best", "video", tk("Best available"), tk("Highest resolution per video")),
  preset("v1080", "video", "1080p", tk("or the closest below"), { height: 1080 }),
  preset("v720", "video", "720p", tk("smaller files"), { height: 720 }),
  preset("v480", "video", "480p", tk("smaller files"), { height: 480 }),
  preset("v360", "video", "360p", tk("smaller files"), { height: 360 }),
  preset("a-mp3", "audio", "MP3", "320 kbps", { audioFormat: "mp3", audioBitrateKbps: 320 }),
];

type Format = "mp4" | "mp3";
type Extent = "whole" | "part";

/** The video's id where its address has it (prism-core `video_id`: YouTube links). */
function videoId(pageUrl: string): string | undefined {
  let u: URL;
  try {
    u = new URL(pageUrl);
  } catch {
    return undefined;
  }
  const host = u.hostname.replace(/^(www\.)+/, "").replace(/^(m\.)+/, "");
  const seg = u.pathname.split("/").slice(1);
  let id: string | null | undefined;
  if (host === "youtube.com" || host === "music.youtube.com") id = u.searchParams.has("v") ? u.searchParams.get("v") : ["shorts", "live", "embed"].includes(seg[0]) ? seg[1] : undefined;
  else if (host === "youtu.be") id = seg[0];
  return id || undefined;
}

/**
 * The name a single video gets without one typed, minus the extension: the name template from
 * Settings (prism-core `render_name_template` and `sanitize_name`). Only shown in the name
 * field; an unchanged name isn't sent, so the core still decides it.
 */
function videoFileStem(template: string, title: string, pageUrl: string): string {
  let tpl = template.trim();
  if (tpl.endsWith(".%(ext)s")) tpl = tpl.slice(0, -".%(ext)s".length);
  let out = title;
  if (tpl) {
    out = tpl.replaceAll("%(title)s", () => title).replaceAll("%(id)s", () => videoId(pageUrl) ?? "");
    // Fields Prism doesn't know, and the brackets they leave empty.
    for (let start = out.indexOf("%("); start >= 0; start = out.indexOf("%(")) {
      const end = out.indexOf(")s", start);
      if (end < 0) break;
      out = out.slice(0, start) + out.slice(end + 2);
    }
    for (const empty of ["[]", "()", "{}"]) out = out.replaceAll(empty, "");
    out = out.split(/\s+/).filter(Boolean).join(" ").replace(/^[-_\s]+|[-_\s]+$/g, "") || title;
  }
  const safe = (out.split(/[/\\]/).pop() ?? "").replace(/[<>:"|?*\u0000-\u001f]/g, "_").trim().replace(/[. ]+$/, "");
  return safe || "download";
}

/** The simple model: MP4 at a chosen quality, or MP3 audio only. */
function splitChoices(all: VideoChoice[]): { video: VideoChoice[]; mp3: VideoChoice | undefined } {
  const seen = new Set<string>();
  const video = all.filter((c) => {
    if (c.kind !== "video") return false;
    const key = `${c.height ?? 0}-${Math.round(c.fps ?? 0)}-${c.hdr ? 1 : 0}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
  const mp3 = all.find((c) => c.kind === "audio" && c.audioFormat === "mp3") ?? all.find((c) => c.kind === "audio");
  return { video, mp3 };
}

export default function VideoWindow({ id }: { id: string }) {
  const settings = useApp((s) => s.settings);
  const [cap, setCap] = useState<VideoCapture | null>(null);
  const [probe, setProbe] = useState<VideoProbe | null>(null);
  const [error, setError] = useState("");
  const [choice, setChoice] = useState<string>("");
  const [format, setFormat] = useState<Format>("mp4");
  const [more, setMore] = useState(false);
  const [entries, setEntries] = useState<Set<string>>(new Set());
  const [subs, setSubs] = useState(settings?.video.embedSubtitles ?? false);
  const [langs, setLangs] = useState(settings?.video.subtitleLangs ?? "en.*");
  const [thumb, setThumb] = useState(settings?.video.embedThumbnail ?? true);
  const [meta, setMeta] = useState(settings?.video.embedMetadata ?? true);
  const [embedChapters, setEmbedChapters] = useState(settings?.video.embedChapters ?? true);
  const [sponsor, setSponsor] = useState<Sponsor>(settings?.video.sponsorblock ?? "off");
  const [dir, setDir] = useState("");
  const [oneByOne, setOneByOne] = useState(false);
  const [extent, setExtent] = useState<Extent>("whole");
  const [range, setRange] = useState<VideoSection | null>(null);
  // The file name as typed; `null` until the user changes it.
  const [typedName, setTypedName] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // The container set in Settings › Video & streams (MP4, MKV or WebM).
  const container = (["mp4", "mkv", "webm"] as const).find((c) => c === settings?.video.container) ?? "mp4";
  const containerName = container === "webm" ? "WebM" : container.toUpperCase();

  useEffect(() => {
    void (async () => {
      const c = await attempt(api.getVideo(id), t("Could not load the video"));
      if (!c) return setError(t("This video request is no longer available."));
      setCap(c);
      try {
        const p = await api.probeVideo(c.pageUrl);
        setProbe(p);
        if (p.kind === "video") {
          const pref = settings?.video.preferredHeight ?? 1080;
          const pick = p.choices.find((x) => x.kind === "video" && (x.height ?? 0) <= pref) ?? p.choices[0];
          // A quality picked in the browser's menu comes picked here; "Audio only" opens on MP3.
          const pre = c.preselected ? p.choices.find((x) => x.id === c.preselected) : undefined;
          if (pre?.kind === "audio") setFormat("mp3");
          setChoice((pre && pre.kind !== "audio" ? pre.id : pick?.id) ?? "");
        } else {
          // The preferred quality from Settings, or the closest preset below it (#99).
          const pref = settings?.video.preferredHeight ?? 1080;
          const pick = PLAYLIST_CHOICES.find((x) => x.kind === "video" && (x.height ?? 0) > 0 && (x.height ?? 0) <= pref);
          setChoice(pick?.id ?? "v1080");
          setEntries(new Set(p.entries.map((e) => e.url)));
        }
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    })();
  }, [id, settings?.video.preferredHeight]);

  const choices = probe?.kind === "video" ? probe.choices : PLAYLIST_CHOICES;
  const { video, mp3 } = useMemo(() => splitChoices(choices), [choices]);
  const picked = format === "mp3" ? mp3 : video.find((c) => c.id === choice) ?? video[0];
  const subLangs = useMemo(() => (probe?.kind === "video" ? Object.keys(probe.subtitles ?? {}) : []), [probe]);
  const named = (probe?.kind === "video" ? probe.title : probe?.kind === "playlist" ? probe.title : null) ?? cap?.title;
  // The English fallback goes to the core; only the shown one is translated.
  const title = named ?? "Video";
  // A single video can be renamed (#94); the format decides the extension, as in the core.
  const stem = cap ? videoFileStem(settings?.video.fileTemplate ?? "", title, cap.pageUrl) : "";
  const ext = format === "mp3" ? (picked?.audioFormat ?? "mp3") : container;
  const fileName = typedName?.trim();

  // "Only a part": a single video of known length that isn't live.
  const info = probe?.kind === "video" ? probe : null;
  const live = !!info && (info.isLive || ["is_live", "is_upcoming", "post_live"].includes(info.liveStatus ?? ""));
  const total = info?.duration && !live ? timelineLength(info.duration) : 0;
  const canCut = total > MIN_PART;
  const chapters = useMemo(() => (info?.chapters ?? []).filter((c) => c.end > c.start), [info]);
  const cutting = canCut && extent === "part" && range != null;
  // Everything selected is the whole video: a normal download.
  const section = cutting && !isWhole(range, total) ? range : null;
  const pickExtent = (v: Extent) => {
    setExtent(v);
    if (v === "part" && !range) setRange(defaultRange(total, chapters));
  };
  // The part panel makes the quality list scroll: keep the picked quality in sight.
  const qualities = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const list = qualities.current;
    const row = list?.querySelector(".qopt.on")?.getBoundingClientRect();
    const box = list?.getBoundingClientRect();
    if (!list || !row || !box) return;
    if (row.bottom > box.bottom) list.scrollTop += row.bottom - box.bottom + 4;
    else if (row.top < box.top) list.scrollTop -= box.top - row.top + 4;
  }, [cutting, picked?.id, probe]);

  const confirm = async (mode: StartMode) => {
    if (!cap || !picked) return;
    setBusy(true);
    const n = await attempt(
      api.confirmVideo({
        captureId: cap.id,
        pageUrl: cap.pageUrl,
        choice: picked,
        options: {
          container,
          audioFormat: "mp3",
          embedSubtitles: subs,
          subLangs: langs.split(/[\s,]+/).filter(Boolean),
          embedThumbnail: thumb,
          embedMetadata: meta,
          embedChapters,
          sponsorblock: { mode: sponsor, categories: ["sponsor", "selfpromo", "interaction"] },
        },
        saveDir: dir || undefined,
        title,
        thumbnail: info?.thumbnail ?? undefined,
        duration: info?.duration ?? undefined,
        entries:
          probe?.kind === "playlist"
            ? probe.entries.flatMap((e, i) => (entries.has(e.url) ? [{ url: e.url, title: e.title ?? null, index: i + 1 }] : []))
            : [],
        oneAtATime: probe?.kind === "playlist" && oneByOne,
        section: section ?? undefined,
        fileName: probe?.kind === "video" && fileName && fileName !== stem ? fileName : undefined,
        mode,
      }),
      t("Could not add the video"),
    );
    setBusy(false);
    if (n !== undefined) void win.close();
  };
  const cancel = () => {
    if (cap) void api.cancelVideo(cap.id);
    void win.close();
  };

  return (
    <WindowFrame kind="video" title={t("Download video")} icon={<Icon name="video" size={17} className="text-accent" />} onClose={cancel} className="video">
      <div className="wbody">
        <div className="vhead">
          <span className="vthumb" style={probe?.kind === "video" && probe.thumbnail ? { backgroundImage: `url("${probe.thumbnail}")` } : undefined}>
            {probe?.kind === "video" && probe.duration ? <span className="thumb__dur">{formatClock(probe.duration)}</span> : null}
            {!probe && !error && <span className="vthumb__shimmer" />}
          </span>
          <div className="vhead__txt">
            <b>{named ?? t("Video")}</b>
            <small>
              {probe?.kind === "video"
                ? [probe.uploader ?? probe.channel, probe.isLive ? t("Live") : null, hostOf(cap?.pageUrl)].filter(Boolean).join(" · ")
                : probe?.kind === "playlist"
                  ? t("Playlist · {count} videos", { count: probe.entries.length })
                  : hostOf(cap?.pageUrl)}
            </small>
          </div>
        </div>

        {!probe && !error && (
          <div className="vloading">
            <Icon name="retry" size={15} className="anim-spin" /> {t("Reading available formats…")}
          </div>
        )}
        {error && (
          <div className="banner banner--warn">
            <Icon name="warning" size={16} />
            <div>
              <b>{t("Couldn’t read this page’s formats")}</b>
              <div className="t-small">{error}</div>
            </div>
          </div>
        )}

        {probe?.kind === "playlist" && <Playlist entries={probe.entries} checked={entries} onChange={setEntries} />}
        {probe?.kind === "playlist" && (
          <div className="vopt vopt--plist">
            <Checkbox checked={oneByOne} onChange={setOneByOne}>
              {t("One at a time")} <span className="text-tertiary">— {t("each video starts when the one before it is done")}</span>
            </Checkbox>
          </div>
        )}

        {probe && (
          <div className="vformat">
            <Segmented<Format>
              label={t("Format")}
              value={format}
              onChange={setFormat}
              options={[
                { value: "mp4", label: t("{format} video", { format: containerName }) },
                { value: "mp3", label: t("MP3 audio") },
              ]}
            />
            {canCut && (
              <Segmented<Extent>
                label={t("Part of the video")}
                value={extent}
                onChange={pickExtent}
                options={[
                  { value: "whole", label: t("Whole video") },
                  { value: "part", label: t("Only a part") },
                ]}
              />
            )}
          </div>
        )}
        {cutting && <PartPicker total={total} chapters={chapters} value={range} onChange={setRange} size={partSize(picked?.estSize, range, info?.duration ?? total)} />}

        {probe && format === "mp4" && (
          <div ref={qualities} className={`choices${cutting && video.length > 2 ? " choices--shrink" : ""}`} role="radiogroup" aria-label={t("Quality")}>
            <div className="qsec">{probe.kind === "playlist" ? t("Quality for every video") : t("Quality")}</div>
            {video.map((c) => (
              <ChoiceRow key={c.id} c={c} on={c.id === picked?.id} onPick={() => setChoice(c.id)} />
            ))}
          </div>
        )}
        {probe && format === "mp3" && mp3 && (
          <div className="choices" role="radiogroup" aria-label={t("Audio")}>
            <div className="qsec">{t("Audio only")}</div>
            <ChoiceRow c={{ ...mp3, label: "MP3", subLabel: `320 kbps${mp3.estSize ? ` · ~${formatBytes(mp3.estSize)}` : ""}` }} on onPick={() => undefined} />
          </div>
        )}

        {probe && (
          <div className="vopts">
            {probe.kind === "video" && (
              <div className="vopt">
                <span>
                  <Icon name="file" size={15} /> {t("File name")}
                </span>
                <div className="withbtn">
                  <TextField value={typedName ?? stem} onChange={setTypedName} label={t("File name")} placeholder={stem} trailing={<span className="input__suffix" dir="ltr">.{ext}</span>} />
                </div>
              </div>
            )}
            <div className="vopt">
              <span>
                <Icon name="folder" size={15} /> {t("Save to")}
              </span>
              <div className="withbtn">
                <TextField value={dir} onChange={setDir} label={t("Save folder")} placeholder={format === "mp3" ? t("Music folder (default)") : t("Video folder (default)")} />
                <button type="button" className="btn" onClick={async () => setDir((await pickFolder(dir)) ?? dir)}>
                  {t("Browse…")}
                </button>
              </div>
            </div>
            <button type="button" className="vmore" aria-expanded={more} onClick={() => setMore(!more)}>
              <Icon name={more ? "chevron-up" : "chevron-down"} size={14} /> {t("More options")}
            </button>
          </div>
        )}

        {probe && more && (
          <div className="vopts">
            {section && (
              <div className="vopt vopt--note">
                <Icon name="info" size={15} />
                <span>{t("A part is saved without subtitles, chapters and SponsorBlock — they follow the whole video’s timing.")}</span>
              </div>
            )}
            <div className={`vopt${section ? " is-off" : ""}`} inert={!!section}>
              <span>
                <Icon name="subtitle" size={15} /> {t("Subtitles")} {subLangs.length ? <em>({subLangs.slice(0, 6).join(", ")})</em> : null}
              </span>
              <div className="inline">
                {subs && <TextField value={langs} onChange={setLangs} label={t("Subtitle languages")} mono />}
                <Switch checked={subs} label={t("Embed subtitles")} onChange={setSubs} />
              </div>
            </div>
            <div className="vopt">
              <span>
                <Icon name="thumbnail" size={15} /> {t("Embed")}
              </span>
              <div className="inline">
                <Checkbox checked={thumb} onChange={setThumb}>
                  {t("Thumbnail")}
                </Checkbox>
                <Checkbox checked={meta} onChange={setMeta}>
                  {t("Metadata")}
                </Checkbox>
                <span className={section ? "is-off" : undefined} inert={!!section}>
                  <Checkbox checked={embedChapters} onChange={setEmbedChapters}>
                    {t("Chapters")}
                  </Checkbox>
                </span>
              </div>
            </div>
            <div className={`vopt${section ? " is-off" : ""}`} inert={!!section}>
              <span>SponsorBlock</span>
              <Segmented<Sponsor>
                size="sm"
                label="SponsorBlock"
                value={sponsor}
                onChange={setSponsor}
                options={[
                  { value: "off", label: t("Off") },
                  { value: "mark", label: t("Mark") },
                  { value: "remove", label: t("Remove") },
                ]}
              />
            </div>
          </div>
        )}

        {cap && cap.streams.length > 0 && <Streams streams={cap.streams} pageUrl={cap.pageUrl} title={title} />}
      </div>
      <footer className="dialog__footer">
        <button type="button" className="btn btn--ghost" onClick={cancel}>
          {t("Cancel")}
        </button>
        <span className="spacer" />
        <div className="split">
          <button type="button" className="btn" disabled={!picked || busy} onClick={() => confirm({ type: "later" })}>
            {t("Later")}
          </button>
          <button
            type="button"
            className="btn"
            aria-label={t("Add to queue")}
            disabled={!picked || busy}
            onClick={(e) => openMenuAt(e.currentTarget, [{ heading: t("Add to queue") }, ...useApp.getState().queues.map((q) => ({ label: displayName(q.name), icon: "queue" as const, onSelect: () => void confirm({ type: "queue", queueId: q.id }) }))])}
          >
            <Icon name="chevron-down" size={13} strokeWidth={2.2} />
          </button>
        </div>
        <button type="button" className="btn btn--primary" disabled={!picked || busy || (probe?.kind === "playlist" && !entries.size)} onClick={() => confirm({ type: "now" })}>
          <Icon name="download" size={15} />
          {probe?.kind === "playlist"
            ? format === "mp3"
              ? t("Download {number} as MP3", { number: entries.size })
              : t("Download {count} videos", { count: entries.size })
            : section
              ? format === "mp3"
                ? t("Download {range} · {format}", { range: ltr(formatRange(section)), format: "MP3" })
                : t("Download {range} · {quality} {format}", { range: ltr(formatRange(section)), quality: quality(picked), format: containerName })
              : format === "mp3"
                ? t("Download MP3")
                : picked
                  ? t("Download {quality} {format}", { quality: quality(picked), format: containerName })
                  : t("Download")}
        </button>
      </footer>
    </WindowFrame>
  );
}

/** `1080p60` from `1080p60 · H.264`. */
const quality = (c: VideoChoice | undefined) => (c ? t(c.label).split(" · ")[0] : "");

function ChoiceRow({ c, on, onPick }: { c: VideoChoice; on: boolean; onPick: () => void }) {
  return (
    <button type="button" role="radio" aria-checked={on} className={`qopt${on ? " on" : ""}`} onClick={onPick}>
      <span className="radio" />
      <span className="lbl">
        {t(c.label)}
        {c.hdr && <span className="badge badge--accent">HDR</span>}
        <em>{t(c.subLabel)}</em>
      </span>
      <span className="sz tnum">{c.estSize && !c.subLabel.includes("~") ? `~${formatBytes(c.estSize)}` : ""}</span>
    </button>
  );
}

function Playlist({ entries, checked, onChange }: { entries: PlaylistEntry[]; checked: Set<string>; onChange: (s: Set<string>) => void }) {
  const all = entries.every((e) => checked.has(e.url));
  return (
    <div className="plist">
      <div className="plist__head">
        <Checkbox checked={all} indeterminate={!all && checked.size > 0} onChange={(v) => onChange(new Set(v ? entries.map((e) => e.url) : []))}>
          {t("{count} of {total} selected", { count: checked.size, total: entries.length })}
        </Checkbox>
      </div>
      <div className="plist__rows">
        {entries.map((e, i) => {
          // The row and its checkbox both toggle (the checkbox stops its click from reaching the row).
          const toggle = () => {
            const n = new Set(checked);
            if (n.has(e.url)) n.delete(e.url);
            else n.add(e.url);
            onChange(n);
          };
          return (
            <div key={e.url} className="plist__row" onClick={toggle}>
              <Checkbox checked={checked.has(e.url)} label={t("Select {name}", { name: e.title ?? e.url })} onChange={toggle} />
              <span className="tnum text-tertiary">{i + 1}</span>
              <span className="thumb" style={e.thumbnail ? { backgroundImage: `url("${e.thumbnail}")` } : undefined} />
              <span className="ellipsis">{e.title ?? e.url}</span>
              <span className="tnum text-tertiary">{formatClock(e.duration)}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function Streams({ streams, pageUrl, title }: { streams: SniffedStream[]; pageUrl: string; title: string }) {
  return (
    <div className="streams">
      <div className="qsec">{t("Streams detected on the page")}</div>
      {streams.map((s) => (
        <div key={s.url} className="stream">
          <span className="badge">{s.kind.toUpperCase()}</span>
          <span className="ellipsis t-mono" title={s.url}>
            {s.label || s.url}
          </span>
          <span className="tnum text-tertiary">{formatBytes(s.size)}</span>
          <button
            type="button"
            className="btn btn--sm"
            onClick={async () => {
              const item = await attempt(api.addStream(s, pageUrl, title), t("Could not add the stream"));
              if (item) toast(t("Stream added"), { kind: "success", body: item.fileName });
            }}
          >
            {t("Download")}
          </button>
        </div>
      ))}
    </div>
  );
}
