import { useEffect, useState } from "react";
import { Panel } from "@/components/tui/Panel";
import { TuiButton } from "@/components/tui/Button";
import { TuiInput, TuiTextarea } from "@/components/tui/Input";
import {
  defaultAction,
  useTasksStore,
  type Task,
  type TaskAction,
  type TaskActionKind,
  type TaskTrigger,
} from "@/stores/tasks";
import { relativeTime } from "@/lib/format";
import { Events, on } from "@/lib/tauri";

const SELECT_CLASS =
  "w-full rounded-[4px] border border-tui-border bg-[var(--fluent-bg-subtle)] px-2 py-1 text-tui-fg outline-none transition-colors hover:bg-[var(--fluent-bg-subtle-hover)] focus:border-b-2 focus:border-b-tui-accent";

const ACTION_LABEL: Record<TaskActionKind, string> = {
  command: "Run command",
  script: "Run script",
  notify: "Show notification",
  prompt: "Agent prompt",
};

export function TasksView() {
  const tasks = useTasksStore((s) => s.tasks);
  const lastRunMessage = useTasksStore((s) => s.lastRunMessage);
  const list = useTasksStore((s) => s.list);
  const create = useTasksStore((s) => s.create);
  const update = useTasksStore((s) => s.update);
  const remove = useTasksStore((s) => s.remove);
  const runNow = useTasksStore((s) => s.runNow);
  const setEnabled = useTasksStore((s) => s.setEnabled);

  // `null`  = dialog closed.
  // `"new"` = create flow.
  // Task    = edit that task.
  const [editing, setEditing] = useState<Task | "new" | null>(null);

  useEffect(() => {
    void list();
  }, [list]);

  // The Rust-side scheduler emits `tasks://tick` after every fire (and
  // the `tasks_run_now` IPC re-lists on its own). Re-pull the table so
  // last_run_at/last_status reflect the latest scheduled run without
  // the user having to refresh manually.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void on<string>(Events.TaskTick, () => {
      if (!cancelled) void list();
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [list]);

  return (
    <div className="flex min-h-0 min-w-0 flex-1 gap-3">
      <Panel
        title="Tasks"
        hint={`${tasks.length} · ${tasks.filter((t) => t.enabled).length} active`}
        className="flex-1"
        flush
      >
        <div className="flex items-center justify-between gap-2 border-b border-tui-border p-3">
          <span className="text-[12px] text-tui-fg-muted">
            Run a command, script, notification, or agent prompt on a schedule
            or on demand.
          </span>
          <TuiButton variant="primary" onClick={() => setEditing("new")}>
            + New task
          </TuiButton>
        </div>
        <ul className="flex-1 overflow-auto">
          {tasks.length === 0 && (
            <li className="p-4 text-[12px] text-tui-fg-muted">
              No tasks yet. Try a notification on a cron, or wire up a script to
              run every morning.
            </li>
          )}
          {tasks.map((t) => (
            <TaskRow
              key={t.id}
              task={t}
              lastRun={lastRunMessage[t.id]}
              onRun={() => void runNow(t.id)}
              onEdit={() => setEditing(t)}
              onToggle={() => void setEnabled(t.id, !t.enabled)}
              onDelete={() => void remove(t.id)}
            />
          ))}
        </ul>
      </Panel>

      {editing && (
        <TaskDialog
          initial={editing === "new" ? null : editing}
          onCancel={() => setEditing(null)}
          onSubmit={async (draft) => {
            if (editing === "new") {
              await create(draft);
            } else {
              await update({ ...editing, ...draft });
            }
            setEditing(null);
          }}
        />
      )}
    </div>
  );
}

interface Draft {
  name: string;
  description: string;
  action: TaskAction;
  trigger: TaskTrigger;
  enabled: boolean;
}

