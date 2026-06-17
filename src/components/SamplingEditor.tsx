import { useCallback, type ReactNode } from "react";
import { TuiInput } from "@/components/tui/Input";
import { type SamplingConfig, EMPTY_SAMPLING } from "@/stores/settings";

/**
 * Shared editor for the three optional sampling knobs (`temperature`,
 * `top_p`, `top_k`). Used in two places:
 *
 *   * Settings → Providers (per-provider defaults, apply to all chats
 *     using that provider).
 *   * Chat header → Sampling popover (per-conversation override,
 *     applies to one chat only).
 *
 * Each field is independently optional. A blank input maps to `null`
 * on the wire, which the backend treats as "don't override at this
 * layer" — the runner's precedence chain (conversation → provider →
 * per-model profile → upstream default) then falls through to the
 * next layer.
 *
 * The `placeholder` props let callers show what the next layer down
 * would resolve to, so the user knows what they're inheriting before
 * they type anything. For Settings → Providers that's typically the
 * per-model profile default; for the chat popover it's the resolved
 * provider value (which may itself be a fallthrough to the profile).
 */
export function SamplingEditor({
  value,
  onChange,
  temperaturePlaceholder,
  topPPlaceholder,
  topKPlaceholder,
  disabled = false,
}: {
  value: SamplingConfig;
  onChange: (next: SamplingConfig) => void;
  /** Greyed-out hint shown when `value.temperature` is unset. */
  temperaturePlaceholder?: string;
  topPPlaceholder?: string;
  topKPlaceholder?: string;
  /** When true, all three inputs render disabled (e.g. while saving). */
  disabled?: boolean;
}) {
  const reset = useCallback(() => {
    onChange({ ...EMPTY_SAMPLING });
  }, [onChange]);

  const allUnset =
    value.temperature == null && value.top_p == null && value.top_k == null;

  return (
    <div className="space-y-2">
      <Row label="Temperature" hint="0.0 – 2.0. Higher = more random.">
        <NumberField
          value={value.temperature}
          onChange={(n) => onChange({ ...value, temperature: n })}
          placeholder={temperaturePlaceholder}
          step={0.1}
          min={0}
          max={2}
          disabled={disabled}
        />
      </Row>
      <Row label="Top P" hint="0.0 – 1.0. Nucleus sampling cutoff.">
        <NumberField
          value={value.top_p}
          onChange={(n) => onChange({ ...value, top_p: n })}
          placeholder={topPPlaceholder}
          step={0.05}
          min={0}
          max={1}
          disabled={disabled}
        />
      </Row>
      <Row label="Top K" hint="Integer. Cap on candidate tokens per step.">
        <NumberField
          value={value.top_k}
          onChange={(n) =>
            onChange({
              ...value,
              top_k: n == null ? null : Math.max(1, Math.round(n)),
            })
          }
          placeholder={topKPlaceholder}
          step={1}
          min={1}
          integer
          disabled={disabled}
        />
      </Row>
      <div className="flex items-center justify-end pt-1">
        <button
          type="button"
          onClick={reset}
          disabled={disabled || allUnset}
          className={
            "rounded-[4px] px-1.5 py-0.5 text-[11px] " +
            (disabled || allUnset
              ? "text-tui-fg-muted opacity-60"
              : "text-tui-fg-dim hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-fg")
          }
        >
          Reset all
        </button>
      </div>
    </div>
  );
}

function Row({
  label,
  hint,
  children,
}: {
  label: string;
  hint: string;
  children: ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-3">
      <div className="min-w-0 flex-1">
        <div className="text-[12px] text-tui-fg">{label}</div>
        <div className="text-[10.5px] text-tui-fg-muted">{hint}</div>
      </div>
      <div className="w-24 shrink-0">{children}</div>
    </div>
  );
}

/**
 * Small wrapper around `TuiInput` that parses to `number | null`.
 * Empty / unparsable input maps to `null` so the caller can distinguish
 * "user blanked the field" from "user typed 0".
 */
function NumberField({
  value,
  onChange,
  placeholder,
  step,
  min,
  max,
  integer = false,
  disabled = false,
}: {
  value: number | null;
  onChange: (next: number | null) => void;
  placeholder?: string;
  step?: number;
  min?: number;
  max?: number;
  integer?: boolean;
  disabled?: boolean;
}) {
  return (
    <TuiInput
      type="number"
      inputMode={integer ? "numeric" : "decimal"}
      value={value == null ? "" : String(value)}
      onChange={(e) => {
        const raw = e.target.value.trim();
        if (raw === "") {
          onChange(null);
          return;
        }
        const n = Number(raw);
        if (Number.isFinite(n)) onChange(n);
      }}
      placeholder={placeholder ?? ""}
      step={step}
      min={min}
      max={max}
      disabled={disabled}
      className="!py-[3px] text-right tabular-nums"
    />
  );
}

/**
 * Tiny one-liner that summarises a `SamplingConfig` for a chip / badge.
 * Returns e.g. `"T 1.0 · p 0.95 · k 64"` for a full override, the
 * placeholder text for an empty one. Useful for the chat header chip
 * label without bloating it with three separate spans.
 */
export function summariseSampling(s: SamplingConfig): string {
  const parts: string[] = [];
  if (s.temperature != null) parts.push(`T ${s.temperature}`);
  if (s.top_p != null) parts.push(`p ${s.top_p}`);
  if (s.top_k != null) parts.push(`k ${s.top_k}`);
  return parts.join(" · ");
}

/**
 * Count of fields with a non-null override. Drives the chat header
 * chip's badge count so it matches the Tools chip's pattern.
 */
export function samplingOverrideCount(s: SamplingConfig): number {
  let n = 0;
  if (s.temperature != null) n++;
  if (s.top_p != null) n++;
  if (s.top_k != null) n++;
  return n;
}
