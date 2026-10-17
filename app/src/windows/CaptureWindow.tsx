import { useEffect, useRef, useState } from "react";
import { Checkbox, Segmented, Select, TextArea, TextField } from "../components/Controls";
import { LogoMark } from "../components/LogoMark";
import { openMenuAt } from "../components/MenuHost";
import { FileTile } from "../components/Visuals";
import { WindowFrame } from "../components/WindowFrame";
import { Icon } from "../design/icons";
import { api, onCoreEvent } from "../lib/api";
import { formatBytes, hostOf } from "../lib/format";
import { folderFor } from "../lib/paths";
import { categoryIcon } from "../lib/present";
import { useApp } from "../lib/store";
import type { CaptureInfo, StartMode } from "../lib/types";
import { attempt } from "../lib/ui";
import { copyText, pickFolder, win } from "../lib/win";

export default function CaptureWindow({ id }: { id: string }) {
  const [cap, setCap] = useState<CaptureInfo | null>(null);
  const [missing, setMissing] = useState(false);
  const [name, setName] = useState("");
  const [dir, setDir] = useState("");
  const [category, setCategory] = useState("");
  const [desc, setDesc] = useState("");
  const [user, setUser] = useState("");
  const [pass, setPass] = useState("");
  const [conflict, setConflict] = useState<"rename" | "overwrite">("rename");
  const [remember, setRemember] = useState(false);
  const [more, setMore] = useState(true);
  const [free, setFree] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const touched = useRef({ name: false, dir: false, category: false });
  const categories = useApp((s) => s.categories);
  const queues = useApp((s) => s.queues);

  const apply = (c: CaptureInfo) => {
    setCap(c);
    if (!touched.current.name) setName(c.fileName);
    if (!touched.current.category) setCategory(c.category);
    if (!touched.current.dir) setDir(c.saveDir);
    setDesc((d) => d || c.description);
  };

  useEffect(() => {
    void api.getCapture(id).then((c) => (c ? apply(c) : setMissing(true)), () => setMissing(true));
    let off: (() => void) | undefined;
    void onCoreEvent((e) => {
      if (e.type === "captureUpdated" && e.payload.id === id) apply(e.payload);
    }).then((u) => (off = u));
    return () => off?.();
  }, [id]);

  useEffect(() => {
    if (!dir) return;
    const t = window.setTimeout(() => void api.freeSpace(dir).then(setFree, () => setFree(null)), 250);
    return () => window.clearTimeout(t);
  }, [dir]);

  const close = () => void win.close();
  const cancel = () => {
    void api.cancelCapture(id);
    close();
  };
  const confirm = async (mode: StartMode) => {
    if (!cap) return;
    setBusy(true);
    const item = await attempt(
      api.confirmCapture({
        id,
        mode,
        fileName: name.trim() || cap.fileName,
        saveDir: dir,
        category,
        description: desc,
        username: user || undefined,
        password: pass || undefined,
        rememberDir: remember,
        conflict: cap.exists ? conflict : undefined,
      }),
      "Could not add the download",
    );
    setBusy(false);
    if (item) close();
  };

  if (missing)
    return (
      <WindowFrame kind="capture" title="Download file">
        <div className="wempty">
          <Icon name="info" size={22} />
          <p>This download was already handled or cancelled.</p>
          <button type="button" className="btn" onClick={close}>
            Close
          </button>
        </div>
      </WindowFrame>
    );

  const lowSpace = cap?.size != null && free != null && free < cap.size;

  return (
    <WindowFrame kind="capture" title="Download file" icon={<LogoMark size={18} glass="indigo" />} onClose={cancel} className="capture">
      <div className="wbody">
        <div className="field">
          <span className="field__label">Address</span>
          <div className="withbtn">
            <div className="input input--readonly">
              <Icon name="link" size={15} />
              <span className="input__value t-mono selectable" title={cap?.url}>
                {cap?.url ?? "…"}
              </span>
            </div>
            <button type="button" className="btn btn--icon" aria-label="Copy address" data-tip="Copy address" onClick={() => cap && void copyText(cap.url)}>
              <Icon name="copy" size={15} />
            </button>
          </div>
        </div>

        <div className="filecard">
          {cap ? <FileTile item={{ fileName: name || cap.fileName, kind: cap.kind, category, media: null }} size={44} /> : <span className="file-tile" style={{ width: 44, height: 44 }} />}
          <div className="grow">
            <input
              className="fname"
              value={name}
              aria-label="File name"
              spellCheck={false}
              onChange={(e) => {
                touched.current.name = true;
                setName(e.target.value);
              }}
            />
            <div className="tags">
              {cap?.probing ? (
                <span className="badge badge--accent">
                  <Icon name="retry" size={11} strokeWidth={2.2} className="anim-spin" /> Checking…
                </span>
              ) : cap?.probeError ? (
                <span className="badge badge--danger" data-tip={cap.probeError}>
                  <Icon name="status-error" size={11} strokeWidth={2.2} /> {cap.probeError.length > 36 ? `${cap.probeError.slice(0, 36)}…` : cap.probeError}
                </span>
              ) : cap?.resumable ? (
                <span className="badge badge--success">
                  <Icon name="check" size={11} strokeWidth={2.4} /> Resumable
                </span>
              ) : cap ? (
                <span className="badge badge--warning" data-tip="The server does not support ranges: one connection, restarts from zero if interrupted">
                  <Icon name="warning" size={11} strokeWidth={2.2} /> Not resumable
                </span>
              ) : null}
              {cap?.mime && <span className="badge">{cap.mime.split(";")[0]}</span>}
              {cap && <span className="badge">{hostOf(cap.finalUrl || cap.url) || cap.kind.toUpperCase()}</span>}
            </div>
          </div>
          <div className="big tnum">
            {cap?.probing && cap.size == null ? "…" : cap?.size == null ? "N/A" : formatBytes(cap.size)}
            <small className={lowSpace ? "text-danger" : undefined}>{free != null ? `${formatBytes(free)} free` : ""}</small>
          </div>
        </div>

        {cap?.needsAuth && (
          <div className="authbox">
            <Icon name="lock-closed" size={16} className="text-warning" />
            <TextField value={user} onChange={setUser} label="User name" placeholder="User name" icon="user" />
            <TextField value={pass} onChange={setPass} label="Password" placeholder="Password" type="password" icon="key" onEnter={() => void api.reprobeCapture(id, user, pass)} />
            <button type="button" className="btn" onClick={() => void attempt(api.reprobeCapture(id, user, pass), "Could not retry")}>
              Retry
            </button>
          </div>
        )}

        {cap?.exists && (
          <div className="conflict">
            <Icon name="warning" size={16} className="text-warning" />
            <span>A file with this name already exists.</span>
            <Segmented
              size="sm"
              label="When the file exists"
              value={conflict}
              onChange={setConflict}
              options={[
                { value: "rename", label: "Keep both" },
                { value: "overwrite", label: "Overwrite" },
              ]}
            />
          </div>
        )}

        <div className="row-save">
          <div className="field">
            <span className="field__label">Save to</span>
            <div className="withbtn">
              <TextField
                value={dir}
                onChange={(v) => {
                  touched.current.dir = true;
                  setDir(v);
                }}
                icon="folder"
                label="Save folder"
              />
              <button
                type="button"
                className="btn"
                onClick={async () => {
                  const p = await pickFolder(dir);
                  if (p) {
                    touched.current.dir = true;
                    setDir(p);
                  }
                }}
              >
                Browse…
              </button>
            </div>
          </div>
          <div className="field">
            <span className="field__label">Category</span>
            <Select
              label="Category"
              value={category}
              onChange={(v) => {
                touched.current.category = true;
                setCategory(v);
                if (!touched.current.dir) setDir(folderFor(v, dir));
              }}
              options={categories.map((c) => ({ value: c.id, label: c.name, icon: categoryIcon(c) }))}
            />
          </div>
        </div>

        <div className="checks">
          <Checkbox checked={remember} onChange={setRemember}>
            Remember this folder for {categories.find((c) => c.id === category)?.name ?? "this category"}
          </Checkbox>
          <button type="button" className="disclose" aria-expanded={more} onClick={() => setMore(!more)}>
            <Icon name={more ? "chevron-down" : "chevron-right"} size={12} strokeWidth={2.2} /> Description
          </button>
        </div>
        {more && <TextArea value={desc} onChange={setDesc} label="Description" placeholder="Optional note shown in the list" />}
      </div>

      <footer className="dialog__footer">
        <div className="split">
          <button type="button" className="btn" disabled={!cap || busy} onClick={() => confirm({ type: "later" })}>
            <Icon name="status-scheduled" size={15} /> Download later
          </button>
          <button
            type="button"
            className="btn"
            aria-label="More start options"
            disabled={!cap || busy}
            onClick={(e) =>
              openMenuAt(e.currentTarget, [
                { label: "Download later", icon: "status-scheduled", onSelect: () => void confirm({ type: "later" }) },
                { label: "Add to queue", icon: "queue", submenu: queues.map((q) => ({ label: q.name, icon: q.running ? "start-queue" : "queue", onSelect: () => void confirm({ type: "queue", queueId: q.id }) })) },
              ])
            }
          >
            <Icon name="chevron-down" size={13} strokeWidth={2.2} />
          </button>
        </div>
        <span className="spacer" />
        <button type="button" className="btn btn--ghost" onClick={cancel}>
          Cancel
        </button>
        <button type="button" className="btn btn--primary" disabled={!cap || busy || lowSpace} onClick={() => confirm({ type: "now" })} data-tip={lowSpace ? "Not enough free space" : undefined}>
          <Icon name="download" size={15} /> Start download
        </button>
      </footer>
    </WindowFrame>
  );
}
