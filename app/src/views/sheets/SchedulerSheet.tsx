import { useEffect, useMemo, useState } from "react";
import { Checkbox, RateField, Segmented, Stepper, Switch, TextField } from "../../components/Controls";
import { FileTile } from "../../components/Visuals";
import { Icon, type IconName } from "../../design/icons";
import { api, webMode } from "../../lib/api";
import { formatBytes } from "../../lib/format";
import { t, tk } from "../../lib/i18n";
import { displayName } from "../../lib/present";
import { useApp } from "../../lib/store";
import type { PostAction, Queue } from "../../lib/types";
import { attempt, toast } from "../../lib/ui";

const DAYS = [tk("Mo"), tk("Tu"), tk("We"), tk("Th"), tk("Fr"), tk("Sa"), tk("Su")];

const AFTER: { type: PostAction["type"]; label: string; icon: IconName }[] = [
  { type: "none", label: tk("Do nothing"), icon: "minus" },
  { type: "sound", label: tk("Play sound"), icon: "sound" },
  { type: "exit", label: tk("Exit Prism"), icon: "close" },
  { type: "sleep", label: tk("Sleep"), icon: "sleep" },
  { type: "hibernate", label: tk("Hibernate"), icon: "moon" },
  { type: "shutdown", label: tk("Shut down"), icon: "power" },
  { type: "lock", label: tk("Lock screen"), icon: "lock" },
  { type: "command", label: tk("Run command"), icon: "external" },
];

const FINISH_ACTIONS = webMode ? AFTER.filter((a) => a.type === "none" || a.type === "sound") : AFTER;

type Repeat = "once" | "daily" | "days";

export function repeatOf(q: Queue): Repeat {
  if (q.schedule.onceDate) return "once";
  return q.schedule.days.length && q.schedule.days.length < 7 ? "days" : "daily";
}

function newQueue(position: number): Queue {
  return {
    id: `q-${Date.now().toString(36)}`,
    name: t("New queue"),
    maxConcurrent: 2,
    speedLimit: 0,
    position,
    builtin: false,
    running: false,
    schedule: { enabled: false, startTime: "01:00", stopTime: null, days: [], onceDate: null, retryFailed: true, onComplete: { type: "none" }, wake: false },
  };
}