function TaskDialog({
  initial,
  onCancel,
  onSubmit,
}: {
  initial: Task | null;
  onCancel: () => void;
  onSubmit: (draft: Draft) => void | Promise<void>;
}) {
  const isEdit = initial !== null;
  const [name, setName] = useState(initial?.name ?? "");
  const [desc, setDesc] = useState(initial?.description ?? "");
  const [action, setAction] = useState<TaskAction>(
    initial?.action ?? defaultAction("notify"),
  );
  const [triggerKind, setTriggerKind] = useState<TaskTrigger["kind"]>(
    initial?.trigger.kind ?? "manual",
  );
  const [cronParts, setCronParts] = useState<CronParts>(() =>
    parseCronParts(
      initial?.trigger.kind === "cron" ? initial.trigger.expr : "0 8 * * *",
    ),
  );
  const [intervalSec, setIntervalSec] = useState(
    initial?.trigger.kind === "interval" ? initial.trigger.seconds : 3600,
  );

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onCancel();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  function changeActionKind(kind: TaskActionKind) {
    if (kind === action.kind) return;
    setAction(defaultAction(kind));
  }

  function submit() {
    const trigger: TaskTrigger =
      triggerKind === "cron"
        ? { kind: "cron", expr: cronParts.join(" ") }
        : triggerKind === "interval"
          ? { kind: "interval", seconds: intervalSec }
          : triggerKind === "startup"
            ? { kind: "startup" }
            : { kind: "manual" };
    void onSubmit({
      name: name || "untitled",
      description: desc,
      action,
      trigger,
      enabled: initial?.enabled ?? true,
    });
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/40 pt-10"
      role="presentation"
      onClick={onCancel}
    >
      <div
        // `max-h` via inline style on purpose: Tailwind's JIT does not
        // reliably emit utilities for arbitrary values that combine `min()`
        // and `calc()` with a comma. Inline `maxHeight` sidesteps that and
        // keeps the dialog from ever overflowing the viewport — the body
        // below scrolls instead.
        className="flex w-[560px] max-w-[94vw] flex-col overflow-hidden rounded-xl border border-tui-border bg-tui-bg-elev shadow-[var(--fluent-shadow-16)]"
        style={{ maxHeight: "min(640px, calc(100vh - 16rem))" }}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-labelledby="new-task-title"
      >
        <header className="flex items-center justify-between gap-3 border-b border-tui-border px-4 py-3">
          <div className="min-w-0">
            <div
              id="new-task-title"
              className="text-[14px] font-semibold text-tui-fg"
            >
              {isEdit ? "Edit task" : "New task"}
            </div>
            <div className="truncate text-[11px] text-tui-fg-muted">
              Pick what should happen and when.
            </div>
          </div>
          <button
            onClick={onCancel}
            className="flex h-7 w-7 items-center justify-center rounded text-tui-fg-muted transition-colors hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-fg"
            title="Close"
            aria-label="Close"
          >
            <svg
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
            >
              <path d="M18 6 6 18M6 6l12 12" />
            </svg>
          </button>
        </header>

        <div className="min-h-0 flex-1 space-y-3 overflow-auto p-4 text-[12px]">
          <Field label="Name">
            <TuiInput value={name} onChange={(e) => setName(e.target.value)} />
          </Field>
          <Field label="Description">
            <TuiInput value={desc} onChange={(e) => setDesc(e.target.value)} />
          </Field>

          <Field label="Action">
            <select
              value={action.kind}
              onChange={(e) =>
                changeActionKind(e.target.value as TaskActionKind)
              }
              className={SELECT_CLASS}
            >
              <option value="command">Run a command</option>
              <option value="script">Run a script</option>
              <option value="notify">Show a notification</option>
              <option value="prompt">Run an agent prompt</option>
            </select>
          </Field>

          <ActionForm action={action} onChange={setAction} />

          <Field label="Trigger">
            <select
              value={triggerKind}
              onChange={(e) =>
                setTriggerKind(e.target.value as TaskTrigger["kind"])
              }
              className={SELECT_CLASS}
            >
              <option value="manual">Manual (click to run)</option>
              <option value="startup">Run on app startup</option>
              <option value="cron">Cron</option>
              <option value="interval">Interval</option>
            </select>
            {triggerKind === "startup" && (
              <p className="mt-1 text-[11px] text-tui-fg-muted">
                Fires once each time zero launches. Quit and reopen the app to
                run it again.
              </p>
            )}
          </Field>
          {triggerKind === "cron" && (
            <CronField parts={cronParts} onChange={setCronParts} />
          )}
          {triggerKind === "interval" && (
            <Field label="Every (seconds)">
              <TuiInput
                type="number"
                value={intervalSec}
                onChange={(e) => setIntervalSec(Number(e.target.value))}
              />
            </Field>
          )}
        </div>

        <footer className="flex justify-end gap-2 border-t border-tui-border px-4 py-3">
          <TuiButton onClick={onCancel}>Cancel</TuiButton>
          <TuiButton variant="primary" onClick={submit}>
            {isEdit ? "Save" : "Create"}
          </TuiButton>
        </footer>
      </div>
    </div>
  );
}

