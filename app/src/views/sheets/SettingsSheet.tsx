import { checkForUpdates } from "../UpdateBanner";
import { BrowserConnect, useExtensionStatus } from "../BrowserConnect";
import { type ReactNode, useEffect, useState } from "react";
import { ListField, NumberField, RateField, Row, Segmented, Select, Slider, Switch, TextField } from "../../components/Controls";
import { BrandTile } from "../../components/LogoMark";
import { Icon, type IconName } from "../../design/icons";
import { type DeepPartial, patchSettings } from "../../lib/actions";
import { api, type HfContext, type IntegrationStatus, type ToolStatus } from "../../lib/api";
import { formatDate } from "../../lib/format";
import { LANGUAGES, t, tk } from "../../lib/i18n";
import { needsRepair } from "../../lib/integration";
import { openUrl, REPO_URL } from "../../lib/links";
import { categoryIcon, displayName } from "../../lib/present";
import { COMPLETE_SOUNDS } from "../../lib/sounds";
import { playSound, useApp } from "../../lib/store";
import type { Settings } from "../../lib/types";
import { attempt, toast, useUi } from "../../lib/ui";
import { pickFile, pickFolder, pickSavePath } from "../../lib/win";
import { scheduleSummary } from "./SchedulerSheet";
import {
  BASE_DARK_COLORS,
  BASE_DIMMED_COLORS,
  BASE_LIGHT_COLORS,
  createNewCustomTheme,
  deleteUserTheme,
  getUserThemes,
  hexToRgbTriplet,
  saveUserTheme,
  THEME_PRESETS,
  type CustomTheme,
} from "../../lib/customThemes";

const str = (s: string) => s;

export const SECTIONS: { id: string; label: string; icon: IconName; group?: boolean }[] = [
  { id: "general", label: tk("General"), icon: "settings" },
  { id: "appearance", label: tk("Appearance"), icon: "sun" },
  { id: "downloads", label: tk("Downloads"), icon: "download" },
  { id: "connection", label: tk("Connection"), icon: "globe" },
  { id: "proxy", label: tk("Proxy"), icon: "proxy" },
  { id: "speed", label: tk("Speed & queues"), icon: "speed-limiter" },
  { id: "capture", label: tk("File types & capture"), icon: "filter", group: true },
  { id: "categories", label: tk("Categories"), icon: "all" },
  { id: "browser", label: tk("Browser integration"), icon: "browser" },
  { id: "video", label: tk("Video & streams"), icon: "video" },
  { id: "models", label: tk("AI models"), icon: "model" },
  { id: "scheduler", label: tk("Scheduler"), icon: "scheduler" },
  { id: "sounds", label: tk("Notifications & sounds"), icon: "bell", group: true },
  { id: "security", label: tk("Security & antivirus"), icon: "shield" },
  { id: "advanced", label: tk("Advanced"), icon: "options" },
  { id: "shortcuts", label: tk("Keyboard shortcuts"), icon: "list" },
  { id: "about", label: tk("About Prism"), icon: "info" },
];

const save = (patch: DeepPartial<Settings>) => void patchSettings(patch);
const DEFAULT_HUB = "https://huggingface.co";
const MIRROR = "https://hf-mirror.com";

export function SettingsSheet({ section }: { section?: string }) {
  const [cur, setCur] = useState(section ?? "general");
  const [q, setQ] = useState("");
  const s = useApp((st) => st.settings);
  if (!s) return null;
  const list = SECTIONS.filter((x) => !q || t(x.label).toLowerCase().includes(q.toLowerCase()));
  const meta = SECTIONS.find((x) => x.id === cur) ?? SECTIONS[0];
  return (
    <div className="settings">
      <aside className="snav">
        <label className="input">
          <Icon name="search" size={15} />
          <input placeholder={t("Search settings")} aria-label={t("Search settings")} value={q} onChange={(e) => setQ(e.target.value)} />
        </label>
        <nav aria-label={t("Settings sections")}>
          {list.map((x) => (
            <div key={x.id}>
              {x.group && !q && <div className="grp" />}
              <button type="button" className="side-item" aria-current={x.id === cur ? "true" : undefined} onClick={() => setCur(x.id)}>
                <Icon name={x.icon} size={16} />
                <span className="side-item__label">{t(x.label)}</span>
              </button>
            </div>
          ))}
        </nav>
      </aside>
      <section className="scontent" aria-label={t(meta.label)}>
        <div className="sbody" key={cur}>
          <h1>{t(meta.label)}</h1>
          <SectionBody id={cur} s={s} />
        </div>
      </section>
    </div>
  );
}

function Group({ title, action, children }: { title?: string; action?: ReactNode; children: ReactNode }) {
  return (
    <div className="group">
      {title && (
        <div className="group__hdr">
          <div className="group__t">{title}</div>
          {action}
        </div>
      )}
      <div className="card">{children}</div>
    </div>
  );
}

const Toggle = ({ title, hint, value, onChange }: { title: string; hint?: string; value: boolean; onChange: (v: boolean) => void }) => (
  <Row title={title} hint={hint}>
    <Switch checked={value} label={title} onChange={onChange} />
  </Row>
);

