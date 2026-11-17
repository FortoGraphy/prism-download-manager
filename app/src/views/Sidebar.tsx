import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { BrandTile } from "../components/LogoMark";
import { openMenuAt } from "../components/MenuHost";
import { ThemeSwitcher } from "../components/ThemeSwitcher";
import { Sparkline } from "../components/Visuals";
import { Icon, type IconName } from "../design/icons";
import { actions } from "../lib/actions";
import { api, type IntegrationStatus } from "../lib/api";
import { formatBytes, formatSpeed } from "../lib/format";
import { t, tr } from "../lib/i18n";
import { categoryIcon, displayName } from "../lib/present";
import { Ring } from "../lib/ring";
import { type AppState, counts, useApp, type View } from "../lib/store";
import { openTools, selectedMediaPaths } from "../lib/tools";
import { attempt, useUi } from "../lib/ui";

const same = (a: View, b: View) => a.kind === b.kind && ("id" in a ? a.id : "") === ("id" in b ? b.id : "");
const str = (s: string) => s;

function NavItem({ view, icon, label, extra, onContext }: { view: View; icon: IconName; label: string; extra?: ReactNode; onContext?: (e: React.MouseEvent) => void }) {
  const current = useApp((s) => same(s.view, view));
  return (
    <button
      type="button"
      className="side-item"
      aria-current={current ? "true" : undefined}
      onClick={() => useApp.getState().setView(view)}
      onContextMenu={
        onContext &&
        ((e) => {
          e.preventDefault();
          onContext(e);
        })
      }
    >
      <Icon name={icon} size={16} />
      <span className="side-item__label">{label}</span>
      {extra}
    </button>
  );
}

const Count = ({ n }: { n: number }) => <span className="count">{n}</span>;

/** A Tools entry: opens the Tools window (with the selected finished media files, if any). */
function ToolItem({ icon, label, tab }: { icon: IconName; label: string; tab: "convert" | "shrink" }) {
  return (
    <button type="button" className="side-item" onClick={() => openTools({ tab, paths: selectedMediaPaths() })}>
      <Icon name={icon} size={16} />
      <span className="side-item__label">{label}</span>
    </button>
  );
}

