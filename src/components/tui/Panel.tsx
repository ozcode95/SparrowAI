import { type ReactNode } from "react";

interface PanelProps {
  title?: string;
  hint?: string;
  /** Optional leading slot in the panel header (e.g. a primary action
   * like "New chat"). Rendered on the left, before any `title` / `hint`,
   * with its own divider so it reads as a distinct affordance rather
   * than blending into the title text. */
  leading?: ReactNode;
  /** Optional trailing slot in the panel header (e.g. buttons). */
  action?: ReactNode;
  children: ReactNode;
  className?: string;
  flush?: boolean;
  scroll?: boolean;
}

/**
 * Fluent v2 card surface — Mica fill, rounded corners (10px), 1px
 * neutral stroke, a soft drop shadow for depth, and an optional inset
 * header strip. Keeps the `Panel` export so existing layouts don't need
 * to change.
 */
export function Panel({
  title,
  hint,
  leading,
  action,
  children,
  className = "",
  flush = false,
  scroll = false,
}: PanelProps) {
  return (
    <div
      className={
        "fluent-mica relative flex min-h-0 min-w-0 flex-col " +
        "rounded-[10px] border border-tui-border " +
        "shadow-[var(--fluent-shadow-2)] " +
        "overflow-hidden " +
        className
      }
    >
      {(title || hint || action || leading) && (
        <div
          className={
            "flex shrink-0 items-center justify-between gap-2 " +
            "border-b border-tui-border px-3.5 py-2 " +
            "bg-[rgba(255,255,255,0.022)]"
          }
        >
          <div className="flex min-w-0 items-center gap-2">
            {leading && (
              <div className="flex shrink-0 items-center">{leading}</div>
            )}
            {title && (
              <span className="truncate text-[12px] font-semibold text-tui-fg">
                {title}
              </span>
            )}
            {hint && (
              <span className="ml-1 truncate text-[11px] text-tui-fg-muted">
                {hint}
              </span>
            )}
          </div>
          {action && (
            <div className="flex shrink-0 items-center gap-1.5">{action}</div>
          )}
        </div>
      )}
      <div
        className={`flex min-h-0 min-w-0 flex-1 flex-col ${
          flush ? "" : "p-3.5"
        } ${scroll ? "overflow-auto" : ""}`}
      >
        {children}
      </div>
    </div>
  );
}