function SectionBody({ id, s }: { id: string; s: Settings }) {
  switch (id) {
    case "general":
      return (
        <>
          <p className="lead">{t("How Prism starts and behaves in the background.")}</p>
          <Group title={t("Language")}>
            <Row title={t("Language")} hint={t("Prism follows your system's language unless you pick one here")}>
              <Select
                label={t("Language")}
                value={s.general.language}
                onChange={(v) => save({ general: { language: v } })}
                options={[{ value: "", label: t("Same as the system") }, ...LANGUAGES.map((l) => ({ value: l.code as string, label: l.name }))]}
                width={200}
              />
            </Row>
          </Group>
          <Group title={t("Startup")}>
            <Toggle
              title={t("Launch at startup")}
              hint={t("Start with Windows, minimized to the tray")}
              value={s.general.launchAtStartup}
              onChange={async (v) => {
                await attempt(api.setAutostart(v), t("Could not change autostart"));
                save({ general: { launchAtStartup: v } });
              }}
            />
            <Toggle title={t("Start minimized")} value={s.general.startMinimized} onChange={(v) => save({ general: { startMinimized: v } })} />
            <Toggle title={t("Close button hides to the tray")} hint={t("Downloads keep running; quit from the tray menu")} value={s.general.closeToTray} onChange={(v) => save({ general: { closeToTray: v } })} />
            {document.documentElement.dataset.os === "mac" && (
              <Toggle title={t("Hide the Dock icon")} hint={t("While Prism waits in the menu bar. The icon is back while its window is open")} value={s.general.hideDockIcon} onChange={(v) => save({ general: { hideDockIcon: v } })} />
            )}
          </Group>
          <Group title={t("Windows")}>
            <Toggle title={t("Show the Download File dialog")} hint={t("Off = new downloads start immediately with default settings")} value={s.general.showCaptureDialog} onChange={(v) => save({ general: { showCaptureDialog: v } })} />
            <Toggle title={t("Open a progress window for new downloads")} value={s.general.showProgressWindow} onChange={(v) => save({ general: { showProgressWindow: v } })} />
            <Toggle title={t("Show the completion dialog")} value={s.general.showCompletionDialog} onChange={(v) => save({ general: { showCompletionDialog: v } })} />
          </Group>
          <Group title={t("Deleting")}>
            <Toggle title={t("Confirm before deleting")} value={s.general.confirmDelete} onChange={(v) => save({ general: { confirmDelete: v } })} />
            <Toggle title={t("Use the trash")} hint={t("Deleted downloads can be restored until you empty the trash")} value={s.general.useTrash} onChange={(v) => save({ general: { useTrash: v } })} />
            <Toggle
              title={t("Remove downloads whose files were deleted")}
              hint={t("Finished downloads leave the list once their file is gone from the computer")}
              value={s.general.removeDeletedFiles}
              onChange={async (v) => {
                // Saved first: the core checks the setting before it removes anything.
                await patchSettings({ general: { removeDeletedFiles: v } });
                if (v) void api.removeDeletedFiles().catch(() => {});
              }}
            />
            <Toggle title={t("Check for updates automatically")} hint={t("Prism asks before installing; it restarts by itself and your downloads continue")} value={s.general.checkUpdates} onChange={(v) => save({ general: { checkUpdates: v } })} />
          </Group>
          <Group title={t("Clipboard")}>
            <Toggle title={t("Watch the clipboard for links")} value={s.clipboard.monitor} onChange={(v) => save({ clipboard: { monitor: v } })} />
            <Toggle title={t("Ask before adding copied links")} hint={t("Off = copied links go straight into the list, to start whenever you want")} value={s.clipboard.ask} onChange={(v) => save({ clipboard: { ask: v } })} />
          </Group>
        </>
      );
    case "appearance":
      return <Appearance s={s} />;
    case "downloads":
      return (
        <>
          <Group title={t("Where files go")}>
            <Row title={t("Default folder")} hint={s.downloads.defaultDir}>
              <button type="button" className="btn" onClick={() => void pickFolder(s.downloads.defaultDir).then((p) => p && save({ downloads: { defaultDir: p } }))}>
                <Icon name="folder-open" size={15} /> {t("Change…")}
              </button>
            </Row>
            <Toggle title={t("Sort into category folders")} hint={t("Video → Downloads\\Video, Programs → Downloads\\Programs …")} value={s.downloads.useCategoryFolders} onChange={(v) => save({ downloads: { useCategoryFolders: v } })} />
            <Row title={t("When the file already exists")}>
              <Select
                label={t("When the file already exists")}
                value={s.downloads.conflict}
                onChange={(v) => save({ downloads: { conflict: v } })}
                options={[
                  { value: "ask", label: t("Ask me") },
                  { value: "rename", label: t("Rename the new file") },
                  { value: "overwrite", label: t("Overwrite") },
                ]}
                width={200}
              />
            </Row>
          </Group>
          <Group title={t("Engine")}>
            <Row title={t("Connections per download")} hint={t("Prism splits files into this many segments (work-stealing keeps all busy)")}>
              <NumberField value={s.downloads.defaultConnections} min={1} max={s.downloads.maxConnections} label={t("Connections per download")} onCommit={(v) => save({ downloads: { defaultConnections: v } })} width={90} />
            </Row>
            <Row title={t("Maximum connections")}>
              <NumberField value={s.downloads.maxConnections} min={1} max={64} label={t("Maximum connections")} onCommit={(v) => save({ downloads: { maxConnections: v } })} width={90} />
            </Row>
            <Row title={t("Retries before failing")} hint={t("Exponential back-off up to 30 s between attempts")}>
              <NumberField value={s.downloads.maxRetries} min={0} max={999} label={t("Retries")} onCommit={(v) => save({ downloads: { maxRetries: v } })} width={90} />
            </Row>
            <Toggle title={t("Start downloads automatically")} value={s.downloads.autoStart} onChange={(v) => save({ downloads: { autoStart: v } })} />
            <Toggle title={t("Verify checksums when known")} value={s.downloads.verifyChecksums} onChange={(v) => save({ downloads: { verifyChecksums: v } })} />
          </Group>
        </>
      );
    case "connection":
      return (
        <Group>
          <Row title={t("HTTP version")} hint={t("HTTP/1.1 opens one TCP connection per segment — fastest for bulk transfers")}>
            <Select
              label={t("HTTP version")}
              value={s.connection.httpVersion}
              onChange={(v) => save({ connection: { httpVersion: v } })}
              options={[
                { value: "http1", label: "HTTP/1.1" },
                { value: "auto", label: t("Automatic (HTTP/2)") },
                { value: "http3", label: t("HTTP/3 (experimental)") },
              ]}
              width={210}
            />
          </Row>
          <Row title={t("Connect timeout")}>
            <NumberField value={s.connection.connectTimeoutSecs} min={1} max={300} suffix="s" label={t("Connect timeout")} onCommit={(v) => save({ connection: { connectTimeoutSecs: v } })} width={90} />
          </Row>
          <Row title={t("Read timeout")}>
            <NumberField value={s.connection.readTimeoutSecs} min={1} max={600} suffix="s" label={t("Read timeout")} onCommit={(v) => save({ connection: { readTimeoutSecs: v } })} width={90} />
          </Row>
          <Row title={t("Connections per server")} hint={t("0 = no limit; Prism also learns limits servers enforce")}>
            <NumberField value={s.connection.maxConnectionsPerHost} min={0} max={64} label={t("Connections per server")} onCommit={(v) => save({ connection: { maxConnectionsPerHost: v } })} width={90} />
          </Row>
          <Row title={t("User agent")} hint={t("Empty = a current Chrome user agent")}>
            <TextField value={s.connection.userAgent} onCommit={(v) => save({ connection: { userAgent: v } })} label={t("User agent")} mono placeholder={t("Default")} />
          </Row>
        </Group>
      );
    case "proxy":
      return (
        <Group>
          <Row title={t("Proxy")}>
            <Segmented
              size="sm"
              label={t("Proxy mode")}
              value={s.proxy.mode}
              onChange={(v) => save({ proxy: { mode: v } })}
              options={[
                { value: "none", label: t("None") },
                { value: "system", label: t("System") },
                { value: "http", label: "HTTP(S)" },
                { value: "socks5", label: "SOCKS5" },
              ]}
            />
          </Row>
          {(s.proxy.mode === "http" || s.proxy.mode === "socks5") && (
            <>
              <Row title={t("Address")}>
                <TextField value={s.proxy.address} onCommit={(v) => save({ proxy: { address: v } })} label={t("Proxy address")} mono placeholder="127.0.0.1:8080" />
              </Row>
              <Row title={t("User name")}>
                <TextField value={s.proxy.username} onCommit={(v) => save({ proxy: { username: v } })} label={t("Proxy user name")} />
              </Row>
              <Row title={t("Password")}>
                <TextField value={s.proxy.password} onCommit={(v) => save({ proxy: { password: v } })} label={t("Proxy password")} type="password" />
              </Row>
              <Row title={t("Bypass for")} hint={t("Hosts that connect directly")} align="start">
                <ListField value={s.proxy.bypass} onCommit={(v) => save({ proxy: { bypass: v } })} label={t("Proxy bypass")} placeholder="localhost, *.lan" rows={2} />
              </Row>
            </>
          )}
        </Group>
      );
    case "speed":
      return <SpeedQueues s={s} />;
    case "capture":
      return (
        <>
          <Group title={t("Browser capture")}>
            <Toggle title={t("Take over downloads from the browser")} value={s.capture.enabled} onChange={(v) => save({ capture: { enabled: v } })} />
            <Row title={t("Skip with key")} hint={t("Hold this key while clicking to let the browser download instead")}>
              <Segmented
                size="sm"
                label={t("Bypass key")}
                value={s.capture.bypassKey}
                onChange={(v) => save({ capture: { bypassKey: v } })}
                options={[
                  { value: "alt", label: "Alt" },
                  { value: "ctrl", label: "Ctrl" },
                  { value: "shift", label: "Shift" },
                  { value: "none", label: t("None") },
                ]}
              />
            </Row>
            <Row title={t("Ignore files smaller than")}>
              <NumberField value={s.capture.minSizeKb} min={0} max={10_000_000} suffix="KB" label={t("Minimum size")} onCommit={(v) => save({ capture: { minSizeKb: v } })} width={120} />
            </Row>
          </Group>
          <Group title={t("What to capture")}>
            <Row title={t("File extensions")} align="start">
              <ListField value={s.capture.extensions} onCommit={(v) => save({ capture: { extensions: v } })} label={t("Captured extensions")} rows={3} />
            </Row>
            <Row title={t("MIME types")} align="start">
              <ListField value={s.capture.mimeTypes} onCommit={(v) => save({ capture: { mimeTypes: v } })} label={t("Captured MIME types")} rows={2} />
            </Row>
            <Row title={t("Never capture from")} hint={t("Wildcards allowed")} align="start">
              <ListField value={s.capture.excludedSites} onCommit={(v) => save({ capture: { excludedSites: v } })} label={t("Excluded sites")} rows={2} placeholder="*.example.com" />
            </Row>
          </Group>
          <Group title={t("Video button")}>
            <Toggle title={t("Show “Download video” on web pages")} value={s.capture.videoButton} onChange={(v) => save({ capture: { videoButton: v } })} />
            <Row title={t("Only for videos longer than")}>
              <NumberField value={s.capture.videoMinDurationSecs} min={0} max={36000} suffix="s" label={t("Minimum video duration")} onCommit={(v) => save({ capture: { videoMinDurationSecs: v } })} width={100} />
            </Row>
          </Group>
        </>
      );
    case "categories":
      return <Categories />;
    case "browser":
      return <BrowserIntegration />;
    case "video":
      return <Video s={s} />;
    case "models":
      return <Models s={s} />;
    case "scheduler":
      return <SchedulerShortcut />;
    case "sounds":
      return (
        <>
          <Group title={t("Notifications")}>
            <Toggle title={t("System notifications")} hint={t("When downloads finish or fail while Prism is in the background")} value={s.general.notifications} onChange={(v) => save({ general: { notifications: v } })} />
          </Group>
          <Group title={t("Sounds")}>
            <Toggle title={t("Play sounds")} value={s.sounds.enabled} onChange={(v) => save({ sounds: { enabled: v } })} />
            <Toggle title={t("Download complete")} value={s.sounds.onComplete} onChange={(v) => save({ sounds: { onComplete: v } })} />
            <Row title={t("Download complete sound")} hint={t("Plays when you pick one")}>
              <Select
                label={t("Download complete sound")}
                value={COMPLETE_SOUNDS.some((c) => c.id === s.sounds.completeSound) ? s.sounds.completeSound : "classic"}
                onChange={(v) => {
                  save({ sounds: { completeSound: v } });
                  playSound("complete", v);
                }}
                options={COMPLETE_SOUNDS.map((c) => ({ value: c.id, label: t(c.name) }))}
              />
            </Row>
            <Toggle title={t("Download failed")} value={s.sounds.onFailed} onChange={(v) => save({ sounds: { onFailed: v } })} />
            <Toggle title={t("Queue finished")} value={s.sounds.onQueueDone} onChange={(v) => save({ sounds: { onQueueDone: v } })} />
            <Row title={t("Volume")}>
              <div className="inline">
                <Slider value={s.sounds.volume} label={t("Volume")} onChange={(v) => save({ sounds: { volume: v } })} />
                <button type="button" className="btn btn--sm" onClick={() => playSound("complete")}>
                  <Icon name="sound" size={14} /> {t("Test")}
                </button>
              </div>
            </Row>
          </Group>
        </>
      );
    case "security":
      return (
        <Group title={t("Scan finished downloads")}>
          <Toggle title={t("Scan with an antivirus")} value={s.antivirus.enabled} onChange={(v) => save({ antivirus: { enabled: v } })} />
          <Row title={t("Scanner")}>
            <Select
              label={t("Scanner")}
              value={s.antivirus.kind}
              onChange={(v) => save({ antivirus: { kind: v } })}
              options={[
                { value: "auto", label: t("Automatic") },
                { value: "defender", label: "Microsoft Defender" },
                { value: "clamav", label: "ClamAV" },
                { value: "custom", label: t("Custom command") },
              ]}
              width={200}
            />
          </Row>
          {s.antivirus.kind === "custom" && (
            <>
              <Row title={t("Program")}>
                <div className="withbtn">
                  <TextField value={s.antivirus.command} onCommit={(v) => save({ antivirus: { command: v } })} label={t("Scanner program")} mono />
                  <button type="button" className="btn" onClick={() => void pickFile([{ name: t("Programs"), extensions: ["exe", "cmd", "bat", "sh"] }]).then((p) => p && save({ antivirus: { command: p } }))}>
                    {t("Browse…")}
                  </button>
                </div>
              </Row>
              <Row title={t("Arguments")} hint={t("[FILE] is replaced by the downloaded file path")}>
                <TextField value={s.antivirus.args} onCommit={(v) => save({ antivirus: { args: v } })} label={t("Scanner arguments")} mono placeholder='"[FILE]"' />
              </Row>
            </>
          )}
          <Row title={t("Only these extensions")} hint={t("Empty = scan every file")} align="start">
            <ListField value={s.antivirus.extensions} onCommit={(v) => save({ antivirus: { extensions: v } })} label={t("Scanned extensions")} rows={2} placeholder="exe, msi, zip" />
          </Row>
        </Group>
      );
    case "advanced":
      return <Advanced s={s} />;
    case "shortcuts":
      return <Shortcuts />;
    case "about":
      return <About />;
    default:
      return null;
  }
}