function ActionForm({
  action,
  onChange,
}: {
  action: TaskAction;
  onChange: (a: TaskAction) => void;
}) {
  switch (action.kind) {
    case "command":
      return (
        <>
          <Field label="Program">
            <TuiInput
              value={action.program}
              onChange={(e) => onChange({ ...action, program: e.target.value })}
              placeholder="e.g. C:\\Windows\\System32\\cmd.exe"
            />
          </Field>
          <Field label="Arguments (one per line)">
            <TuiTextarea
              rows={3}
              value={action.args.join("\n")}
              onChange={(e) =>
                onChange({
                  ...action,
                  args: e.target.value
                    .split("\n")
                    .map((s) => s.trim())
                    .filter(Boolean),
                })
              }
              placeholder={"/c\necho hello"}
            />
          </Field>
          <Field label="Working directory (optional)">
            <TuiInput
              value={action.cwd ?? ""}
              onChange={(e) =>
                onChange({ ...action, cwd: e.target.value || null })
              }
            />
          </Field>
        </>
      );
    case "script":
      return (
        <>
          <Field label="Script path">
            <TuiInput
              value={action.path}
              onChange={(e) => onChange({ ...action, path: e.target.value })}
              placeholder="C:\\Users\\me\\scripts\\backup.ps1"
            />
          </Field>
          <Field label="Interpreter (optional)">
            <TuiInput
              value={action.interpreter ?? ""}
              onChange={(e) =>
                onChange({
                  ...action,
                  interpreter: e.target.value || null,
                })
              }
              placeholder="powershell, python, bash…"
            />
          </Field>
          <Field label="Working directory (optional)">
            <TuiInput
              value={action.cwd ?? ""}
              onChange={(e) =>
                onChange({ ...action, cwd: e.target.value || null })
              }
            />
          </Field>
        </>
      );
    case "notify":
      return (
        <>
          <Field label="Title">
            <TuiInput
              value={action.title}
              onChange={(e) => onChange({ ...action, title: e.target.value })}
              placeholder="Stand up time"
            />
          </Field>
          <Field label="Body">
            <TuiTextarea
              rows={3}
              value={action.body}
              onChange={(e) => onChange({ ...action, body: e.target.value })}
              placeholder="Stretch, breathe, drink water."
            />
          </Field>
        </>
      );
    case "prompt":
      return (
        <>
          <Field label="Prompt (what should the agent do?)">
            <TuiTextarea
              rows={5}
              value={action.prompt}
              onChange={(e) => onChange({ ...action, prompt: e.target.value })}
              placeholder="e.g. summarise unread email from the past 30 minutes in 5 bullets"
            />
          </Field>
          <label className="flex items-center gap-2 text-[11px] text-tui-fg-dim">
            <input
              type="checkbox"
              checked={action.notify}
              onChange={(e) =>
                onChange({ ...action, notify: e.target.checked })
              }
            />
            Deliver the result as an OS notification
          </label>
          <div className="rounded border border-tui-border bg-[var(--fluent-bg-subtle)] px-2 py-1.5 text-[11px] text-tui-fg-muted">
            Agent prompts aren't wired up to the runner yet — saving this task
            works, but running it will report an error until the integration
            lands.
          </div>
        </>
      );
  }
}

function TaskRow({
  task,
  lastRun,
  onRun,
  onEdit,
  onToggle,
  onDelete,
}: {
  task: Task;
  lastRun?: { ok: boolean; message: string };
  onRun: () => void;
  onEdit: () => void;
  onToggle: () => void;
  onDelete: () => void;
}) {
  // Pause/Resume only makes sense for scheduled triggers — a manual
  // task already requires a click for every run, so toggling `enabled`
  // on it has no observable effect.
  const showToggle = task.trigger.kind !== "manual";
  return (
    <li className="group border-b border-tui-border px-3 py-2 text-[12px]">
      <div className="flex items-center justify-between gap-2">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 truncate">
            <span
              aria-hidden="true"
              className={`h-1.5 w-1.5 rounded-full ${task.enabled ? "bg-tui-accent" : "bg-tui-fg-muted"}`}
            />
            <span className="truncate font-medium text-tui-fg">
              {task.name}
            </span>
            <span className="shrink-0 rounded border border-tui-border px-1.5 py-px text-[10px] uppercase tracking-wide text-tui-fg-muted">
              {ACTION_LABEL[task.action.kind]}
            </span>
            <span className="shrink-0 text-[11px] text-tui-fg-muted">
              {triggerLabel(task.trigger)}
            </span>
          </div>
          {task.description && (
            <div className="truncate text-[11px] text-tui-fg-dim">
              {task.description}
            </div>
          )}
          <div className="truncate text-[11px] text-tui-fg-muted">
            {actionSummary(task.action)}
          </div>
          <div className="text-[11px] text-tui-fg-muted">
            Last run:{" "}
            {task.last_run_at ? relativeTime(task.last_run_at) : "never"}
            {task.last_status && ` · ${task.last_status}`}
          </div>
          {lastRun && (
            <div
              className={`truncate text-[11px] ${lastRun.ok ? "text-tui-ok" : "text-tui-err"}`}
              title={lastRun.message}
            >
              {lastRun.ok ? "✓" : "✗"} {lastRun.message}
            </div>
          )}
        </div>
        <div className="flex shrink-0 gap-1.5">
          <TuiButton onClick={onRun}>Run</TuiButton>
          <TuiButton variant="ghost" onClick={onEdit}>
            Edit
          </TuiButton>
          {showToggle && (
            <TuiButton variant="ghost" onClick={onToggle}>
              {task.enabled ? "Pause" : "Resume"}
            </TuiButton>
          )}
          <TuiButton variant="ghost" onClick={onDelete} aria-label="Delete">
            <svg
              width="12"
              height="12"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
            >
              <path d="M18 6 6 18M6 6l12 12" />
            </svg>
          </TuiButton>
        </div>
      </div>
    </li>
  );
}

