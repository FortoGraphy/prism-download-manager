import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useLayoutEffect, useMemo, useRef } from "react";
import { Checkbox } from "../components/Controls";
import { IconButton } from "../components/IconButton";
import { openMenuAt } from "../components/MenuHost";
import { FileTile, ProgressBar } from "../components/Visuals";
import { Icon } from "../design/icons";
import { actions } from "../lib/actions";
import { webMode } from "../lib/api";
import { type ColumnDef, gridTemplate, minRowWidth, resolveColumns } from "../lib/columns";
import { formatBytes, formatDate, formatDuration, formatSpeed } from "../lib/format";
import { t } from "../lib/i18n";
import { bytesLine, canPause, formatPct, metaLine, progressOf, statusView } from "../lib/present";
import { clickSelect, type MoveKey, moveSelect } from "../lib/selection";
import { useApp } from "../lib/store";
import { useUi } from "../lib/ui";
import { EmptyState } from "./EmptyState";
import { itemMenu } from "./itemMenu";
import { useVisibleIds } from "./useListing";

const ROW = { comfortable: 44, compact: 30 } as const;

export function DownloadTable() {
  const ids = useVisibleIds();
  const visible = useApp((s) => s.settings?.appearance.visibleColumns);
  const density = useApp((s) => s.settings?.appearance.density ?? "comfortable");
  const widths = useUi((s) => s.widths);
  const selection = useApp((s) => s.selection);
  const cols = useMemo(() => resolveColumns(visible), [visible]);
  const selected = useMemo(() => new Set(selection), [selection]);
  const scrollRef = useRef<HTMLDivElement>(null);
  const rowH = ROW[density];

  const virt = useVirtualizer({
    count: ids.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowH,
    overscan: 10,
    getItemKey: (i) => ids[i],
  });
  useLayoutEffect(() => virt.measure(), [rowH, virt]);

  // Rows added after the first render play a short entrance animation (only those).
  const known = useRef<Set<string> | null>(null);
  const fresh = known.current ? new Set(ids.filter((id) => !known.current!.has(id))) : new Set<string>();
  useEffect(() => {
    known.current ??= new Set();
    ids.forEach((id) => known.current!.add(id));
  }, [ids]);

  const template = gridTemplate(cols, widths);
  const minWidth = minRowWidth(cols, widths);

  const onKeyDown = (e: React.KeyboardEvent) => {
    const st = useApp.getState();
    const keys: Record<string, MoveKey> = { ArrowUp: "up", ArrowDown: "down", Home: "home", End: "end", PageUp: "pageUp", PageDown: "pageDown" };
    const move = keys[e.key];
    if (move) {
      e.preventDefault();
      const page = Math.max(1, Math.floor((scrollRef.current?.clientHeight ?? 400) / rowH) - 1);
      const next = moveSelect(ids, { selection: st.selection, anchor: st.anchor }, move, e.shiftKey, page);
      st.select(next.selection, next.anchor);
      const focus = next.selection[next.selection.length - 1];
      if (focus) virt.scrollToIndex(ids.indexOf(focus), { align: "auto" });
      return;
    }
    const focus = st.selection[st.selection.length - 1];
    if (e.key === "Enter" && focus) actions.activate(focus);
    else if (e.key === " " && st.selection.length) actions.toggle(st.selection);
    else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "c" && st.selection.length) void actions.copyLinks(st.selection);
    else if (e.key === "ContextMenu" && st.selection.length) {
      const row = scrollRef.current?.querySelector<HTMLElement>(`[data-row-id="${CSS.escape(focus)}"]`);
      const r = row?.getBoundingClientRect();
      openMenuAt({ x: (r?.left ?? 200) + 120, y: r?.bottom ?? 200 }, itemMenu(st.selection));
    } else return;
    e.preventDefault();
  };

  const onRowMouseDown = (e: React.MouseEvent, id: string) => {
    if (e.button === 2 && selected.has(id)) return; // right-click keeps a multi-selection
    const st = useApp.getState();
    const next = clickSelect(ids, { selection: st.selection, anchor: st.anchor }, id, { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey });
    st.select(next.selection, next.anchor);
  };

  const onContextMenu = (e: React.MouseEvent, id: string) => {
    e.preventDefault();
    const st = useApp.getState();
    const target = st.selection.includes(id) ? st.selection : [id];
    if (!st.selection.includes(id)) st.select([id], id);
    openMenuAt({ x: e.clientX, y: e.clientY }, itemMenu(target));
  };

  return (
    <section className="table" style={{ ["--grid" as string]: template, ["--row-min" as string]: `${minWidth}px`, ["--row-h" as string]: `${rowH}px` }}>
      <div
        ref={scrollRef}
        className="table__body"
        role="grid"
        aria-label={t("Downloads")}
        aria-rowcount={ids.length}
        aria-multiselectable="true"
        tabIndex={0}
        onKeyDown={onKeyDown}
        onMouseDown={(e) => {
          if (e.target === e.currentTarget) useApp.getState().select([]);
        }}
      >
        <Header cols={cols} />
        <div className="table__inner" style={{ height: virt.getTotalSize() }}>
          {virt.getVirtualItems().map((v) => {
            const id = ids[v.index];
            return (
              <Row
                key={id}
                id={id}
                index={v.index}
                top={v.start}
                cols={cols}
                selected={selected.has(id)}
                fresh={fresh.has(id)}
                compact={density === "compact"}
                onMouseDown={onRowMouseDown}
                onContextMenu={onContextMenu}
              />
            );
          })}
        </div>
        {!ids.length && <EmptyState />}
      </div>
    </section>
  );
}

