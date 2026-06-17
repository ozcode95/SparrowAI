/**
 * TypeScript mirror of `model_profile()` in
 * `src-tauri/src/chat/runner.rs`.
 *
 * Lets the UI show concrete per-model sampling defaults next to the
 * empty Settings → Providers inputs (and the chat header sampling
 * popover) instead of a generic "model default" placeholder. The chat
 * runner remains the source of truth at request time — this helper is
 * purely presentational, so if the backend grows a new family-specific
 * profile, update both sides in lockstep.
 *
 * Keep `isGemma4Family` byte-for-byte aligned with `is_gemma4_family`
 * on the Rust side so the placeholders never disagree with what the
 * runner will actually send on the wire.
 */

export interface ModelSamplingDefaults {
  /** Always concrete: the runner uses the profile temperature as the
   * floor when no override is set, so this field never falls through
   * to an upstream default. */
  temperature: number;
  /** `null` means "let the upstream decide" — the runner omits the
   * field entirely from the wire request. */
  top_p: number | null;
  /** Same semantics as `top_p`: `null` → omitted on the wire. */
  top_k: number | null;
}

const GEMMA4_DEFAULTS: ModelSamplingDefaults = {
  temperature: 1.0,
  top_p: 0.95,
  top_k: 64,
};

const DEFAULT_DEFAULTS: ModelSamplingDefaults = {
  temperature: 0.7,
  top_p: null,
  top_k: null,
};

function isGemma4Family(model: string): boolean {
  const lower = model.toLowerCase();
  return (
    lower.includes("gemma-4") ||
    lower.includes("gemma4") ||
    lower.includes("gemma_4")
  );
}

/**
 * Resolve a model id to its sampling defaults. Accepts `null` /
 * `undefined` for the convenience of callers reading from server-store
 * snapshots that may not have settled yet — those land on the generic
 * default profile so the UI still shows usable numbers at cold start.
 */
export function modelSamplingDefaults(
  model: string | null | undefined,
): ModelSamplingDefaults {
  if (model && isGemma4Family(model)) return GEMMA4_DEFAULTS;
  return DEFAULT_DEFAULTS;
}

/**
 * Format a sampling-default value for use as an input `placeholder`.
 * `null` means the profile leaves the field unset (upstream decides);
 * we show an em dash so the empty input still has a visible affordance.
 */
export function formatSamplingDefault(n: number | null): string {
  return n == null ? "—" : String(n);
}