// ---- sections with more logic -------------------------------------------------------

const ACCENTS = [
  ["prism", "Prism", "#6461f2", "#5653ea"],
  ["cyan", tk("Cyan"), "#36c5e6", "#0b7fa0"],
  ["emerald", tk("Emerald"), "#2fc489", "#087f53"],
  ["amber", tk("Amber"), "#f2b54a", "#a86200"],
  ["rose", tk("Rose"), "#d03a64", "#cf3760"],
  ["graphite", tk("Graphite"), "#e6e8ee", "#1f2229"],
] as const;

function Appearance({ s }: { s: Settings }) {
  const light = document.documentElement.dataset.theme === "light";
  const currentTheme = s.appearance.theme || "system";
  const [userThemes, setUserThemes] = useState(() => getUserThemes());
  const [editorTarget, setEditorTarget] = useState<{ open: boolean; theme?: CustomTheme } | null>(null);

  useEffect(() => {
    const handler = () => setUserThemes(getUserThemes());
    window.addEventListener("prism:themes-changed", handler);
    return () => window.removeEventListener("prism:themes-changed", handler);
  }, []);

  const themes = [
    ["dark", t("Dark"), "Obsidian", "pv-dark"],
    ["dimmed", t("Dark Dimmed"), "Deep slate", "pv-dimmed"],
    ["light", t("Light"), "Porcelain", "pv-light"],
    ["system", t("Match system"), "Auto follow", "pv-sys"],
  ] as const;

  return (
    <>
      <p className="lead">{t("Changes apply instantly. Prism follows your system theme unless you pick one.")}</p>
      <Group title={t("Theme")}>
        <div className="themes">
          {themes.map(([v, label, sub, cls]) => {
            const on = currentTheme === v;
            return (
              <button
                key={v}
                type="button"
                className={`tcard${on ? " on" : ""}`}
                aria-pressed={on}
                onClick={() => save({ appearance: { theme: v } })}
              >
                <span className={`tprev ${cls}`}>
                  {v === "system" ? (
                    <div className="tprev-window tprev-split">
                      <div className="tprev-half light">
                        <div className="tprev-titlebar">
                          <span className="tprev-dot r" />
                          <span className="tprev-dot y" />
                          <span className="tprev-dot g" />
                        </div>
                        <div className="tprev-body">
                          <div className="tprev-side">
                            <span className="tprev-nav-item active" />
                            <span className="tprev-nav-item" />
                          </div>
                          <div className="tprev-content">
                            <span className="tprev-line accent" />
                            <span className="tprev-line" />
                          </div>
                        </div>
                      </div>
                      <div className="tprev-half dark">
                        <div className="tprev-titlebar">
                          <span className="tprev-dot r" />
                          <span className="tprev-dot y" />
                          <span className="tprev-dot g" />
                        </div>
                        <div className="tprev-body">
                          <div className="tprev-side">
                            <span className="tprev-nav-item active" />
                            <span className="tprev-nav-item" />
                          </div>
                          <div className="tprev-content">
                            <span className="tprev-line accent" />
                            <span className="tprev-line" />
                          </div>
                        </div>
                      </div>
                      <div className="tprev-split-icon">
                        <Icon name="monitor" size={12} />
                      </div>
                    </div>
                  ) : (
                    <div className="tprev-window">
                      <div className="tprev-titlebar">
                        <span className="tprev-dot r" />
                        <span className="tprev-dot y" />
                        <span className="tprev-dot g" />
                      </div>
                      <div className="tprev-body">
                        <div className="tprev-side">
                          <span className="tprev-nav-item active" />
                          <span className="tprev-nav-item" />
                        </div>
                        <div className="tprev-content">
                          <span className="tprev-line accent" />
                          <span className="tprev-line" />
                        </div>
                      </div>
                    </div>
                  )}
                </span>
                <span className="tcard__meta">
                  <div className="tcard__title-row">
                    <span className="tcard__title">{label}</span>
                    {on && (
                      <span className="tcard__check">
                        <Icon name="check" size={10} strokeWidth={3} />
                      </span>
                    )}
                  </div>
                  <span className="tcard__sub">{sub}</span>
                </span>
              </button>
            );
          })}
        </div>
      </Group>

      <Group title={str("Popular Themes")}>
        <div className="popular-themes-grid">
          {THEME_PRESETS.map((preset) => {
            const on = currentTheme === preset.id;
            const thAccent = preset.colors.accent || "var(--accent)";
            return (
              <button
                key={preset.id}
                type="button"
                className={`popular-theme-card${on ? " on" : ""}`}
                aria-pressed={on}
                onClick={() => save({ appearance: { theme: preset.id as any } })}
                style={{ "--theme-accent": thAccent } as React.CSSProperties}
              >
                <div className="popular-theme-card__swatch">
                  <span className="stripe bg" style={{ background: preset.colors.bgCanvas }} />
                  <span className="stripe surface" style={{ background: preset.colors.surfaceRaised }} />
                  <span className="stripe accent" style={{ background: thAccent }} />
                </div>
                <div className="popular-theme-card__info">
                  <div className="popular-theme-card__header">
                    <div className="popular-theme-card__title-row">
                      <span className="popular-theme-card__title">{preset.name}</span>
                      <span className="popular-theme-card__badge">{preset.base === "dark" ? "Dark" : "Light"}</span>
                    </div>
                    {on && (
                      <span className="popular-theme-card__check">
                        <Icon name="check" size={10} strokeWidth={3} />
                      </span>
                    )}
                  </div>
                  <span className="popular-theme-card__desc">{preset.description}</span>
                </div>
              </button>
            );
          })}
        </div>
      </Group>

      {/* Custom User Themes Group */}
      <Group
        title={str("Custom Themes")}
        action={
          <button
            type="button"
            className="btn btn--sm"
            onClick={() => setEditorTarget({ open: true })}
          >
            <Icon name="plus" size={13} /> {str("Create Theme")}
          </button>
        }
      >
        {userThemes.length === 0 ? (
          <div className="custom-themes-empty">
            <div className="custom-themes-empty__icon">
              <Icon name="prism" size={22} />
            </div>
            <div className="custom-themes-empty__txt">
              <h4>{str("Create your own custom theme")}</h4>
              <p>{str("Design personalized palettes with custom canvas, surface, text, and accent colors.")}</p>
            </div>
            <button
              type="button"
              className="btn btn--primary btn--sm"
              onClick={() => setEditorTarget({ open: true })}
            >
              <Icon name="plus" size={12} /> {str("New Theme")}
            </button>
          </div>
        ) : (
          <div className="popular-themes-grid">
            {userThemes.map((userTh) => {
              const on = currentTheme === userTh.id;
              const thAccent = userTh.colors.accent || "var(--accent)";
              return (
                <div
                  key={userTh.id}
                  className={`popular-theme-card${on ? " on" : ""}`}
                  style={{ "--theme-accent": thAccent } as React.CSSProperties}
                  onClick={() => save({ appearance: { theme: userTh.id as any } })}
                  role="button"
                  tabIndex={0}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      save({ appearance: { theme: userTh.id as any } });
                    }
                  }}
                >
                  <div className="popular-theme-card__swatch">
                    <span className="stripe bg" style={{ background: userTh.colors.bgCanvas }} />
                    <span className="stripe surface" style={{ background: userTh.colors.surfaceRaised }} />
                    <span className="stripe accent" style={{ background: thAccent }} />
                  </div>
                  <div className="popular-theme-card__info">
                    <div className="popular-theme-card__header">
                      <div className="popular-theme-card__title-row">
                        <span className="popular-theme-card__title">{userTh.name}</span>
                        <span className="popular-theme-card__badge custom">{str("Custom")}</span>
                      </div>
                      <div className="popular-theme-card__actions" onClick={(e) => e.stopPropagation()}>
                        <button
                          type="button"
                          className="icon-action-btn"
                          title={str("Edit theme")}
                          onClick={() => setEditorTarget({ open: true, theme: userTh })}
                        >
                          <Icon name="rename" size={13} />
                        </button>
                        <button
                          type="button"
                          className="icon-action-btn icon-action-btn--del"
                          title={str("Delete theme")}
                          onClick={() => {
                            deleteUserTheme(userTh.id);
                            if (currentTheme === userTh.id) {
                              save({ appearance: { theme: "system" } });
                            }
                            toast(str("Deleted custom theme"));
                          }}
                        >
                          <Icon name="delete" size={13} />
                        </button>
                        {on && (
                          <span className="popular-theme-card__check">
                            <Icon name="check" size={10} strokeWidth={3} />
                          </span>
                        )}
                      </div>
                    </div>
                    <span className="popular-theme-card__desc">{userTh.description || str("Custom user theme")}</span>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </Group>

      {editorTarget?.open && (
        <CustomThemeModal
          initialTheme={editorTarget.theme}
          onClose={() => setEditorTarget(null)}
          onSave={(saved) => {
            saveUserTheme(saved);
            save({ appearance: { theme: saved.id as any } });
            setEditorTarget(null);
            toast(str("Custom theme saved!"));
          }}
        />
      )}
      <Group title={t("Accent colour")}>
        <div className="accents">
          {ACCENTS.map(([id, label, dark, lightC]) => {
            const on = (s.appearance.accent || "prism") === id;
            return (
              <button key={id} type="button" className={`sw${on ? " on" : ""}`} aria-pressed={on} style={{ ["--c" as string]: light ? lightC : dark }} onClick={() => save({ appearance: { accent: id } })}>
                <b>{on && <Icon name="check" size={16} strokeWidth={2.5} />}</b>
                {t(label)}
              </button>
            );
          })}
          <p className="accents__note">{t("Accent tints selection, focus rings, progress and the window’s ambient light. Status colours never change.")}</p>
        </div>
      </Group>
      <div className="twocol">
        <Group title={t("Materials & motion")}>
          <Toggle title={t("Liquid glass")} hint={t("Frosted, translucent sidebar & toolbars")} value={!s.appearance.reduceTransparency} onChange={(v) => save({ appearance: { reduceTransparency: !v } })} />
          <Toggle title={t("Window material")} hint={t("Mica / vibrancy behind the window (restart window)")} value={s.appearance.windowMaterial} onChange={(v) => save({ appearance: { windowMaterial: v } })} />
          <Row title={t("Motion")}>
            <Segmented
              size="sm"
              label={t("Motion")}
              value={s.appearance.motion}
              onChange={(v) => save({ appearance: { motion: v } })}
              options={[
                { value: "system", label: t("System") },
                { value: "full", label: t("Full") },
                { value: "reduced", label: t("Reduced") },
              ]}
            />
          </Row>
        </Group>
        <Group title={t("Layout")}>
          <Toggle title={t("Compact view")} hint={t("Just the download list, without the sidebar, details pane and tips")} value={s.appearance.compactView} onChange={(v) => save({ appearance: { compactView: v } })} />
          <Row title={t("Row density")} hint={t("Compact fits ~45% more rows")}>
            <Segmented
              size="sm"
              label={t("Row density")}
              value={s.appearance.density}
              onChange={(v) => save({ appearance: { density: v } })}
              options={[
                { value: "comfortable", label: t("Comfortable") },
                { value: "compact", label: t("Compact") },
              ]}
            />
          </Row>
          <Row title={t("Details pane")} hint={t("Connections, speed and info under the list")}>
            <Switch checked={useUi.getState().bottomOpen && !s.appearance.compactView} disabled={s.appearance.compactView} label={t("Details pane")} onChange={(v) => useUi.getState().setBottom({ bottomOpen: v })} />
          </Row>
        </Group>
      </div>
    </>
  );
}

function CustomThemeModal({
  initialTheme,
  onClose,
  onSave,
}: {
  initialTheme?: CustomTheme;
  onClose: () => void;
  onSave: (theme: CustomTheme) => void;
}) {
  const [name, setName] = useState(initialTheme?.name || "");
  const [base, setBase] = useState<"dark" | "dimmed" | "light">(initialTheme?.base || "dark");
  const baseDefaults = base === "light" ? BASE_LIGHT_COLORS : base === "dimmed" ? BASE_DIMMED_COLORS : BASE_DARK_COLORS;

  const [bgCanvas, setBgCanvas] = useState(initialTheme?.colors.bgCanvas || baseDefaults.bgCanvas);
  const [surfaceRaised, setSurfaceRaised] = useState(initialTheme?.colors.surfaceRaised || baseDefaults.surfaceRaised);
  const [accent, setAccent] = useState(initialTheme?.colors.accent || baseDefaults.accent || "#6461f2");
  const [textPrimary, setTextPrimary] = useState(initialTheme?.colors.textPrimary || baseDefaults.textPrimary);

  const handleBaseChange = (newBase: "dark" | "dimmed" | "light") => {
    setBase(newBase);
    const d = newBase === "light" ? BASE_LIGHT_COLORS : newBase === "dimmed" ? BASE_DIMMED_COLORS : BASE_DARK_COLORS;
    setBgCanvas(d.bgCanvas);
    setSurfaceRaised(d.surfaceRaised);
    setTextPrimary(d.textPrimary);
    if (d.accent) setAccent(d.accent);
  };

  const handleApplyPresetColors = (cBg: string, cSurface: string, cAccent: string, cText?: string) => {
    setBgCanvas(cBg);
    setSurfaceRaised(cSurface);
    setAccent(cAccent);
    if (cText) setTextPrimary(cText);
  };

  const handleSave = () => {
    const themeName = name.trim() || str("Custom Theme");
    const themeToSave = initialTheme
      ? {
          ...initialTheme,
          name: themeName,
          base,
          colors: {
            ...initialTheme.colors,
            bgCanvas,
            surfaceBase: bgCanvas,
            surfaceSunken: bgCanvas,
            surfaceRaised,
            surfaceOverlay: surfaceRaised,
            accent,
            accentRgb: hexToRgbTriplet(accent),
            textPrimary,
          },
        }
      : createNewCustomTheme(base, themeName, {
          bgCanvas,
          surfaceBase: bgCanvas,
          surfaceSunken: bgCanvas,
          surfaceRaised,
          surfaceOverlay: surfaceRaised,
          accent,
          accentRgb: hexToRgbTriplet(accent),
          textPrimary,
        });

    onSave(themeToSave);
  };

  return (
    <div className="custom-theme-backdrop" onClick={onClose}>
      <div className="custom-theme-dialog" onClick={(e) => e.stopPropagation()}>
        <div className="custom-theme-dialog__header">
          <div className="custom-theme-dialog__title-wrap">
            <span className="custom-theme-dialog__icon">
              <Icon name="prism" size={16} />
            </span>
            <h3>{initialTheme ? str("Edit Theme") : str("Create Custom Theme")}</h3>
          </div>
          <button type="button" className="close-btn" onClick={onClose} aria-label="Close">
            <Icon name="close" size={14} />
          </button>
        </div>

        <div className="custom-theme-dialog__body">
          {/* Live Micro-Preview Window */}
          <div className="custom-theme-preview-box" style={{ background: bgCanvas, color: textPrimary }}>
            <div className="preview-topbar" style={{ borderBottom: "1px solid rgba(255, 255, 255, 0.08)" }}>
              <div className="preview-dots">
                <span className="dot" style={{ background: "#ff5f56" }} />
                <span className="dot" style={{ background: "#ffbd2e" }} />
                <span className="dot" style={{ background: "#27c93f" }} />
              </div>
              <span className="preview-title" style={{ opacity: 0.75 }}>
                {name || "Custom Theme Preview"}
              </span>
              <span className="preview-badge" style={{ background: accent, color: "#fff" }}>
                Active
              </span>
            </div>
            <div className="preview-body">
              <div
                className="preview-sidebar"
                style={{
                  background: "rgba(0,0,0,0.18)",
                  borderRight: "1px solid rgba(255,255,255,0.06)",
                }}
              >
                <span className="preview-nav-pill" style={{ background: accent, width: "65%" }} />
                <span className="preview-nav-pill" style={{ background: "rgba(255,255,255,0.15)", width: "80%" }} />
                <span className="preview-nav-pill" style={{ background: "rgba(255,255,255,0.15)", width: "70%" }} />
              </div>
              <div className="preview-content">
                <div
                  className="preview-card"
                  style={{ background: surfaceRaised, border: "1px solid rgba(255,255,255,0.1)" }}
                >
                  <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                    <span style={{ fontSize: 11, fontWeight: 600 }}>Prism Download</span>
                    <span style={{ fontSize: 9.5, color: accent, fontWeight: 700 }}>88%</span>
                  </div>
                  <div className="preview-bar" style={{ background: "rgba(255,255,255,0.12)" }}>
                    <span className="preview-fill" style={{ background: accent, width: "75%" }} />
                  </div>
                </div>
              </div>
            </div>
          </div>

          {/* Theme Name */}
          <div className="custom-theme-field">
            <label className="custom-theme-label">{str("Theme Name")}</label>
            <input
              type="text"
              className="custom-theme-input"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Emerald Midnight"
              autoFocus
            />
          </div>

          {/* Base Contrast Mode */}
          <div className="custom-theme-field">
            <label className="custom-theme-label">{str("Base Contrast Mode")}</label>
            <div className="custom-theme-base-tabs">
              {(["dark", "dimmed", "light"] as const).map((b) => (
                <button
                  key={b}
                  type="button"
                  className={`base-tab-btn${base === b ? " on" : ""}`}
                  onClick={() => handleBaseChange(b)}
                >
                  {b === "dark" ? str("Dark (Obsidian)") : b === "dimmed" ? str("Dark Dimmed") : str("Light (Porcelain)")}
                </button>
              ))}
            </div>
          </div>

          {/* Color Tokens Form */}
          <div className="custom-theme-tokens-grid">
            <div className="token-picker-item">
              <span className="token-picker-label">{str("Canvas Background")}</span>
              <div className="token-picker-ctrl">
                <input
                  type="color"
                  value={bgCanvas}
                  onChange={(e) => setBgCanvas(e.target.value)}
                  className="color-dot-picker"
                />
                <input
                  type="text"
                  value={bgCanvas}
                  onChange={(e) => setBgCanvas(e.target.value)}
                  className="hex-code-input"
                />
              </div>
            </div>

            <div className="token-picker-item">
              <span className="token-picker-label">{str("Raised Card Surface")}</span>
              <div className="token-picker-ctrl">
                <input
                  type="color"
                  value={surfaceRaised}
                  onChange={(e) => setSurfaceRaised(e.target.value)}
                  className="color-dot-picker"
                />
                <input
                  type="text"
                  value={surfaceRaised}
                  onChange={(e) => setSurfaceRaised(e.target.value)}
                  className="hex-code-input"
                />
              </div>
            </div>

            <div className="token-picker-item">
              <span className="token-picker-label">{str("Accent Brand Color")}</span>
              <div className="token-picker-ctrl">
                <input
                  type="color"
                  value={accent}
                  onChange={(e) => setAccent(e.target.value)}
                  className="color-dot-picker"
                />
                <input
                  type="text"
                  value={accent}
                  onChange={(e) => setAccent(e.target.value)}
                  className="hex-code-input"
                />
              </div>
            </div>

            <div className="token-picker-item">
              <span className="token-picker-label">{str("Primary Text Color")}</span>
              <div className="token-picker-ctrl">
                <input
                  type="color"
                  value={textPrimary}
                  onChange={(e) => setTextPrimary(e.target.value)}
                  className="color-dot-picker"
                />
                <input
                  type="text"
                  value={textPrimary}
                  onChange={(e) => setTextPrimary(e.target.value)}
                  className="hex-code-input"
                />
              </div>
            </div>
          </div>

          {/* Quick Palette Inspirations */}
          <div className="custom-theme-field">
            <label className="custom-theme-label">{str("Quick Palettes (Inspirations)")}</label>
            <div className="custom-theme-presets-row">
              {[
                { name: "Emerald Slate", bg: "#0d1514", surf: "#152422", acc: "#10b981", txt: "#e2f8f0" },
                { name: "Cyberpunk Cyan", bg: "#080b14", surf: "#121727", acc: "#06b6d4", txt: "#e0f2fe" },
                { name: "Amber Gold", bg: "#141109", surf: "#211c10", acc: "#f59e0b", txt: "#fef3c7" },
                { name: "Rose Velvet", bg: "#160b13", surf: "#261321", acc: "#f43f5e", txt: "#ffe4e6" },
                { name: "Royal Purple", bg: "#110b1a", surf: "#1d142b", acc: "#a855f7", txt: "#f3e8ff" },
              ].map((p) => (
                <button
                  key={p.name}
                  type="button"
                  className="palette-inspiration-pill"
                  onClick={() => handleApplyPresetColors(p.bg, p.surf, p.acc, p.txt)}
                  title={p.name}
                >
                  <span className="pill-dot" style={{ background: p.acc }} />
                  <span className="pill-name">{p.name}</span>
                </button>
              ))}
            </div>
          </div>
        </div>

        <div className="custom-theme-dialog__footer">
          <button type="button" className="btn" onClick={onClose}>
            {t("Cancel")}
          </button>
          <button type="button" className="btn btn--primary" onClick={handleSave}>
            <Icon name="check" size={13} strokeWidth={2.5} /> {str("Save & Apply Theme")}
          </button>
        </div>
      </div>
    </div>
  );
}

function SpeedQueues({ s }: { s: Settings }) {
  const queues = useApp((st) => st.queues);
  return (
    <>
      <Group title={t("Global speed limiter")}>
        <Toggle title={t("Limit total download speed")} hint={t("Quick toggle: Ctrl+L or the toolbar")} value={s.downloads.speedLimiterEnabled} onChange={(v) => save({ downloads: { speedLimiterEnabled: v } })} />
        <Row title={t("Limit")}>
          <RateField value={s.downloads.speedLimit} onCommit={(v) => save({ downloads: { speedLimit: v } })} label={t("Speed limit")} width={150} />
        </Row>
      </Group>
      <Group title={t("Queues")}>
        {queues.map((q) => (
          <Row key={q.id} title={displayName(q.name)} hint={`${t("{count} at a time", { count: q.maxConcurrent })} · ${q.schedule.enabled ? scheduleSummary(q) : t("manual start")}${q.running ? ` · ${t("running")}` : ""}`}>
            <button type="button" className="btn btn--sm" onClick={() => useUi.getState().openSheet({ kind: "scheduler", queueId: q.id })}>
              {t("Edit…")}
            </button>
          </Row>
        ))}
      </Group>
    </>
  );
}

function Categories() {
  const cats = useApp((st) => st.categories);
  return (
    <>
      <p className="lead">{t("New downloads are sorted by extension. Built-in categories can be edited but not deleted.")}</p>
      <Group>
        {cats.map((c) => (
          <Row key={c.id} title={<span className="inline"><Icon name={categoryIcon(c)} size={16} /> {displayName(c.name)}</span>} hint={[c.extensions.slice(0, 10).join(" "), c.saveDir].filter(Boolean).join(" · ") || t("No extensions")}>
            <button type="button" className="btn btn--sm" onClick={() => useUi.getState().openDialog({ kind: "category", category: c })}>
              {t("Edit…")}
            </button>
          </Row>
        ))}
      </Group>
      <button type="button" className="btn" onClick={() => useUi.getState().openDialog({ kind: "category" })}>
        <Icon name="plus" size={15} /> {t("New category")}
      </button>
    </>
  );
}

export function useIntegration() {
  const [status, setStatus] = useState<IntegrationStatus | null>(null);
  const [dir, setDir] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    void api.integrationStatus().then(setStatus, () => undefined);
    void api.extensionDir().then(setDir, () => undefined);
  }, []);
  const repair = async () => {
    setBusy(true);
    const r = await attempt(api.repairIntegration(), t("Could not repair the integration"));
    setBusy(false);
    if (r) {
      setStatus(r);
      toast(t("Browser integration repaired"), { kind: "success" });
    }
  };
  return { status, dir, busy, repair };
}

export function IntegrationList({ status }: { status: IntegrationStatus | null }) {
  if (!status) return <p className="hint">{t("Checking…")}</p>;
  return (
    <div className="card">
      {status.browsers.map((b) => {
        const ok = b.registered && b.pointsToThisHost;
        return (
          <div className="srow" key={b.browser}>
            <Icon name="browser" size={18} className={b.installed ? "" : "text-tertiary"} />
            <div className="srow__txt">
              <div className="srow__t1">{b.name ?? b.browser}</div>
              <div className="srow__t2">{!b.installed ? t("Not installed") : ok ? t("Native messaging host registered") : b.registered ? t("Registered for another Prism copy") : t("Not registered")}</div>
            </div>
            {b.installed && (
              <span className={`badge ${ok ? "badge--success" : "badge--warning"}`}>
                <Icon name={ok ? "status-completed" : "warning"} size={12} strokeWidth={2} />
                {ok ? t("Ready") : t("Needs repair")}
              </span>
            )}
          </div>
        );
      })}
    </div>
  );
}

function BrowserIntegration() {
  const { status, busy, repair } = useIntegration();
  const live = useExtensionStatus();
  return (
    <>
      <p className="lead">{t("The Prism extension hands downloads and videos to this app through a native messaging host.")}</p>
      <div className={`banner ${status?.extensionConnected && !status.extensionOutdated ? "banner--ok" : "banner--warn"}`}>
        <Icon name={status?.extensionConnected && !status.extensionOutdated ? "status-completed" : "warning"} size={18} />
        <div>
          <b>
            {status?.extensionOutdated
              ? t("The browser runs an older extension ({version}, Prism is {appVersion})", { version: status.extensionVersion ?? "", appVersion: status.appVersion ?? "" })
              : status?.extensionConnected
                ? `${t("Extension connected")}${status.extensionVersion ? ` · ${status.extensionVersion}` : ""}`
                : t("Extension not seen recently")}
          </b>
          <div className="t-small">
            {status?.extensionOutdated
              ? status.extensionInstallType === "development"
                ? t("This one was added by hand and doesn't update by itself. Prism's extension is now in the Chrome Web Store: remove this one and add it from the store (steps below).")
                : t("Browsers don't reload an unpacked extension's files by themselves. Click ↻ Reload next to Prism on the extensions page, or restart the browser; if it stays old, remove it and load it again from the folder below.")
              : status?.lastSeen
                ? t("Last message {date}", { date: formatDate(status.lastSeen) })
                : t("Install the extension, then open any web page.")}
          </div>
        </div>
        <span className="spacer" />
        {needsRepair(status) && (
          <button type="button" className="btn btn--primary" disabled={busy} onClick={repair}>
            <Icon name="refresh-url" size={15} /> {busy ? t("Repairing…") : t("Repair")}
          </button>
        )}
      </div>
      <div className="group">
        <div className="group__t">{t("Install the extension")}</div>
        <BrowserConnect status={live.status ?? status} connected={live.connected} hideOwnStatus />
      </div>
      <div className="group">
        <div className="group__t">{t("Browsers")}</div>
        <IntegrationList status={status} />
      </div>
    </>
  );
}

/** Settings › AI models: the Hugging Face server, access token and folder (#29). */
function Models({ s }: { s: Settings }) {
  const hf = s.huggingFace;
  const [ctx, setCtx] = useState<HfContext | null>(null);
  const [custom, setCustom] = useState(() => !!hf.endpoint && hf.endpoint !== MIRROR);
  useEffect(() => {
    void api.hfContext().then(setCtx, () => undefined);
  }, [hf.endpoint, hf.token, hf.folder, s.downloads.defaultDir]);
  const server = custom ? "custom" : hf.endpoint === MIRROR ? "mirror" : "";
  const fromEnv = !hf.endpoint && ctx && ctx.endpoint !== DEFAULT_HUB;
  return (
    <>
      <Group title={t("Hugging Face")}>
        <Row title={t("Download a model")} hint={t("Find a model by its name, a link or a search. Prism picks the files worth downloading and they download one after another, each over many connections.")}>
          <button type="button" className="btn" onClick={() => void attempt(api.openWindow("model"))}>
            <Icon name="model" size={15} /> {t("Open…")}
          </button>
        </Row>
        <Row title={t("Server")} hint={fromEnv ? t("From HF_ENDPOINT: {address}", { address: ctx.endpoint }) : t("hf-mirror.com is a copy of Hugging Face for places where it is blocked.")}>
          <Select
            label={t("Server")}
            value={server}
            onChange={(v) => {
              setCustom(v === "custom");
              if (v !== "custom") save({ huggingFace: { endpoint: v === "mirror" ? MIRROR : "" } });
            }}
            options={[
              { value: "", label: "huggingface.co" },
              { value: "mirror", label: "hf-mirror.com" },
              { value: "custom", label: t("Other…") },
            ]}
            width={200}
          />
        </Row>
        {custom && (
          <Row title={t("Server address")}>
            <TextField value={hf.endpoint} onCommit={(v) => save({ huggingFace: { endpoint: v.trim() } })} label={t("Server address")} placeholder="https://" mono />
          </Row>
        )}
        <Row
          title={t("Access token")}
          hint={
            <>
              {ctx?.token === "login"
                ? t("Using the token the Hugging Face tools saved (hf auth login).")
                : ctx?.token === "environment"
                  ? t("Using the token from HF_TOKEN.")
                  : t("For gated and private models. Sent only to the server above, never stored with the downloads.")}{" "}
              <button type="button" className="linkbtn" onClick={() => void openUrl("https://huggingface.co/settings/tokens")}>
                {t("Create a token")}
              </button>
            </>
          }
        >
          <TextField value={hf.token} onCommit={(v) => save({ huggingFace: { token: v.trim() } })} type="password" label={t("Access token")} placeholder="hf_…" mono />
        </Row>
        <Row title={t("Models folder")} hint={ctx?.folder}>
          {hf.folder && (
            <button type="button" className="btn btn--ghost" onClick={() => save({ huggingFace: { folder: "" } })}>
              {t("Reset")}
            </button>
          )}
          <button type="button" className="btn" onClick={() => void pickFolder(ctx?.folder ?? hf.folder).then((p) => p && save({ huggingFace: { folder: p } }))}>
            <Icon name="folder-open" size={15} /> {t("Change…")}
          </button>
        </Row>
      </Group>
    </>
  );
}

function Video({ s }: { s: Settings }) {
  const [tools, setTools] = useState<ToolStatus | null>(null);
  const [busy, setBusy] = useState<"" | "ensure" | "update">("");
  useEffect(() => {
    void api.toolStatus().then(setTools, () => undefined);
  }, []);
  const run = async (kind: "ensure" | "update") => {
    setBusy(kind);
    const r = kind === "ensure" ? await attempt(api.ensureTools(), t("Could not install the tools")) : await attempt(api.updateYtdlp(), t("Could not update yt-dlp"));
    setBusy("");
    if (r) {
      toast(kind === "ensure" ? t("Video tools ready") : t("yt-dlp is up to date"), { kind: "success" });
      void api.toolStatus().then(setTools);
    }
  };
  const tool = (key: keyof ToolStatus, label: string) => {
    const info = tools?.[key];
    return (
      <Row key={key} title={label} hint={info ? `${info.version ?? t("unknown version")} · ${info.source}${info.path ? ` · ${info.path}` : ""}` : t("Checking…")}>
        <span className={`badge ${info && info.source !== "missing" ? "badge--success" : "badge--warning"}`}>{info && info.source !== "missing" ? (info.source === "managed" ? t("Managed") : info.source === "system" ? t("System") : t("Custom")) : t("Missing")}</span>
      </Row>
    );
  };
  return (
    <>
      <Group title={t("Tools")}>
        {tool("ytDlp", "yt-dlp")}
        {tool("ffmpeg", "ffmpeg")}
        {tool("deno", t("Deno (YouTube signatures)"))}
        <Row title={t("Keep yt-dlp updated automatically")}>
          <Switch checked={s.video.autoUpdate} label={t("Auto-update yt-dlp")} onChange={(v) => save({ video: { autoUpdate: v } })} />
        </Row>
        <div className="srow srow--actions">
          <button type="button" className="btn" disabled={!!busy} onClick={() => run("ensure")}>
            {busy === "ensure" ? t("Installing…") : t("Install / repair tools")}
          </button>
          <button type="button" className="btn" disabled={!!busy} onClick={() => run("update")}>
            {busy === "update" ? t("Updating…") : t("Update yt-dlp now")}
          </button>
        </div>
      </Group>
      <Group title={t("Defaults")}>
        <Row title={t("Preferred quality")}>
          <Select
            label={t("Preferred quality")}
            value={s.video.preferredHeight}
            onChange={(v) => save({ video: { preferredHeight: v } })}
            options={[2160, 1440, 1080, 720, 480].map((h) => ({ value: h, label: `${h}p` }))}
            width={140}
          />
        </Row>
        <Row title={t("Container")}>
          <Segmented size="sm" label={t("Container")} value={s.video.container} onChange={(v) => save({ video: { container: v } })} options={["mp4", "mkv", "webm"].map((c) => ({ value: c, label: c.toUpperCase() }))} />
        </Row>
        <Row title={t("Audio format")}>
          <Segmented size="sm" label={t("Audio format")} value={s.video.audioFormat} onChange={(v) => save({ video: { audioFormat: v } })} options={["mp3", "m4a", "opus", "flac"].map((c) => ({ value: c, label: c.toUpperCase() }))} />
        </Row>
        <Toggle title={t("Embed subtitles")} value={s.video.embedSubtitles} onChange={(v) => save({ video: { embedSubtitles: v } })} />
        <Row title={t("Subtitle languages")} hint={t("e.g. en.*, cs, all")}>
          <TextField value={s.video.subtitleLangs} onCommit={(v) => save({ video: { subtitleLangs: v } })} label={t("Subtitle languages")} mono />
        </Row>
        <Toggle title={t("Embed thumbnail")} value={s.video.embedThumbnail} onChange={(v) => save({ video: { embedThumbnail: v } })} />
        <Toggle title={t("Embed metadata")} value={s.video.embedMetadata} onChange={(v) => save({ video: { embedMetadata: v } })} />
        <Toggle title={t("Embed chapters")} value={s.video.embedChapters} onChange={(v) => save({ video: { embedChapters: v } })} />
        <Row title="SponsorBlock">
          <Segmented
            size="sm"
            label="SponsorBlock"
            value={s.video.sponsorblock}
            onChange={(v) => save({ video: { sponsorblock: v } })}
            options={[
              { value: "off", label: t("Off") },
              { value: "mark", label: t("Mark chapters") },
              { value: "remove", label: t("Remove") },
            ]}
          />
        </Row>
        <Toggle title={t("Accelerate with Prism’s engine")} hint={t("Multi-connection download of direct formats, then merge with ffmpeg")} value={s.video.accelerate} onChange={(v) => save({ video: { accelerate: v } })} />
        <Row title={t("Cookies from browser")} hint={t("Lets yt-dlp access age-restricted / member videos. On Windows, Chrome and Edge lock their cookies while open: Firefox works best")}>
          <Select
            label={t("Cookies from browser")}
            value={s.video.cookiesFromBrowser}
            onChange={(v) => save({ video: { cookiesFromBrowser: v } })}
            options={["", "chrome", "edge", "firefox", "brave", "opera", "vivaldi"].map((b) => ({ value: b, label: b ? b[0].toUpperCase() + b.slice(1) : t("None") }))}
            width={140}
          />
        </Row>
        <Row title={t("File name template")}>
          <TextField value={s.video.fileTemplate} onCommit={(v) => save({ video: { fileTemplate: v } })} label={t("File name template")} mono />
        </Row>
        <Toggle title={t("Number playlist videos")} hint={t("Starts each file name with the video's position in the playlist: 01 - Title")} value={s.video.numberPlaylistVideos} onChange={(v) => save({ video: { numberPlaylistVideos: v } })} />
      </Group>
      <Group title={t("Custom tool paths")}>
        <Row title="yt-dlp" hint={t("Empty = managed by Prism")}>
          <TextField value={s.video.ytdlpPath} onCommit={(v) => save({ video: { ytdlpPath: v } })} label={t("yt-dlp path")} mono placeholder={t("Managed")} />
        </Row>
        <Row title="ffmpeg" hint={t("Empty = managed / system")}>
          <TextField value={s.video.ffmpegPath} onCommit={(v) => save({ video: { ffmpegPath: v } })} label={t("ffmpeg path")} mono placeholder={t("Managed")} />
        </Row>
      </Group>
    </>
  );
}

function SchedulerShortcut() {
  const queues = useApp((st) => st.queues);
  const [next, setNext] = useState<[string, string | null][]>([]);
  useEffect(() => {
    void api.nextScheduleRuns().then(setNext, () => undefined);
  }, []);
  return (
    <>
      <p className="lead">{t("Queues start, stop and run an action on their own schedule.")}</p>
      <Group>
        {queues.map((q) => (
          <Row key={q.id} title={displayName(q.name)} hint={q.schedule.enabled ? `${scheduleSummary(q)}${next.find(([id]) => id === q.id)?.[1] ? ` · ${t("next {time}", { time: next.find(([id]) => id === q.id)?.[1] ?? "" })}` : ""}` : t("Not scheduled")}>
            <span className={`badge ${q.running ? "badge--accent" : q.schedule.enabled ? "badge--info" : ""}`}>{q.running ? t("Running") : q.schedule.enabled ? t("Scheduled") : t("Manual")}</span>
          </Row>
        ))}
      </Group>
      <button type="button" className="btn btn--primary" onClick={() => useUi.getState().openSheet({ kind: "scheduler" })}>
        <Icon name="scheduler" size={15} /> {t("Open scheduler")}
      </button>
    </>
  );
}

function Advanced({ s }: { s: Settings }) {
  const dataDir = useApp((st) => st.dataDir);
  const date = new Date().toISOString().slice(0, 10);
  const json = [{ name: "JSON", extensions: ["json"] }];
  const exp = async (kind: "list" | "settings" | "diagnostics") => {
    const name = kind === "list" ? `prism-downloads-${date}.json` : kind === "settings" ? `prism-settings-${date}.json` : `prism-diagnostics-${date}.zip`;
    const path = await pickSavePath(name, kind === "diagnostics" ? [{ name: "Zip", extensions: ["zip"] }] : json);
    if (!path) return;
    const job = kind === "list" ? api.exportList(path) : kind === "settings" ? api.exportSettings(path) : api.exportDiagnostics(path);
    if (await attempt(job.then(() => true), t("Export failed"))) toast(t("Exported"), { kind: "success", body: path });
  };
  const imp = async (kind: "list" | "settings") => {
    const path = await pickFile(json);
    if (!path) return;
    if (kind === "list") {
      const n = await attempt(api.importList(path), t("Import failed"));
      if (n !== undefined) toast(t("{count} downloads imported", { count: n }), { kind: "success" });
    } else {
      const st = await attempt(api.importSettings(path), t("Import failed"));
      if (st) {
        useApp.getState().applySettings(st);
        toast(t("Settings imported"), { kind: "success" });
      }
    }
  };
  return (
    <>
      <Group title={t("Backup")}>
        <Row title={t("Download list")} hint={t("All downloads with their state, as JSON")}>
          <div className="inline">
            <button type="button" className="btn btn--sm" onClick={() => exp("list")}>
              <Icon name="export" size={14} /> {t("Export…")}
            </button>
            <button type="button" className="btn btn--sm" onClick={() => imp("list")}>
              <Icon name="import" size={14} /> {t("Import…")}
            </button>
          </div>
        </Row>
        <Row title={t("Settings")}>
          <div className="inline">
            <button type="button" className="btn btn--sm" onClick={() => exp("settings")}>
              <Icon name="export" size={14} /> {t("Export…")}
            </button>
            <button type="button" className="btn btn--sm" onClick={() => imp("settings")}>
              <Icon name="import" size={14} /> {t("Import…")}
            </button>
          </div>
        </Row>
      </Group>
      <Group title={t("Site grabber defaults")}>
        <Row title={t("Link depth")}>
          <NumberField value={s.grabber.defaultDepth} min={0} max={10} label={t("Default depth")} onCommit={(v) => save({ grabber: { defaultDepth: v } })} width={90} />
        </Row>
        <Row title={t("Maximum pages")}>
          <NumberField value={s.grabber.maxPages} min={1} max={100000} label={t("Maximum pages")} onCommit={(v) => save({ grabber: { maxPages: v } })} width={110} />
        </Row>
        <Row title={t("Parallel requests")}>
          <NumberField value={s.grabber.concurrentRequests} min={1} max={32} label={t("Parallel requests")} onCommit={(v) => save({ grabber: { concurrentRequests: v } })} width={90} />
        </Row>
        <Toggle title={t("Respect robots.txt")} value={s.grabber.respectRobots} onChange={(v) => save({ grabber: { respectRobots: v } })} />
      </Group>
      <Group title={t("Troubleshooting")}>
        <Row title={t("Data folder")} hint={dataDir}>
          <button type="button" className="btn btn--sm" onClick={() => void attempt(api.openPath(dataDir))}>
            <Icon name="folder-open" size={14} /> {t("Open")}
          </button>
        </Row>
        <Row title={t("Diagnostics")} hint={t("Logs, settings (passwords removed) and integration status in one zip")}>
          <button type="button" className="btn btn--sm" onClick={() => exp("diagnostics")}>
            <Icon name="export" size={14} /> {t("Export…")}
          </button>
        </Row>
        <Row title={t("Run the welcome guide again")}>
          <button type="button" className="btn btn--sm" onClick={() => save({ onboardingDone: false })}>
            {t("Start")}
          </button>
        </Row>
      </Group>
    </>
  );
}

export const SHORTCUTS: [string, string][] = [
  ["Ctrl N", tk("Add URL")],
  ["Ctrl K / Ctrl F", tk("Search")],
  ["Ctrl A", tk("Select all")],
  ["Space", tk("Pause / resume selected")],
  ["Enter", tk("Open finished file / details")],
  ["Delete", tk("Delete selected (Shift: with files)")],
  ["F2", tk("Properties")],
  ["Ctrl C", tk("Copy links of selected")],
  ["Ctrl L", tk("Toggle speed limiter")],
  ["Ctrl ,", tk("Settings")],
  ["Ctrl Shift M", tk("Mini window")],
  ["↑ ↓ Home End", tk("Move selection (Shift extends)")],
  ["Esc", tk("Close dialog / clear search")],
];

function Shortcuts() {
  return (
    <Group>
      {SHORTCUTS.map(([k, label]) => (
        <Row key={k} title={t(label)}>
          <span className="kbds">
            {k.split(" / ").map((combo) => (
              <span key={combo} className="kbd-combo">
                {combo.split(" ").map((p) => (
                  <span key={p} className="kbd">
                    {p}
                  </span>
                ))}
              </span>
            ))}
          </span>
        </Row>
      ))}
    </Group>
  );
}

function About() {
  const version = useApp((st) => st.version);
  return (
    <div className="about">
      <BrandTile size={72} />
      <h2>Prism</h2>
      <p className="text-secondary">{t("Version {version}", { version })}</p>
      <div className="inline" style={{ justifyContent: "center" }}>
        <button type="button" className="btn" onClick={() => void checkForUpdates(true)}>
          <Icon name="refresh-url" size={15} /> {t("Check for updates")}
        </button>
        <button type="button" className="btn btn--ghost" onClick={() => useUi.getState().openSheet({ kind: "whatsnew" })}>
          <Icon name="star" size={15} /> {t("What's new")}
        </button>
      </div>
      <p className="lead">{t("One beam, many rays. A fast multi-connection download manager with browser integration, video downloading, queues, a scheduler and a site grabber.")}</p>
      <button type="button" className="linkbtn about__star" onClick={() => void openUrl(REPO_URL)}>
        {t("Prism is open source. Star it on GitHub")}
      </button>
      <p className="t-small text-tertiary">{t("Inter and JetBrains Mono are used under the SIL Open Font License. yt-dlp, ffmpeg and librqbit are separate projects under their own licenses.")}</p>
    </div>
  );
}
