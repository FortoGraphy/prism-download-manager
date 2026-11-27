import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Icon } from "../design/icons";
import { patchSettings } from "../lib/actions";
import {
  applyCustomThemeTokens,
  clearInjectedThemeTokens,
  findThemeById,
  getAllThemes,
} from "../lib/customThemes";
import { t } from "../lib/i18n";
import { useApp } from "../lib/store";
import { useUi } from "../lib/ui";

const str = (s: string) => s;

const ACCENTS = [
  ["prism", "Prism", "#6461f2", "#5653ea"],
  ["cyan", "Cyan", "#36c5e6", "#0b7fa0"],
  ["emerald", "Emerald", "#2fc489", "#087f53"],
  ["amber", "Amber", "#f2b54a", "#a86200"],
  ["rose", "Rose", "#d03a64", "#cf3760"],
  ["graphite", "Graphite", "#e6e8ee", "#1f2229"],
] as const;

export function ThemeSwitcher({ className }: { className?: string }) {
  const [open, setOpen] = useState(false);
  const btnRef = useRef<HTMLButtonElement>(null);
  const popoverRef = useRef<HTMLDivElement>(null);

  const rawTheme = useApp((s) => s.settings?.appearance.theme ?? "system");
  const currentAccent = useApp((s) => s.settings?.appearance.accent ?? "prism");
  const openSheet = useUi((s) => s.openSheet);

  const resolvedTheme = typeof document !== "undefined" ? document.documentElement.dataset.theme : "dark";
  const customTheme = findThemeById(rawTheme);

  // Close when clicking outside or pressing Escape
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: MouseEvent) => {
      const target = e.target as Node | null;
      if (btnRef.current?.contains(target) || popoverRef.current?.contains(target)) return;
      setOpen(false);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  // Determine button icon and aria/tooltip labels
  const iconName =
    rawTheme === "light"
      ? "sun"
      : rawTheme === "dimmed"
      ? "moon"
      : rawTheme === "dark"
      ? "moon"
      : rawTheme === "system"
      ? "monitor"
      : "prism";

  const themeLabel =
    customTheme
      ? customTheme.name
      : rawTheme === "dimmed"
      ? t("Dark Dimmed")
      : rawTheme === "dark"
      ? t("Dark")
      : rawTheme === "light"
      ? t("Light")
      : t("Match system");

  const toggleAria = t("Toggle theme");
  const tipLabel =
    rawTheme === "dark"
      ? t("Theme: dark")
      : rawTheme === "light"
      ? t("Theme: light")
      : t("Theme: system");

  const handleApplyTheme = (id: string) => {
    if (id === "dark" || id === "dimmed" || id === "light" || id === "system") {
      clearInjectedThemeTokens();
      void patchSettings({ appearance: { theme: id as any } });
      const sysDark = window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? true;
      document.documentElement.dataset.theme = id === "system" ? (sysDark ? "dark" : "light") : id;
    } else {
      const found = findThemeById(id);
      if (found) {
        applyCustomThemeTokens(found);
        void patchSettings({ appearance: { theme: id as any } });
      }
    }
  };

  const handleApplyAccent = (accentId: string) => {
    void patchSettings({ appearance: { accent: accentId } });
    document.documentElement.dataset.accent = accentId;
  };

  // Quick cycle with right click / alt click
  const handleQuickCycle = (e: React.MouseEvent) => {
    if (e.altKey) {
      e.preventDefault();
      const sequence = ["system", "dark", "dimmed", "light"];
      const nextIdx = (sequence.indexOf(rawTheme) + 1) % sequence.length;
      handleApplyTheme(sequence[nextIdx]);
    } else {
      setOpen((prev) => !prev);
    }
  };

  // Dynamic positioning for bottom-anchored or top-anchored popover
  const [popoverPos, setPopoverPos] = useState<{
    left: number;
    top?: number;
    bottom?: number;
    maxHeight: number;
  }>({ left: 12, bottom: 42, maxHeight: 520 });

  useEffect(() => {
    if (open && btnRef.current) {
      const r = btnRef.current.getBoundingClientRect();
      const spaceAbove = r.top;
      const spaceBelow = window.innerHeight - r.bottom;
      let left = Math.min(r.left, window.innerWidth - 352);
      if (left < 12) left = 12;

      // Popover should never be cut off by screen boundaries.
      // If the button is in the status bar (at the bottom), anchor upwards using CSS bottom!
      if (spaceAbove >= 360 || spaceAbove >= spaceBelow) {
        setPopoverPos({
          left,
          bottom: window.innerHeight - r.top + 8,
          maxHeight: Math.min(540, spaceAbove - 20),
        });
      } else {
        setPopoverPos({
          left,
          top: r.bottom + 8,
          maxHeight: Math.min(540, spaceBelow - 20),
        });
      }
    }
  }, [open]);

  const [allCustomAndPresets, setAllCustomAndPresets] = useState(() =>
    getAllThemes().filter((t) => t.id !== "dark" && t.id !== "dimmed" && t.id !== "light")
  );

  useEffect(() => {
    const handler = () => {
      setAllCustomAndPresets(
        getAllThemes().filter((t) => t.id !== "dark" && t.id !== "dimmed" && t.id !== "light")
      );
    };
    window.addEventListener("prism:themes-changed", handler);
    return () => window.removeEventListener("prism:themes-changed", handler);
  }, []);

  return (
    <>
      <button
        ref={btnRef}
        type="button"
        className={`tb-btn theme-switcher-btn${open ? " on" : ""} ${className || ""}`}
        aria-label={toggleAria}
        aria-expanded={open}
        data-tip={tipLabel}
        onClick={handleQuickCycle}
      >
        <Icon name={iconName} size={16} />
        {rawTheme === "dimmed" && <span className="dimmed-dot" />}
      </button>

      {open &&
        createPortal(
          <div
            ref={popoverRef}
            className="theme-popover glass-strong"
            style={{
              position: "fixed",
              left: `${popoverPos.left}px`,
              ...(popoverPos.bottom != null
                ? { bottom: `${popoverPos.bottom}px` }
                : { top: `${popoverPos.top}px` }),
              maxHeight: `${popoverPos.maxHeight}px`,
              zIndex: 9999,
            }}
          >
            {/* Popover Header with Active Pill */}
            <div className="theme-popover__header">
              <div className="theme-popover__brand">
                <span className="theme-popover__icon-wrap">
                  <Icon name="prism" size={15} className="text-accent" />
                </span>
                <span className="theme-popover__title">{str("Theme & Appearance")}</span>
              </div>
              <div className="theme-popover__active-pill">
                <span
                  className="dot-pulse"
                  style={{
                    background:
                      rawTheme === "light"
                        ? "#f5b544"
                        : rawTheme === "dimmed"
                        ? "#539bf5"
                        : rawTheme === "dark"
                        ? "#8e8cff"
                        : "var(--accent)",
                  }}
                />
                <span className="theme-popover__pill-text">{themeLabel}</span>
              </div>
            </div>

            {/* Core Primary Themes (2x2 Cards with Window Silhouettes) */}
            <div className="theme-popover__primary-grid">
              {[
                {
                  id: "dark",
                  label: t("Dark"),
                  sub: str("Obsidian"),
                  surface: "#0d0e13",
                  sidebar: "#141722",
                  accent: "#8e8cff",
                },
                {
                  id: "dimmed",
                  label: t("Dark Dimmed"),
                  sub: str("Deep slate"),
                  surface: "#22272e",
                  sidebar: "#1a1f27",
                  accent: "#539bf5",
                },
                {
                  id: "light",
                  label: t("Light"),
                  sub: str("Porcelain"),
                  surface: "#fbfbfd",
                  sidebar: "#edf1f7",
                  accent: "#4f4be0",
                },
                {
                  id: "system",
                  label: t("Match system"),
                  sub: str("Auto follow"),
                  surface: "system",
                  sidebar: "#161b22",
                  accent: "#6461f2",
                },
              ].map((item) => {
                const isActive = rawTheme === item.id;
                return (
                  <button
                    key={item.id}
                    type="button"
                    className={`theme-tile${isActive ? " on" : ""}`}
                    onClick={() => {
                      handleApplyTheme(item.id);
                      setOpen(false);
                    }}
                  >
                    {/* Visual Window Silhouette Swatch */}
                    <div className="theme-tile__preview">
                      {item.id === "system" ? (
                        <div className="theme-tile__system-split">
                          <div className="half light">
                            <div className="mini-titlebar">
                              <span className="mini-dot r" />
                              <span className="mini-dot y" />
                              <span className="mini-dot g" />
                            </div>
                            <div className="mini-body">
                              <div className="mini-side" />
                              <div className="mini-content">
                                <span className="mini-line accent" />
                              </div>
                            </div>
                          </div>
                          <div className="half dark">
                            <div className="mini-titlebar">
                              <span className="mini-dot r" />
                              <span className="mini-dot y" />
                              <span className="mini-dot g" />
                            </div>
                            <div className="mini-body">
                              <div className="mini-side" />
                              <div className="mini-content">
                                <span className="mini-line accent" />
                              </div>
                            </div>
                          </div>
                          <div className="split-badge">
                            <Icon name="monitor" size={11} />
                          </div>
                        </div>
                      ) : (
                        <div
                          className="theme-tile__window"
                          style={{
                            background: item.surface,
                            borderColor: isActive ? "var(--accent)" : "rgba(255, 255, 255, 0.12)",
                          }}
                        >
                          <div className="mini-titlebar">
                            <span className="mini-dot r" />
                            <span className="mini-dot y" />
                            <span className="mini-dot g" />
                          </div>
                          <div className="mini-body">
                            <div className="mini-side" style={{ background: item.sidebar }} />
                            <div className="mini-content">
                              <span className="mini-line accent" style={{ background: item.accent }} />
                              <span
                                className="mini-line"
                                style={{
                                  background:
                                    item.id === "light"
                                      ? "rgba(0, 0, 0, 0.18)"
                                      : "rgba(255, 255, 255, 0.2)",
                                }}
                              />
                            </div>
                          </div>
                        </div>
                      )}
                    </div>

                    <div className="theme-tile__txt">
                      <div className="theme-tile__label-row">
                        <span className="theme-tile__label">{item.label}</span>
                        {isActive && (
                          <span className="theme-tile__check-badge">
                            <Icon name="check" size={10} strokeWidth={3} />
                          </span>
                        )}
                      </div>
                      <span className="theme-tile__sub">{item.sub}</span>
                    </div>
                  </button>
                );
              })}
            </div>

            {/* Curated Popular Themes */}
            <div className="theme-popover__section">
              <div className="theme-popover__section-title">
                <span>{str("Popular Themes")}</span>
              </div>

              <div className="theme-popover__chips-grid">
                {allCustomAndPresets.map((th) => {
                  const isActive = rawTheme === th.id;
                  const thAccent = th.colors.accent || "var(--accent)";
                  return (
                    <button
                      key={th.id}
                      type="button"
                      className={`theme-chip-card${isActive ? " on" : ""}`}
                      title={th.description || th.name}
                      onClick={() => {
                        handleApplyTheme(th.id);
                        setOpen(false);
                      }}
                      style={
                        {
                          "--theme-accent": thAccent,
                        } as React.CSSProperties
                      }
                    >
                      <div className="theme-chip-card__duo">
                        <span
                          className="theme-chip-card__duo-bg"
                          style={{ background: th.colors.bgCanvas }}
                        />
                        <span
                          className="theme-chip-card__duo-accent"
                          style={{ background: thAccent }}
                        />
                      </div>
                      <span className="theme-chip-card__name">{th.name}</span>
                      {isActive && (
                        <span className="theme-chip-card__check">
                          <Icon name="check" size={10} strokeWidth={3} />
                        </span>
                      )}
                    </button>
                  );
                })}
              </div>
            </div>

            {/* Accent Color Tint Palette */}
            <div className="theme-popover__section">
              <span className="theme-popover__section-title">{str("Accent Tint")}</span>
              <div className="theme-popover__accents">
                {ACCENTS.map(([id, label, darkC, lightC]) => {
                  const isLight = resolvedTheme === "light";
                  const color = isLight ? lightC : darkC;
                  const on = currentAccent === id;
                  const isWhiteGraphite = id === "graphite" && !isLight;
                  return (
                    <button
                      key={id}
                      type="button"
                      className={`accent-dot${on ? " on" : ""}`}
                      style={
                        {
                          background: color,
                          color: isWhiteGraphite ? "#111318" : "#ffffff",
                          "--accent-glow": color,
                        } as React.CSSProperties
                      }
                      title={label}
                      onClick={() => handleApplyAccent(id)}
                    >
                      {on && <Icon name="check" size={12} strokeWidth={3} />}
                    </button>
                  );
                })}
              </div>
            </div>

            {/* Popover Footer: Open Appearance Settings */}
            <div className="theme-popover__footer">
              <button
                type="button"
                className="theme-popover__settings-btn"
                onClick={() => {
                  setOpen(false);
                  openSheet({ kind: "settings", section: "appearance" });
                }}
              >
                <div className="theme-popover__settings-btn-left">
                  <span className="theme-popover__settings-btn-icon">
                    <Icon name="settings" size={13} />
                  </span>
                  <span>{t("Settings")}</span>
                </div>
                <Icon name="chevron-right" size={12} className="text-tertiary" />
              </button>
            </div>
          </div>,
          document.body,
        )}
    </>
  );
}
