import { useEffect, useMemo, useRef, useState } from "react";
import { Checkbox, TextField } from "../components/Controls";
import { openMenuAt } from "../components/MenuHost";
import { WindowFrame } from "../components/WindowFrame";
import { Icon } from "../design/icons";
import { patchSettings } from "../lib/actions";
import { api, webMode, type HfContext, type HfHit, type HfPresent, type HfRepo } from "../lib/api";
import { formatBytes } from "../lib/format";
import { getLocale, t } from "../lib/i18n";
import { openUrl } from "../lib/links";
import { displayName, fileVisual } from "../lib/present";
import { useApp } from "../lib/store";
import { buildTree, nodeState, type TreeNode } from "../lib/tree";
import type { StartMode } from "../lib/types";
import { attempt } from "../lib/ui";
import { pickFolder, win } from "../lib/win";

/** A name (`owner/name`, `datasets/owner/name`) or a link: opened straight away, not searched. */
const looksLikeRepo = (q: string) => /^(https?:\/\/|(datasets\/)?[\w.-]+\/[\w.-]+$)/i.test(q.trim());

const compact = (n?: number | null) => (n == null ? "" : new Intl.NumberFormat(getLocale(), { notation: "compact", maximumFractionDigits: 1 }).format(n));

function problemText(code: string): string {
  switch (code) {
    case "hf:notFound":
      return t("Hugging Face has no model with this name.");
    case "hf:needsToken":
      return t("Not found. If it’s a private or gated model, add your Hugging Face access token below.");
    case "hf:badToken":
      return t("Hugging Face didn’t accept the access token.");
    case "hf:gated":
      return t("This model is gated: accept its terms on its Hugging Face page, then try again.");
    case "hf:rateLimited":
      return t("Hugging Face asks to slow down. Try again in a minute.");
    case "hf:network":
      return t("Couldn’t reach Hugging Face. Check the connection, or the server in Settings › AI models.");
    default:
      return t("Hugging Face didn’t answer as expected. Try again later.");
  }
}

/**
 * "Download a model" from Hugging Face (#29): find a model by name, link or search, pick its
 * files (Prism suggests one GGUF version, or the weights in one format) and they download one
 * after another in the "Hugging Face" queue, each with many connections and checked by SHA-256.
 */
