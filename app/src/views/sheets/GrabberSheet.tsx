import { useEffect, useMemo, useRef, useState } from "react";
import { Checkbox, Field, ListField, NumberField, Segmented, Stepper, Switch, TextField } from "../../components/Controls";
import { ProgressBar } from "../../components/Visuals";
import { Icon, type IconName } from "../../design/icons";
import { api, onCoreEvent, webMode } from "../../lib/api";
import { fileExtension, formatBytes, hostOf } from "../../lib/format";
import { t, tk, tr } from "../../lib/i18n";
import { fileVisual } from "../../lib/present";
import { useApp } from "../../lib/store";
import type { FoundFile, GrabberConfig, GrabberProject, GrabberScope } from "../../lib/types";
import { attempt, toast, useUi } from "../../lib/ui";
import { pickFolder } from "../../lib/win";

type Step = 0 | 1 | 2 | 3;
const STEPS = [tk("Start page"), tk("Rules"), tk("Crawl"), tk("Download")];

export const PRESETS: { id: string; label: string; icon: IconName; exts: string[] }[] = [
  { id: "images", label: tk("Images"), icon: "images", exts: ["jpg", "jpeg", "png", "gif", "webp", "avif", "svg", "tif", "tiff", "bmp"] },
  { id: "video", label: tk("Video"), icon: "video", exts: ["mp4", "mkv", "webm", "mov", "avi", "m4v"] },
  { id: "audio", label: tk("Audio"), icon: "music", exts: ["mp3", "m4a", "flac", "wav", "ogg", "opus"] },
  { id: "documents", label: tk("Documents"), icon: "documents", exts: ["pdf", "epub", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt"] },
  { id: "archives", label: tk("Archives"), icon: "compressed", exts: ["zip", "rar", "7z", "tar", "gz", "xz", "zst"] },
  { id: "programs", label: tk("Programs"), icon: "programs", exts: ["exe", "msi", "dmg", "deb", "rpm", "appimage", "iso"] },
];

function defaults(): GrabberConfig {
  const g = useApp.getState().settings?.grabber;
  return {
    startUrl: "",
    depth: g?.defaultDepth ?? 2,
    scope: "sameDomain",
    extensions: PRESETS[0].exts,
    includeEmbedded: true,
    filesAnyHost: true,
    maxPages: g?.maxPages ?? 2000,
    concurrency: g?.concurrentRequests ?? 6,
    exclude: ["logout", "signout"],
    fetchSizes: true,
    login: null,
    userAgent: "",
    cookies: "",
  };
}

interface Live {
  pages: number;
  queued: number;
  errors: number;
  current: string;
  running: boolean;
  files: FoundFile[];
}

export function GrabberSheet({ projectId }: { projectId?: string | null }) {
  const existing = useApp((s) => (projectId ? s.grabberProjects.find((p) => p.id === projectId) : undefined));
  const [step, setStep] = useState<Step>(existing?.results.files?.length ? 3 : 0);
  const [name, setName] = useState(existing?.name ?? "");
  const [cfg, setCfg] = useState<GrabberConfig>(existing?.config ?? defaults());
  const [pid, setPid] = useState<string | null>(existing?.id ?? null);
  const [live, setLive] = useState<Live>({ pages: existing?.results.pages ?? 0, queued: 0, errors: 0, current: "", running: !!existing?.results.running, files: existing?.results.files ?? [] });
  const pidRef = useRef(pid);
  pidRef.current = pid;

  // Live crawl view from core `grabber` events.
  useEffect(() => {
    let off: (() => void) | undefined;
    void onCoreEvent((e) => {
      if (e.type !== "grabber" || e.payload.project_id !== pidRef.current) return;
      const ev = e.payload.event;
      setLive((l) => {
        switch (ev.type) {
          case "pageVisited":
            return { ...l, pages: ev.pages, queued: ev.queued, current: ev.url, running: true };
          case "fileFound":
            return { ...l, files: [...l.files, ev.file] };
          case "pageError":
            return { ...l, errors: l.errors + 1 };
          case "finished":
            return { ...l, pages: ev.pages, running: false, current: "" };
        }
      });
      if (ev.type === "finished") {
        toast(ev.cancelled ? t("Crawl stopped") : t("Crawl finished"), { kind: "success", body: t("{files} on {count} pages", { count: ev.pages, files: t("{count} files", { count: ev.files }) }) });
        setStep(3);
      }
    }).then((u) => (off = u));
    return () => off?.();
  }, []);

  const set = (p: Partial<GrabberConfig>) => setCfg((c) => ({ ...c, ...p }));
  const validUrl = /^https?:\/\/\S+\.\S+/.test(cfg.startUrl.trim());

  const start = async () => {
    const proj = await attempt(api.startGrabber(pid, name.trim() || hostOf(cfg.startUrl) || t("Grabber project"), { ...cfg, startUrl: cfg.startUrl.trim() }), t("Could not start the crawl"));
    if (!proj) return;
    setPid(proj.id);
    setLive({ pages: 0, queued: 0, errors: 0, current: cfg.startUrl, running: true, files: [] });
    setStep(2);
    void api.grabberProjects().then((g: GrabberProject[]) => useApp.setState({ grabberProjects: g }));
  };

  return (
    <div className="grab">
      <ol className="steps">
        {STEPS.map((label, i) => (
          <li key={label} className={`st${i < step ? " done" : ""}${i === step ? " cur" : ""}`}>
            <button type="button" disabled={i > step && !(i === 3 && live.files.length)} onClick={() => setStep(i as Step)}>
              <i>{i < step ? <Icon name="check" size={12} strokeWidth={2.4} /> : i + 1}</i>
              {t(label)}
            </button>
          </li>
        ))}
      </ol>

      <div className="grab__body">
        {step === 0 && (
          <div className="grab__form">
            <Field label={t("Start page")} hint={t("Prism follows links from here according to the rules on the next step.")}>
              <TextField value={cfg.startUrl} onChange={(v) => set({ startUrl: v })} label={t("Start page")} icon="globe" mono placeholder="https://example.org/gallery/" autoFocus />
            </Field>
            <Field label={t("Project name")}>
              <TextField value={name} onChange={setName} label={t("Project name")} placeholder={hostOf(cfg.startUrl) || t("My crawl")} />
            </Field>
            <div className="grab__login">
              <Checkbox checked={!!cfg.login} onChange={(v) => set({ login: v ? { username: "", password: "", formUrl: null, formFields: [] } : null })}>
                {t("The site needs a login")}
              </Checkbox>
              {cfg.login && (
                <div className="row3">
                  <Field label={t("User name")}>
                    <TextField value={cfg.login.username} onChange={(v) => set({ login: { ...cfg.login!, username: v } })} label={t("User name")} icon="user" />
                  </Field>
                  <Field label={t("Password")}>
                    <TextField value={cfg.login.password} onChange={(v) => set({ login: { ...cfg.login!, password: v } })} label={t("Password")} type="password" icon="key" />
                  </Field>
                  <Field label={t("Login form address")} hint={t("Leave empty for HTTP authentication.")}>
                    <TextField value={cfg.login.formUrl ?? ""} onChange={(v) => set({ login: { ...cfg.login!, formUrl: v || null } })} label={t("Login form address")} mono />
                  </Field>
                </div>
              )}
            </div>
            <Field label={t("Cookies")} hint={t("Optional — paste a Cookie header to reuse a browser session.")}>
              <TextField value={cfg.cookies} onChange={(v) => set({ cookies: v })} label={t("Cookies")} mono placeholder="name=value; other=value" />
            </Field>
          </div>
        )}

        {step === 1 && (
          <div className="grab__form">
            <Field label={t("File types to collect")}>
              <div className="chips">
                {PRESETS.map((p) => {
                  const on = p.exts.every((x) => cfg.extensions.includes(x));
                  return (
                    <button
                      key={p.id}
                      type="button"
                      className={`chip${on ? " on" : ""}`}
                      aria-pressed={on}
                      onClick={() => set({ extensions: on ? cfg.extensions.filter((x) => !p.exts.includes(x)) : Array.from(new Set([...cfg.extensions, ...p.exts])) })}
                    >
                      <Icon name={p.icon} size={13} strokeWidth={2} /> {t(p.label)}
                    </button>
                  );
                })}
              </div>
            </Field>
            <Field label={t("Extensions")} hint={t("Comma separated, edit freely.")}>
              <ListField value={cfg.extensions} onCommit={(v) => set({ extensions: v.map((x) => x.replace(/^\./, "").toLowerCase()) })} label={t("Extensions")} rows={2} />
            </Field>
            <div className="row2">
              <Field label={t("Link depth")}>
                <Stepper value={cfg.depth} min={0} max={10} label={t("link depth")} onChange={(v) => set({ depth: v })} />
              </Field>
              <Field label={t("Follow links on")}>
                <Segmented<GrabberScope>
                  size="sm"
                  label={t("Scope")}
                  value={cfg.scope}
                  onChange={(v) => set({ scope: v })}
                  options={[
                    { value: "subPath", label: t("This folder") },
                    { value: "sameHost", label: t("Same host") },
                    { value: "sameDomain", label: t("Same site") },
                    { value: "any", label: t("Any site") },
                  ]}
                />
              </Field>
            </div>
            <div className="row2">
              <Field label={t("Maximum pages")}>
                <NumberField value={cfg.maxPages} min={1} max={100000} label={t("Maximum pages")} onCommit={(v) => set({ maxPages: v })} />
              </Field>
              <Field label={t("Parallel requests")}>
                <Stepper value={cfg.concurrency} min={1} max={32} label={t("parallel requests")} onChange={(v) => set({ concurrency: v })} />
              </Field>
            </div>
            <div className="toggles">
              <label className="toggle">
                <Switch checked={cfg.includeEmbedded} label={t("Include embedded media")} onChange={(v) => set({ includeEmbedded: v })} />
                {t("Include embedded images & media (img, video, audio, srcset)")}
              </label>
              <label className="toggle">
                <Switch checked={cfg.filesAnyHost} label={t("Files from any host")} onChange={(v) => set({ filesAnyHost: v })} />
                {t("Accept files hosted on other domains (CDNs)")}
              </label>
              <label className="toggle">
                <Switch checked={cfg.fetchSizes} label={t("Fetch file sizes")} onChange={(v) => set({ fetchSizes: v })} />
                {t("Look up file sizes (one HEAD request per file)")}
              </label>
            </div>
            <Field label={t("Skip addresses containing")}>
              <ListField value={cfg.exclude} onCommit={(v) => set({ exclude: v })} label={t("Exclude patterns")} rows={2} placeholder="logout, /tag/, ?sort=" />
            </Field>
          </div>
        )}

        {step === 2 && (
          <div className="crawl">
            <div className="kp">
              <div>
                <div className="k">{t("Pages scanned")}</div>
                <div className="v tnum">
                  {live.pages}
                  <small> / {cfg.maxPages}</small>
                </div>
              </div>
              <div>
                <div className="k">{t("In queue")}</div>
                <div className="v tnum">{live.queued}</div>
              </div>
              <div>
                <div className="k">{t("Matching files")}</div>
                <div className="v tnum">{live.files.length}</div>
              </div>
              <div>
                <div className="k">{t("Errors")}</div>
                <div className={`v tnum${live.errors ? " text-warning" : ""}`}>{live.errors}</div>
              </div>
            </div>
            <ProgressBar value={live.running ? (live.queued ? live.pages / (live.pages + live.queued) : null) : 1} state={live.running ? "active" : "completed"} label={t("Crawl progress")} />
            <div className="cur-url">
              {live.running ? <Icon name="retry" size={13} className="anim-spin" /> : <Icon name="status-completed" size={13} className="text-success" />}
              <span className="ellipsis">{live.running ? live.current : t("Finished")}</span>
            </div>
            <div className="found">
              {live.files.slice(-8).reverse().map((f) => (
                <div key={f.url} className="found__row anim-fade-in">
                  <Icon name={fileVisual({ fileName: f.fileName, kind: "http", category: "" }).icon} size={14} />
                  <span className="ellipsis">{f.fileName}</span>
                  <span className="text-tertiary tnum">{formatBytes(f.size)}</span>
                </div>
              ))}
            </div>
          </div>
        )}

        {step === 3 && pid && <Results projectId={pid} files={live.files} />}
      </div>

      <footer className="dialog__footer">
        {step > 0 && step !== 2 && (
          <button type="button" className="btn btn--ghost" onClick={() => setStep((step - 1) as Step)}>
            {t("Back")}
          </button>
        )}
        <span className="spacer" />
        {step === 0 && (
          <button type="button" className="btn btn--primary" disabled={!validUrl} onClick={() => setStep(1)}>
            {t("Next: rules")}
          </button>
        )}
        {step === 1 && (
          <button type="button" className="btn btn--primary" disabled={!validUrl || !cfg.extensions.length} onClick={start}>
            <Icon name="grabber" size={16} /> {t("Start crawl")}
          </button>
        )}
        {step === 2 &&
          (live.running ? (
            <button type="button" className="btn" onClick={() => pid && void attempt(api.stopGrabber(pid))}>
              <Icon name="stop" size={15} /> {t("Stop crawl")}
            </button>
          ) : (
            <button type="button" className="btn btn--primary" onClick={() => setStep(3)}>
              {t("Review {count} files", { count: live.files.length })}
            </button>
          ))}
      </footer>
    </div>
  );
}

function Results({ projectId, files }: { projectId: string; files: FoundFile[] }) {
  const [checked, setChecked] = useState<Set<string>>(() => new Set(files.map((f) => f.url)));
  const [type, setType] = useState<string>("all");
  const [q, setQ] = useState("");
  const [dir, setDir] = useState(useApp.getState().settings?.downloads.defaultDir ?? "");
  const [keep, setKeep] = useState(false);
  useEffect(() => setChecked((c) => new Set([...c, ...files.filter((f) => !c.has(f.url)).map((f) => f.url)])), [files]);

  const groups = useMemo(() => {
    const g: Record<string, number> = {};
    for (const f of files) {
      const p = PRESETS.find((x) => x.exts.includes(fileExtension(f.fileName)))?.id ?? "other";
      g[p] = (g[p] ?? 0) + 1;
    }
    return g;
  }, [files]);
  const shown = files.filter((f) => {
    const p = PRESETS.find((x) => x.exts.includes(fileExtension(f.fileName)))?.id ?? "other";
    return (type === "all" || p === type) && (!q || f.fileName.toLowerCase().includes(q.toLowerCase()) || f.url.toLowerCase().includes(q.toLowerCase()));
  });
  const sel = files.filter((f) => checked.has(f.url));
  const allShown = shown.length > 0 && shown.every((f) => checked.has(f.url));
  const toggleAll = () => {
    const n = new Set(checked);
    shown.forEach((f) => (allShown ? n.delete(f.url) : n.add(f.url)));
    setChecked(n);
  };

  const download = async () => {
    const n = await attempt(api.downloadGrabbed(projectId, sel.map((f) => f.url), dir || null, keep), t("Could not add the files"));
    if (n === undefined) return;
    if (n > 0) {
      toast(t("{count} downloads added", { count: n }), { kind: "success" });
      useUi.getState().closeSheet();
      useApp.getState().setView({ kind: "grabber", id: projectId });
    } else {
      toast(t("Nothing to download"));
    }
  };

  return (
    <div className="results">
      <div className="chips">
        <button type="button" className={`chip${type === "all" ? " on" : ""}`} onClick={() => setType("all")}>
          {t("All")} <em>{files.length}</em>
        </button>
        {[...PRESETS, { id: "other", label: tk("Other"), icon: "other" as IconName, exts: [] }]
          .filter((p) => groups[p.id])
          .map((p) => (
            <button key={p.id} type="button" className={`chip${type === p.id ? " on" : ""}`} onClick={() => setType(p.id)}>
              <Icon name={p.icon} size={13} strokeWidth={2} /> {t(p.label)} <em>{groups[p.id]}</em>
            </button>
          ))}
        <span className="spacer" />
        <TextField value={q} onChange={setQ} label={t("Filter results")} icon="search" placeholder={t("Filter")} />
      </div>
      <div className="gtable">
        <div className="grow2 h">
          <Checkbox checked={allShown} indeterminate={!allShown && shown.some((f) => checked.has(f.url))} label={t("Select all shown")} onChange={toggleAll} />
          <span />
          <span>{t("File")}</span>
          <span className="r">{t("Depth")}</span>
          <span className="r">{t("Size")}</span>
        </div>
        <div className="gtable__rows">
          {shown.map((f) => {
            const isImg = PRESETS[0].exts.includes(fileExtension(f.fileName));
            const v = fileVisual({ fileName: f.fileName, kind: "http", category: "" });
            return (
              <div
                key={f.url}
                className="grow2"
                onClick={() => {
                  const n = new Set(checked);
                  if (n.has(f.url)) n.delete(f.url);
                  else n.add(f.url);
                  setChecked(n);
                }}
              >
                <Checkbox checked={checked.has(f.url)} label={t("Select {name}", { name: f.fileName })} onChange={() => undefined} />
                {isImg ? (
                  <img className="ph" src={f.url} alt="" loading="lazy" decoding="async" onError={(e) => (e.currentTarget.style.visibility = "hidden")} />
                ) : (
                  <span className="file-tile" style={{ ["--tint" as string]: `var(${v.tint})` }}>
                    <Icon name={v.icon} size={16} />
                  </span>
                )}
                <span className="nm">
                  <b>{f.fileName}</b>
                  <small>{f.referer}</small>
                </span>
                <span className="r dim tnum">{f.depth}</span>
                <span className="r tnum">{formatBytes(f.size)}</span>
              </div>
            );
          })}
        </div>
      </div>
      <div className="results__foot">
        <div className="withbtn">
          <TextField value={dir} onChange={setDir} icon="folder" label={t("Save folder")} />
          {!webMode && (
            <button type="button" className="btn" onClick={async () => setDir((await pickFolder(dir)) ?? dir)}>
              {t("Browse…")}
            </button>
          )}
        </div>
        <Checkbox checked={keep} onChange={setKeep}>
          {t("Keep the site’s folder structure")}
        </Checkbox>
        <span className="spacer" />
        <span className="sumtext">
          {tr("{n} selected · {size}", { n: <b>{sel.length}</b>, size: <b>{formatBytes(sel.reduce((s, f) => s + (f.size ?? 0), 0))}</b> })}
        </span>
        <button type="button" className="btn btn--primary" disabled={!sel.length} onClick={download}>
          <Icon name="download" size={16} /> {t("Download selected")}
        </button>
      </div>
    </div>
  );
}
