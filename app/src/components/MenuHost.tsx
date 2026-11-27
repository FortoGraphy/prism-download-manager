import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Icon } from "../design/icons";
import { type MenuEntry, useUi } from "../lib/ui";
import { keyLabel } from "../lib/platform";

type Item = Extract<MenuEntry, { label: string }>;
const isItem = (e: MenuEntry): e is Item => "label" in e;

/** Open a menu below an element (dropdowns) or at the pointer (context menus). */
export function openMenuAt(target: { x: number; y: number } | HTMLElement, entries: MenuEntry[]) {
  if (target instanceof HTMLElement) {
    const r = target.getBoundingClientRect();
    useUi.getState().openMenu({ entries, x: r.left, y: r.bottom + 4, minWidth: r.width });
  } else {
    useUi.getState().openMenu({ entries, x: target.x, y: target.y });
  }
}

/** Renders the single active menu (context menus, dropdowns, selects). */
export function MenuHost() {
  const menu = useUi((s) => s.menu);
  const close = useUi((s) => s.closeMenu);
  if (!menu) return null;
  return createPortal(<MenuPanel entries={menu.entries} x={menu.x} y={menu.y} minWidth={menu.minWidth} onClose={close} root />, document.body);
}

function MenuPanel({
  entries,
  x,
  y,
  minWidth,
  onClose,
  root,
}: {
  entries: MenuEntry[];
  x: number;
  y: number;
  minWidth?: number;
  onClose: () => void;
  root?: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x, y, ready: false });
  const [active, setActive] = useState(-1);
  const [sub, setSub] = useState<{ index: number; x: number; y: number } | null>(null);

  // Keep inside the viewport (flip left/up when needed).
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    let nx = x;
    let ny = y;
    if (nx + r.width > window.innerWidth - 6) nx = Math.max(6, root ? window.innerWidth - r.width - 6 : x - r.width - (minWidth ?? 0));
    if (ny + r.height > window.innerHeight - 6) ny = Math.max(6, window.innerHeight - r.height - 6);
    setPos({ x: nx, y: ny, ready: true });
  }, [x, y, root, minWidth]);

  // Take focus once the menu is visible (a `visibility: hidden` element can't be focused), so
  // arrows / Enter / Esc go to the menu and not to the control or dialog behind it. On close,
  // focus goes back to where it was (the select / button that opened the menu).
  useEffect(() => {
    const el = ref.current;
    if (!root || !pos.ready || !el) return;
    const prev = document.activeElement as HTMLElement | null;
    el.focus({ preventScroll: true });
    return () => {
      const now = document.activeElement;
      if (prev?.isConnected && (!now || now === document.body || el.contains(now))) prev.focus({ preventScroll: true });
    };
  }, [root, pos.ready]);

  useEffect(() => {
    if (!root) return;
    const down = (e: MouseEvent) => {
      if (!(e.target as HTMLElement).closest(".menu--floating")) onClose();
    };
    const blur = () => onClose();
    // Only real size changes close the menu (WebViews can emit no-op resize events).
    const size = { w: window.innerWidth, h: window.innerHeight };
    const resize = () => (window.innerWidth !== size.w || window.innerHeight !== size.h) && onClose();
    // The menu is fixed-position: when the list / sheet under it scrolls it would float away
    // from its control, so close it (scrolling inside the menu itself is fine).
    const scroll = (e: Event) => !(e.target instanceof Node && (e.target as Element).closest?.(".menu--floating")) && onClose();
    window.addEventListener("mousedown", down, true);
    window.addEventListener("blur", blur);
    window.addEventListener("resize", resize);
    window.addEventListener("scroll", scroll, true);
    return () => {
      window.removeEventListener("mousedown", down, true);
      window.removeEventListener("blur", blur);
      window.removeEventListener("resize", resize);
      window.removeEventListener("scroll", scroll, true);
    };
  }, [root, onClose]);

  const selectable = entries.map((e, i) => (isItem(e) && !e.disabled ? i : -1)).filter((i) => i >= 0);

  const openSub = (index: number) => {
    const el = ref.current?.querySelector<HTMLElement>(`[data-index="${index}"]`);
    if (!el) return;
    const r = el.getBoundingClientRect();
    setSub({ index, x: r.right + 2, y: r.top - 5 });
  };

  const run = (e: Item, index: number) => {
    if (e.disabled) return;
    if (e.submenu) return openSub(index);
    onClose();
    e.onSelect?.();
  };

  const onKey = (ev: React.KeyboardEvent) => {
    const at = selectable.indexOf(active);
    if (ev.key === "ArrowDown") setActive(selectable[(at + 1) % selectable.length]);
    else if (ev.key === "ArrowUp") setActive(selectable[at < 0 ? selectable.length - 1 : (at - 1 + selectable.length) % selectable.length]);
    else if (ev.key === "Enter" || ev.key === " " || ev.key === "ArrowRight") {
      const e = entries[active];
      if (e && isItem(e)) {
        if (ev.key === "ArrowRight" && !e.submenu) return;
        run(e, active);
      }
    } else if (ev.key === "Escape" || ev.key === "Tab" || (ev.key === "ArrowLeft" && !root)) onClose();
    else return;
    ev.preventDefault();
    ev.stopPropagation();
  };

  return (
    <div
      ref={ref}
      className="menu menu--floating glass-strong blur-sm anim-pop-in"
      role="menu"
      tabIndex={-1}
      onKeyDown={onKey}
      onContextMenu={(e) => e.preventDefault()}
      style={{ left: pos.x, top: pos.y, minWidth: Math.max(minWidth ?? 0, 200), visibility: pos.ready ? "visible" : "hidden" }}
    >
      {entries.map((e, i) => {
        if ("separator" in e) return <div key={i} className="menu__sep" role="separator" />;
        if ("heading" in e) return <div key={i} className="menu__label">{e.heading}</div>;
        const hot = i === active || sub?.index === i;
        return (
          <div
            key={i}
            data-index={i}
            role={e.checked !== undefined ? "menuitemcheckbox" : "menuitem"}
            aria-checked={e.checked}
            aria-disabled={e.disabled}
            aria-haspopup={e.submenu ? "menu" : undefined}
            className={`menu__item${e.danger ? " menu__item--danger" : ""}${hot && !e.disabled ? " is-hover" : ""}${e.disabled ? " is-disabled" : ""}`}
            onMouseEnter={() => {
              setActive(i);
              if (e.submenu) openSub(i);
              else setSub(null);
            }}
            onClick={() => run(e, i)}
          >
            {e.checked !== undefined ? (
              <span className="menu__check">{e.checked && <Icon name="check" size={14} strokeWidth={2.2} />}</span>
            ) : e.icon ? (
              <Icon name={e.icon} size={15} />
            ) : (
              <span className="menu__check" />
            )}
            <span className="menu__text">{e.label}</span>
            {e.kbd && <span className="kbd-hint">{keyLabel(e.kbd)}</span>}
            {e.submenu && <Icon name="chevron-right" size={13} className="menu__chev" />}
          </div>
        );
      })}
      {sub && (() => {
        const e = entries[sub.index];
        return isItem(e) && e.submenu ? createPortal(<MenuPanel entries={e.submenu} x={sub.x} y={sub.y} onClose={onClose} />, document.body) : null;
      })()}
    </div>
  );
}
