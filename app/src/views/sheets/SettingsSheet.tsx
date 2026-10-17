import { checkForUpdates } from "../UpdateBanner";
import { BrowserConnect, useExtensionStatus } from "../BrowserConnect";
import { type ReactNode, useEffect, useState } from "react";
import { ListField, NumberField, RateField, Row, Segmented, Select, Slider, Switch, TextField } from "../../components/Controls";
import { BrandTile } from "../../components/LogoMark";
import { Icon, type IconName } from "../../design/icons";
import { type DeepPartial, patchSettings } from "../../lib/actions";
import { api, type IntegrationStatus, type ToolStatus } from "../../lib/api";
import { formatDate } from "../../lib/format";
import { categoryIcon } from "../../lib/present";
import { playSound, useApp } from "../../lib/store";
import type { Settings } from "../../lib/types";
import { attempt, toast, useUi } from "../../lib/ui";
import { pickFile, pickFolder, pickSavePath } from "../../lib/win";
import { scheduleSummary } from "./SchedulerSheet";

export const SECTIONS: { id: string; label: string; icon: IconName; group?: boolean }[] = [
  { id: "general", label: "General", icon: "settings" },
  { id: "appearance", label: "Appearance", icon: "sun" },
  { id: "downloads", label: "Downloads", icon: "download" },
  { id: "connection", label: "Connection", icon: "globe" },
  { id: "proxy", label: "Proxy", icon: "proxy" },
  { id: "speed", label: "Speed & queues", icon: "speed-limiter" },
  { id: "capture", label: "File types & capture", icon: "filter", group: true },
  { id: "categories", label: "Categories", icon: "all" },
  { id: "browser", label: "Browser integration", icon: "browser" },
  { id: "video", label: "Video & streams", icon: "video" },
  { id: "scheduler", label: "Scheduler", icon: "scheduler" },
  { id: "sounds", label: "Notifications & sounds", icon: "bell", group: true },
  { id: "security", label: "Security & antivirus", icon: "shield" },
  { id: "advanced", label: "Advanced", icon: "options" },
  { id: "shortcuts", label: "Keyboard shortcuts", icon: "list" },
  { id: "about", label: "About Prism", icon: "info" },
];

const save = (patch: DeepPartial<Settings>) => void patchSettings(patch);

export function SettingsSheet({ section }: { section?: string }) {
  const [cur, setCur] = useState(section ?? "general");
  const [q, setQ] = useState("");
  const s = useApp((st) => st.settings);
  if (!s) return null;
  const list = SECTIONS.filter((x) => !q || x.label.toLowerCase().includes(q.toLowerCase()));
  const meta = SECTIONS.find((x) => x.id === cur) ?? SECTIONS[0];
  return (
    <div className="settings">
      <aside className="snav">
        <label className="input">
          <Icon name="search" size={15} />
          <input placeholder="Search settings" aria-label="Search settings" value={q} onChange={(e) => setQ(e.target.value)} />
        </label>
        <nav aria-label="Settings sections">
          {list.map((x) => (
            <div key={x.id}>
              {x.group && !q && <div className="grp" />}
              <button type="button" className="side-item" aria-current={x.id === cur ? "true" : undefined} onClick={() => setCur(x.id)}>
                <Icon name={x.icon} size={16} />
                <span className="side-item__label">{x.label}</span>
              </button>
            </div>
          ))}
        </nav>
      </aside>
      <section className="scontent" aria-label={meta.label}>
        <div className="sbody" key={cur}>
          <h1>{meta.label}</h1>
          <SectionBody id={cur} s={s} />
        </div>
      </section>
    </div>
  );
}