export function Sidebar() {
  const items = useApp((s) => s.items);
  const c = useMemo(() => counts({ items } as AppState), [items]);
  const categories = useApp((s) => s.categories);
  const queues = useApp((s) => s.queues);
  const projects = useApp((s) => s.grabberProjects);
  const ui = useUi.getState;

  // Collapsible section state persisted in localStorage
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>(() => {
    try {
      const raw = localStorage.getItem("prism_sidebar_collapsed");
      return raw ? JSON.parse(raw) : {};
    } catch {
      return {};
    }
  });

  const toggleSection = (sectionId: string) => {
    setCollapsed((prev) => {
      const next = { ...prev, [sectionId]: !prev[sectionId] };
      try {
        localStorage.setItem("prism_sidebar_collapsed", JSON.stringify(next));
      } catch {}
      return next;
    });
  };

  // The list scrolls without a scrollbar: a soft fade at an edge says there's more there
  const navRef = useRef<HTMLElement>(null);
  const [moreBelow, setMoreBelow] = useState(false);
  const [moreAbove, setMoreAbove] = useState(false);
  useEffect(() => {
    const nav = navRef.current;
    if (!nav) return;
    const check = () => {
      setMoreAbove(nav.scrollTop > 2);
      setMoreBelow(nav.scrollTop + nav.clientHeight < nav.scrollHeight - 2);
    };
    check();
    nav.addEventListener("scroll", check, { passive: true });
    const ro = new ResizeObserver(check);
    ro.observe(nav);
    for (const child of Array.from(nav.children)) ro.observe(child);
    return () => {
      nav.removeEventListener("scroll", check);
      ro.disconnect();
    };
  }, [categories.length, queues.length, projects.length]);

  const categoryMenu = (id: string) => (e: React.MouseEvent) => {
    const cat = useApp.getState().categories.find((x) => x.id === id);
    if (!cat) return;
    openMenuAt({ x: e.clientX, y: e.clientY }, [
      { label: t("Edit category…"), icon: "rename", onSelect: () => ui().openDialog({ kind: "category", category: cat }) },
      { label: t("Open folder"), icon: "folder-open", disabled: !cat.saveDir, onSelect: () => cat.saveDir && void attempt(api.openPath(cat.saveDir)) },
      { separator: true },
      { label: t("Delete category"), icon: "delete", danger: true, disabled: cat.builtin, onSelect: () => void attempt(api.deleteCategory(cat.id), t("Could not delete the category")) },
    ]);
  };
  const queueMenu = (id: string) => (e: React.MouseEvent) => {
    const q = useApp.getState().queues.find((x) => x.id === id);
    if (!q) return;
    openMenuAt({ x: e.clientX, y: e.clientY }, [
      q.running
        ? { label: t("Stop queue"), icon: "stop-queue", onSelect: () => void attempt(api.stopQueue(q.id)) }
        : { label: t("Start queue"), icon: "start-queue", onSelect: () => void attempt(api.startQueue(q.id)) },
      { label: t("Edit schedule…"), icon: "scheduler", onSelect: () => ui().openSheet({ kind: "scheduler", queueId: q.id }) },
      { separator: true },
      { label: t("Delete queue"), icon: "delete", danger: true, disabled: q.builtin, onSelect: () => void attempt(api.deleteQueue(q.id), t("Could not delete the queue")) },
    ]);
  };

  return (
    <aside className="sidebar glass" aria-label={t("Library")}>
      <div className="brand" data-tauri-drag-region>
        <BrandTile size={28} />
        <span className="brand__name" data-tauri-drag-region>
          Prism
        </span>
      </div>
      <nav className="nav" ref={navRef} data-more-below={moreBelow || undefined} data-more-above={moreAbove || undefined}>
        <NavItem view={{ kind: "all" }} icon="all" label={t("All downloads")} extra={<Count n={c.all} />} />
        <NavItem view={{ kind: "status", id: "unfinished" }} icon="unfinished" label={t("Unfinished")} extra={<Count n={c.unfinished} />} />
        <NavItem view={{ kind: "status", id: "finished" }} icon="finished" label={t("Finished")} extra={<Count n={c.finished} />} />

        {/* Categories Section */}
        <Section
          id="categories"
          title={t("Categories")}
          collapsed={collapsed.categories}
          onToggle={() => toggleSection("categories")}
          onAdd={() => ui().openDialog({ kind: "category" })}
          addLabel={t("New category")}
        />
        {!collapsed.categories &&
          categories.map((cat) => (
            <NavItem
              key={cat.id}
              view={{ kind: "category", id: cat.id }}
              icon={categoryIcon(cat)}
              label={displayName(cat.name)}
              extra={<Count n={c.byCategory[cat.id] ?? 0} />}
              onContext={categoryMenu(cat.id)}
            />
          ))}

        {/* Queues Section */}
        <Section
          id="queues"
          title={t("Queues")}
          collapsed={collapsed.queues}
          onToggle={() => toggleSection("queues")}
          onAdd={() => ui().openSheet({ kind: "scheduler", queueId: "new" })}
          addLabel={t("New queue")}
        />
        {!collapsed.queues &&
          queues.map((q) => (
            <NavItem
              key={q.id}
              view={{ kind: "queue", id: q.id }}
              icon={q.schedule.enabled && !q.running ? "status-scheduled" : "queue"}
              label={displayName(q.name)}
              onContext={queueMenu(q.id)}
              extra={
                q.running ? (
                  <span className="q-live" data-tip={t("Running")}>
                    <span className="status-dot status-dot--live pulse" />
                    {c.byQueue[q.id] ?? 0}
                  </span>
                ) : q.schedule.enabled && q.schedule.startTime ? (
                  <span className="q-time" data-tip={t("Next scheduled start")}>
                    {q.schedule.startTime}
                  </span>
                ) : (
                  <Count n={c.byQueue[q.id] ?? 0} />
                )
              }
            />
          ))}

        {/* Tools Section */}
        <Section
          id="tools"
          title={t("Tools")}
          collapsed={collapsed.tools}
          onToggle={() => toggleSection("tools")}
        />
        {!collapsed.tools && (
          <>
            <ToolItem icon="convert" label={t("Convert")} tab="convert" />
            <ToolItem icon="shrink" label={t("Make smaller")} tab="shrink" />
            <button type="button" className="side-item" title={t("Download a model from Hugging Face")} onClick={() => void attempt(api.openWindow("model"))}>
              <Icon name="model" size={16} />
              <span className="side-item__label">{t("AI models")}</span>
            </button>
          </>
        )}

        {/* Grabber Section */}
        <Section
          id="grabber"
          title={t("Grabber")}
          collapsed={collapsed.grabber}
          onToggle={() => toggleSection("grabber")}
          onAdd={() => ui().openSheet({ kind: "grabber", projectId: null })}
          addLabel={t("New grabber project")}
        />
        {!collapsed.grabber &&
          projects.map((p) => (
            <NavItem
              key={p.id}
              view={{ kind: "grabber", id: p.id }}
              icon="grabber-project"
              label={p.name}
              extra={<Count n={p.results.files?.length ?? 0} />}
              onContext={(e) =>
                openMenuAt({ x: e.clientX, y: e.clientY }, [
                  { label: t("Open project…"), icon: "grabber", onSelect: () => ui().openSheet({ kind: "grabber", projectId: p.id }) },
                  { separator: true },
                  { label: t("Delete project"), icon: "delete", danger: true, onSelect: () => void attempt(api.deleteGrabberProject(p.id).then(() => api.grabberProjects()).then((g) => useApp.setState({ grabberProjects: g }))) },
                ])
              }
            />
          ))}
        <NavItem
          view={{ kind: "trash" }}
          icon="trash"
          label={t("Trash")}
          extra={<Count n={c.trash} />}
          onContext={(e) => openMenuAt({ x: e.clientX, y: e.clientY }, [{ label: t("Empty trash"), icon: "delete", danger: true, disabled: !c.trash, onSelect: () => void attempt(api.emptyTrash(), t("Could not empty the trash")) }])}
        />
      </nav>
      <SpeedCard />
      <SideFooter />
    </aside>
  );
}

