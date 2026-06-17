import { useEffect, useMemo, useState } from "react";
import { Panel } from "@/components/tui/Panel";
import { TuiButton } from "@/components/tui/Button";
import { TuiTextarea } from "@/components/tui/Input";
import { ProgressBar } from "@/components/tui/ProgressBar";
import {
  snapshotPercent,
  useMemoryStore,
  type MemorySnapshot,
  type MemoryTarget,
} from "@/stores/memory";

/**
 * Persistent memory page.
 *
 * Two parallel surfaces — `memory` (agent's notes) and `user` (user
 * profile) — each backed by a small `~/.zero/memories/*.md` file. The
 * model curates these via the built-in `memory` tool; this page is the
 * human-facing window onto the same files, so a user can reset or
 * correct what the model has remembered.
 *
 * The design intentionally mirrors Hermes Agent's memory UX: a usage
 * gauge, a list of entries, an "add" affordance, and an "edit raw"
 * escape hatch for power users who want to rewrite the file as
 * delimiter-separated text.
 */
export function MemoryView() {
  const state = useMemoryStore((s) => s.state);
  const loading = useMemoryStore((s) => s.loading);
  const load = useMemoryStore((s) => s.load);
  const error = useMemoryStore((s) => s.error);
  const clearError = useMemoryStore((s) => s.clearError);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-3">
      <Panel
        title="Memory"
        hint="persistent notes the assistant uses every turn"
      >
        <div className="space-y-2 text-[12px] text-tui-fg-muted">
          <p>
            Two small markdown files under{" "}
            <code className="rounded bg-[var(--fluent-bg-subtle)] px-1">
              ~/.zero/memories/
            </code>{" "}
            are injected into the system prompt as a frozen snapshot at the
            start of every conversation. The assistant edits them itself with
            the built-in <code>memory</code> tool — you can also edit them here.
          </p>
          {error && (
            <div className="flex items-start justify-between gap-2 rounded border border-tui-err/40 bg-[rgba(255,153,164,0.08)] px-2 py-1 text-tui-err">
              <span className="break-words">{error}</span>
              <button
                type="button"
                onClick={clearError}
                className="shrink-0 text-tui-err/80 hover:text-tui-err"
              >
                ✕
              </button>
            </div>
          )}
          {loading && !state && <p>Loading…</p>}
        </div>
      </Panel>

      <div className="grid min-h-0 flex-1 grid-cols-1 gap-3 md:grid-cols-2">
        <MemoryStorePanel
          target="memory"
          title="MEMORY.md"
          hint="agent notes — environment, conventions, lessons"
          snapshot={state?.memory ?? null}
        />
        <MemoryStorePanel
          target="user"
          title="USER.md"
          hint="user profile — preferences, identity, style"
          snapshot={state?.user ?? null}
        />
      </div>
    </div>
  );
}

interface StorePanelProps {
  target: MemoryTarget;
  title: string;
  hint: string;
  snapshot: MemorySnapshot | null;
}