export function SchedulerSheet({ initial }: { initial?: string }) {
  const queues = useApp((s) => s.queues);
  const [init] = useState(() => {
    const fresh = initial === "new" ? newQueue(useApp.getState().queues.length) : null;
    return { selected: fresh?.id ?? (initial && initial !== "new" ? initial : (useApp.getState().queues[0]?.id ?? "")), draft: fresh };
  });
  const [selected, setSelected] = useState(init.selected);
  const [draft, setDraft] = useState<Queue | null>(init.draft);
  const [tab, setTab] = useState<"schedule" | "files">("schedule");
  const [nextRuns, setNextRuns] = useState<[string, string | null][]>([]);

  const saved = queues.find((q) => q.id === selected);
  useEffect(() => {
    if (saved && (!draft || draft.id !== saved.id)) setDraft(structuredClone(saved));
  }, [saved, draft]);

  useEffect(() => {
    void api.nextScheduleRuns().then(setNextRuns, () => undefined);
  }, [queues]);

  const dirty = useMemo(() => !!draft && JSON.stringify({ ...draft, running: false }) !== JSON.stringify({ ...(saved ?? {}), running: false }), [draft, saved]);
  if (!draft) return null;
  const s = draft.schedule;
  const set = (patch: Partial<Queue>) => setDraft({ ...draft, ...patch });
  const setS = (patch: Partial<Queue["schedule"]>) => setDraft({ ...draft, schedule: { ...draft.schedule, ...patch } });
  const repeat = repeatOf(draft);
  const isNew = !saved;
  const next = nextRuns.find(([id]) => id === draft.id)?.[1];

  const save = async () => {
    // A queue seeded by the desktop app can still carry a finish action prism-web rejects.
    const queue = FINISH_ACTIONS.some((a) => a.type === draft.schedule.onComplete.type)
      ? draft
      : { ...draft, schedule: { ...draft.schedule, onComplete: { type: "none" as const } } };
    const res = await attempt(api.upsertQueue(queue), t("Could not save the queue"));
    if (res) {
      useApp.setState({ queues: res });
      toast(t("{name} saved", { name: draft.name }), { kind: "success", ms: 1800 });
    }
  };

  return (
    <div className="sched">
      <aside className="sched__list">
        {queues.map((q) => (
          <button key={q.id} type="button" className={`qitem${q.id === selected ? " on" : ""}`} onClick={() => setSelected(q.id)}>
            <Icon name={q.running ? "queue" : q.schedule.enabled ? "status-scheduled" : "queue"} size={16} />
            <b>{displayName(q.name)}</b>
            <small>{q.running ? t("Running") : q.schedule.enabled ? scheduleSummary(q) : t("Manual start")}</small>
          </button>
        ))}
        {isNew && (
          <button type="button" className="qitem on">
            <Icon name="plus" size={16} />
            <b>{draft.name}</b>
            <small>{t("Not saved yet")}</small>
          </button>
        )}
        <button
          type="button"
          className="newq"
          onClick={() => {
            const q = newQueue(queues.length);
            setSelected(q.id);
            setDraft(q);
          }}
        >
          <Icon name="plus" size={15} /> {t("New queue")}
        </button>
      </aside>

      <section className="sched__pane">
        <div className="qhead">
          <input className="qhead__name" aria-label={t("Queue name")} value={draft.name} onChange={(e) => set({ name: e.target.value })} />
          {s.enabled && next && (
            <span className="badge badge--info">
              <Icon name="status-scheduled" size={12} strokeWidth={2} /> {t("Next run {time}", { time: next })}
            </span>
          )}
          <Switch checked={s.enabled} label={t("Schedule enabled")} onChange={(v) => setS({ enabled: v })} />
        </div>
        <div className="qtabs" role="tablist">
          <button type="button" role="tab" aria-selected={tab === "schedule"} className={`qtab${tab === "schedule" ? " on" : ""}`} onClick={() => setTab("schedule")}>
            {t("Schedule")}
          </button>
          <button type="button" role="tab" aria-selected={tab === "files"} className={`qtab${tab === "files" ? " on" : ""}`} onClick={() => setTab("files")}>
            {t("Files")} <QueueCount id={draft.id} />
          </button>
        </div>

        {tab === "schedule" ? (
          <div className={`qbody${s.enabled ? "" : " is-off"}`}>
            <div className="frow">
              <span>{t("Repeat")}</span>
              <Segmented<Repeat>
                size="sm"
                label={t("Repeat")}
                value={repeat}
                onChange={(r) =>
                  setS(
                    r === "once"
                      ? { onceDate: new Date().toISOString().slice(0, 10), days: [] }
                      : r === "daily"
                        ? { onceDate: null, days: [] }
                        : { onceDate: null, days: s.days.length && s.days.length < 7 ? s.days : [1, 2, 3, 4, 5] },
                  )
                }
                options={[
                  { value: "once", label: t("Once") },
                  { value: "daily", label: t("Every day") },
                  { value: "days", label: t("On days") },
                ]}
              />
            </div>
            <div className="frow">
              <span>{t("Start at")}</span>
              <div className="inline">
                <label className="input time">
                  <Icon name="clock" size={15} />
                  <input type="time" aria-label={t("Start time")} value={s.startTime ?? ""} onChange={(e) => setS({ startTime: e.target.value || null })} />
                </label>
                {repeat === "once" && (
                  <label className="input date">
                    <Icon name="calendar" size={15} />
                    <input type="date" aria-label={t("Date")} value={s.onceDate ?? ""} onChange={(e) => setS({ onceDate: e.target.value || null })} />
                  </label>
                )}
                {repeat === "days" && (
                  <div className="days" role="group" aria-label={t("Weekdays")}>
                    {DAYS.map((d, i) => {
                      const n = i + 1;
                      const on = s.days.includes(n);
                      return (
                        <button key={d} type="button" className={`day${on ? " on" : ""}`} aria-pressed={on} onClick={() => setS({ days: on ? s.days.filter((x) => x !== n) : [...s.days, n].sort() })}>
                          {t(d)}
                        </button>
                      );
                    })}
                  </div>
                )}
              </div>
            </div>
            <div className="frow">
              <span>{t("Stop at")}</span>
              <div className="inline">
                <Checkbox checked={!!s.stopTime} label={t("Stop at a time")} onChange={(v) => setS({ stopTime: v ? "07:00" : null })} />
                <label className="input time">
                  <input type="time" aria-label={t("Stop time")} disabled={!s.stopTime} value={s.stopTime ?? ""} onChange={(e) => setS({ stopTime: e.target.value || null })} />
                </label>
                <span className="hint">{t("Unfinished files pause and continue next time")}</span>
              </div>
            </div>
            <div className="frow">
              <span>{t("Simultaneous")}</span>
              <div className="inline">
                <Stepper value={draft.maxConcurrent} min={1} max={16} label={t("simultaneous downloads")} onChange={(v) => set({ maxConcurrent: v })} />
                <span className="hint">{t("downloads at a time")}</span>
              </div>
            </div>
            <div className="frow">
              <span>{t("Speed limit")}</span>
              <div className="inline">
                <RateField value={draft.speedLimit} onCommit={(v) => set({ speedLimit: v })} label={t("Queue speed limit")} />
                <span className="hint">{t("for the whole queue")}</span>
              </div>
            </div>
            <div className="frow">
              <span>{t("Options")}</span>
              <div className="inline">
                <Checkbox checked={s.retryFailed} onChange={(v) => setS({ retryFailed: v })}>
                  {t("Retry failed downloads")}
                </Checkbox>
                <Checkbox checked={s.wake} onChange={(v) => setS({ wake: v })}>
                  {t("Wake the computer to start")}
                </Checkbox>
              </div>
            </div>
            <div className="frow frow--start">
              <span>{t("When finished")}</span>
              <div>
                <div className="after">
                  {FINISH_ACTIONS.map((a) => (
                    <button
                      key={a.type}
                      type="button"
                      className={`aft${s.onComplete.type === a.type ? " on" : ""}`}
                      aria-pressed={s.onComplete.type === a.type}
                      onClick={() => setS({ onComplete: a.type === "command" ? { type: "command", value: s.onComplete.type === "command" ? s.onComplete.value : "" } : ({ type: a.type } as PostAction) })}
                    >
                      <Icon name={a.icon} size={16} />
                      {t(a.label)}
                    </button>
                  ))}
                </div>
                {webMode && <p className="hint">{t("Other finish actions need the desktop app.")}</p>}
                {!webMode && s.onComplete.type === "command" && (
                  <div style={{ marginTop: 8 }}>
                    <TextField
                      value={s.onComplete.value}
                      onChange={(v) => setS({ onComplete: { type: "command", value: v } })}
                      label={t("Command to run")}
                      mono
                      icon="external"
                      placeholder={t("e.g. {command}", { command: '"C:\\Tools\\sync.cmd" --after-downloads' })}
                    />
                  </div>
                )}
              </div>
            </div>
          </div>
        ) : (
          <QueueFiles id={draft.id} />
        )}

        <footer className="dialog__footer">
          {saved?.running ? (
            <button type="button" className="btn" onClick={() => void attempt(api.stopQueue(draft.id))}>
              <Icon name="stop-queue" size={16} /> {t("Stop")}
            </button>
          ) : (
            <button type="button" className="btn" disabled={isNew} onClick={() => void attempt(api.startQueue(draft.id))}>
              <Icon name="start-queue" size={16} /> {t("Run now")}
            </button>
          )}
          {!draft.builtin && !isNew && (
            <button
              type="button"
              className="btn btn--ghost btn--danger"
              onClick={async () => {
                const res = await attempt(api.deleteQueue(draft.id), t("Could not delete the queue"));
                if (res) {
                  useApp.setState({ queues: res });
                  setSelected(res[0]?.id ?? "");
                  setDraft(null);
                }
              }}
            >
              {t("Delete queue")}
            </button>
          )}
          <span className="spacer" />
          <button type="button" className="btn btn--ghost" disabled={!dirty || isNew} onClick={() => saved && setDraft(structuredClone(saved))}>
            {t("Revert")}
          </button>
          <button type="button" className="btn btn--primary" disabled={!dirty && !isNew} onClick={save}>
            {isNew ? t("Create queue") : t("Save schedule")}
          </button>
        </footer>
      </section>
    </div>
  );
}