function Header({ cols }: { cols: ColumnDef[] }) {
  const sort = useApp((s) => s.sort);
  const startResize = (e: React.PointerEvent, col: ColumnDef) => {
    e.preventDefault();
    e.stopPropagation();
    const cell = (e.currentTarget as HTMLElement).parentElement!;
    const startW = cell.getBoundingClientRect().width;
    const x0 = e.clientX;
    // The handle sits on the column's end edge: in a right-to-left layout that's its left side.
    const dir = document.documentElement.dir === "rtl" ? -1 : 1;
    const move = (ev: PointerEvent) => useUi.getState().setWidth(col.key, Math.max(col.min, startW + (ev.clientX - x0) * dir));
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      document.body.classList.remove("is-resizing");
    };
    document.body.classList.add("is-resizing");
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };
  return (
    <div className="thead cols" role="row">
      <div role="columnheader" aria-label={t("Selection")} />
      {cols.map((c) => {
        const active = c.sort && sort.key === c.sort;
        return (
          <div
            key={c.key}
            role="columnheader"
            aria-sort={active ? (sort.dir === 1 ? "ascending" : "descending") : undefined}
            className={`th${c.align ? " num" : ""}${active ? " sorted" : ""}${c.sort ? " sortable" : ""}`}
            onClick={() => c.sort && useApp.getState().setSort(c.sort)}
          >
            <span className="th__label">{t(c.label)}</span>
            {active && <Icon name={sort.dir === 1 ? "chevron-up" : "chevron-down"} size={12} strokeWidth={2.2} />}
            {c.key !== "name" && <span className="th__resize" onPointerDown={(e) => startResize(e, c)} onClick={(e) => e.stopPropagation()} aria-hidden="true" />}
          </div>
        );
      })}
      <div role="columnheader" aria-label={t("Actions")} />
    </div>
  );
}

interface RowProps {
  id: string;
  index: number;
  top: number;
  cols: ColumnDef[];
  selected: boolean;
  fresh: boolean;
  compact: boolean;
  onMouseDown: (e: React.MouseEvent, id: string) => void;
  onContextMenu: (e: React.MouseEvent, id: string) => void;
}