function MemoryStorePanel({ target, title, hint, snapshot }: StorePanelProps) {
  const add = useMemoryStore((s) => s.add);
  const remove = useMemoryStore((s) => s.remove);
  const replace = useMemoryStore((s) => s.replace);
  const setRaw = useMemoryStore((s) => s.setRaw);

  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [editingIndex, setEditingIndex] = useState<number | null>(null);
  const [editingText, setEditingText] = useState("");
  const [rawMode, setRawMode] = useState(false);
  const [rawText, setRawText] = useState("");

  const pct = snapshot ? snapshotPercent(snapshot) : 0;
  const tone = useMemo<"default" | "warn" | "err">(() => {
    if (pct >= 95) return "err";
    if (pct >= 80) return "warn";
    return "default";
  }, [pct]);

  async function onAdd() {
    if (!draft.trim() || busy) return;
    setBusy(true);
    try {
      await add(target, draft);
      setDraft("");
    } catch {
      // store already captured the error
    } finally {
      setBusy(false);
    }
  }

  function startEdit(idx: number, current: string) {
    setEditingIndex(idx);
    setEditingText(current);
  }

  async function saveEdit() {
    if (editingIndex === null || !snapshot) return;
    const original = snapshot.entries[editingIndex];
    if (!original || editingText.trim() === original.trim()) {
      setEditingIndex(null);
      return;
    }
    setBusy(true);
    try {
      // Use the original entry as the substring to match — guaranteed
      // unique because every entry on disk is distinct (the backend
      // rejects exact duplicates on `add`).
      await replace(target, original, editingText);
      setEditingIndex(null);
    } catch {
      // keep editing open so the user can fix the input
    } finally {
      setBusy(false);
    }
  }

  async function onRemove(entry: string) {
    if (busy) return;
    setBusy(true);
    try {
      await remove(target, entry);
    } catch {
      // surfaced via store.error
    } finally {
      setBusy(false);
    }
  }

  function enterRawMode() {
    if (!snapshot) return;
    setRawText(snapshot.entries.join("\n§\n"));
    setRawMode(true);
  }

  async function saveRaw() {
    setBusy(true);
    try {
      await setRaw(target, rawText);
      setRawMode(false);
    } catch {
      // keep raw editor open so the user can fix and retry
    } finally {
      setBusy(false);
    }
  }

  return (
    <Panel title={title} hint={hint} scroll>
      <div className="flex flex-col gap-3 text-[12px]">
        <ProgressBar
          value={snapshot?.used ?? 0}
          max={snapshot?.limit ?? 1}
          tone={tone}
          label={
            snapshot
              ? `${snapshot.used} / ${snapshot.limit} chars · ${pct}% · ${snapshot.entries.length} ${snapshot.entries.length === 1 ? "entry" : "entries"}`
              : "loading…"
          }
        />

        {rawMode ? (
          <div className="flex flex-col gap-2">
            <p className="text-tui-fg-muted">
              Raw mode — entries are separated by{" "}
              <code className="rounded bg-[var(--fluent-bg-subtle)] px-1">
                §
              </code>{" "}
              on its own line.
            </p>
            <TuiTextarea
              value={rawText}
              onChange={(e) => setRawText(e.target.value)}
              rows={12}
              spellCheck={false}
              className="font-mono"
            />
            <div className="flex gap-2">
              <TuiButton variant="primary" onClick={saveRaw} disabled={busy}>
                Save
              </TuiButton>
              <TuiButton variant="ghost" onClick={() => setRawMode(false)}>
                Cancel
              </TuiButton>
            </div>
          </div>
        ) : (
          <>
            <ul className="flex flex-col gap-1.5">
              {snapshot?.entries.length === 0 && (
                <li className="rounded border border-dashed border-tui-border px-2 py-3 text-center text-tui-fg-muted">
                  No entries yet. The assistant will add things here as it
                  learns them.
                </li>
              )}
              {snapshot?.entries.map((entry, i) => (
                <li
                  key={`${i}-${entry.slice(0, 32)}`}
                  className="group flex items-start gap-2 rounded border border-tui-border bg-[var(--fluent-bg-subtle)] px-2 py-1.5"
                >
                  <span className="mt-0.5 w-5 shrink-0 text-right text-tui-fg-muted tabular-nums">
                    {i + 1}.
                  </span>
                  <div className="flex min-w-0 flex-1 flex-col gap-1">
                    {editingIndex === i ? (
                      <>
                        <TuiTextarea
                          value={editingText}
                          onChange={(e) => setEditingText(e.target.value)}
                          rows={3}
                          autoFocus
                        />
                        <div className="flex gap-1.5">
                          <TuiButton
                            variant="primary"
                            onClick={saveEdit}
                            disabled={busy}
                          >
                            Save
                          </TuiButton>
                          <TuiButton
                            variant="ghost"
                            onClick={() => setEditingIndex(null)}
                          >
                            Cancel
                          </TuiButton>
                        </div>
                      </>
                    ) : (
                      <p className="whitespace-pre-wrap break-words text-tui-fg">
                        {entry}
                      </p>
                    )}
                  </div>
                  {editingIndex !== i && (
                    <div className="flex shrink-0 gap-1 opacity-0 transition-opacity group-hover:opacity-100">
                      <TuiButton
                        variant="ghost"
                        onClick={() => startEdit(i, entry)}
                        title="Edit entry"
                      >
                        Edit
                      </TuiButton>
                      <TuiButton
                        variant="danger"
                        onClick={() => onRemove(entry)}
                        disabled={busy}
                        title="Remove entry"
                      >
                        ✕
                      </TuiButton>
                    </div>
                  )}
                </li>
              ))}
            </ul>

            <div className="flex flex-col gap-1.5">
              <TuiTextarea
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                placeholder={`Add a new ${target === "memory" ? "note" : "user-profile entry"}…`}
                rows={2}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
                    e.preventDefault();
                    void onAdd();
                  }
                }}
              />
              <div className="flex items-center justify-between gap-2">
                <span className="text-[11px] text-tui-fg-muted">
                  {draft.length > 0 && `${draft.length} chars`}{" "}
                  <span className="text-tui-fg-muted/60">Ctrl+Enter</span>
                </span>
                <div className="flex gap-1.5">
                  <TuiButton variant="ghost" onClick={enterRawMode}>
                    Edit raw
                  </TuiButton>
                  <TuiButton
                    variant="primary"
                    onClick={onAdd}
                    disabled={busy || !draft.trim()}
                  >
                    Add
                  </TuiButton>
                </div>
              </div>
            </div>
          </>
        )}
      </div>
    </Panel>
  );
}
