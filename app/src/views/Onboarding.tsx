import { AnimatePresence, motion } from "motion/react";
import { useState } from "react";
import { BrandTile } from "../components/LogoMark";
import { Modal } from "../components/Modal";
import { Icon } from "../design/icons";
import { patchSettings } from "../lib/actions";
import { t, tr } from "../lib/i18n";
import { openUrl, REPO_URL } from "../lib/links";
import { useApp } from "../lib/store";
import { pickFolder } from "../lib/win";
import { BrowserConnect, useExtensionStatus } from "./BrowserConnect";

const STEPS = ["Welcome", "Browser", "Preferences", "Done"];

/** First-run guide (settings.onboardingDone === false). */
export function Onboarding() {
  const done = useApp((s) => s.settings?.onboardingDone ?? true);
  const [step, setStep] = useState(0);
  const finish = () => void patchSettings({ onboardingDone: true });
  return (
    <Modal open={!done} onClose={finish} width={720} height={660} variant="sheet" className="onboard">
      <div className="onboard__steps" aria-hidden="true">
        {STEPS.map((s, i) => (
          <span key={s} className={i <= step ? "on" : undefined} />
        ))}
      </div>
      <div className="onboard__body">
        <AnimatePresence mode="wait" initial={false}>
          <motion.div key={step} className="onboard__page" initial={{ opacity: 0, x: 16 }} animate={{ opacity: 1, x: 0 }} exit={{ opacity: 0, x: -16 }} transition={{ duration: 0.22, ease: [0.16, 1, 0.3, 1] }}>
            {step === 0 && <Welcome />}
            {step === 1 && <Browser />}
            {step === 2 && <Preferences />}
            {step === 3 && <Ready />}
          </motion.div>
        </AnimatePresence>
      </div>
      <footer className="dialog__footer">
        {step > 0 && step < 3 && (
          <button type="button" className="btn btn--ghost" onClick={() => setStep(step - 1)}>
            {t("Back")}
          </button>
        )}
        <span className="spacer" />
        {step < 3 && (
          <button type="button" className="btn btn--ghost" onClick={finish}>
            {t("Skip")}
          </button>
        )}
        {step < 3 ? (
          <button type="button" className="btn btn--primary" onClick={() => setStep(step + 1)} data-autofocus>
            {step === 0 ? t("Get started") : t("Continue")}
          </button>
        ) : (
          <button type="button" className="btn btn--primary" onClick={finish} data-autofocus>
            {t("Start using Prism")}
          </button>
        )}
      </footer>
    </Modal>
  );
}

function Welcome() {
  return (
    <div className="onboard__hero">
      <BrandTile size={84} />
      <h1>
        {t("One beam.")} <span className="prism-text">{t("Many rays.")}</span>
      </h1>
      <p>{t("Prism splits every download into parallel segments, resumes where it stopped and grabs videos from the pages you visit. Three quick steps and you are ready.")}</p>
    </div>
  );
}

function Browser() {
  const { status, connected } = useExtensionStatus();
  return (
    <>
      <h2>{t("Connect your browser")}</h2>
      <p className="text-secondary">
        {t("Add the Prism extension to your browser once — then clicking a download link, or the “{button}” button on a page, sends it straight to Prism. It takes about a minute.", {
          button: "Download video",
        })}
      </p>
      <BrowserConnect status={status} connected={connected} />
    </>
  );
}

function Preferences() {
  const s = useApp((st) => st.settings);
  if (!s) return null;
  return (
    <>
      <h2>{t("Make it yours")}</h2>
      <div className="card">
        <div className="srow">
          <Icon name="folder" size={18} />
          <div className="srow__txt">
            <div className="srow__t1">{t("Download folder")}</div>
            <div className="srow__t2 t-mono">{s.downloads.defaultDir}</div>
          </div>
          <button type="button" className="btn btn--sm" onClick={async () => {
              const p = await pickFolder(s.downloads.defaultDir);
              if (p) await patchSettings({ downloads: { defaultDir: p } });
            }}>
            {t("Change…")}
          </button>
        </div>
      </div>
      <div className="themes themes--onboard">
        {(
          [
            ["dark", t("Dark"), "pv-dark"],
            ["dimmed", t("Dark Dimmed"), "pv-dimmed"],
            ["light", t("Light"), "pv-light"],
            ["system", t("Match system"), "pv-sys"],
          ] as const
        ).map(([v, label, cls]) => (
          <button key={v} type="button" className={`tcard${s.appearance.theme === v ? " on" : ""}`} aria-pressed={s.appearance.theme === v} onClick={() => void patchSettings({ appearance: { theme: v } })}>
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
    </>
  );
}

function Ready() {
  return (
    <div className="onboard__hero">
      <span className="burst">
        <span>
          <Icon name="check" size={30} strokeWidth={2} />
        </span>
      </span>
      <h1>{t("You’re all set")}</h1>
      <p>
        {tr("Press {keys} to add a link, drop links anywhere on the window, or just download from your browser.", {
          keys: (
            <>
              <span className="kbd">Ctrl</span> <span className="kbd">N</span>
            </>
          ),
        })}
      </p>
      <StarCard />
    </div>
  );
}

/** A friendly, skippable ask: Prism is free, and stars on GitHub help people find it. */
function StarCard() {
  const [thanks, setThanks] = useState(false);
  return (
    <div className="onboard__star card">
      <span className="onboard__staricon" aria-hidden="true">
        <Icon name="star" size={18} />
      </span>
      <div className="onboard__startxt">
        <b>{thanks ? t("Thank you!") : t("Like Prism?")}</b>
        <span className="t-small text-secondary">{t("It’s free and open source. A star on GitHub helps more people find it.")}</span>
      </div>
      <button
        type="button"
        className="btn btn--sm"
        onClick={() => {
          setThanks(true);
          void openUrl(REPO_URL);
        }}
      >
        <Icon name="star" size={14} /> {t("Star on GitHub")}
      </button>
    </div>
  );
}