const Row = memo(function Row({ id, index, top, cols, selected, fresh, compact, onMouseDown, onContextMenu }: RowProps) {
  const item = useApp((s) => s.items[id]);
  const progress = useApp((s) => s.progress[id]);
  const categories = useApp((s) => s.categories);
  if (!item) return null;
  const sv = statusView(item, progress);
  const p = progressOf(item, progress);
  const statusShown = cols.some((c) => c.key === "status");
  const status = progress?.status ?? item.status;
  const live = status === "downloading";

  const cell = (c: ColumnDef) => {
    switch (c.key) {
      case "name":
        return (
          <div className="namecell" key="name">
            <span className="row-lead">
              <FileTile item={item} size={compact ? 20 : 28} thumbWidth={compact ? 28 : 44} />
            </span>
            <div className="name">
              <div className="name__t" title={item.fileName}>
                {item.fileName}
              </div>
              {!compact && <div className="name__m">{metaLine(item, categories)}</div>}
            </div>
          </div>
        );
      case "size":
        return (
          <div className="num" key="size">
            {formatBytes(p.total ?? item.size)}
          </div>
        );
      case "status":
        return (
          <div key="status" className={`st st--${sv.tone}`} data-tip={sv.detail !== sv.label ? sv.detail : undefined}>
            <Icon name={sv.icon} size={13} strokeWidth={2} />
            <span>{status === "failed" || status === "processing" ? sv.detail : sv.label}</span>
          </div>
        );
      case "progress": {
        const left = !statusShown && !live ? (
          <span className={`st st--${sv.tone}`}>
            <Icon name={sv.icon} size={12} strokeWidth={2} />
            <span>{status === "paused" && p.pct != null ? t("Paused · {percent}", { percent: formatPct(p.pct) }) : sv.detail}</span>
          </span>
        ) : (
          <b className={live ? undefined : "muted"}>{formatPct(p.pct)}</b>
        );
        return (
          <div className="prog" key="progress">
            <div className="prog__top">
              {left}
              {!compact && <span>{bytesLine(item, progress)}</span>}
            </div>
            <ProgressBar value={status === "processing" ? null : p.fraction} state={sv.bar} label={t("{name} progress", { name: item.fileName })} />
          </div>
        );
      }
      case "speed":
        return (
          <div className="num" key="speed">
            {progress?.speed ? formatSpeed(progress.speed) : <span className="muted">—</span>}
          </div>
        );
      case "eta":
        return (
          <div className="num" key="eta">
            {progress?.etaSecs != null && live ? formatDuration(progress.etaSecs) : <span className="muted">—</span>}
          </div>
        );
      case "lastTry":
        return (
          <div className="num muted cell-date" key="lastTry">
            {formatDate(item.lastTryAt)}
          </div>
        );
      case "added":
        return (
          <div className="num muted cell-date" key="added">
            {formatDate(item.addedAt)}
          </div>
        );
      case "saveDir":
        return (
          <div className="muted ellipsis t-mono path" key="saveDir" title={item.saveDir}>
            {item.saveDir}
          </div>
        );
      case "category":
        return (
          <div className="muted ellipsis" key="category">
            {categories.find((x) => x.id === item.category)?.name ?? item.category}
          </div>
        );
      case "description":
        return (
          <div className="muted ellipsis" key="description" title={item.description}>
            {item.description}
          </div>
        );
    }
  };

  return (
    <div
      className={`row cols${selected ? " is-selected" : ""}${fresh ? " row--new" : ""}`}
      role="row"
      aria-rowindex={index + 1}
      aria-selected={selected}
      data-row-id={id}
      style={{ transform: `translateY(${top}px)` }}
      onMouseDown={(e) => onMouseDown(e, id)}
      onDoubleClick={() => actions.activate(id)}
      onContextMenu={(e) => onContextMenu(e, id)}
    >
      {/* The row selects on mouse-down; the checkbox toggles on click. Without stopping the
          mouse-down here a click would select the row and then immediately unselect it. */}
      <div className="chk" onMouseDown={(e) => e.stopPropagation()} onDoubleClick={(e) => e.stopPropagation()}>
        <Checkbox
          checked={selected}
          label={t("Select {name}", { name: item.fileName })}
          onChange={() => {
            const st = useApp.getState();
            st.select(selected ? st.selection.filter((s) => s !== id) : [...st.selection, id], id);
          }}
        />
      </div>
      {cols.map(cell)}
      <div className="more">
        <button
          type="button"
          className="more__btn"
          aria-label={t("More actions")}
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => {
            const st = useApp.getState();
            const target = st.selection.includes(id) ? st.selection : [id];
            openMenuAt(e.currentTarget, itemMenu(target));
          }}
        >
          <Icon name="more" size={16} />
        </button>
      </div>
      <div className="row-actions" onMouseDown={(e) => e.stopPropagation()} onDoubleClick={(e) => e.stopPropagation()}>
        {item.status === "completed" ? (
          !webMode && <IconButton icon="open-file" label={t("Open")} size={16} onClick={() => actions.open(id)} />
        ) : canPause(status) ? (
          <IconButton icon="pause" label={t("Pause")} size={16} onClick={() => actions.pause([id])} />
        ) : (
          <IconButton icon="resume" label={t("Resume")} size={16} onClick={() => actions.resume([id])} />
        )}
        {!webMode && <IconButton icon="folder-open" label={t("Show in folder")} size={16} onClick={() => actions.reveal(id)} />}
        <IconButton icon="link" label={t("Copy link")} size={16} onClick={() => void actions.copyLinks([id])} />
      </div>
    </div>
  );
});
