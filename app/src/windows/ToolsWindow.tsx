import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { NumberField, Segmented } from "../components/Controls";
import { ProgressBar } from "../components/Visuals";
import { WindowFrame } from "../components/WindowFrame";
import { Icon } from "../design/icons";
import { api, inTauri, onAppEvent, onCoreEvent, webMode } from "../lib/api";
import { formatBytes } from "../lib/format";
import { t } from "../lib/i18n";
import {
  GIF_MAX_SECONDS,
  GRADE_NAMES,
  LOSSY,
  MB,
  PRESETS,
  TARGET_NAMES,
  baseName,
  beforeAfter,
  describe,
  kindIcon,
  kindTint,
  planSummary,
  planWarning,
  presetFileLabel,
  presetLabel,
  problemText,
  targetHint,
  type MediaFile,
  type MediaJob,
  type MediaRequest,
  type MediaTarget,
  type PresetId,
  type Quality,
  type ShrinkPlan,
  type ToolsRequest,
  type ToolsTab,
} from "../lib/media";
import { attempt } from "../lib/ui";
import { win } from "../lib/win";

type Choice = MediaTarget | "zip";

const KEY = "prism.tools";

/** The size and quality picked last time (this window only; a convenience, not a setting). */
function remembered(): { preset?: PresetId; customMb?: number; quality?: Quality } {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "{}");
  } catch {
    return {};
  }
}

function remember(patch: Record<string, unknown>) {
  try {
    localStorage.setItem(KEY, JSON.stringify({ ...remembered(), ...patch }));
  } catch {
    /* storage unavailable */
  }
}

/** Every extension the tools read, for the file picker. */
const PICK_EXTENSIONS = ["jpg", "jpeg", "png", "webp", "heic", "heif", "avif", "gif", "bmp", "tif", "tiff", "mp3", "m4a", "aac", "flac", "wav", "ogg", "opus", "wma", "mp4", "m4v", "mkv", "webm", "mov", "avi", "wmv", "ts"];

async function chooseFiles(input: HTMLInputElement | null): Promise<string[]> {
  if (inTauri) {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const r = await open({
      multiple: true,
      directory: false,
      filters: [
        { name: t("Pictures, music and videos"), extensions: PICK_EXTENSIONS },
        { name: t("All files"), extensions: ["*"] },
      ],
    });
    return Array.isArray(r) ? r : typeof r === "string" ? [r] : [];
  }
  // Browser preview: the browser's own picker, the files "in" the demo's Downloads folder.
  input?.click();
  return [];
}

const demoPath = (name: string) => `C:\\Users\\you\\Downloads\\${name}`;

/**
 * Tools: convert pictures, music and videos into other formats, or make them smaller for a size
 * limit (Discord, e-mail, WhatsApp). Results go next to the originals; the jobs keep running in
 * Prism when the window closes.
 */