function Section({
  id: _id,
  title,
  collapsed,
  onToggle,
  onAdd,
  addLabel,
}: {
  id?: string;
  title: string;
  collapsed?: boolean;
  onToggle?: () => void;
  onAdd?: () => void;
  addLabel?: string;
}) {
  return (
    <div className="side-section">
      <button
        type="button"
        className="side-section__hdr-btn"
        onClick={onToggle}
        aria-expanded={!collapsed}
      >
        {onToggle && (
          <span className={`side-section__chevron${collapsed ? " collapsed" : ""}`}>
            <Icon name="chevron-down" size={11} strokeWidth={2.5} />
          </span>
        )}
        <span className="t-overline">{title}</span>
      </button>
      {onAdd ? (
        <button type="button" className="side-section__add" aria-label={addLabel} data-tip={addLabel} onClick={onAdd}>
          <Icon name="plus" size={13} />
        </button>
      ) : (
        <span className="side-section__add" style={{ visibility: "hidden" }} aria-hidden="true" />
      )}
    </div>
  );
}

/** Total speed with a 60-sample sparkline (1 sample/s, component-level ring buffer). */
function SpeedCard() {
  const ring = useRef(new Ring(60));
  const [samples, setSamples] = useState<number[]>([]);
  const limiter = useApp((s) => s.settings?.downloads.speedLimiterEnabled ?? false);
  const limit = useApp((s) => s.settings?.downloads.speedLimit ?? 0);
  const active = useApp((s) => Object.values(s.progress).filter((p) => p.speed > 0).length);
  const [peak, setPeak] = useState(0);

  useEffect(() => {
    const sample = () => {
      const total = Object.values(useApp.getState().progress).reduce((sum, p) => sum + (p.speed || 0), 0);
      ring.current.push(total);
      setSamples(ring.current.values());
      setPeak((p) => Math.max(p, total));
    };
    sample();
    const timer = window.setInterval(sample, 1000);
    return () => window.clearInterval(timer);
  }, []);

  const now = samples[samples.length - 1] ?? 0;
  const [num, unit] = useMemo(() => {
    const s = formatSpeed(now);
    if (s === "—") return ["0", "B/s"];
    const i = s.indexOf(" ");
    return [s.slice(0, i), s.slice(i + 1)];
  }, [now]);

  return (
    <section className="speedcard" aria-label={t("Total speed")}>
      <div className="speedcard__top">
        <span className="t-overline">{t("Total speed")}</span>
        {active > 0 ? (
          <span className="speedcard__live-badge">
            <span className="status-dot status-dot--live pulse" />
            {t("{count} active", { count: active })}
          </span>
        ) : (
          <span className="speedcard__idle-badge">{str("0 active")}</span>
        )}
      </div>
      <div className="speedcard__val tnum">
        {num}
        <small>{unit}</small>
      </div>
      <Sparkline samples={samples.length > 1 ? samples : [0, 0]} />
      <div className="speedcard__row">
        <button type="button" className="linklike" onClick={() => void actions.toggleLimiter()} data-tip={t("Toggle speed limiter")} data-kbd="Ctrl L">
          <Icon name="speed-limiter" size={13} /> {tr("Limit {limit}", { limit: <b>{limiter ? `${formatBytes(limit)}/s` : t("Off")}</b> })}
        </button>
        <span>{tr("Peak {speed}", { speed: <b>{formatSpeed(peak)}</b> })}</span>
      </div>
    </section>
  );
}

function SideFooter() {
  const [status, setStatus] = useState<IntegrationStatus | null>(null);
  useEffect(() => {
    const load = () => void api.integrationStatus().then(setStatus, () => setStatus(null));
    load();
    const timer = window.setInterval(load, 30_000);
    return () => window.clearInterval(timer);
  }, []);
  const ok = !!status?.extensionConnected;
  return (
    <div className="side-foot">
      <button
        type="button"
        className="side-foot__status"
        onClick={() => useUi.getState().openSheet({ kind: "settings", section: "browser" })}
        data-tip={ok ? t("The browser extension is connected") : t("Set up browser integration")}
      >
        <span className={`status-dot status-dot--${ok ? "success" : "warning"}`} />
        <span className="ellipsis">
          {tr("Browser {status}", { status: <span className={ok ? "text-success" : "text-warning"}>{status ? (ok ? t("connected") : t("not connected")) : "…"}</span> })}
        </span>
      </button>
      <ThemeSwitcher />
      <button type="button" className="tb-btn" aria-label={t("Settings")} data-tip={t("Settings")} data-kbd="Ctrl ," onClick={() => useUi.getState().openSheet({ kind: "settings" })}>
        <Icon name="settings" size={16} />
      </button>
    </div>
  );
}
