import { clamp } from "@/lib/format";

interface ProgressBarProps {
  value: number;
  max?: number;
  /** Ignored; retained for source-compat with the previous TUI version. */
  width?: number;
  label?: string;
  tone?: "default" | "ok" | "warn" | "err";
}

/**
 * Fluent UI v2 linear ProgressBar (determinate). Replaces the ASCII
 * `█░` block bar — the API (value/max/label/tone) is the same so
 * callers don't need to change.
 */
export function ProgressBar({
  value,
  max = 1,
  label,
  tone = "default",
}: ProgressBarProps) {
  const ratio = max > 0 ? clamp(value / max, 0, 1) : 0;
  const pct = Math.round(ratio * 100);
  const fill = {
    default: "bg-tui-accent",
    ok: "bg-tui-accent",
    warn: "bg-tui-warn",
    err: "bg-tui-err",
  }[tone];
  return (
    <div className="flex flex-col gap-1">
      <div
        role="progressbar"
        aria-valuenow={pct}
        aria-valuemin={0}
        aria-valuemax={100}
        className="h-[3px] w-full overflow-hidden rounded-full bg-[rgba(255,255,255,0.08)]"
      >
        <div
          className={`h-full rounded-full ${fill} transition-[width] duration-200 ease-out`}
          style={{ width: `${pct}%` }}
        />
      </div>
      {label && <span className="text-[11px] text-tui-fg-muted">{label}</span>}
    </div>
  );
}