export default function ModelWindow({ source }: { source: string }) {
  const queues = useApp((s) => s.queues);
  const [ctx, setCtx] = useState<HfContext | null>(null);
  const [query, setQuery] = useState(source);
  const [hits, setHits] = useState<HfHit[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [repo, setRepo] = useState<HfRepo | null>(null);
  const [loading, setLoading] = useState(false);
  const [problem, setProblem] = useState("");
  const [sel, setSel] = useState<Set<number>>(new Set());
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [variant, setVariant] = useState<string | null>(null);
  const [folder, setFolder] = useState("");
  const [present, setPresent] = useState<HfPresent>({ onDisk: [], listed: [] });
  const [free, setFree] = useState<number | null>(null);
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const loaded = useRef("");

  useEffect(() => {
    void api.hfContext().then(setCtx, () => undefined);
  }, []);

  const load = async (input: string) => {
    const q = input.trim();
    if (!q) return;
    loaded.current = q;
    setLoading(true);
    setProblem("");
    setRepo(null);
    try {
      const r = await api.hfRepo(q);
      if (loaded.current !== q) return;
      setRepo(r);
      setFolder(r.folder);
      setVariant(r.suggestion.variant ?? null);
      const index = new Map(r.files.map((f, i) => [f.path, i]));
      setSel(new Set(r.suggestion.files.map((p) => index.get(p)).filter((i): i is number => i !== undefined)));
      setOpen(new Set(buildTree(r.files.map((f, i) => ({ index: i, path: f.path, size: f.size }))).children.filter((c) => c.children.length && r.suggestion.files.some((p) => p.startsWith(`${c.path}/`))).map((c) => c.path)));
    } catch (e) {
      if (loaded.current === q) setProblem(String(e instanceof Error ? e.message : e));
    } finally {
      if (loaded.current === q) setLoading(false);
    }
  };

  // A link or name the window was opened with loads right away; otherwise what's trending.
  useEffect(() => {
    if (source) void load(source);
    else void search("");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [source]);

  // The last search asked for: an older answer arriving late doesn't replace a newer one.
  const asked = useRef("");
  const search = async (q: string) => {
    asked.current = q;
    setSearching(true);
    try {
      const h = await api.hfSearch(q);
      if (asked.current === q) setHits(h);
    } catch (e) {
      setProblem(String(e instanceof Error ? e.message : e));
    } finally {
      setSearching(false);
    }
  };

  // Typing searches (a moment after the last key); a name or link waits for Enter.
  useEffect(() => {
    const q = query.trim();
    if (!q || looksLikeRepo(q) || q === loaded.current) return;
    const timer = window.setTimeout(() => void search(q), 350);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query]);

  // What's in the folder or the list already isn't downloaded again.
  useEffect(() => {
    if (!repo || !folder) return;
    const timer = window.setTimeout(() => {
      void api.hfPresent({ kind: repo.kind, id: repo.id, sha: repo.sha, folder, files: repo.files }).then((p) => {
        setPresent(p);
        const done = new Set([...p.onDisk, ...p.listed]);
        setSel((s) => new Set([...s].filter((i) => !done.has(repo.files[i]?.path ?? ""))));
      }, () => undefined);
      void api.freeSpace(folder).then(setFree, () => setFree(null));
    }, 250);
    return () => window.clearTimeout(timer);
  }, [repo, folder]);

  const files = useMemo(() => repo?.files.map((f, i) => ({ index: i, path: f.path, size: f.size })) ?? [], [repo]);
  const tree = useMemo(() => (repo ? buildTree(files) : null), [repo, files]);
  const size = repo ? [...sel].reduce((s, i) => s + (repo.files[i]?.size ?? 0), 0) : 0;
  const lowSpace = free != null && size > free;
  const done = useMemo(() => new Set([...present.onDisk, ...present.listed]), [present]);
  const needsToken = problem === "hf:needsToken" || problem === "hf:badToken" || (!!repo?.gated && !!ctx && !ctx.token);
  // A gated model's files need the token: without one every file would fail.
  const blocked = !!repo?.gated && !!ctx && !ctx.token;
  const ready = !!repo && sel.size > 0 && !busy && !!folder.trim() && !blocked;

  const pickVariant = (label: string) => {
    if (!repo) return;
    const v = repo.suggestion.variants.find((x) => x.label === label);
    if (!v) return;
    setVariant(label);
    const index = new Map(repo.files.map((f, i) => [f.path, i]));
    setSel(new Set([...v.files, ...repo.suggestion.extras].filter((p) => !done.has(p)).map((p) => index.get(p)).filter((i): i is number => i !== undefined)));
  };

  const recommended = () => {
    if (!repo) return;
    if (variant && repo.suggestion.variants.length) return pickVariant(variant);
    const index = new Map(repo.files.map((f, i) => [f.path, i]));
    setSel(new Set(repo.suggestion.files.filter((p) => !done.has(p)).map((p) => index.get(p)).filter((i): i is number => i !== undefined)));
  };

  const toggle = (node: TreeNode) => {
    const n = new Set(sel);
    const st = nodeState(node, sel);
    node.files.forEach((i) => (st === "all" ? n.delete(i) : n.add(i)));
    setSel(n);
  };

  const saveToken = async () => {
    const v = token.trim();
    if (!v) return;
    await patchSettings({ huggingFace: { token: v } });
    setToken("");
    setCtx(await api.hfContext());
    if (!repo) void load(query || source);
  };

  const add = async (mode: StartMode | null) => {
    if (!repo) return;
    setBusy(true);
    const picked = [...sel].sort((a, b) => a - b).map((i) => repo.files[i]);
    const n = await attempt(api.hfAdd({ kind: repo.kind, id: repo.id, sha: repo.sha, files: picked, folder: folder.trim(), mode }), t("Could not add the model"));
    setBusy(false);
    if (n != null) void win.close();
  };

  const row = (node: TreeNode, depth: number): React.ReactNode => {
    const folderNode = node.children.length > 0;
    const isOpen = open.has(node.path);
    const st = nodeState(node, sel);
    const have = !folderNode && done.has(node.path);
    return (
      <div key={node.path}>
        <div className="trow" style={{ paddingInlineStart: 10 + depth * 18 }}>
          {folderNode ? (
            <button
              type="button"
              className="trow__twist"
              aria-label={isOpen ? t("Collapse") : t("Expand")}
              aria-expanded={isOpen}
              onClick={() => {
                const n = new Set(open);
                if (isOpen) n.delete(node.path);
                else n.add(node.path);
                setOpen(n);
              }}
            >
              <Icon name={isOpen ? "chevron-down" : "chevron-right"} size={13} strokeWidth={2.2} />
            </button>
          ) : (
            <span className="trow__twist" />
          )}
          <Checkbox checked={st === "all"} indeterminate={st === "some"} label={t("Include {name}", { name: node.name })} onChange={() => toggle(node)} />
          <Icon name={folderNode ? "folder" : fileVisual({ fileName: node.name, kind: "http", category: "" }).icon} size={15} className="text-tertiary" />
          <span className="ellipsis">
            {node.name}
            {have && (
              <span className="badge badge--success hf-have">{present.listed.includes(node.path) ? t("In the list") : t("Downloaded")}</span>
            )}
          </span>
          <span className="tnum text-tertiary">{formatBytes(node.size)}</span>
        </div>
        {folderNode && isOpen && node.children.map((c) => row(c, depth + 1))}
      </div>
    );
  };

  const variants = repo?.suggestion.variants ?? [];
  return (
    <WindowFrame kind="model" title={t("Download a model")} icon={<Icon name="model" size={17} className="text-accent" />} className="torrent hfwin">
      <div className="wbody">
        <div className="hf-find">
          <TextField
            value={query}
            onChange={(v) => {
              setQuery(v);
              if (repo && v.trim() !== loaded.current) {
                setRepo(null);
                loaded.current = "";
              }
            }}
            onEnter={() => {
              const q = query.trim();
              if (looksLikeRepo(q)) void load(q);
              else if (hits?.[0]) {
                setQuery(hits[0].id);
                void load(hits[0].id);
              }
            }}
            icon="search"
            label={t("Model")}
            placeholder={t("Model name, link or search (e.g. Qwen/Qwen3-8B)")}
            autoFocus={!source}
            trailing={loading || searching ? <Icon name="retry" size={14} className="anim-spin text-tertiary" /> : undefined}
          />
        </div>

        {problem && (
          <div className="banner banner--danger" role="alert">
            <Icon name="warning" size={16} />
            <span className="grow">{problemText(problem)}</span>
            {problem === "hf:gated" && repo && (
              <button type="button" className="btn btn--sm" onClick={() => void openUrl(repo.pageUrl)}>
                {t("Open the model’s page")}
              </button>
            )}
          </div>
        )}

        {needsToken && (
          <div className="hf-token card">
            <div className="t-small text-secondary">
              {repo?.gated && !problem
                ? t("This model is gated: accept its terms on its Hugging Face page, then add an access token so Prism can download it.")
                : t("An access token lets Prism download gated and private models. Read access is enough.")}{" "}
              <button type="button" className="linkbtn" onClick={() => void openUrl("https://huggingface.co/settings/tokens")}>
                {t("Create a token")}
              </button>
            </div>
            <div className="withbtn">
              <TextField value={token} onChange={setToken} onEnter={() => void saveToken()} type="password" icon="key" label={t("Access token")} placeholder="hf_…" mono />
              <button type="button" className="btn" disabled={!token.trim()} onClick={() => void saveToken()}>
                {t("Save")}
              </button>
            </div>
          </div>
        )}

        {!repo && !loading && hits && (
          <div className="hf-hits" role="list" aria-label={t("Models")}>
            <span className="field__label">{query.trim() && !looksLikeRepo(query) ? t("Models") : t("Trending on Hugging Face")}</span>
            {hits.length === 0 && <p className="t-small text-tertiary">{t("No models found.")}</p>}
            {hits.map((h) => (
              <button
                key={h.id}
                type="button"
                role="listitem"
                className="hf-hit"
                onClick={() => {
                  setQuery(h.id);
                  void load(h.id);
                }}
              >
                <Icon name="model" size={16} className="text-tertiary" />
                <span className="ellipsis">{h.id}</span>
                <span className="hf-hit__tags">
                  {h.gguf && <span className="badge badge--accent">GGUF</span>}
                  {h.gated && <Icon name="lock-closed" size={13} className="text-tertiary" />}
                  {h.downloads != null && (
                    <span className="t-small text-tertiary tnum" data-tip={t("Downloads")}>
                      <Icon name="download" size={12} /> {compact(h.downloads)}
                    </span>
                  )}
                </span>
              </button>
            ))}
          </div>
        )}

        {repo && (
          <>
            <div className="filecard">
              <span className="file-tile" style={{ ["--tint" as string]: "var(--type-other)", width: 44, height: 44 }}>
                <Icon name="model" size={24} />
              </span>
              <div className="grow">
                <b className="fname-static ellipsis">{repo.id}</b>
                <div className="tags">
                  {repo.pipelineTag && <span className="badge">{repo.pipelineTag}</span>}
                  {repo.gguf && <span className="badge badge--accent">GGUF</span>}
                  {repo.gated && (
                    <span className="badge badge--warning">
                      <Icon name="lock-closed" size={11} /> {t("Gated")}
                    </span>
                  )}
                  {repo.downloads != null && (
                    <span className="badge" data-tip={t("Downloads")}>
                      <Icon name="download" size={11} /> {compact(repo.downloads)}
                    </span>
                  )}
                  <button type="button" className="linkbtn t-small" onClick={() => void openUrl(repo.pageUrl)}>
                    {t("Open on Hugging Face")}
                  </button>
                </div>
              </div>
              <div className="big tnum">
                {formatBytes(size)}
                <small className={lowSpace ? "text-danger" : undefined}>{free != null ? t("{size} free", { size: formatBytes(free) }) : ""}</small>
              </div>
            </div>

            {variants.length > 0 && (
              <div className="hf-variants">
                <span className="field__label">{t("Version")}</span>
                <div className="hf-chips" role="radiogroup" aria-label={t("Version")}>
                  {variants.map((v) => (
                    <button key={v.label} type="button" role="radio" aria-checked={variant === v.label} className={`hf-chip${variant === v.label ? " on" : ""}`} onClick={() => pickVariant(v.label)}>
                      <b>{v.label}</b>
                      <span className="tnum">{formatBytes(v.size)}</span>
                      {v.label === repo.suggestion.variant && <span className="hf-chip__star" data-tip={t("Recommended: a good balance of size and quality")}>★</span>}
                    </button>
                  ))}
                </div>
              </div>
            )}

            <div className="tree-head">
              <span className="field__label">{t("Files")}</span>
              <span className="t-small text-tertiary">{t("{count} of {total} selected", { count: sel.size, total: repo.files.length })}</span>
              <span className="spacer" />
              <button type="button" className="btn btn--ghost btn--sm" onClick={recommended}>
                {t("Recommended")}
              </button>
              <button type="button" className="btn btn--ghost btn--sm" onClick={() => setSel(new Set(repo.files.map((_, i) => i)))}>
                {t("Select all")}
              </button>
              <button type="button" className="btn btn--ghost btn--sm" onClick={() => setSel(new Set())}>
                {t("None")}
              </button>
            </div>
            <div className="tree" role="tree" aria-label={t("Files")}>
              {tree?.children.map((c) => row(c, 0))}
            </div>

            <div className="field">
              <span className="field__label">{t("Save to")}</span>
              <div className="withbtn">
                <TextField value={folder} onChange={setFolder} icon="folder" label={t("Save folder")} />
                {!webMode && (
                  <button
                    type="button"
                    className="btn"
                    onClick={async () => {
                      const p = await pickFolder(folder);
                      if (p) setFolder(p);
                    }}
                  >
                    {t("Browse…")}
                  </button>
                )}
              </div>
              {ctx?.lmStudio && (
                <button type="button" className="linkbtn t-small hf-lmstudio" onClick={() => setFolder(`${ctx.lmStudio}/${repo.id}`.replace(/[\\/]+/g, ctx.lmStudio?.includes("\\") ? "\\" : "/"))}>
                  {t("Save into LM Studio’s models folder")}
                </button>
              )}
            </div>
          </>
        )}
      </div>

      <footer className="dialog__footer">
        <div className="split">
          <button type="button" className="btn" disabled={!ready} onClick={() => void add({ type: "later" })}>
            <Icon name="status-scheduled" size={15} /> {t("Download later")}
          </button>
          <button
            type="button"
            className="btn"
            aria-label={t("More start options")}
            disabled={!ready}
            onClick={(e) =>
              openMenuAt(e.currentTarget, [
                { label: t("Start all now"), icon: "download", onSelect: () => void add({ type: "now" }) },
                { label: t("Download later"), icon: "status-scheduled", onSelect: () => void add({ type: "later" }) },
                { label: t("Add to queue"), icon: "queue", submenu: queues.map((q) => ({ label: displayName(q.name), icon: q.running ? "start-queue" : "queue", onSelect: () => void add({ type: "queue", queueId: q.id }) })) },
              ])
            }
          >
            <Icon name="chevron-down" size={13} strokeWidth={2.2} />
          </button>
        </div>
        <span className="spacer" />
        <button type="button" className="btn btn--ghost" onClick={() => void win.close()}>
          {t("Cancel")}
        </button>
        <button
          type="button"
          className="btn btn--primary"
          disabled={!ready || lowSpace}
          onClick={() => void add(null)}
          data-tip={blocked ? t("Add an access token first") : lowSpace ? t("Not enough free space") : t("The files download one after another in the Hugging Face queue")}
        >
          <Icon name="download" size={15} /> {t("Download")}
        </button>
      </footer>
    </WindowFrame>
  );
}
