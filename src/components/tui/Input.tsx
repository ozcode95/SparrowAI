import {
  forwardRef,
  type InputHTMLAttributes,
  type TextareaHTMLAttributes,
} from "react";

/**
 * Fluent UI v2 text input — subtle filled background with a 1px
 * neutral stroke and a 2px accent underline on focus (the Fluent
 * "indicator" pattern). Keeps the `TuiInput` export name so existing
 * pages don't have to change.
 */
const inputBase =
  "allow-select w-full rounded-[6px] " +
  "bg-[var(--fluent-bg-subtle)] " +
  "border border-tui-border " +
  "border-b-[var(--fluent-stroke-strong)] " +
  "px-3 py-[5px] text-[12px] text-tui-fg " +
  "placeholder:text-tui-fg-muted " +
  "transition-[background-color,border-color] duration-150 ease-out " +
  "hover:bg-[var(--fluent-bg-subtle-hover)] " +
  "focus:outline-none focus:bg-[var(--fluent-bg-subtle)] " +
  "focus:border-b-2 focus:border-b-tui-accent focus:pb-[4px] " +
  "disabled:opacity-50 disabled:cursor-not-allowed";

export const TuiInput = forwardRef<
  HTMLInputElement,
  InputHTMLAttributes<HTMLInputElement>
>(function TuiInput({ className = "", ...props }, ref) {
  return <input ref={ref} {...props} className={`${inputBase} ${className}`} />;
});

export const TuiTextarea = forwardRef<
  HTMLTextAreaElement,
  TextareaHTMLAttributes<HTMLTextAreaElement>
>(function TuiTextarea({ className = "", ...props }, ref) {
  return (
    <textarea
      ref={ref}
      {...props}
      className={`${inputBase} resize-none leading-relaxed ${className}`}
    />
  );
});

/**
 * Fluent v2 styled native `<select>`. Provided as a reusable building
 * block so per-page selects look identical to the text inputs.
 */
export const TuiSelect = forwardRef<
  HTMLSelectElement,
  React.SelectHTMLAttributes<HTMLSelectElement>
>(function TuiSelect({ className = "", children, ...props }, ref) {
  return (
    <select
      ref={ref}
      {...props}
      className={
        "w-full appearance-none rounded-[6px] " +
        "bg-[var(--fluent-bg-subtle)] " +
        "border border-tui-border " +
        "border-b-[var(--fluent-stroke-strong)] " +
        "px-3 py-[5px] pr-7 text-[12px] text-tui-fg " +
        "transition-[background-color,border-color] duration-150 ease-out " +
        "hover:bg-[var(--fluent-bg-subtle-hover)] " +
        "focus:outline-none focus:bg-[var(--fluent-bg-subtle)] " +
        "focus:border-b-2 focus:border-b-tui-accent focus:pb-[4px] " +
        "disabled:opacity-50 disabled:cursor-not-allowed " +
        className
      }
    >
      {children}
    </select>
  );
});
