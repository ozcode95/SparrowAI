import { useEffect, useMemo, useRef, useState } from "react";
import { useUiStore, type ViewId } from "@/stores/ui";

interface Command {
  id: string;
  label: string;
  hint?: string;
  icon?: React.ReactNode;
  run: () => void | Promise<void>;
}

const stroke = {
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.5,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
};

const VIEW_ICONS: Record<ViewId, React.ReactNode> = {
  chat: (
    <svg width="14" height="14" viewBox="0 0 24 24" {...stroke}>
      <path d="M21 12a8 8 0 0 1-11.6 7.15L4 21l1.85-5.4A8 8 0 1 1 21 12Z" />
    </svg>
  ),
  models: (
    <svg width="14" height="14" viewBox="0 0 24 24" {...stroke}>
      <path d="M12 3 3 7.5 12 12l9-4.5L12 3Z" />
      <path d="M3 12.5 12 17l9-4.5" />
      <path d="M3 17.5 12 22l9-4.5" />
    </svg>
  ),
  tasks: (
    <svg width="14" height="14" viewBox="0 0 24 24" {...stroke}>
      <rect x="4" y="4" width="16" height="16" rx="2" />
      <path d="m8 12 3 3 5-6" />
    </svg>
  ),
  memory: (
    <svg width="14" height="14" viewBox="0 0 24 24" {...stroke}>
      <path d="M9 5a4 4 0 1 0-4 4v6a4 4 0 1 0 4 4 4 4 0 1 0 6 0 4 4 0 1 0 4-4V9a4 4 0 1 0-4-4 4 4 0 1 0-6 0Z" />
    </svg>
  ),
  tools: (
    <svg width="14" height="14" viewBox="0 0 24 24" {...stroke}>
      <path d="M14.7 6.3a4 4 0 0 0-5.4 5.4L3 18v3h3l6.3-6.3a4 4 0 0 0 5.4-5.4l-2.6 2.6-2.4-2.4 2.6-2.6Z" />
    </svg>
  ),
  skills: (
    <svg width="14" height="14" viewBox="0 0 24 24" {...stroke}>
      <path d="M12 3 14.5 9l6 .5-4.6 4 1.5 6L12 16l-5.4 3.5 1.5-6L3.5 9.5 9.5 9 12 3Z" />
    </svg>
  ),
  settings: (
    <svg width="14" height="14" viewBox="0 0 24 24" {...stroke}>
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3h.1a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8v.1a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1Z" />
    </svg>
  ),
};

/**
 * Fluent UI "CommandPalette"-style dialog. Floats above the shell on
 * an acrylic backdrop with a rounded card and reveal shadow. The
 * keyboard model (↑/↓/Enter/Esc) is unchanged.
 */
export function CommandPalette() {
  const close = useUiStore((s) => s.toggleCommandPalette);
  const setView = useUiStore((s) => s.setView);
  const inputRef = useRef<HTMLInputElement>(null);
  const [q, setQ] = useState("");
  const [idx, setIdx] = useState(0);

  const commands = useMemo<Command[]>(() => {
    const views: { id: ViewId; label: string }[] = [
      { id: "chat", label: "Go to Chat" },
      { id: "models", label: "Go to Models" },
      { id: "tasks", label: "Go to Tasks" },
      { id: "memory", label: "Go to Memory" },
      { id: "tools", label: "Go to Tools" },
      { id: "skills", label: "Go to Skills" },
      { id: "settings", label: "Go to Settings" },
    ];
    return views.map((v) => ({
      id: `view:${v.id}`,
      label: v.label,
      hint: "Navigation",
      icon: VIEW_ICONS[v.id],
      run: () => {
        setView(v.id);
        close();
      },
    }));
  }, [setView, close]);

  const filtered = useMemo(() => {
    const needle = q.toLowerCase();
    return commands.filter((c) => c.label.toLowerCase().includes(needle));
  }, [commands, q]);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  useEffect(() => {
    setIdx(0);
  }, [q]);

  function onKeyDown(e: React.KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setIdx((i) => Math.min(filtered.length - 1, i + 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setIdx((i) => Math.max(0, i - 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      filtered[idx]?.run();
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/40 pt-[18vh]"
      onClick={close}
      role="presentation"
      style={{
        animation: "fluent-view-in 180ms var(--fluent-curve-decel) both",
      }}
    >
      <div
        className={
          "w-[560px] max-w-[92vw] overflow-hidden rounded-[12px] " +
          "fluent-acrylic border border-tui-border " +
          "shadow-[var(--fluent-shadow-28)]"
        }
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
      >
        <div className="flex items-center gap-3 px-5 pt-4 pb-3">
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.8"
            className="text-tui-fg-muted"
          >
            <circle cx="11" cy="11" r="7" />
            <path d="m21 21-4.3-4.3" />
          </svg>
          <input
            ref={inputRef}
            value={q}
            onChange={(e) => setQ(e.target.value)}
            onKeyDown={onKeyDown}
            placeholder="Search commands, pages, models…"
            className="allow-select w-full bg-transparent text-[15px] text-tui-fg placeholder:text-tui-fg-muted focus:outline-none"
          />
          <kbd className="rounded-[4px] border border-tui-border bg-[var(--fluent-bg-subtle)] px-1.5 py-0.5 text-[10px] text-tui-fg-muted">
            Esc
          </kbd>
        </div>
        <div className="h-px bg-tui-border" />
        <ul className="max-h-96 overflow-auto p-1.5">
          {filtered.map((c, i) => (
            <li key={c.id}>
              <button
                onClick={() => c.run()}
                onMouseEnter={() => setIdx(i)}
                className={
                  "flex w-full items-center gap-3 rounded-[6px] px-3 py-2 text-left text-[13px] " +
                  "transition-colors duration-100 " +
                  (i === idx
                    ? "bg-[var(--fluent-bg-subtle-selected)] text-tui-fg"
                    : "text-tui-fg-dim hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-fg")
                }
              >
                <span
                  className={
                    "flex h-6 w-6 shrink-0 items-center justify-center rounded-[4px] " +
                    (i === idx
                      ? "bg-tui-selection text-tui-accent"
                      : "bg-[var(--fluent-bg-subtle)] text-tui-fg-dim")
                  }
                >
                  {c.icon}
                </span>
                <span className="flex-1 truncate">{c.label}</span>
                {c.hint && (
                  <span className="text-[10px] uppercase tracking-wide text-tui-fg-muted">
                    {c.hint}
                  </span>
                )}
              </button>
            </li>
          ))}
          {filtered.length === 0 && (
            <li className="px-3 py-6 text-center text-[12px] text-tui-fg-muted">
              No matching commands
            </li>
          )}
        </ul>
        <div className="flex items-center justify-between gap-3 border-t border-tui-border px-4 py-2 text-[10px] text-tui-fg-muted">
          <span className="inline-flex items-center gap-2">
            <kbd className="rounded-[3px] border border-tui-border bg-[var(--fluent-bg-subtle)] px-1 py-px">
              ↑↓
            </kbd>
            Navigate
            <kbd className="ml-2 rounded-[3px] border border-tui-border bg-[var(--fluent-bg-subtle)] px-1 py-px">
              Enter
            </kbd>
            Open
          </span>
          <span>
            {filtered.length} result{filtered.length === 1 ? "" : "s"}
          </span>
        </div>
      </div>
    </div>
  );
}
