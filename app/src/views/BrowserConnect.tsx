import { AnimatePresence, motion } from "motion/react";
import { useEffect, useMemo, useState } from "react";
import { Icon, type IconName } from "../design/icons";
import { api, type BrowserIntegration, type IntegrationStatus } from "../lib/api";
import { t, tr } from "../lib/i18n";
import { useApp } from "../lib/store";
import { toast } from "../lib/ui";
import { copyText } from "../lib/win";

/** Browsers the guide offers, in display order (ids match `prism_native_host::Browser::id`). */
const ORDER = ["chrome", "edge", "brave", "opera", "vivaldi", "helium", "chromium", "firefox", "zen", "waterfox"] as const;
const SHORT: Record<string, string> = {
  chrome: "Chrome",
  edge: "Edge",
  brave: "Brave",
  opera: "Opera",
  vivaldi: "Vivaldi",
  helium: "Helium",
  chromium: "Chromium",
  firefox: "Firefox",
  zen: "Zen",
  waterfox: "Waterfox",
};
const PAGE: Record<string, string> = {
  chrome: "chrome://extensions",
  edge: "edge://extensions",
  brave: "brave://extensions",
  opera: "opera://extensions",
  vivaldi: "vivaldi://extensions",
  helium: "chrome://extensions",
  chromium: "chrome://extensions",
  firefox: "about:debugging#/runtime/this-firefox",
  zen: "about:debugging#/runtime/this-firefox",
  waterfox: "about:debugging#/runtime/this-firefox",
};
/** Firefox-based browsers: Firefox's extension, from Firefox Add-ons. */
const FIREFOX_FAMILY = ["firefox", "zen", "waterfox"];

/** Prism's extension in the Chrome Web Store: Chrome, Edge, Brave, Opera, Vivaldi, Helium and Chromium install it from there. */
export const EXTENSION_STORE_URL = "https://chromewebstore.google.com/detail/prism-download-manager/mmcdeiglkbfbicdjboocmhehodogilfl";
/** Prism's add-on in Firefox Add-ons: Firefox and Zen install it from there. */
export const FIREFOX_ADDON_URL = "https://addons.mozilla.org/firefox/addon/prism-download-manager/";
/** The store's install button as each browser labels it (Chrome's label elsewhere). */
const ADD_LABEL: Record<string, string> = { brave: "Add to Brave", opera: "Add to Opera" };

const OS: "windows" | "mac" | "linux" = /Mac/i.test(navigator.userAgent) ? "mac" : /Linux/i.test(navigator.userAgent) ? "linux" : "windows";

