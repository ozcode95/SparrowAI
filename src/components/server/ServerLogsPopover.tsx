import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useServerStore } from "@/stores/server";
import { TuiButton } from "@/components/tui/Button";

type Tone = "default" | "ok" | "warn" | "err";

const DOT: Record<Tone, string> = {
  default: "bg-tui-fg-muted",
  ok: "bg-tui-accent",
  warn: "bg-tui-warn",
  err: "bg-tui-err",
};

const TEXT: Record<Tone, string> = {
  default: "text-tui-fg",
  ok: "text-tui-accent",
  warn: "text-tui-warn",
  err: "text-tui-err",
};

/**
 * OVMS status pill that doubles as a popover trigger. Lives in the
 * bottom status strip; when clicked, opens a log tail panel anchored
 * directly above the pill (the logs used to occupy half of the now-
 * removed Server page).
 *
 * The popover positions itself absolutely from the trigger so it stays
 * visually "stuck" to the OVMS status item even if the bottom bar
 * width changes.
 */
export function ServerLogsPopover({
  label,
  status,
  tone,
}: {
  label: string;
  status: string;
  tone: Tone;
}) {
  const logs = useServerStore((s) => s.logs);
  const clearLogs = useServerStore((s) => s.clearLogs);
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  // Close on outside click / Escape.
  useEffect(() => {
    if (!open) return;
    function onPointer(e: MouseEvent) {
      if (containerRef.current?.contains(e.target as Node)) return;
      setOpen(false);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    window.addEventListener("mousedown", onPointer);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onPointer);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  // Pin to the bottom whenever logs grow while the popover is open so
  // the live tail behaviour is preserved.
  useLayoutEffect(() => {
    if (!open) return;
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [open, logs.length]);

  return (
    <div ref={containerRef} className="relative inline-flex items-center">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        title={open ? "Hide OVMS logs" : "Show OVMS logs"}
        className={
          "inline-flex items-center gap-2 rounded-[4px] px-1.5 py-0.5 " +
          "transition-colors hover:bg-[var(--fluent-bg-subtle-hover)] " +
          (open ? "bg-[var(--fluent-bg-subtle-selected)]" : "")
        }
      >
        <span
          className={`h-1.5 w-1.5 rounded-full ${DOT[tone]}`}
          aria-hidden="true"
        />
        <span className="text-tui-fg-muted">{label}</span>
        <span className={TEXT[tone]}>{status}</span>
      </button>

      {open && (
        <div
          role="dialog"
          aria-label="OVMS logs"
          className={
            "fluent-mica absolute right-0 bottom-full z-40 mb-1.5 " +
            "flex w-[560px] max-w-[92vw] flex-col overflow-hidden " +
            "rounded-[8px] border border-tui-border " +
            "shadow-[var(--fluent-shadow-16)]"
          }
        >
          <div className="flex items-center justify-between gap-2 border-b border-tui-border px-3 py-1.5">
            <div className="flex items-baseline gap-2 text-[11px]">
              <span className="font-semibold text-tui-fg">OVMS logs</span>
              <span className="text-tui-fg-muted">
                {logs.length} {logs.length === 1 ? "line" : "lines"}
              </span>
            </div>
            <TuiButton variant="ghost" onClick={clearLogs}>
              Clear
            </TuiButton>
          </div>
          <div
            ref={scrollRef}
            className="allow-select max-h-[320px] min-h-[120px] overflow-auto p-2.5 font-mono text-[11px] leading-tight"
          >
            {logs.length === 0 ? (
              <span className="text-tui-fg-muted">No logs.</span>
            ) : (
              logs.map((l, i) => (
                <div key={i} className="whitespace-pre-wrap">
                  <span className="text-tui-fg-muted">
                    {l.ts.slice(11, 19)}{" "}
                  </span>
                  <span
                    className={
                      l.level === "ERROR"
                        ? "text-tui-err"
                        : l.level === "WARN"
                          ? "text-tui-warn"
                          : "text-tui-fg-dim"
                    }
                  >
                    {l.line}
                  </span>
                </div>
              ))
            )}
          </div>
        </div>
      )}
    </div>
  );
}