export default function ToolsWindow({ request }: { request: ToolsRequest }) {
  const saved = useMemo(remembered, []);
  const [tab, setTab] = useState<ToolsTab>(request.tab);
  const [files, setFiles] = useState<MediaFile[]>([]);
  const [reading, setReading] = useState<string[]>([]);
  const [choice, setChoice] = useState<Choice | null>(null);
  const [quality, setQuality] = useState<Quality>(saved.quality ?? "standard");
  const [preset, setPreset] = useState<PresetId>(request.preset ?? saved.preset ?? "discord");
  const [customMb, setCustomMb] = useState(saved.customMb ?? 10);
  const [plans, setPlans] = useState<Record<string, ShrinkPlan>>({});
  const [jobs, setJobs] = useState<MediaJob[]>([]);
  const [over, setOver] = useState(false);
  const [fetching, setFetching] = useState<{ downloaded: number; total?: number | null } | null>(null);
  const [ffmpegMissing, setFfmpegMissing] = useState(false);
  const [busy, setBusy] = useState(false);
  const picker = useRef<HTMLInputElement>(null);
  const known = useRef<Set<string>>(new Set());

  const add = useCallback(async (paths: string[], replace = false) => {
    if (replace) known.current = new Set();
    const fresh = [...new Set(paths)].filter((p) => !known.current.has(p));
    if (replace) setFiles([]);
    if (!fresh.length) return;
    fresh.forEach((p) => known.current.add(p));
    setReading((r) => [...r, ...fresh]);
    const infos = await attempt(api.mediaInspect(fresh), t("Could not read the files"));
    setReading((r) => r.filter((p) => !fresh.includes(p)));
    setFetching(null);
    if (infos) setFiles((f) => [...f.filter((x) => !fresh.includes(x.path)), ...infos.filter((i) => known.current.has(i.path))]);
    else fresh.forEach((p) => known.current.delete(p));
  }, []);

  const remove = (path: string) => {
    known.current.delete(path);
    setFiles((f) => f.filter((x) => x.path !== path));
  };

  // What the window was opened for, and what later requests ask for (it's already open).
  useEffect(() => {
    if (request.paths?.length) void add(request.paths, true);
    let off: (() => void) | undefined;
    let gone = false;
    void onAppEvent<ToolsRequest>("tools-request", (r) => {
      setTab(r.tab);
      if (r.preset) setPreset(r.preset);
      if (r.paths?.length) void add(r.paths, true);
    }).then((u) => (gone ? u() : (off = u)));
    return () => {
      gone = true;
      off?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Jobs (also those started from the list's menu) and the converter's first download.
  useEffect(() => {
    void api.mediaJobs().then(setJobs, () => undefined);
    void api.toolStatus().then((s) => setFfmpegMissing(!s.ffmpeg?.path || !s.ffprobe?.path), () => undefined);
    let off: (() => void) | undefined;
    let gone = false;
    void onCoreEvent((e) => {
      if (e.type === "mediaJob") {
        const j = e.payload;
        setJobs((list) => (list.some((x) => x.id === j.id) ? list.map((x) => (x.id === j.id ? j : x)) : [...list, j]));
        if (j.state === "running" || j.state === "failed") {
          setFfmpegMissing(false);
          setFetching(null);
        }
      } else if (e.type === "toolsProgress" && e.payload.tool === "ffmpeg") {
        const { downloaded, total } = e.payload;
        setFetching(total && downloaded >= total ? null : { downloaded, total });
      }
    }).then((u) => (gone ? u() : (off = u)));
    return () => {
      gone = true;
      off?.();
    };
  }, []);

  // Files dropped on the window: Tauri hands over their paths (the browser preview only names).
  useEffect(() => {
    if (!inTauri) return;
    let off: (() => void) | undefined;
    let gone = false;
    void import("@tauri-apps/api/webview").then(({ getCurrentWebview }) =>
      getCurrentWebview()
        .onDragDropEvent((e) => {
          const p = e.payload;
          if (p.type === "enter" || p.type === "over") setOver(true);
          else if (p.type === "leave") setOver(false);
          else if (p.type === "drop") {
            setOver(false);
            if (p.paths.length) void add(p.paths);
          }
        })
        .then((u) => (gone ? u() : (off = u))),
    );
    return () => {
      gone = true;
      off?.();
    };
  }, [add]);

  // ---- convert ----------------------------------------------------------------------------
  const usable = files.filter((f) => !f.problem);
  const kinds = new Set(usable.map((f) => f.kind));
  const targets: MediaTarget[] = usable.length ? usable[0].targets.filter((x) => usable.every((f) => f.targets.includes(x))) : [];
  const choices: Choice[] = [...targets, ...(files.length > 1 || (files.length === 1 && !targets.length && !reading.length) ? (["zip"] as const) : [])];
  const fromKind = kinds.size === 1 ? [...kinds][0] : "other";
  const gifTooLong = usable.some((f) => (f.duration ?? 0) > GIF_MAX_SECONDS);

  useEffect(() => {
    if (choice && choices.includes(choice) && !(choice === "gif" && gifTooLong)) return;
    setChoice(choices.find((c) => !(c === "gif" && gifTooLong)) ?? null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [choices.join(","), gifTooLong]);

  // ---- make smaller ---------------------------------------------------------------------------
  const targetBytes = Math.round((preset === "custom" ? customMb : (PRESETS.find((p) => p.id === preset)?.mb ?? 20)) * MB);
  useEffect(() => {
    if (!files.length) return setPlans({});
    const timer = window.setTimeout(
      () => void api.mediaPlan(files, targetBytes).then((p) => setPlans(Object.fromEntries(files.map((f, i) => [f.path, p[i]]))), () => setPlans({})),
      120,
    );
    return () => window.clearTimeout(timer);
  }, [files, targetBytes]);
  const planOf = (f: MediaFile): ShrinkPlan | undefined => plans[f.path];
  const shrinkable = files.filter((f) => !f.problem && f.kind !== "other" && planOf(f)?.verdict === "ok");

  // ---- start ------------------------------------------------------------------------------------
  const canStart = !busy && !reading.length && (tab === "convert" ? !!choice && (choice === "zip" ? files.length > 0 : usable.length > 0) : shrinkable.length > 0);
  const start = async () => {
    if (!canStart) return;
    setBusy(true);
    const request: MediaRequest | null =
      tab === "shrink"
        ? { op: "shrink", paths: shrinkable.map((f) => f.path), targetBytes, label: presetFileLabel(preset) }
        : choice === "zip"
          ? { op: "zip", paths: files.map((f) => f.path) }
          : choice
            ? { op: "convert", paths: usable.map((f) => f.path), target: choice, quality }
            : null;
    if (request) await attempt(api.mediaStart(request), t("Could not start"));
    setBusy(false);
  };

  const startLabel =
    tab === "shrink" ? t("Make smaller") : choice === "zip" ? t("Pack into ZIP") : choice ? t("Convert to {format}", { format: TARGET_NAMES[choice] }) : t("Convert");
  const finished = jobs.filter((j) => ["done", "failed", "cancelled"].includes(j.state)).length;

  const pick = async () => {
    const paths = await chooseFiles(picker.current);
    if (paths.length) void add(paths);
  };

  return (
    <WindowFrame kind="tools" title={t("Tools")} icon={<Icon name={tab === "convert" ? "convert" : "shrink"} size={17} className="text-accent" />} className="toolswin" minimizable>
      <div
        className={`wbody tools${over ? " is-over" : ""}`}
        onDragOver={(e) => {
          if (inTauri) return;
          e.preventDefault();
          setOver(true);
        }}
        onDragLeave={() => !inTauri && setOver(false)}
        onDrop={(e) => {
          if (inTauri) return;
          e.preventDefault();
          setOver(false);
          void add(Array.from(e.dataTransfer.files).map((f) => demoPath(f.name)));
        }}
      >
        <div className="tools__tabs">
          <Segmented
            label={t("Tool")}
            value={tab}
            onChange={setTab}
            options={[
              { value: "convert", label: (<><Icon name="convert" size={15} /> {t("Convert")}</>) },
              { value: "shrink", label: (<><Icon name="shrink" size={15} /> {t("Make smaller")}</>) },
            ]}
          />
        </div>

        {fetching && (
          <div className="banner banner--info tools__fetch" role="status">
            <Icon name="download" size={16} />
            <span className="grow">
              {t("Getting the converter ready (FFmpeg, once)…")}
              <ProgressBar value={fetching.total ? fetching.downloaded / fetching.total : null} state="active" size="thin" label={t("Getting the converter ready (FFmpeg, once)…")} />
            </span>
            <span className="tnum t-small">{fetching.total ? `${Math.round((fetching.downloaded / fetching.total) * 100)}%` : formatBytes(fetching.downloaded)}</span>
          </div>
        )}

        {files.length === 0 && reading.length === 0 ? (
          webMode ? (
            <div className="tdrop">
              <span className="tdrop__icon">
                <Icon name={tab === "convert" ? "convert" : "shrink"} size={26} />
              </span>
              <b>{t("Pick a finished download in the list to convert it.")}</b>
            </div>
          ) : (
            <button type="button" className={`tdrop${over ? " is-over" : ""}`} onClick={() => void pick()}>
              <span className="tdrop__icon">
                <Icon name={tab === "convert" ? "convert" : "shrink"} size={26} />
              </span>
              <b>{t("Drop pictures, music or videos here")}</b>
              <span className="t-small text-tertiary">{tab === "convert" ? t("Or choose files. Several at once get the same format.") : t("Or choose files. Prism fits each one under the size you pick.")}</span>
              <span className="btn btn--sm">
                <Icon name="folder-open" size={14} /> {t("Choose files…")}
              </span>
            </button>
          )
        ) : (
          <div className={`tfiles${over ? " is-over" : ""}`} role="list" aria-label={t("Files")}>
            {files.map((f) => {
              const plan = tab === "shrink" ? planOf(f) : undefined;
              const warning = tab === "shrink" && plan && f.kind !== "other" && !f.problem ? planWarning(f, plan, targetBytes) : "";
              const small = plan?.verdict === "alreadySmall";
              return (
                <div key={f.path} role="listitem" className="tfile" title={f.path}>
                  <span className="file-tile" style={{ ["--tint" as string]: `var(${kindTint(f.kind)})`, width: 32, height: 32, borderRadius: 9 }}>
                    <Icon name={f.folder ? "folder" : kindIcon(f.kind)} size={18} />
                  </span>
                  <div className="tfile__txt">
                    <b className="ellipsis">{f.name}</b>
                    {f.problem ? (
                      <small className="text-danger">{problemText(f.problem)}</small>
                    ) : tab === "shrink" && f.kind === "other" ? (
                      <small className="text-tertiary">{t("Only pictures, music and videos can be made smaller.")}</small>
                    ) : small ? (
                      <small className="text-success">{problemText("alreadySmall", { size: targetBytes })}</small>
                    ) : (
                      <small className="ellipsis">{plan && plan.verdict === "ok" ? planSummary(f, plan) : describe(f)}</small>
                    )}
                    {warning && <small className={plan?.verdict === "ok" ? "text-warning" : "text-danger"}>{warning}</small>}
                  </div>
                  <span className="tfile__size tnum">
                    {plan?.verdict === "ok" && plan.estSize ? (
                      <>
                        <span className="text-tertiary">{formatBytes(f.size)} →</span> ≈ {formatBytes(plan.estSize)}
                      </>
                    ) : (
                      formatBytes(f.size)
                    )}
                  </span>
                  {plan?.verdict === "ok" && plan.grade ? (
                    <span className={`badge badge--${plan.grade === "good" ? "success" : plan.grade === "ok" ? "accent" : "warning"}`} data-tip={t("How it will look or sound")}>
                      {t(GRADE_NAMES[plan.grade])}
                    </span>
                  ) : null}
                  <button type="button" className="btn btn--icon btn--ghost btn--sm" aria-label={t("Remove {name}", { name: f.name })} data-tip={t("Remove")} onClick={() => remove(f.path)}>
                    <Icon name="close" size={13} />
                  </button>
                </div>
              );
            })}
            {reading.map((p) => (
              <div key={p} role="listitem" className="tfile is-reading">
                <span className="file-tile" style={{ width: 32, height: 32, borderRadius: 9 }}>
                  <Icon name="retry" size={16} className="anim-spin" />
                </span>
                <div className="tfile__txt">
                  <b className="ellipsis">{baseName(p)}</b>
                  <small className="text-tertiary">{t("Reading…")}</small>
                </div>
              </div>
            ))}
            <div className="tfiles__more">
              <button type="button" className="btn btn--ghost btn--sm" onClick={() => void pick()}>
                <Icon name="plus" size={14} /> {t("Add files…")}
              </button>
              <span className="t-small text-tertiary">{t("or drop more here")}</span>
            </div>
          </div>
        )}

        {tab === "convert" ? (
          <section className="tsec" aria-label={t("Convert to")}>
            <div className="tsec__head">
              <span className="field__label">{t("Convert to")}</span>
              <span className="spacer" />
              {choice && choice !== "zip" && LOSSY.includes(choice) && (
                <Segmented
                  size="sm"
                  label={t("Quality")}
                  value={quality}
                  onChange={(q) => {
                    setQuality(q);
                    remember({ quality: q });
                  }}
                  options={[
                    { value: "standard", label: t("Standard") },
                    { value: "high", label: t("High quality") },
                  ]}
                />
              )}
            </div>
            {choices.length ? (
              <div className="tchoices" role="radiogroup" aria-label={t("Convert to")}>
                {choices.map((c) => {
                  const off = c === "gif" && gifTooLong;
                  return (
                    <button
                      key={c}
                      type="button"
                      role="radio"
                      aria-checked={choice === c}
                      disabled={off}
                      className={`tchoice${choice === c ? " on" : ""}`}
                      data-tip={off ? t("GIF is for short clips: up to a minute.") : undefined}
                      onClick={() => setChoice(c)}
                    >
                      <b>{c === "zip" ? "ZIP" : TARGET_NAMES[c]}</b>
                      <span>{c === "zip" ? t("All files in one") : targetHint(c, fromKind)}</span>
                    </button>
                  );
                })}
              </div>
            ) : (
              <p className="t-small text-tertiary">{files.length ? t("These files can’t be converted together. Pick files of one kind.") : t("Pick files to see what they can become.")}</p>
            )}
          </section>
        ) : (
          <section className="tsec" aria-label={t("Make it fit")}>
            <div className="tsec__head">
              <span className="field__label">{t("Make it fit")}</span>
            </div>
            <div className="tchoices" role="radiogroup" aria-label={t("Size")}>
              {PRESETS.map((p) => (
                <button
                  key={p.id}
                  type="button"
                  role="radio"
                  aria-checked={preset === p.id}
                  className={`tchoice${preset === p.id ? " on" : ""}`}
                  onClick={() => {
                    setPreset(p.id);
                    remember({ preset: p.id });
                  }}
                >
                  <b>{presetLabel(p.id)}</b>
                  <span>{p.id === "email" ? t("Gmail’s limit") : p.id === "whatsapp" ? t("Photos and videos") : t("Free upload limit")}</span>
                </button>
              ))}
              <div className={`tchoice tchoice--custom${preset === "custom" ? " on" : ""}`}>
                <button
                  type="button"
                  role="radio"
                  aria-checked={preset === "custom"}
                  onClick={() => {
                    setPreset("custom");
                    remember({ preset: "custom" });
                  }}
                >
                  <b>{t("Custom size")}</b>
                  <span>{t("Any size in MB")}</span>
                </button>
                {preset === "custom" && (
                  <NumberField
                    value={customMb}
                    min={1}
                    max={100_000}
                    width={92}
                    suffix="MB"
                    label={t("Size in MB")}
                    onCommit={(v) => {
                      setCustomMb(v);
                      remember({ customMb: v });
                    }}
                  />
                )}
              </div>
            </div>
            <p className="t-small text-tertiary">{t("The originals stay as they are. Upload limits as of 2026; services can change them.")}</p>
          </section>
        )}

        {jobs.length > 0 && (
          <section className="tjobs" aria-label={t("Recent")}>
            <div className="tsec__head">
              <span className="field__label">{t("Recent")}</span>
              <span className="spacer" />
              {finished > 0 && (
                <button type="button" className="btn btn--ghost btn--sm" onClick={() => void api.mediaClear().then(setJobs, () => undefined)}>
                  {t("Clear finished")}
                </button>
              )}
            </div>
            <div className="tjobs__list">
              {[...jobs].reverse().map((j) => (
                <JobRow key={j.id} job={j} fetching={fetching} />
              ))}
            </div>
          </section>
        )}
        <input
          ref={picker}
          type="file"
          multiple
          hidden
          onChange={(e) => {
            const names = Array.from(e.target.files ?? []).map((f) => demoPath(f.name));
            e.target.value = "";
            if (names.length) void add(names);
          }}
        />
      </div>

      <footer className="dialog__footer">
        {ffmpegMissing && !fetching && <span className="t-small text-tertiary tools__note">{t("The first time, Prism downloads the converter (FFmpeg).")}</span>}
        <span className="spacer" />
        <button type="button" className="btn btn--ghost" onClick={() => void win.close()}>
          {t("Close")}
        </button>
        <button type="button" className="btn btn--primary" disabled={!canStart} onClick={() => void start()}>
          <Icon name={tab === "convert" ? (choice === "zip" ? "compressed" : "convert") : "shrink"} size={15} /> {startLabel}
        </button>
      </footer>
    </WindowFrame>
  );
}

function JobRow({ job: j, fetching }: { job: MediaJob; fetching: { downloaded: number; total?: number | null } | null }) {
  const pct = j.progress != null ? `${Math.round(j.progress * 100)}%` : "";
  const active = j.state === "queued" || j.state === "preparing" || j.state === "running";
  const shown = j.output && (j.state === "running" || j.state === "done") ? baseName(j.output) : j.name;
  let status: string;
  switch (j.state) {
    case "queued":
      status = t("Waiting…");
      break;
    case "preparing":
      status = fetching ? t("Getting the converter ready…") : t("Reading the file…");
      break;
    case "running":
      status = j.retrying
        ? t("Trying a little smaller… {percent}", { percent: pct })
        : j.op.type === "zip"
          ? t("Packing… {percent}", { percent: pct })
          : j.op.type === "shrink"
            ? t("Making it smaller… {percent}", { percent: pct })
            : t("Converting… {percent}", { percent: pct });
      break;
    case "done":
      status = beforeAfter(j.before, j.after);
      break;
    case "failed":
      status = problemText(j.problem ?? "failed", { size: j.op.type === "shrink" ? j.op.targetBytes : undefined, target: j.op.type === "convert" ? j.op.target : undefined });
      break;
    default:
      status = t("Stopped");
  }
  const icon = j.state === "done" ? "status-completed" : j.state === "failed" ? "status-error" : j.state === "cancelled" ? "x-circle" : j.op.type === "shrink" ? "shrink" : j.op.type === "zip" ? "compressed" : "convert";
  return (
    <div className="tjob" data-state={j.state}>
      <Icon name={icon} size={16} className="tjob__icon" />
      <div className="tjob__txt">
        <b className="ellipsis" title={j.output ?? j.inputs[0]}>
          {shown}
        </b>
        {active && <ProgressBar value={j.state === "running" ? (j.progress ?? null) : null} state={j.state === "running" ? "active" : "queued"} size="thin" label={shown} />}
        <small className={`ellipsis${j.state === "failed" ? " text-danger" : ""}`} title={j.detail ?? undefined}>
          {status}
        </small>
      </div>
      <div className="tjob__actions">
        {active && (
          <button type="button" className="btn btn--icon btn--ghost btn--sm" aria-label={t("Stop")} data-tip={t("Stop")} onClick={() => void api.mediaCancel(j.id)}>
            <Icon name="close" size={13} />
          </button>
        )}
        {!webMode && j.state === "done" && j.output && (
          <>
            <button type="button" className="btn btn--sm" onClick={() => void attempt(api.openFile(j.output!), t("Could not open the file"))}>
              {t("Open")}
            </button>
            <button type="button" className="btn btn--icon btn--sm" aria-label={t("Show in folder")} data-tip={t("Show in folder")} onClick={() => void attempt(api.revealPath(j.output!), t("Could not open the folder"))}>
              <Icon name="folder-open" size={14} />
            </button>
          </>
        )}
      </div>
    </div>
  );
}