/** Polls the integration status while mounted; `connected` turns true when the extension says hello. */
export function useExtensionStatus(intervalMs = 2000) {
  const [status, setStatus] = useState<IntegrationStatus | null>(null);
  useEffect(() => {
    let alive = true;
    const tick = () =>
      void api.integrationStatus().then(
        (s) => alive && setStatus(s),
        () => undefined,
      );
    tick();
    const timer = window.setInterval(tick, intervalMs);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [intervalMs]);
  return { status, connected: !!status?.extensionConnected };
}

function pickDefault(browsers: BrowserIntegration[]): string {
  const installed = ORDER.filter((id) => browsers.some((b) => b.browser === id && b.installed && b.supported !== false));
  return installed[0] ?? "chrome";
}

/**
 * Step-by-step guide that gets the browser extension installed: pick your browser, open Prism in
 * the Chrome Web Store (Firefox and Zen: Firefox Add-ons) and add it — with a live check that
 * turns green the moment the extension connects. Adding it by hand stays one click away.
 *
 * Button and setting names of the browser itself ("Add to Chrome", "Developer mode", …) stay in
 * English inside the translated sentences: the browser shows them in its own language.
 */
export function BrowserConnect({ status, connected, hideOwnStatus }: { status: IntegrationStatus | null; connected: boolean; hideOwnStatus?: boolean }) {
  const browsers = useMemo(() => {
    const list = status?.browsers ?? [];
    const shown = ORDER.filter((id) => list.some((b) => b.browser === id && b.installed && b.supported !== false));
    return shown.length ? shown : (["chrome", "edge", "firefox"] as string[]);
  }, [status]);
  const [browser, setBrowser] = useState<string | null>(null);
  const current = browser ?? (status ? pickDefault(status.browsers) : "chrome");
  const firefox = FIREFOX_FAMILY.includes(current);
  // "Prefer to add it by hand?" (below the store guide): the unpacked extension Prism ships.
  const [byHand, setByHand] = useState(false);
  const [folder, setFolder] = useState<string | null>(null);
  useEffect(() => {
    if (byHand) void api.extensionFolder(firefox).then(setFolder, () => setFolder(null));
  }, [firefox, byHand]);
  // A Prism extension someone loaded by hand earlier ("Load unpacked"): the store version
  // updates by itself, but the browser won't add it while the old one is still installed.
  const manual = !firefox && !byHand && status?.extensionInstallType === "development";
  // Store steps start after the "remove the copy added by hand" step when that one shows.
  const step = manual ? 1 : 0;

  const name = SHORT[current] ?? current;
  const paste = OS === "mac" ? "⌘ V" : "Ctrl V";
  const openPage = async () => {
    const copied = await copyText(PAGE[current]);
    let opened = true;
    try {
      await api.openBrowser(current);
    } catch {
      opened = false;
    }
    toast(copied ? t("Address copied") : t("Type {address}", { address: PAGE[current] }), {
      kind: copied ? "success" : undefined,
      body: opened
        ? t("click the address bar, press {keys} and Enter.", { keys: paste })
        : t("Open {browser}, then click the address bar, press {keys} and Enter.", { browser: name, keys: paste }),
      ms: 6000,
    });
  };
  const openStore = async () => {
    try {
      await api.openExtensionStore(current);
    } catch {
      toast(t("Couldn't open the browser"), { body: t("Copy the store link and open it in your browser."), ms: 6000 });
    }
  };
  const copyStoreLink = async () => {
    if (await copyText(firefox ? FIREFOX_ADDON_URL : EXTENSION_STORE_URL)) toast(t("Store link copied"), { kind: "success", ms: 1800 });
  };
  const addLabel = firefox ? "Add to Firefox" : (ADD_LABEL[current] ?? "Add to Chrome");
  const copyFolder = async () => {
    if (folder && (await copyText(folder))) toast(t("Folder path copied"), { kind: "success", ms: 1800 });
  };
  const pasteHint =
    OS === "mac"
      ? tr("In the window that opens press {keys}, paste the path and press {enter}.", {
          keys: (
            <>
              <span className="kbd">⌘</span> <span className="kbd">⇧</span> <span className="kbd">G</span>
            </>
          ),
          enter: <span className="kbd">Return</span>,
        })
      : OS === "linux"
        ? tr("In the window that opens press {keys}, paste the path and press {enter}.", {
            keys: (
              <>
                <span className="kbd">Ctrl</span> <span className="kbd">L</span>
              </>
            ),
            enter: <span className="kbd">Enter</span>,
          })
        : // "Select Folder" is the system dialog's own button, in the system's language.
          tr("Paste the path into the folder field of the window that opens and press {button}.", { button: <b>Select Folder</b> });

  return (
    <div className="bconnect">
      <div className="bconnect__pick" role="radiogroup" aria-label={t("Your browser")}>
        {browsers.map((id) => (
          <button key={id} type="button" role="radio" aria-checked={id === current} className={`bchip${id === current ? " on" : ""}`} onClick={() => setBrowser(id)}>
            <Icon name="browser" size={14} /> {SHORT[id] ?? id}
          </button>
        ))}
      </div>

      <ol className="bsteps">
        {manual && (
          <li className="bstep">
            <span className="bstep__n">1</span>
            <div className="bstep__txt">
              <div className="bstep__t">{t("Remove the Prism you added by hand")}</div>
              <div className="bstep__d">
                {tr("The store version updates by itself. On {address} click {button} under Prism. The button copies the address and opens {browser}.", {
                  address: <code className="selectable">{PAGE[current]}</code>,
                  button: <b>Remove</b>,
                  browser: name,
                })}
              </div>
            </div>
            <button type="button" className="btn btn--sm" onClick={openPage}>
              <Icon name="copy" size={14} /> {t("Copy & open {browser}", { browser: name })}
            </button>
          </li>
        )}
        <li className="bstep">
          <span className="bstep__n">{step + 1}</span>
          <div className="bstep__txt">
            <div className="bstep__t">{firefox ? t("Open Prism in Firefox Add-ons") : t("Open Prism in the Chrome Web Store")}</div>
            <div className="bstep__d">
              {current === "edge"
                ? t("Edge asks once to allow extensions from other stores: click {button} at the top of the page.", { button: "Allow" })
                : current === "opera"
                  ? t("Opera first offers its free “{addon}” add-on: add it, then come back to the page.", { addon: "Install Chrome Extensions" })
                  : t("The store page opens in {browser}.", { browser: name })}{" "}
              <button type="button" className="linkbtn" onClick={copyStoreLink}>
                {t("Copy link")}
              </button>
            </div>
          </div>
          <button type="button" className="btn btn--primary" onClick={openStore}>
            <Icon name="browser" size={15} /> {t("Open in {browser}", { browser: name })}
          </button>
        </li>
        <li className="bstep">
          <span className="bstep__n">{step + 2}</span>
          <div className="bstep__txt">
            <div className="bstep__t">{t("Click “{button}”", { button: addLabel })}</div>
            <div className="bstep__d">{t("The blue button at the top right of the store page. It's free.")}</div>
          </div>
          <FakeButton label={addLabel} />
        </li>
        <li className="bstep">
          <span className="bstep__n">{step + 3}</span>
          <div className="bstep__txt">
            <div className="bstep__t">{t("Confirm with “{button}”", { button: firefox ? "Add" : "Add extension" })}</div>
            <div className="bstep__d">{t("{browser} asks once whether to add Prism. From then on it updates Prism by itself.", { browser: name })}</div>
          </div>
          <FakeButton label={firefox ? "Add" : "Add extension"} />
        </li>
        {!firefox && (
          <li className="bstep">
            <span className="bstep__n">{step + 4}</span>
            <div className="bstep__txt">
              <div className="bstep__t">{t("Pin Prism next to the address bar")}</div>
              <div className="bstep__d">{t("Click the puzzle icon, then the pin next to Prism. The check below turns green as soon as it's connected.")}</div>
            </div>
            <FakeButton label={t("Pin")} icon="pin" />
          </li>
        )}
      </ol>

      <AnimatePresence mode="wait" initial={false}>
        {hideOwnStatus && connected && !manual ? null : connected && !manual ? (
          <motion.div
            key="ok"
            className="bstatus bstatus--ok"
            role="status"
            initial={{ opacity: 0, scale: 0.97 }}
            animate={{ opacity: 1, scale: 1 }}
            transition={{ type: "spring", stiffness: 420, damping: 26 }}
          >
            <span className="bstatus__icon">
              <Icon name="check" size={16} strokeWidth={2.4} />
            </span>
            <div>
              <b>{t("Connected!")}</b> {t("Downloads and videos from your browser now open in Prism.")}
              <div className="bstatus__sub">{t("Tip: pin the Prism icon next to the address bar (puzzle icon → pin).")}</div>
            </div>
          </motion.div>
        ) : (
          <motion.div key="wait" className="bstatus" role="status" initial={{ opacity: 0 }} animate={{ opacity: 1 }}>
            <span className="bstatus__spin" aria-hidden="true" />
            <div>
              {manual ? t("Waiting for the store version…") : t("Waiting for the extension…")} <span className="text-tertiary">{t("this turns green as soon as it's installed.")}</span>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      <div className="bconnect__manual">
        <button type="button" className="disclose" aria-expanded={byHand} onClick={() => setByHand(!byHand)}>
          <Icon name={byHand ? "chevron-down" : "chevron-right"} size={12} strokeWidth={2.2} /> {t("Prefer to add it by hand?")}
        </button>
        {byHand && firefox && (
          <>
            <p className="t-small text-tertiary">{t("{browser} removes a temporary add-on when it restarts; the one from Firefox Add-ons stays and updates itself.", { browser: name })}</p>
            <ol className="bsteps">
              <li className="bstep">
                <span className="bstep__n">1</span>
                <div className="bstep__txt">
                  <div className="bstep__t">{t("Open the add-ons debugging page")}</div>
                  <div className="bstep__d">
                    {tr("The button copies {address} and opens {browser} — click its address bar, press {keys} and {enter}.", {
                      address: <code className="selectable">{PAGE[current]}</code>,
                      browser: name,
                      keys: paste.split(" ").map((k, i) => (
                        <span key={k}>
                          {i > 0 && " "}
                          <span className="kbd">{k}</span>
                        </span>
                      )),
                      enter: <span className="kbd">Enter</span>,
                    })}
                  </div>
                </div>
                <button type="button" className="btn btn--sm" onClick={openPage}>
                  <Icon name="copy" size={14} /> {t("Copy & open {browser}", { browser: name })}
                </button>
              </li>
              <li className="bstep">
                <span className="bstep__n">2</span>
                <div className="bstep__txt">
                  <div className="bstep__t">{t("Click “{button}”", { button: "Load Temporary Add-on…" })}</div>
                  <div className="bstep__d">{t("It's in the “{section}” section of that page.", { section: `This ${name}` })}</div>
                </div>
                <FakeButton label="Load Temporary Add-on…" />
              </li>
              <li className="bstep bstep--wide">
                <span className="bstep__n">3</span>
                <div className="bstep__txt">
                  <div className="bstep__t">{t("Pick manifest.json in the Prism folder")}</div>
                  <div className="bstep__d">{pasteHint}</div>
                </div>
                <div className="howto__path bstep__full">
                  <span className="t-mono ellipsis selectable" title={folder ?? undefined}>
                    {folder ?? "…"}
                  </span>
                  <button type="button" className="btn btn--sm" disabled={!folder} onClick={copyFolder}>
                    <Icon name="copy" size={14} /> {t("Copy path")}
                  </button>
                  <button type="button" className="btn btn--sm btn--ghost" disabled={!folder} onClick={() => folder && void api.openPath(folder)}>
                    <Icon name="folder-open" size={14} /> {t("Show")}
                  </button>
                </div>
              </li>
            </ol>
          </>
        )}
        {byHand && !firefox && (
          <>
            <p className="t-small text-tertiary">
              {t("Without the store: load the folder Prism keeps up to date. A copy added this way doesn't update by itself — after a Prism update, click ↻ next to Prism on the extensions page.")}
            </p>
            <ol className="bsteps">
              <li className="bstep">
                <span className="bstep__n">1</span>
                <div className="bstep__txt">
                  <div className="bstep__t">{t("Open the extensions page")}</div>
                  <div className="bstep__d">
                    {tr("The button copies {address} and opens {browser}.", { address: <code className="selectable">{PAGE[current]}</code>, browser: name })}
                  </div>
                </div>
                <button type="button" className="btn btn--sm" onClick={openPage}>
                  <Icon name="copy" size={14} /> {t("Copy & open {browser}", { browser: name })}
                </button>
              </li>
              <li className="bstep">
                <span className="bstep__n">2</span>
                <div className="bstep__txt">
                  <div className="bstep__t">{t("Turn on {toggle}", { toggle: "Developer mode" })}</div>
                  <div className="bstep__d">{current === "edge" ? t("The switch is in the left sidebar of the page.") : t("The switch is in the top-right corner of the page.")}</div>
                </div>
                <FakeToggle />
              </li>
              <li className="bstep bstep--wide">
                <span className="bstep__n">3</span>
                <div className="bstep__txt">
                  <div className="bstep__t">{t("Click “{button}” and choose the Prism folder", { button: "Load unpacked" })}</div>
                  <div className="bstep__d">{pasteHint}</div>
                </div>
                <FakeButton label="Load unpacked" />
                <div className="howto__path bstep__full">
                  <span className="t-mono ellipsis selectable" title={folder ?? undefined}>
                    {folder ?? "…"}
                  </span>
                  <button type="button" className="btn btn--sm" disabled={!folder} onClick={copyFolder}>
                    <Icon name="copy" size={14} /> {t("Copy path")}
                  </button>
                  <button type="button" className="btn btn--sm btn--ghost" disabled={!folder} onClick={() => folder && void api.openPath(folder)}>
                    <Icon name="folder-open" size={14} /> {t("Show")}
                  </button>
                </div>
              </li>
            </ol>
          </>
        )}
      </div>
    </div>
  );
}

/** Miniature of the browser's Developer-mode switch, so people know what to look for. */
function FakeToggle() {
  return (
    <span className="bmini" aria-hidden="true">
      <span className="bmini__label">Developer mode</span>
      <span className="bmini__toggle" />
    </span>
  );
}

function FakeButton({ label, icon }: { label: string; icon?: IconName }) {
  return (
    <span className="bmini" aria-hidden="true">
      <span className="bmini__btn">
        {icon && <Icon name={icon} size={12} />} {label}
      </span>
    </span>
  );
}

const DISMISS_KEY = "prism.connectBanner.dismissed";
const OUTDATED_DISMISS_KEY = "prism.extensionOutdatedDismissed";

/**
 * Slim reminder above the download list until the extension has ever connected (for people who
 * skipped the onboarding step). Dismissable; the sidebar keeps a "Browser not connected" entry.
 */
export function ConnectBanner({ onOpen }: { onOpen: () => void }) {
  const { status } = useExtensionStatus(15_000);
  const [dismissed, setDismissed] = useState(() => {
    try {
      return localStorage.getItem(DISMISS_KEY) === "1";
    } catch {
      return false;
    }
  });
  // An extension older than the app: after an update the browser keeps running the old files
  // of an unpacked extension until it is reloaded. Hidden per version pair once dismissed.
  const outdatedKey = status?.extensionOutdated ? `${status.extensionVersion}>${status.appVersion}` : "";
  const [outdatedDismissed, setOutdatedDismissed] = useState<string | null>(() => {
    try {
      return localStorage.getItem(OUTDATED_DISMISS_KEY);
    } catch {
      return null;
    }
  });
  const onboarded = useApp((st) => st.settings?.onboardingDone ?? true);
  const outdated = !!outdatedKey && outdatedDismissed !== outdatedKey;
  const show = outdated || (onboarded && !!status && !status.lastSeen && !dismissed);
  const dismiss = () => {
    if (outdated) {
      setOutdatedDismissed(outdatedKey);
      try {
        localStorage.setItem(OUTDATED_DISMISS_KEY, outdatedKey);
      } catch {
        /* private storage unavailable: hide for this session only */
      }
      return;
    }
    setDismissed(true);
    try {
      localStorage.setItem(DISMISS_KEY, "1");
    } catch {
      /* private storage unavailable: hide for this session only */
    }
  };
  // Always rendered: the slot is a fixed row of the main grid (empty when hidden).
  return (
    <div className="cbanner-slot">
      <AnimatePresence initial={false}>
        {show && (
          <motion.div
            className="cbanner"
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.22, ease: [0.16, 1, 0.3, 1] }}
          >
            <div className="cbanner__inner">
              <span className="cbanner__icon">
                <Icon name="browser" size={16} />
              </span>
              {outdated ? (
                status?.extensionInstallType === "development" ? (
                  <span className="cbanner__txt">
                    <b>{t("Prism's extension is now in the Chrome Web Store.")}</b>{" "}
                    {t("Yours ({version}) was added by hand and won't update by itself: switch to the store version, it takes a minute.", { version: status?.extensionVersion ?? "" })}
                  </span>
                ) : (
                  <span className="cbanner__txt">
                    {tr("{older} ({version}, Prism is {appVersion}).", {
                      older: <b>{t("Your browser still runs an older Prism extension")}</b>,
                      version: status?.extensionVersion,
                      appVersion: status?.appVersion,
                    })}{" "}
                    {OS === "mac"
                      ? t("Reload it: on {page} click ↻ next to Prism, or restart the browser.", { page: "chrome://extensions" })
                      : t("Reload it: on {page} or {page2} click ↻ next to Prism, or restart the browser.", { page: "chrome://extensions", page2: "edge://extensions" })}
                  </span>
                )
              ) : (
                <span className="cbanner__txt">
                  <b>{t("Catch downloads straight from your browser.")}</b> {t("Add the Prism extension — it takes about a minute.")}
                </span>
              )}
              <button type="button" className="btn btn--primary btn--sm" onClick={onOpen}>
                {outdated ? (status?.extensionInstallType === "development" ? t("Switch") : t("Show how")) : t("Connect browser")}
              </button>
              <button type="button" className="tb-btn" aria-label={t("Hide this reminder")} data-tip={t("Hide")} onClick={dismiss}>
                <Icon name="close" size={14} />
              </button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