function triggerLabel(t: TaskTrigger): string {
  switch (t.kind) {
    case "cron":
      return `cron: ${t.expr}`;
    case "interval":
      return `every ${t.seconds}s`;
    case "once":
      return `once at ${t.at}`;
    case "manual":
      return "manual";
    case "startup":
      return "on app startup";
  }
}

function actionSummary(a: TaskAction): string {
  switch (a.kind) {
    case "command":
      return `${a.program || "(no program)"}${a.args.length ? " " + a.args.join(" ") : ""}`;
    case "script":
      return a.interpreter
        ? `${a.interpreter} ${a.path || "(no path)"}`
        : a.path || "(no path)";
    case "notify":
      return a.title ? `“${a.title}”` : "(no title)";
    case "prompt":
      return a.prompt ? a.prompt.slice(0, 140) : "(empty prompt)";
  }
}

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <label className="block">
      <span className="mb-1 block text-[11px] font-medium text-tui-fg-dim">
        {label}
      </span>
      {children}
    </label>
  );
}

// ---- Cron expression editor ------------------------------------------------
//
// A 5-field cron expression (minute hour day-of-month month day-of-week) is
// easier to author and proof-read as discrete inputs than a single string.
// The form keeps the five parts in state, joins them with a space at submit
// time, and previews the assembled expression so the user can sanity-check
// the result without leaving the dialog.

type CronParts = [string, string, string, string, string];

interface CronFieldSpec {
  label: string;
  placeholder: string;
  hint: string;
}

const CRON_FIELDS: readonly CronFieldSpec[] = [
  { label: "Minute", placeholder: "0", hint: "0\u201359" },
  { label: "Hour", placeholder: "8", hint: "0\u201323" },
  { label: "Day", placeholder: "*", hint: "1\u201331" },
  { label: "Month", placeholder: "*", hint: "1\u201312" },
  { label: "Weekday", placeholder: "*", hint: "0\u20136 (Sun=0)" },
] as const;

function parseCronParts(expr: string): CronParts {
  const p = expr.trim().split(/\s+/);
  return [p[0] ?? "*", p[1] ?? "*", p[2] ?? "*", p[3] ?? "*", p[4] ?? "*"];
}

function CronField({
  parts,
  onChange,
}: {
  parts: CronParts;
  onChange: (next: CronParts) => void;
}) {
  function updateAt(index: number, value: string) {
    const next = [...parts] as CronParts;
    next[index] = value;
    onChange(next);
  }
  return (
    <div className="block">
      <span className="mb-1 block text-[11px] font-medium text-tui-fg-dim">
        Cron schedule
      </span>
      <div className="grid grid-cols-5 gap-2">
        {CRON_FIELDS.map((f, i) => (
          <div key={f.label} className="min-w-0">
            <div
              className="mb-0.5 truncate text-[10px] text-tui-fg-muted"
              title={f.hint}
            >
              {f.label}
            </div>
            <TuiInput
              value={parts[i]}
              onChange={(e) => updateAt(i, e.target.value)}
              placeholder={f.placeholder}
            />
            <div className="mt-0.5 truncate text-[10px] text-tui-fg-muted">
              {f.hint}
            </div>
          </div>
        ))}
      </div>
      <div className="mt-1.5 font-mono text-[11px] text-tui-fg-muted">
        {parts.join(" ")}
      </div>
    </div>
  );
}
