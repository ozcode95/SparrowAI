import { type ButtonHTMLAttributes } from "react";

interface TuiButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "default" | "primary" | "danger" | "ghost" | "subtle";
  size?: "sm" | "md";
}

/**
 * Fluent UI v2 button. We kept the `TuiButton` name (and the variant
 * vocabulary) so existing call sites continue to work — only the
 * presentation changed.
 *
 *  - `default`  → Fluent "Standard" subtle-filled button.
 *  - `primary`  → Fluent "Accent" button (filled with the system accent).
 *  - `danger`   → Subtle button keyed off the critical foreground.
 *  - `ghost`    → Fluent "Subtle" / link-style button, no chrome at rest.
 *  - `subtle`   → Fluent "Subtle" elevated, lighter chrome than default.
 */
export function TuiButton({
  variant = "default",
  size = "md",
  className = "",
  children,
  ...rest
}: TuiButtonProps) {
  const base =
    "inline-flex items-center justify-center gap-1.5 rounded-[6px] " +
    "font-medium leading-tight " +
    "transition-[background-color,border-color,color,transform,box-shadow] " +
    "duration-150 ease-out " +
    "active:scale-[0.98] " +
    "disabled:cursor-not-allowed disabled:opacity-40 disabled:active:scale-100 " +
    "focus-visible:outline-2 " +
    "focus-visible:outline-offset-2 focus-visible:outline-tui-accent";

  const sz =
    size === "sm"
      ? "px-2.5 py-[3px] text-[11px]"
      : "px-3.5 py-[5px] text-[12px]";

  const v = {
    default:
      "border border-tui-border bg-[var(--fluent-bg-subtle)] text-tui-fg " +
      "hover:bg-[var(--fluent-bg-subtle-hover)] " +
      "active:bg-[var(--fluent-bg-subtle-pressed)] active:text-tui-fg-dim",
    subtle:
      "border border-transparent bg-[var(--fluent-bg-subtle)] text-tui-fg-dim " +
      "hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-fg " +
      "active:bg-[var(--fluent-bg-subtle-pressed)]",
    primary:
      "border border-transparent bg-tui-accent-dim text-white " +
      "shadow-[var(--fluent-shadow-2)] " +
      "hover:bg-[var(--fluent-accent-hover)] hover:shadow-[var(--fluent-shadow-4)] " +
      "active:bg-[var(--fluent-accent-pressed)]",
    danger:
      "border border-tui-border bg-[var(--fluent-bg-subtle)] text-tui-err " +
      "hover:bg-[rgba(255,153,164,0.10)] hover:border-tui-err/40 " +
      "active:bg-[rgba(255,153,164,0.16)]",
    ghost:
      "border border-transparent bg-transparent text-tui-fg-dim " +
      "hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-fg " +
      "active:bg-[var(--fluent-bg-subtle-pressed)]",
  }[variant];

  return (
    <button {...rest} className={`${base} ${sz} ${v} ${className}`}>
      {children}
    </button>
  );
}
