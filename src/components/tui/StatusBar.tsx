import { type ReactNode } from "react";

interface KeyHintProps {
  k: string;
  label: string;
  onClick?: () => void;
}

/**
 * A label/shortcut pairing rendered in Fluent's "keyboard-tip" style —
 * a small rounded chip for the key, a normal-case label after it.
 */
export function KeyHint({ k, label, onClick }: KeyHintProps) {
  const Tag = onClick ? "button" : "span";
  return (
    <Tag
      onClick={onClick}
      className="group inline-flex items-center gap-1.5 text-tui-fg-muted hover:text-tui-fg transition-colors"
    >
      <kbd
        className={
          "rounded-[3px] border border-tui-border " +
          "bg-[var(--fluent-bg-subtle)] px-1.5 py-px " +
          "text-[10px] font-medium text-tui-fg-dim " +
          "group-hover:border-[var(--fluent-stroke-strong)] " +
          "group-hover:text-tui-fg"
        }
      >
        {k}
      </kbd>
      <span className="text-[11px]">{label}</span>
    </Tag>
  );
}

export function StatusBar({ children }: { children: ReactNode }) {
  return (
    <div className="flex items-center gap-3 border-t border-tui-border bg-tui-bg-elev px-3 py-1 text-[11px] text-tui-fg-dim">
      {children}
    </div>
  );
}

export function StatusSpacer() {
  return <div className="flex-1" />;
}

/**
 * Fluent "InfoBadge"-style status pill. Tone maps to the standard
 * neutral / informational / caution / critical foreground colours.
 */
export function StatusItem({
  label,
  value,
  tone = "default",
}: {
  label: string;
  value: ReactNode;
  tone?: "default" | "ok" | "warn" | "err";
}) {
  const dot = {
    default: "bg-tui-fg-muted",
    ok: "bg-tui-accent",
    warn: "bg-tui-warn",
    err: "bg-tui-err",
  }[tone];
  const text = {
    default: "text-tui-fg",
    ok: "text-tui-accent",
    warn: "text-tui-warn",
    err: "text-tui-err",
  }[tone];
  return (
    <div className="inline-flex items-center gap-2">
      <span className={`h-1.5 w-1.5 rounded-full ${dot}`} aria-hidden="true" />
      <span className="text-tui-fg-muted">{label}</span>
      <span className={text}>{value}</span>
    </div>
  );
}
