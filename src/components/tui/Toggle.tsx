/**
 * Fluent UI v2 toggle switch — a proper pill-shaped on/off control.
 *
 * Used in place of native checkboxes for boolean settings where the
 * affordance benefits from a clearer "switch" metaphor (auto-start,
 * destructive-tool confirmation, …). Native `<input type="checkbox">`
 * still works fine for inline list-row toggles where space is tight.
 */
export function Toggle({
  checked,
  onChange,
  disabled = false,
  label,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  /** `aria-label` for screen readers when no visible label sits next to the switch. */
  label?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={
        "relative inline-flex h-5 w-10 shrink-0 items-center rounded-full " +
        "border transition-colors duration-150 ease-out " +
        "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-tui-accent " +
        "disabled:cursor-not-allowed disabled:opacity-40 " +
        (checked
          ? "border-tui-accent-dim bg-tui-accent-dim hover:bg-[var(--fluent-accent-hover)]"
          : "border-[var(--fluent-stroke-control-strong)] bg-transparent hover:bg-[var(--fluent-bg-subtle-hover)]")
      }
    >
      <span
        className={
          "absolute h-3 w-3 rounded-full transition-all duration-150 ease-out " +
          (checked
            ? "left-[22px] bg-white"
            : "left-[5px] bg-[var(--fluent-stroke-control-strong)]")
        }
      />
    </button>
  );
}