export function scheduleSummary(q: Queue): string {
  const s = q.schedule;
  const at = s.startTime ?? "—";
  if (s.onceDate) return t("{date} at {time}", { date: s.onceDate, time: at });
  if (!s.days.length || s.days.length === 7) return t("Daily at {time}", { time: at });
  if (s.days.join() === "1,2,3,4,5") return t("Weekdays at {time}", { time: at });
  if (s.days.join() === "6,7") return t("Weekends at {time}", { time: at });
  return t("{days} at {time}", { days: s.days.map((d) => t(DAYS[d - 1])).join(" "), time: at });
}

function QueueCount({ id }: { id: string }) {
  const n = useApp((s) => Object.values(s.items).filter((i) => i.queueId === id && !i.trashed).length);
  return <span className="count">{n}</span>;
}

/** Files assigned to the queue in their order; drag (or Alt+↑/↓) to reorder. */
function QueueFiles({ id }: { id: string }) {
  const all = useApp((s) => s.items);
  const files = useMemo(() => Object.values(all).filter((i) => i.queueId === id && !i.trashed).sort((a, b) => a.position - b.position), [all, id]);
  const [drag, setDrag] = useState<{ from: number; over: number } | null>(null);
  if (!files.length)
    return (
      <div className="qbody">
        <p className="hint">{t("This queue is empty. Right-click downloads in the list and choose {command}.", { command: t("Move to queue") })}</p>
      </div>
    );
  const move = (from: number, to: number) => {
    if (from !== to) void attempt(api.moveInQueue([files[from].id], to - from), t("Could not reorder"));
  };
  return (
    <div className="qbody qbody--files">
      <div className="files" role="list" aria-label={t("Files in queue")}>
        {files.map((f, i) => (
          <div
            key={f.id}
            role="listitem"
            tabIndex={0}
            draggable
            className={`frow2${drag?.from === i ? " dragging" : ""}${drag && drag.over === i && drag.from !== i ? (drag.from < i ? " drop-after" : " drop-before") : ""}`}
            onDragStart={(e) => {
              e.dataTransfer.effectAllowed = "move";
              e.dataTransfer.setData("text/x-prism-row", f.id);
              setDrag({ from: i, over: i });
            }}
            onDragOver={(e) => {
              if (!drag) return;
              e.preventDefault();
              if (drag.over !== i) setDrag({ ...drag, over: i });
            }}
            onDrop={(e) => {
              e.preventDefault();
              e.stopPropagation();
              if (drag) move(drag.from, i);
              setDrag(null);
            }}
            onDragEnd={() => setDrag(null)}
            onKeyDown={(e) => {
              if (e.altKey && e.key === "ArrowUp" && i > 0) move(i, i - 1);
              if (e.altKey && e.key === "ArrowDown" && i < files.length - 1) move(i, i + 1);
            }}
          >
            <Icon name="drag-handle" size={16} className="dh" />
            <span className="pos tnum">{i + 1}</span>
            <FileTile item={f} size={24} thumbWidth={34} />
            <span className="nm">{f.fileName}</span>
            <span className="sz tnum">{formatBytes(f.size)}</span>
          </div>
        ))}
      </div>
      <p className="hint">{t("Drag to reorder, or focus a row and press Alt+↑ / Alt+↓.")}</p>
    </div>
  );
}
