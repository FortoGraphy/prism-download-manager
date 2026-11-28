import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { keyLabel } from "../lib/platform";

interface Tip {
  text: string;
  kbd?: string;
  x: number;
  y: number;
  below: boolean;
}

/**
 * One delegated tooltip for the whole window: any element with `data-tip` gets a styled
 * tooltip after 500 ms hover (or immediately on keyboard focus). Cheaper than a component
 * per button and keeps markup clean.
 */
export function TooltipHost() {
  const [tip, setTip] = useState<Tip | null>(null);
  const timer = useRef<number | undefined>(undefined);
  const ref = useRef<HTMLDivElement>(null);
  const [shift, setShift] = useState(0);

  useEffect(() => {
    const show = (el: HTMLElement, delay: number) => {
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => {
        const r = el.getBoundingClientRect();
        const below = r.top < 48;
        setShift(0);
        setTip({ text: el.dataset.tip ?? "", kbd: el.dataset.kbd && keyLabel(el.dataset.kbd), x: r.left + r.width / 2, y: below ? r.bottom + 6 : r.top - 6, below });
      }, delay);
    };
    const hide = () => {
      window.clearTimeout(timer.current);
      setTip(null);
    };
    const over = (e: Event) => {
      const el = (e.target as HTMLElement | null)?.closest?.<HTMLElement>("[data-tip]");
      if (el && el.dataset.tip) show(el, 500);
      else hide();
    };
    const focus = (e: FocusEvent) => {
      const el = e.target as HTMLElement;
      if (el?.dataset?.tip && el.matches(":focus-visible")) show(el, 0);
    };
    document.addEventListener("mouseover", over);
    document.addEventListener("mousedown", hide, true);
    document.addEventListener("focusin", focus);
    document.addEventListener("focusout", hide);
    window.addEventListener("blur", hide);
    window.addEventListener("wheel", hide, { passive: true });
    return () => {
      document.removeEventListener("mouseover", over);
      document.removeEventListener("mousedown", hide, true);
      document.removeEventListener("focusin", focus);
      document.removeEventListener("focusout", hide);
      window.removeEventListener("blur", hide);
      window.removeEventListener("wheel", hide);
    };
  }, []);

  // keep inside the viewport horizontally
  useEffect(() => {
    if (!tip || !ref.current) return;
    const r = ref.current.getBoundingClientRect();
    const over = r.right - (window.innerWidth - 8);
    const under = 8 - r.left;
    setShift(over > 0 ? -over : under > 0 ? under : 0);
  }, [tip]);

  if (!tip) return null;
  return createPortal(
    <div
      ref={ref}
      className="tooltip tooltip--floating anim-fade-in"
      role="tooltip"
      style={{ left: tip.x + shift, top: tip.y, transform: `translate(-50%, ${tip.below ? "0" : "-100%"})` }}
    >
      {tip.text}
      {tip.kbd && <span className="kbd">{tip.kbd}</span>}
    </div>,
    document.body,
  );
}
