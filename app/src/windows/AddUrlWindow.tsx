import { useEffect, useState } from "react";
import { TextField } from "../components/Controls";
import { LogoMark } from "../components/LogoMark";
import { WindowFrame } from "../components/WindowFrame";
import { Icon } from "../design/icons";
import { api, webMode } from "../lib/api";
import { t } from "../lib/i18n";
import { isTorrentLink, looksLikeUrl } from "../lib/present";
import { hexEncode } from "../lib/route";
import { readClipboardText, win } from "../lib/win";

export default function AddUrlWindow() {
  const [url, setUrl] = useState("");
  const [auth, setAuth] = useState(false);
  const [user, setUser] = useState("");
  const [pass, setPass] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void readClipboardText().then((text) => {
      const v = text.trim();
      if (looksLikeUrl(v)) setUrl((cur) => cur || v);
    });
  }, []);

  // A Hugging Face model's name ("Qwen/Qwen3-8B") opens the model window.
  const model = /^(datasets\/)?[A-Za-z0-9][\w-]*\/[\w.-]+$/.test(url.trim());
  const valid = model || looksLikeUrl(url) || /^ftps?:\/\//i.test(url.trim());
  const torrent = isTorrentLink(url);

  const submit = async () => {
    if (!valid || busy) return;
    setBusy(true);
    setError("");
    try {
      if (model) await api.openWindow("model", hexEncode(url.trim()));
      else if (torrent) await api.openWindow("torrent", hexEncode(url.trim()));
      else await api.addUrl({ url: url.trim(), username: auth ? user : undefined, password: auth ? pass : undefined });
      // In web mode the core event has already routed this tab to the next dialog; closing
      // here would send it back to the main view.
      if (!webMode) await win.close();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <WindowFrame kind="addurl" title={t("Add URL")} icon={<LogoMark size={18} glass="indigo" />} className="addurl">
      <div className="wbody">
        <div className="field">
          <span className="field__label">{t("Address")}</span>
          <TextField
            value={url}
            onChange={(v) => {
              setUrl(v);
              setError("");
            }}
            label={t("Download address")}
            icon={torrent ? "magnet" : "link"}
            mono
            autoFocus
            placeholder={t("https://… , ftp://… or magnet:?…")}
            invalid={!!error}
            onEnter={submit}
          />
          <span className={`field__hint${error ? " text-danger" : ""}`}>
            {error || (torrent ? t("Torrent — you will choose the files next.") : t("Prism checks the address and shows the Download File dialog."))}
          </span>
        </div>
        <button type="button" className="disclose" aria-expanded={auth} onClick={() => setAuth(!auth)}>
          <Icon name={auth ? "chevron-down" : "chevron-right"} size={12} strokeWidth={2.2} /> {t("Use a user name and password")}
        </button>
        {auth && (
          <div className="row2">
            <TextField value={user} onChange={setUser} label={t("User name")} placeholder={t("User name")} icon="user" />
            <TextField value={pass} onChange={setPass} label={t("Password")} placeholder={t("Password")} type="password" icon="key" onEnter={submit} />
          </div>
        )}
      </div>
      <footer className="dialog__footer">
        <span className="spacer" />
        <button type="button" className="btn btn--ghost" onClick={() => void win.close()}>
          {t("Cancel")}
        </button>
        <button type="button" className="btn btn--primary" disabled={!valid || busy} onClick={submit}>
          {busy ? t("Checking…") : t("Continue")}
        </button>
      </footer>
    </WindowFrame>
  );
}