function Group({ title, children }: { title?: string; children: ReactNode }) {
  return (
    <div className="group">
      {title && <div className="group__t">{title}</div>}
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
          <p className="lead">How Prism starts and behaves in the background.</p>
          <Group title="Startup">
            <Toggle
              title="Launch at startup"
              hint="Start with Windows, minimized to the tray"
              value={s.general.launchAtStartup}
              onChange={async (v) => {
                await attempt(api.setAutostart(v), "Could not change autostart");
                save({ general: { launchAtStartup: v } });
              }}
            />
            <Toggle title="Start minimized" value={s.general.startMinimized} onChange={(v) => save({ general: { startMinimized: v } })} />
            <Toggle title="Close button hides to the tray" hint="Downloads keep running; quit from the tray menu" value={s.general.closeToTray} onChange={(v) => save({ general: { closeToTray: v } })} />
          </Group>
          <Group title="Windows">
            <Toggle title="Show the Download File dialog" hint="Off = new downloads start immediately with default settings" value={s.general.showCaptureDialog} onChange={(v) => save({ general: { showCaptureDialog: v } })} />
            <Toggle title="Open a progress window for new downloads" value={s.general.showProgressWindow} onChange={(v) => save({ general: { showProgressWindow: v } })} />
            <Toggle title="Show the completion dialog" value={s.general.showCompletionDialog} onChange={(v) => save({ general: { showCompletionDialog: v } })} />
          </Group>
          <Group title="Deleting">
            <Toggle title="Confirm before deleting" value={s.general.confirmDelete} onChange={(v) => save({ general: { confirmDelete: v } })} />
            <Toggle title="Use the trash" hint="Deleted downloads can be restored until you empty the trash" value={s.general.useTrash} onChange={(v) => save({ general: { useTrash: v } })} />
            <Toggle title="Check for updates automatically" hint="Prism asks before installing; it restarts by itself and your downloads continue" value={s.general.checkUpdates} onChange={(v) => save({ general: { checkUpdates: v } })} />
          </Group>
          <Group title="Clipboard">
            <Toggle title="Watch the clipboard for links" value={s.clipboard.monitor} onChange={(v) => save({ clipboard: { monitor: v } })} />
            <Toggle title="Ask before adding copied links" value={s.clipboard.ask} onChange={(v) => save({ clipboard: { ask: v } })} />
          </Group>
        </>
      );
    case "appearance":
      return <Appearance s={s} />;
    case "downloads":
      return (
        <>
          <Group title="Where files go">
            <Row title="Default folder" hint={s.downloads.defaultDir}>
              <button type="button" className="btn" onClick={() => void pickFolder(s.downloads.defaultDir).then((p) => p && save({ downloads: { defaultDir: p } }))}>
                <Icon name="folder-open" size={15} /> Change…
              </button>
            </Row>
            <Toggle title="Sort into category folders" hint="Video → Downloads\Video, Programs → Downloads\Programs …" value={s.downloads.useCategoryFolders} onChange={(v) => save({ downloads: { useCategoryFolders: v } })} />
            <Row title="When the file already exists">
              <Select
                label="When the file already exists"
                value={s.downloads.conflict}
                onChange={(v) => save({ downloads: { conflict: v } })}
                options={[
                  { value: "ask", label: "Ask me" },
                  { value: "rename", label: "Rename the new file" },
                  { value: "overwrite", label: "Overwrite" },
                ]}
                width={200}
              />
            </Row>
          </Group>
          <Group title="Engine">
            <Row title="Connections per download" hint="Prism splits files into this many segments (work-stealing keeps all busy)">
              <NumberField value={s.downloads.defaultConnections} min={1} max={s.downloads.maxConnections} label="Connections per download" onCommit={(v) => save({ downloads: { defaultConnections: v } })} width={90} />
            </Row>
            <Row title="Maximum connections">
              <NumberField value={s.downloads.maxConnections} min={1} max={64} label="Maximum connections" onCommit={(v) => save({ downloads: { maxConnections: v } })} width={90} />
            </Row>
            <Row title="Retries before failing" hint="Exponential back-off up to 30 s between attempts">
              <NumberField value={s.downloads.maxRetries} min={0} max={999} label="Retries" onCommit={(v) => save({ downloads: { maxRetries: v } })} width={90} />
            </Row>
            <Toggle title="Start downloads automatically" value={s.downloads.autoStart} onChange={(v) => save({ downloads: { autoStart: v } })} />
            <Toggle title="Verify checksums when known" value={s.downloads.verifyChecksums} onChange={(v) => save({ downloads: { verifyChecksums: v } })} />
          </Group>
        </>
      );
    case "connection":
      return (
        <Group>
          <Row title="HTTP version" hint="HTTP/1.1 opens one TCP connection per segment — fastest for bulk transfers">
            <Select
              label="HTTP version"
              value={s.connection.httpVersion}
              onChange={(v) => save({ connection: { httpVersion: v } })}
              options={[
                { value: "http1", label: "HTTP/1.1" },
                { value: "auto", label: "Automatic (HTTP/2)" },
                { value: "http3", label: "HTTP/3 (experimental)" },
              ]}
              width={210}
            />
          </Row>
          <Row title="Connect timeout">
            <NumberField value={s.connection.connectTimeoutSecs} min={1} max={300} suffix="s" label="Connect timeout" onCommit={(v) => save({ connection: { connectTimeoutSecs: v } })} width={90} />
          </Row>
          <Row title="Read timeout">
            <NumberField value={s.connection.readTimeoutSecs} min={1} max={600} suffix="s" label="Read timeout" onCommit={(v) => save({ connection: { readTimeoutSecs: v } })} width={90} />
          </Row>
          <Row title="Connections per server" hint="0 = no limit; Prism also learns limits servers enforce">
            <NumberField value={s.connection.maxConnectionsPerHost} min={0} max={64} label="Connections per server" onCommit={(v) => save({ connection: { maxConnectionsPerHost: v } })} width={90} />
          </Row>
          <Row title="User agent" hint="Empty = a current Chrome user agent">
            <TextField value={s.connection.userAgent} onCommit={(v) => save({ connection: { userAgent: v } })} label="User agent" mono placeholder="Default" />
          </Row>
        </Group>
      );
    case "proxy":
      return (
        <Group>
          <Row title="Proxy">
            <Segmented
              size="sm"
              label="Proxy mode"
              value={s.proxy.mode}
              onChange={(v) => save({ proxy: { mode: v } })}
              options={[
                { value: "none", label: "None" },
                { value: "system", label: "System" },
                { value: "http", label: "HTTP(S)" },
                { value: "socks5", label: "SOCKS5" },
              ]}
            />
          </Row>
          {(s.proxy.mode === "http" || s.proxy.mode === "socks5") && (
            <>
              <Row title="Address">
                <TextField value={s.proxy.address} onCommit={(v) => save({ proxy: { address: v } })} label="Proxy address" mono placeholder="127.0.0.1:8080" />
              </Row>
              <Row title="User name">
                <TextField value={s.proxy.username} onCommit={(v) => save({ proxy: { username: v } })} label="Proxy user name" />
              </Row>
              <Row title="Password">
                <TextField value={s.proxy.password} onCommit={(v) => save({ proxy: { password: v } })} label="Proxy password" type="password" />
              </Row>
              <Row title="Bypass for" hint="Hosts that connect directly" align="start">
                <ListField value={s.proxy.bypass} onCommit={(v) => save({ proxy: { bypass: v } })} label="Proxy bypass" placeholder="localhost, *.lan" rows={2} />
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
          <Group title="Browser capture">
            <Toggle title="Take over downloads from the browser" value={s.capture.enabled} onChange={(v) => save({ capture: { enabled: v } })} />
            <Row title="Skip with key" hint="Hold this key while clicking to let the browser download instead">
              <Segmented
                size="sm"
                label="Bypass key"
                value={s.capture.bypassKey}
                onChange={(v) => save({ capture: { bypassKey: v } })}
                options={[
                  { value: "alt", label: "Alt" },
                  { value: "ctrl", label: "Ctrl" },
                  { value: "shift", label: "Shift" },
                  { value: "none", label: "None" },
                ]}
              />
            </Row>
            <Row title="Ignore files smaller than">
              <NumberField value={s.capture.minSizeKb} min={0} max={10_000_000} suffix="KB" label="Minimum size" onCommit={(v) => save({ capture: { minSizeKb: v } })} width={120} />
            </Row>
          </Group>
          <Group title="What to capture">
            <Row title="File extensions" align="start">
              <ListField value={s.capture.extensions} onCommit={(v) => save({ capture: { extensions: v } })} label="Captured extensions" rows={3} />
            </Row>
            <Row title="MIME types" align="start">
              <ListField value={s.capture.mimeTypes} onCommit={(v) => save({ capture: { mimeTypes: v } })} label="Captured MIME types" rows={2} />
            </Row>
            <Row title="Never capture from" hint="Wildcards allowed" align="start">
              <ListField value={s.capture.excludedSites} onCommit={(v) => save({ capture: { excludedSites: v } })} label="Excluded sites" rows={2} placeholder="*.example.com" />
            </Row>
          </Group>
          <Group title="Video button">
            <Toggle title="Show “Download video” on web pages" value={s.capture.videoButton} onChange={(v) => save({ capture: { videoButton: v } })} />
            <Row title="Only for videos longer than">
              <NumberField value={s.capture.videoMinDurationSecs} min={0} max={36000} suffix="s" label="Minimum video duration" onCommit={(v) => save({ capture: { videoMinDurationSecs: v } })} width={100} />
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
    case "scheduler":
      return <SchedulerShortcut />;
    case "sounds":
      return (
        <>
          <Group title="Notifications">
            <Toggle title="System notifications" hint="When downloads finish or fail while Prism is in the background" value={s.general.notifications} onChange={(v) => save({ general: { notifications: v } })} />
          </Group>
          <Group title="Sounds">
            <Toggle title="Play sounds" value={s.sounds.enabled} onChange={(v) => save({ sounds: { enabled: v } })} />
            <Toggle title="Download complete" value={s.sounds.onComplete} onChange={(v) => save({ sounds: { onComplete: v } })} />
            <Toggle title="Download failed" value={s.sounds.onFailed} onChange={(v) => save({ sounds: { onFailed: v } })} />
            <Toggle title="Queue finished" value={s.sounds.onQueueDone} onChange={(v) => save({ sounds: { onQueueDone: v } })} />
            <Row title="Volume">
              <div className="inline">
                <Slider value={s.sounds.volume} label="Volume" onChange={(v) => save({ sounds: { volume: v } })} />
                <button type="button" className="btn btn--sm" onClick={() => playSound("complete")}>
                  <Icon name="sound" size={14} /> Test
                </button>
              </div>
            </Row>
          </Group>
        </>
      );
    case "security":
      return (
        <Group title="Scan finished downloads">
          <Toggle title="Scan with an antivirus" value={s.antivirus.enabled} onChange={(v) => save({ antivirus: { enabled: v } })} />
          <Row title="Scanner">
            <Select
              label="Scanner"
              value={s.antivirus.kind}
              onChange={(v) => save({ antivirus: { kind: v } })}
              options={[
                { value: "auto", label: "Automatic" },
                { value: "defender", label: "Microsoft Defender" },
                { value: "clamav", label: "ClamAV" },
                { value: "custom", label: "Custom command" },
              ]}
              width={200}
            />
          </Row>
          {s.antivirus.kind === "custom" && (
            <>
              <Row title="Program">
                <div className="withbtn">
                  <TextField value={s.antivirus.command} onCommit={(v) => save({ antivirus: { command: v } })} label="Scanner program" mono />
                  <button type="button" className="btn" onClick={() => void pickFile([{ name: "Programs", extensions: ["exe", "cmd", "bat", "sh"] }]).then((p) => p && save({ antivirus: { command: p } }))}>
                    Browse…
                  </button>
                </div>
              </Row>
              <Row title="Arguments" hint="[FILE] is replaced by the downloaded file path">
                <TextField value={s.antivirus.args} onCommit={(v) => save({ antivirus: { args: v } })} label="Scanner arguments" mono placeholder='"[FILE]"' />
              </Row>
            </>
          )}
          <Row title="Only these extensions" hint="Empty = scan every file" align="start">
            <ListField value={s.antivirus.extensions} onCommit={(v) => save({ antivirus: { extensions: v } })} label="Scanned extensions" rows={2} placeholder="exe, msi, zip" />
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
  ["cyan", "Cyan", "#36c5e6", "#0b7fa0"],
  ["emerald", "Emerald", "#2fc489", "#087f53"],
  ["amber", "Amber", "#f2b54a", "#a86200"],
  ["rose", "Rose", "#d03a64", "#cf3760"],
  ["graphite", "Graphite", "#e6e8ee", "#1f2229"],
] as const;

function Appearance({ s }: { s: Settings }) {
  const light = document.documentElement.dataset.theme === "light";
  const themes = [
    ["light", "Light", "pv-light"],
    ["dark", "Dark", "pv-dark"],
    ["system", "Match system", "pv-sys"],
  ] as const;
  return (
    <>
      <p className="lead">Changes apply instantly. Prism follows your system theme unless you pick one.</p>
      <Group title="Theme">
        <div className="themes">
          {themes.map(([v, label, cls]) => (
            <button key={v} type="button" className={`tcard${s.appearance.theme === v ? " on" : ""}`} aria-pressed={s.appearance.theme === v} onClick={() => save({ appearance: { theme: v } })}>
              <span className={`tprev ${cls}`}>
                <span className="sb">
                  <i />
                  <i />
                  <i />
                </span>
                <span className="sh">
                  <i className="a" />
                  <i />
                  <i />
                  <i />
                </span>
              </span>
              {label}
            </button>
          ))}
        </div>
      </Group>
      <Group title="Accent colour">
        <div className="accents">
          {ACCENTS.map(([id, label, dark, lightC]) => {
            const on = (s.appearance.accent || "prism") === id;
            return (
              <button key={id} type="button" className={`sw${on ? " on" : ""}`} aria-pressed={on} style={{ ["--c" as string]: light ? lightC : dark }} onClick={() => save({ appearance: { accent: id } })}>
                <b>{on && <Icon name="check" size={16} strokeWidth={2.5} />}</b>
                {label}
              </button>
            );
          })}
          <p className="accents__note">Accent tints selection, focus rings, progress and the window’s ambient light. Status colours never change.</p>
        </div>
      </Group>
      <div className="twocol">
        <Group title="Materials & motion">
          <Toggle title="Liquid glass" hint="Frosted, translucent sidebar & toolbars" value={!s.appearance.reduceTransparency} onChange={(v) => save({ appearance: { reduceTransparency: !v } })} />
          <Toggle title="Window material" hint="Mica / vibrancy behind the window (restart window)" value={s.appearance.windowMaterial} onChange={(v) => save({ appearance: { windowMaterial: v } })} />
          <Row title="Motion">
            <Segmented
              size="sm"
              label="Motion"
              value={s.appearance.motion}
              onChange={(v) => save({ appearance: { motion: v } })}
              options={[
                { value: "system", label: "System" },
                { value: "full", label: "Full" },
                { value: "reduced", label: "Reduced" },
              ]}
            />
          </Row>
        </Group>
        <Group title="Layout">
          <Row title="Row density" hint="Compact fits ~45% more rows">
            <Segmented
              size="sm"
              label="Row density"
              value={s.appearance.density}
              onChange={(v) => save({ appearance: { density: v } })}
              options={[
                { value: "comfortable", label: "Comfortable" },
                { value: "compact", label: "Compact" },
              ]}
            />
          </Row>
          <Row title="Details pane" hint="Connections, speed and info under the list">
            <Switch checked={useUi.getState().bottomOpen} label="Details pane" onChange={(v) => useUi.getState().setBottom({ bottomOpen: v })} />
          </Row>
        </Group>
      </div>
    </>
  );
}

function SpeedQueues({ s }: { s: Settings }) {
  const queues = useApp((st) => st.queues);
  return (
    <>
      <Group title="Global speed limiter">
        <Toggle title="Limit total download speed" hint="Quick toggle: Ctrl+L or the toolbar" value={s.downloads.speedLimiterEnabled} onChange={(v) => save({ downloads: { speedLimiterEnabled: v } })} />
        <Row title="Limit">
          <RateField value={s.downloads.speedLimit} onCommit={(v) => save({ downloads: { speedLimit: v } })} label="Speed limit" width={150} />
        </Row>
      </Group>
      <Group title="Queues">
        {queues.map((q) => (
          <Row key={q.id} title={q.name} hint={`${q.maxConcurrent} at a time · ${q.schedule.enabled ? scheduleSummary(q) : "manual start"}${q.running ? " · running" : ""}`}>
            <button type="button" className="btn btn--sm" onClick={() => useUi.getState().openSheet({ kind: "scheduler", queueId: q.id })}>
              Edit…
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
      <p className="lead">New downloads are sorted by extension. Built-in categories can be edited but not deleted.</p>
      <Group>
        {cats.map((c) => (
          <Row key={c.id} title={<span className="inline"><Icon name={categoryIcon(c)} size={16} /> {c.name}</span>} hint={[c.extensions.slice(0, 10).join(" "), c.saveDir].filter(Boolean).join(" · ") || "No extensions"}>
            <button type="button" className="btn btn--sm" onClick={() => useUi.getState().openDialog({ kind: "category", category: c })}>
              Edit…
            </button>
          </Row>
        ))}
      </Group>
      <button type="button" className="btn" onClick={() => useUi.getState().openDialog({ kind: "category" })}>
        <Icon name="plus" size={15} /> New category
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
    const r = await attempt(api.repairIntegration(), "Could not repair the integration");
    setBusy(false);
    if (r) {
      setStatus(r);
      toast("Browser integration repaired", { kind: "success" });
    }
  };
  return { status, dir, busy, repair };
}

export function IntegrationList({ status }: { status: IntegrationStatus | null }) {
  if (!status) return <p className="hint">Checking…</p>;
  return (
    <div className="card">
      {status.browsers.map((b) => {
        const ok = b.registered && b.pointsToThisHost;
        return (
          <div className="srow" key={b.browser}>
            <Icon name="browser" size={18} className={b.installed ? "" : "text-tertiary"} />
            <div className="srow__txt">
              <div className="srow__t1">{b.name ?? b.browser}</div>
              <div className="srow__t2">{!b.installed ? "Not installed" : ok ? "Native messaging host registered" : b.registered ? "Registered for another Prism copy" : "Not registered"}</div>
            </div>
            {b.installed && (
              <span className={`badge ${ok ? "badge--success" : "badge--warning"}`}>
                <Icon name={ok ? "status-completed" : "warning"} size={12} strokeWidth={2} />
                {ok ? "Ready" : "Needs repair"}
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
      <p className="lead">The Prism extension hands downloads and videos to this app through a native messaging host.</p>
      <div className={`banner ${status?.extensionConnected && !status.extensionOutdated ? "banner--ok" : "banner--warn"}`}>
        <Icon name={status?.extensionConnected && !status.extensionOutdated ? "status-completed" : "warning"} size={18} />
        <div>
          <b>
            {status?.extensionOutdated
              ? `The browser runs an older extension (${status.extensionVersion}, Prism is ${status.appVersion})`
              : status?.extensionConnected
                ? `Extension connected${status.extensionVersion ? ` · ${status.extensionVersion}` : ""}`
                : "Extension not seen recently"}
          </b>
          <div className="t-small">
            {status?.extensionOutdated
              ? status.extensionInstallType === "development"
                ? "Browsers don't reload an unpacked extension's files by themselves. Click ↻ Reload next to Prism on the extensions page; if it stays old, remove it and load it again from the folder below."
                : "Browsers don't reload an unpacked extension's files by themselves. Click ↻ Reload next to Prism on the extensions page, or restart the browser; if it stays old, remove it and load it again from the folder below."
              : status?.lastSeen
                ? `Last message ${formatDate(status.lastSeen)}`
                : "Install the extension, then open any web page."}
          </div>
        </div>
        <span className="spacer" />
        {(!status?.extensionConnected || status.extensionOutdated) && (
          <button type="button" className="btn btn--primary" disabled={busy} onClick={repair}>
            <Icon name="refresh-url" size={15} /> {busy ? "Repairing…" : "Repair"}
          </button>
        )}
      </div>
      <div className="group">
        <div className="group__t">Install the extension</div>
        <BrowserConnect status={live.status ?? status} connected={live.connected} />
      </div>
      <div className="group">
        <div className="group__t">Browsers</div>
        <IntegrationList status={status} />
      </div>
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
    const r = kind === "ensure" ? await attempt(api.ensureTools(), "Could not install the tools") : await attempt(api.updateYtdlp(), "Could not update yt-dlp");
    setBusy("");
    if (r) {
      toast(kind === "ensure" ? "Video tools ready" : "yt-dlp is up to date", { kind: "success" });
      void api.toolStatus().then(setTools);
    }
  };
  const tool = (key: keyof ToolStatus, label: string) => {
    const t = tools?.[key];
    return (
      <Row key={key} title={label} hint={t ? `${t.version ?? "unknown version"} · ${t.source}${t.path ? ` · ${t.path}` : ""}` : "Checking…"}>
        <span className={`badge ${t && t.source !== "missing" ? "badge--success" : "badge--warning"}`}>{t && t.source !== "missing" ? (t.source === "managed" ? "Managed" : t.source === "system" ? "System" : "Custom") : "Missing"}</span>
      </Row>
    );
  };
  return (
    <>
      <Group title="Tools">
        {tool("ytDlp", "yt-dlp")}
        {tool("ffmpeg", "ffmpeg")}
        {tool("deno", "Deno (YouTube signatures)")}
        <Row title="Keep yt-dlp updated automatically">
          <Switch checked={s.video.autoUpdate} label="Auto-update yt-dlp" onChange={(v) => save({ video: { autoUpdate: v } })} />
        </Row>
        <div className="srow srow--actions">
          <button type="button" className="btn" disabled={!!busy} onClick={() => run("ensure")}>
            {busy === "ensure" ? "Installing…" : "Install / repair tools"}
          </button>
          <button type="button" className="btn" disabled={!!busy} onClick={() => run("update")}>
            {busy === "update" ? "Updating…" : "Update yt-dlp now"}
          </button>
        </div>
      </Group>
      <Group title="Defaults">
        <Row title="Preferred quality">
          <Select
            label="Preferred quality"
            value={s.video.preferredHeight}
            onChange={(v) => save({ video: { preferredHeight: v } })}
            options={[2160, 1440, 1080, 720, 480].map((h) => ({ value: h, label: `${h}p` }))}
            width={140}
          />
        </Row>
        <Row title="Container">
          <Segmented size="sm" label="Container" value={s.video.container} onChange={(v) => save({ video: { container: v } })} options={["mp4", "mkv", "webm"].map((c) => ({ value: c, label: c.toUpperCase() }))} />
        </Row>
        <Row title="Audio format">
          <Segmented size="sm" label="Audio format" value={s.video.audioFormat} onChange={(v) => save({ video: { audioFormat: v } })} options={["mp3", "m4a", "opus", "flac"].map((c) => ({ value: c, label: c.toUpperCase() }))} />
        </Row>
        <Toggle title="Embed subtitles" value={s.video.embedSubtitles} onChange={(v) => save({ video: { embedSubtitles: v } })} />
        <Row title="Subtitle languages" hint="e.g. en.*, cs, all">
          <TextField value={s.video.subtitleLangs} onCommit={(v) => save({ video: { subtitleLangs: v } })} label="Subtitle languages" mono />
        </Row>
        <Toggle title="Embed thumbnail" value={s.video.embedThumbnail} onChange={(v) => save({ video: { embedThumbnail: v } })} />
        <Toggle title="Embed metadata" value={s.video.embedMetadata} onChange={(v) => save({ video: { embedMetadata: v } })} />
        <Toggle title="Embed chapters" value={s.video.embedChapters} onChange={(v) => save({ video: { embedChapters: v } })} />
        <Row title="SponsorBlock">
          <Segmented
            size="sm"
            label="SponsorBlock"
            value={s.video.sponsorblock}
            onChange={(v) => save({ video: { sponsorblock: v } })}
            options={[
              { value: "off", label: "Off" },
              { value: "mark", label: "Mark chapters" },
              { value: "remove", label: "Remove" },
            ]}
          />
        </Row>
        <Toggle title="Accelerate with Prism’s engine" hint="Multi-connection download of direct formats, then merge with ffmpeg" value={s.video.accelerate} onChange={(v) => save({ video: { accelerate: v } })} />
        <Toggle title="Fast YouTube format lookup" hint="Reads formats directly from your signed-in YouTube session; falls back to yt-dlp for anything it can’t handle" value={s.video.useNativeYoutube} onChange={(v) => save({ video: { useNativeYoutube: v } })} />
        <Row title="Cookies from browser" hint="Lets yt-dlp access age-restricted / member videos. On Windows, Chrome and Edge lock their cookies while open: Firefox works best">
          <Select
            label="Cookies from browser"
            value={s.video.cookiesFromBrowser}
            onChange={(v) => save({ video: { cookiesFromBrowser: v } })}
            options={["", "chrome", "edge", "firefox", "brave", "opera", "vivaldi"].map((b) => ({ value: b, label: b ? b[0].toUpperCase() + b.slice(1) : "None" }))}
            width={140}
          />
        </Row>
        <Row title="File name template">
          <TextField value={s.video.fileTemplate} onCommit={(v) => save({ video: { fileTemplate: v } })} label="File name template" mono />
        </Row>
      </Group>
      <Group title="Custom tool paths">
        <Row title="yt-dlp" hint="Empty = managed by Prism">
          <TextField value={s.video.ytdlpPath} onCommit={(v) => save({ video: { ytdlpPath: v } })} label="yt-dlp path" mono placeholder="Managed" />
        </Row>
        <Row title="ffmpeg" hint="Empty = managed / system">
          <TextField value={s.video.ffmpegPath} onCommit={(v) => save({ video: { ffmpegPath: v } })} label="ffmpeg path" mono placeholder="Managed" />
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
      <p className="lead">Queues start, stop and run an action on their own schedule.</p>
      <Group>
        {queues.map((q) => (
          <Row key={q.id} title={q.name} hint={q.schedule.enabled ? `${scheduleSummary(q)}${next.find(([id]) => id === q.id)?.[1] ? ` · next ${next.find(([id]) => id === q.id)?.[1]}` : ""}` : "Not scheduled"}>
            <span className={`badge ${q.running ? "badge--accent" : q.schedule.enabled ? "badge--info" : ""}`}>{q.running ? "Running" : q.schedule.enabled ? "Scheduled" : "Manual"}</span>
          </Row>
        ))}
      </Group>
      <button type="button" className="btn btn--primary" onClick={() => useUi.getState().openSheet({ kind: "scheduler" })}>
        <Icon name="scheduler" size={15} /> Open scheduler
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
    if (await attempt(job.then(() => true), "Export failed")) toast("Exported", { kind: "success", body: path });
  };
  const imp = async (kind: "list" | "settings") => {
    const path = await pickFile(json);
    if (!path) return;
    if (kind === "list") {
      const n = await attempt(api.importList(path), "Import failed");
      if (n !== undefined) toast(`${n} downloads imported`, { kind: "success" });
    } else {
      const st = await attempt(api.importSettings(path), "Import failed");
      if (st) {
        useApp.getState().applySettings(st);
        toast("Settings imported", { kind: "success" });
      }
    }
  };
  return (
    <>
      <Group title="Backup">
        <Row title="Download list" hint="All downloads with their state, as JSON">
          <div className="inline">
            <button type="button" className="btn btn--sm" onClick={() => exp("list")}>
              <Icon name="export" size={14} /> Export…
            </button>
            <button type="button" className="btn btn--sm" onClick={() => imp("list")}>
              <Icon name="import" size={14} /> Import…
            </button>
          </div>
        </Row>
        <Row title="Settings">
          <div className="inline">
            <button type="button" className="btn btn--sm" onClick={() => exp("settings")}>
              <Icon name="export" size={14} /> Export…
            </button>
            <button type="button" className="btn btn--sm" onClick={() => imp("settings")}>
              <Icon name="import" size={14} /> Import…
            </button>
          </div>
        </Row>
      </Group>
      <Group title="Site grabber defaults">
        <Row title="Link depth">
          <NumberField value={s.grabber.defaultDepth} min={0} max={10} label="Default depth" onCommit={(v) => save({ grabber: { defaultDepth: v } })} width={90} />
        </Row>
        <Row title="Maximum pages">
          <NumberField value={s.grabber.maxPages} min={1} max={100000} label="Maximum pages" onCommit={(v) => save({ grabber: { maxPages: v } })} width={110} />
        </Row>
        <Row title="Parallel requests">
          <NumberField value={s.grabber.concurrentRequests} min={1} max={32} label="Parallel requests" onCommit={(v) => save({ grabber: { concurrentRequests: v } })} width={90} />
        </Row>
        <Toggle title="Respect robots.txt" value={s.grabber.respectRobots} onChange={(v) => save({ grabber: { respectRobots: v } })} />
      </Group>
      <Group title="Troubleshooting">
        <Row title="Data folder" hint={dataDir}>
          <button type="button" className="btn btn--sm" onClick={() => void attempt(api.openPath(dataDir))}>
            <Icon name="folder-open" size={14} /> Open
          </button>
        </Row>
        <Row title="Diagnostics" hint="Logs, settings (passwords removed) and integration status in one zip">
          <button type="button" className="btn btn--sm" onClick={() => exp("diagnostics")}>
            <Icon name="export" size={14} /> Export…
          </button>
        </Row>
        <Row title="Run the welcome guide again">
          <button type="button" className="btn btn--sm" onClick={() => save({ onboardingDone: false })}>
            Start
          </button>
        </Row>
      </Group>
    </>
  );
}

export const SHORTCUTS: [string, string][] = [
  ["Ctrl N", "Add URL"],
  ["Ctrl K / Ctrl F", "Search"],
  ["Ctrl A", "Select all"],
  ["Space", "Pause / resume selected"],
  ["Enter", "Open finished file / details"],
  ["Delete", "Delete selected (Shift: with files)"],
  ["F2", "Properties"],
  ["Ctrl C", "Copy links of selected"],
  ["Ctrl L", "Toggle speed limiter"],
  ["Ctrl ,", "Settings"],
  ["Ctrl Shift M", "Mini window"],
  ["↑ ↓ Home End", "Move selection (Shift extends)"],
  ["Esc", "Close dialog / clear search"],
];

function Shortcuts() {
  return (
    <Group>
      {SHORTCUTS.map(([k, label]) => (
        <Row key={k} title={label}>
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
      <p className="text-secondary">Version {version}</p>
      <div className="inline" style={{ justifyContent: "center" }}>
        <button type="button" className="btn" onClick={() => void checkForUpdates(true)}>
          <Icon name="refresh-url" size={15} /> Check for updates
        </button>
      </div>
      <p className="lead">One beam, many rays. A fast multi-connection download manager with browser integration, video downloading, queues, a scheduler and a site grabber.</p>
      <p className="t-small text-tertiary">Inter and JetBrains Mono are used under the SIL Open Font License. yt-dlp, ffmpeg and librqbit are separate projects under their own licenses.</p>
    </div>
  );
}
