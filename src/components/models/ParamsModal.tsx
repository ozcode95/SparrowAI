import { useEffect, useMemo, useState } from "react";
import { TuiButton } from "@/components/tui/Button";
import { TuiInput } from "@/components/tui/Input";
import { Spinner } from "@/components/tui/Spinner";
import { useSettingsStore } from "@/stores/settings";
import {
  defaultDeployParams,
  defaultTaskFor,
  COMPOUND_DEVICE_SEEDS,
  DEVICE_CHOICES,
  PIPELINE_TYPE_CHOICES,
  POOLING_CHOICES,
  REASONING_PARSER_CHOICES,
  TASK_KIND_LABELS,
  TOOL_PARSER_CHOICES,
  type EmbeddingsPooling,
  type ImageGenParams,
  type ModelDeployParams,
  type ModelTask,
  type ModelTaskKind,
  type ReasoningParser,
  type RerankParams,
  type S2tParams,
  type T2sParams,
  type TextGenParams,
  type TextGenPipelineType,
  type ToolParser,
} from "@/lib/modelParams";

interface Props {
  /** Display name (HF id) of the model being configured. */
  modelId: string;
  /**
   * Async loader — typically the store's `getParams`. The modal renders
   * a spinner until this resolves and falls back to defaults on error.
   */
  load: (id: string) => Promise<ModelDeployParams>;
  /** Persists params + triggers graph regeneration. */
  save: (id: string, params: ModelDeployParams) => Promise<void>;
  onClose: () => void;
  /**
   * Optional single-line note rendered under the header. Used to flag
   * pre-download mode where `load` is `models_suggest_params` and the
   * values shown were inferred from HF metadata rather than read off
   * an existing sidecar.
   */
  subtitle?: string;
}

// ─── Task rail metadata ────────────────────────────────────────────────────
//
// Every task category gets a short blurb + a 20×20 line icon that matches
// the rest of the app's iconography. Keeping the metadata adjacent to the
// component (not in `modelParams.ts`) on purpose — labels and icons are a
// rendering concern, not a wire-shape concern, and they stay in lockstep
// with the modal's design.

interface TaskMeta {
  label: string;
  short: string;
  blurb: string;
  icon: React.ReactNode;
}

const ICON_STROKE = {
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.5,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
};

const TASK_META: Record<ModelTaskKind, TaskMeta> = {
  text_generation: {
    label: TASK_KIND_LABELS.text_generation,
    short: "Text gen",
    blurb: "Chat / completion / tool-calling",
    icon: (
      <svg width="16" height="16" viewBox="0 0 24 24" {...ICON_STROKE}>
        <path d="M21 12a8 8 0 0 1-11.6 7.15L4 21l1.85-5.4A8 8 0 1 1 21 12Z" />
      </svg>
    ),
  },
  embeddings: {
    label: TASK_KIND_LABELS.embeddings,
    short: "Embeddings",
    blurb: "Vector representations of text",
    icon: (
      <svg width="16" height="16" viewBox="0 0 24 24" {...ICON_STROKE}>
        <circle cx="6" cy="6" r="2" />
        <circle cx="18" cy="6" r="2" />
        <circle cx="6" cy="18" r="2" />
        <circle cx="18" cy="18" r="2" />
        <circle cx="12" cy="12" r="2" />
        <path d="M8 7l3 4M16 7l-3 4M8 17l3-4M16 17l-3-4" />
      </svg>
    ),
  },
  rerank: {
    label: TASK_KIND_LABELS.rerank,
    short: "Rerank",
    blurb: "Cross-encoder relevance scoring",
    icon: (
      <svg width="16" height="16" viewBox="0 0 24 24" {...ICON_STROKE}>
        <path d="M4 6h12M4 12h8M4 18h14" />
        <path d="M19 4l3 3-3 3M21 14l-3 3 3 3" />
      </svg>
    ),
  },
  image_generation: {
    label: TASK_KIND_LABELS.image_generation,
    short: "Image gen",
    blurb: "Diffusion / text-to-image",
    icon: (
      <svg width="16" height="16" viewBox="0 0 24 24" {...ICON_STROKE}>
        <rect x="3" y="4" width="18" height="16" rx="2" />
        <circle cx="9" cy="10" r="2" />
        <path d="m4 18 5-5 4 4 3-3 4 4" />
      </svg>
    ),
  },
  text2speech: {
    label: TASK_KIND_LABELS.text2speech,
    short: "TTS",
    blurb: "Text → audio synthesis",
    icon: (
      <svg width="16" height="16" viewBox="0 0 24 24" {...ICON_STROKE}>
        <path d="M11 5 6 9H3v6h3l5 4Z" />
        <path d="M16 9a4 4 0 0 1 0 6M19 6a8 8 0 0 1 0 12" />
      </svg>
    ),
  },
  speech2text: {
    label: TASK_KIND_LABELS.speech2text,
    short: "STT",
    blurb: "Audio → text transcription",
    icon: (
      <svg width="16" height="16" viewBox="0 0 24 24" {...ICON_STROKE}>
        <rect x="9" y="3" width="6" height="12" rx="3" />
        <path d="M5 11a7 7 0 0 0 14 0M12 18v3M9 21h6" />
      </svg>
    ),
  },
};

const TASK_KINDS_ORDERED: ModelTaskKind[] = [
  "text_generation",
  "embeddings",
  "rerank",
  "image_generation",
  "text2speech",
  "speech2text",
];

/**
 * Per-model deployment parameter editor.
 *
 * Surfaces the same knobs as upstream `export_model.py`, organised
 * around a left rail of task categories (Text gen / Embeddings / …) so
 * the user picks the role first and then sees only the relevant
 * settings instead of one giant scrolling form. The Device override
 * sits in a sticky context header above the body because it's a
 * cross-cutting concern that applies to every task variant.
 *
 * The wire shape is unchanged — everything still serialises to the
 * `.zero_model.json` sidecar that the OVMS graph generator on the Rust
 * side consumes.
 */
export function ParamsModal({ modelId, load, save, onClose, subtitle }: Props) {
  const [params, setParams] = useState<ModelDeployParams | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const p = await load(modelId);
        if (!cancelled) setParams(p);
      } catch (e) {
        if (!cancelled) {
          setLoadError(String(e));
          setParams(defaultDeployParams);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [modelId, load]);

  // Close on Escape — matches the `TaskDialog` UX on the Tasks page.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const kind: ModelTaskKind = params?.task.kind ?? "text_generation";
  const runtimeDevice = useSettingsStore((st) => st.ovms.device);

  function patchTask(next: ModelTask) {
    setParams((prev) =>
      prev ? { ...prev, task: next } : { version: 1, device: null, task: next },
    );
  }

  function setDevice(next: string | null) {
    setParams((prev) =>
      prev
        ? { ...prev, device: next }
        : { ...defaultDeployParams, device: next },
    );
  }

  function changeKind(nextKind: ModelTaskKind) {
    if (nextKind === kind) return;
    patchTask(defaultTaskFor(nextKind));
  }

  async function onSave() {
    if (!params) return;
    setSaving(true);
    setSaveError(null);
    try {
      await save(modelId, params);
      onClose();
    } catch (e) {
      setSaveError(String(e));
    } finally {
      setSaving(false);
    }
  }

  const meta = TASK_META[kind] ?? TASK_META.text_generation;

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/40 pt-10"
      onClick={onClose}
      role="presentation"
    >
      <div
        className="flex max-h-[72vh] w-[820px] max-w-[96vw] flex-col overflow-hidden rounded-xl border border-tui-border bg-tui-bg-elev shadow-[var(--fluent-shadow-16)]"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
      >
        {/* ── Header ──────────────────────────────────────────────── */}
        <header className="flex items-start justify-between gap-3 border-b border-tui-border px-5 py-3">
          <div className="min-w-0">
            <div
              className="text-[14px] font-semibold text-tui-fg"
              style={{ fontFamily: "var(--font-display)" }}
            >
              Model parameters
            </div>
            <div
              className="truncate text-[11px] text-tui-fg-muted"
              title={modelId}
            >
              {modelId}
            </div>
          </div>
          <button
            onClick={onClose}
            className="flex h-7 w-7 items-center justify-center rounded text-tui-fg-muted transition-colors hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-fg"
            title="Close"
            aria-label="Close"
          >
            <svg
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
            >
              <path d="M18 6 6 18M6 6l12 12" />
            </svg>
          </button>
        </header>

        {/* ── Body ────────────────────────────────────────────────── */}
        {params == null ? (
          <div className="flex flex-1 items-center justify-center gap-2 p-10 text-[12px] text-tui-fg-muted">
            <Spinner size="sm" /> Loading…
          </div>
        ) : (
          <div className="flex min-h-0 flex-1">
            {/* Task rail */}
            <nav className="fluent-mica flex w-[200px] shrink-0 flex-col border-r border-tui-border py-2">
              <div className="px-3 pt-1 pb-2 text-[10px] font-semibold uppercase tracking-wide text-tui-fg-muted">
                Task category
              </div>
              <ul className="flex flex-col gap-px px-2">
                {TASK_KINDS_ORDERED.map((k) => (
                  <TaskRailItem
                    key={k}
                    meta={TASK_META[k]}
                    active={k === kind}
                    onClick={() => changeKind(k)}
                  />
                ))}
              </ul>
              <div className="mt-3 border-t border-tui-border px-3 pt-2 text-[10px] text-tui-fg-muted">
                Selects which OVMS calculator pipeline goes into{" "}
                <code className="font-mono">graph.pbtxt</code>.
              </div>
            </nav>

            {/* Detail pane */}
            <div className="flex min-h-0 flex-1 flex-col">
              {/* Subtitle / load-error banner */}
              {(subtitle || loadError) && (
                <div className="space-y-1.5 border-b border-tui-border px-5 pt-3 pb-2">
                  {subtitle && (
                    <div className="rounded-md border-l-2 border-tui-accent bg-[rgba(96,205,255,0.06)] px-3 py-1.5 text-[11px] text-tui-fg-dim">
                      {subtitle}
                    </div>
                  )}
                  {loadError && (
                    <div className="rounded-md border border-tui-warn/40 bg-[rgba(252,225,0,0.06)] px-3 py-1.5 text-[11px] text-tui-warn">
                      Failed to load existing params ({loadError}) — starting
                      from defaults.
                    </div>
                  )}
                </div>
              )}

              {/* Sticky context header — task title + device picker. */}
              <div className="flex items-start gap-4 border-b border-tui-border bg-[var(--fluent-bg-subtle)]/40 px-5 py-3">
                <div className="flex min-w-0 flex-1 items-start gap-2.5">
                  <span className="mt-px flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-tui-accent-dim text-white shadow-[var(--fluent-shadow-2)]">
                    {meta.icon}
                  </span>
                  <div className="min-w-0">
                    <div className="truncate text-[13px] font-semibold text-tui-fg">
                      {meta.label}
                    </div>
                    <div className="truncate text-[11px] text-tui-fg-muted">
                      {meta.blurb}
                    </div>
                  </div>
                </div>
                <div className="w-[220px] shrink-0">
                  <div className="mb-1 text-[10px] font-semibold uppercase tracking-wide text-tui-fg-muted">
                    Device
                  </div>
                  <DeviceSelect
                    value={params.device}
                    runtimeDefault={runtimeDevice}
                    onChange={setDevice}
                  />
                </div>
              </div>

              {/* Scrollable form body */}
              <div className="min-h-0 flex-1 overflow-auto px-5 py-4 text-[12px]">
                <TaskBody task={params.task} patch={patchTask} />
              </div>
            </div>
          </div>
        )}

        {/* ── Footer ──────────────────────────────────────────────── */}
        <footer className="flex items-center justify-between gap-2 border-t border-tui-border px-5 py-3">
          <div className="truncate text-[11px] text-tui-fg-muted">
            Saved to{" "}
            <code className="font-mono text-tui-fg-dim">.zero_model.json</code>
            {" · "}
            <code className="font-mono text-tui-fg-dim">graph.pbtxt</code>{" "}
            regenerated immediately
          </div>
          <div className="flex items-center gap-2">
            {saveError && (
              <span className="max-w-[260px] truncate text-[11px] text-tui-err">
                {saveError}
              </span>
            )}
            <TuiButton onClick={onClose}>Cancel</TuiButton>
            <TuiButton
              variant="primary"
              onClick={() => void onSave()}
              disabled={saving || !params}
            >
              {saving ? <Spinner size="sm" /> : "Save"}
            </TuiButton>
          </div>
        </footer>
      </div>
    </div>
  );
}

function TaskRailItem({
  meta,
  active,
  onClick,
}: {
  meta: TaskMeta;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <li className="list-none">
      <button
        type="button"
        onClick={onClick}
        aria-current={active ? "true" : undefined}
        className={
          "group relative flex w-full items-center gap-2.5 rounded-[6px] " +
          "px-2.5 py-1.5 text-left text-[12px] " +
          "transition-[background-color,color] duration-150 ease-out " +
          (active
            ? "bg-[var(--fluent-bg-subtle-selected)] text-tui-fg"
            : "text-tui-fg-dim hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-fg active:bg-[var(--fluent-bg-subtle-pressed)]")
        }
      >
        <span
          aria-hidden="true"
          className={
            "absolute left-0 top-1/2 -translate-y-1/2 w-[3px] rounded-full bg-tui-accent " +
            "transition-all duration-200 ease-out " +
            (active ? "h-3.5 opacity-100" : "h-0 opacity-0")
          }
        />
        <span
          className={
            "flex h-5 w-5 shrink-0 items-center justify-center transition-colors " +
            (active
              ? "text-tui-accent"
              : "text-tui-fg-dim group-hover:text-tui-fg")
          }
        >
          {meta.icon}
        </span>
        <span className="min-w-0 flex-1">
          <div className="truncate font-medium">{meta.short}</div>
          <div className="truncate text-[10px] text-tui-fg-muted">
            {meta.blurb}
          </div>
        </span>
      </button>
    </li>
  );
}

// ─── Form bodies per task ──────────────────────────────────────────────────

function TaskBody({
  task,
  patch,
}: {
  task: ModelTask;
  patch: (t: ModelTask) => void;
}) {
  switch (task.kind) {
    case "text_generation":
      return (
        <TextGenForm
          value={task}
          patch={(p) => patch({ kind: "text_generation", ...p })}
        />
      );
    case "embeddings":
      return (
        <EmbeddingsForm
          value={task}
          patch={(p) => patch({ kind: "embeddings", ...p })}
        />
      );
    case "rerank":
      return (
        <RerankForm
          value={task}
          patch={(p) => patch({ kind: "rerank", ...p })}
        />
      );
    case "image_generation":
      return (
        <ImageGenForm
          value={task}
          patch={(p) => patch({ kind: "image_generation", ...p })}
        />
      );
    case "text2speech":
      return (
        <T2sForm
          value={task}
          patch={(p) => patch({ kind: "text2speech", ...p })}
        />
      );
    case "speech2text":
      return (
        <S2tForm
          value={task}
          patch={(p) => patch({ kind: "speech2text", ...p })}
        />
      );
  }
}

function TextGenForm({
  value,
  patch,
}: {
  value: TextGenParams;
  patch: (next: TextGenParams) => void;
}) {
  const set = <K extends keyof TextGenParams>(k: K, v: TextGenParams[K]) =>
    patch({ ...value, [k]: v });

  return (
    <div className="space-y-4">
      <Section title="Pipeline" hint="How OVMS frames the model at load time.">
        <Field label="Pipeline type">
          <NullableSelect<TextGenPipelineType>
            value={value.pipeline_type}
            choices={PIPELINE_TYPE_CHOICES}
            onChange={(v) => set("pipeline_type", v)}
            placeholder="(auto — let OVMS pick)"
          />
        </Field>
        <Row>
          <ToggleField
            label="Enable prefix caching"
            hint="reuse KV across prompts that share a prefix"
            checked={value.enable_prefix_caching}
            onChange={(b) => set("enable_prefix_caching", b)}
          />
          <ToggleField
            label="Dynamic split fuse"
            hint="merge prefill/decode work for higher throughput"
            checked={value.dynamic_split_fuse}
            onChange={(b) => set("dynamic_split_fuse", b)}
          />
        </Row>
      </Section>

      <Section
        title="Capacity"
        hint="KV cache and concurrency limits. Defaults are sane for a single user."
      >
        <Row>
          <Field label="Cache size (GB)" hint="0 = dynamic">
            <NumberInput
              value={value.cache_size}
              min={0}
              onChange={(n) => set("cache_size", n)}
            />
          </Field>
          <Field label="Max num seqs">
            <NumberInput
              value={value.max_num_seqs}
              min={1}
              onChange={(n) => set("max_num_seqs", n)}
            />
          </Field>
        </Row>

        <Row>
          <Field label="KV cache precision">
            <NullableSelect
              value={value.kv_cache_precision}
              choices={["u8"] as const}
              onChange={(v) => set("kv_cache_precision", v as "u8" | null)}
              placeholder="(model default)"
            />
          </Field>
          <Field label="Max num batched tokens">
            <NullableNumberInput
              value={value.max_num_batched_tokens}
              onChange={(v) => set("max_num_batched_tokens", v)}
              placeholder="(unset)"
            />
          </Field>
        </Row>
      </Section>

      <Section
        title="Tool calling & reasoning"
        hint="Pick parsers that match the model family — required for structured tool calls and `<think>` segmentation."
      >
        <Row>
          <Field label="Reasoning parser">
            <NullableSelect<ReasoningParser>
              value={value.reasoning_parser}
              choices={REASONING_PARSER_CHOICES}
              onChange={(v) => set("reasoning_parser", v)}
              placeholder="(none)"
            />
          </Field>
          <Field label="Tool parser">
            <NullableSelect<ToolParser>
              value={value.tool_parser}
              choices={TOOL_PARSER_CHOICES}
              onChange={(v) => set("tool_parser", v)}
              placeholder="(none)"
            />
          </Field>
        </Row>

        <ToggleField
          label="Enable tool-guided generation"
          hint="constrains decoding to valid tool-call shapes — requires a tool parser"
          checked={value.enable_tool_guided_generation}
          onChange={(b) => set("enable_tool_guided_generation", b)}
        />
      </Section>

      <CollapsibleSection title="Advanced (NPU)" subtle>
        <Row>
          <Field
            label="Max prompt length"
            hint="NPU only — caps the static prompt slot"
          >
            <NullableNumberInput
              value={value.max_prompt_len}
              onChange={(v) => set("max_prompt_len", v)}
              placeholder="(unset)"
            />
          </Field>
          <ToggleField
            label="Prompt-lookup decoding"
            hint="speculative decoding from the prompt itself"
            checked={value.prompt_lookup_decoding}
            onChange={(b) => set("prompt_lookup_decoding", b)}
          />
        </Row>
      </CollapsibleSection>
    </div>
  );
}

function EmbeddingsForm({
  value,
  patch,
}: {
  value: import("@/lib/modelParams").EmbeddingsParams;
  patch: (next: import("@/lib/modelParams").EmbeddingsParams) => void;
}) {
  const set = <K extends keyof typeof value>(k: K, v: (typeof value)[K]) =>
    patch({ ...value, [k]: v });
  return (
    <div className="space-y-4">
      <Section title="Pooling">
        <Row>
          <Field label="Pooling strategy">
            <select
              value={value.pooling}
              onChange={(e) =>
                set("pooling", e.target.value as EmbeddingsPooling)
              }
              className={SELECT_CLASS}
            >
              {POOLING_CHOICES.map((p) => (
                <option key={p}>{p}</option>
              ))}
            </select>
          </Field>
          <Field label="Num streams">
            <NumberInput
              value={value.num_streams}
              min={1}
              onChange={(n) => set("num_streams", n)}
            />
          </Field>
        </Row>
      </Section>
      <Section title="Output">
        <Row>
          <ToggleField
            label="Normalize embeddings"
            hint="L2-normalise so cosine = dot product"
            checked={value.normalize}
            onChange={(b) => set("normalize", b)}
          />
          <ToggleField
            label="Truncate over-length inputs"
            hint="otherwise OVMS errors on input > model max"
            checked={value.truncate}
            onChange={(b) => set("truncate", b)}
          />
        </Row>
      </Section>
    </div>
  );
}

function RerankForm({
  value,
  patch,
}: {
  value: RerankParams;
  patch: (next: RerankParams) => void;
}) {
  const set = <K extends keyof RerankParams>(k: K, v: RerankParams[K]) =>
    patch({ ...value, [k]: v });
  return (
    <div className="space-y-4">
      <Section title="Rerank">
        <Row>
          <Field label="Num streams">
            <NumberInput
              value={value.num_streams}
              min={1}
              onChange={(n) => set("num_streams", n)}
            />
          </Field>
          <Field
            label="Max doc length (tokens)"
            hint="informational only — applied at tokenizer conversion time"
          >
            <NumberInput
              value={value.max_doc_length}
              min={1}
              onChange={(n) => set("max_doc_length", n)}
            />
          </Field>
        </Row>
      </Section>
    </div>
  );
}

function ImageGenForm({
  value,
  patch,
}: {
  value: ImageGenParams;
  patch: (next: ImageGenParams) => void;
}) {
  const set = <K extends keyof ImageGenParams>(k: K, v: ImageGenParams[K]) =>
    patch({ ...value, [k]: v });
  return (
    <div className="space-y-4">
      <Section title="Runtime">
        <Row>
          <Field label="Num streams">
            <NumberInput
              value={value.num_streams}
              min={0}
              onChange={(n) => set("num_streams", n)}
            />
          </Field>
          <Field label="Default resolution (WxH)">
            <NullableStringInput
              value={value.default_resolution}
              onChange={(v) => set("default_resolution", v)}
              placeholder="e.g. 1024x1024"
            />
          </Field>
        </Row>
        <Row>
          <Field label="Resolution (WxH)">
            <NullableStringInput
              value={value.resolution}
              onChange={(v) => set("resolution", v)}
              placeholder="e.g. 1024x768"
            />
          </Field>
          <Field label="Max resolution (WxH)">
            <NullableStringInput
              value={value.max_resolution}
              onChange={(v) => set("max_resolution", v)}
              placeholder="e.g. 1536x1536"
            />
          </Field>
        </Row>
      </Section>

      <Section title="Sampling">
        <Row>
          <Field label="Guidance scale">
            <NullableNumberInput
              value={value.guidance_scale ?? null}
              onChange={(v) => set("guidance_scale", v)}
              step={0.1}
              placeholder="(default 7.5)"
              float
            />
          </Field>
          <Field label="Default inference steps">
            <NullableNumberInput
              value={value.default_num_inference_steps}
              onChange={(v) => set("default_num_inference_steps", v)}
              placeholder="(model default)"
            />
          </Field>
        </Row>
        <Row>
          <Field label="Num images per prompt">
            <NullableNumberInput
              value={value.num_images_per_prompt}
              onChange={(v) => set("num_images_per_prompt", v)}
              placeholder="(default 1)"
            />
          </Field>
          <Field label="Max images per prompt">
            <NullableNumberInput
              value={value.max_num_images_per_prompt}
              onChange={(v) => set("max_num_images_per_prompt", v)}
              placeholder="(unlimited)"
            />
          </Field>
        </Row>
        <Field label="Max inference steps">
          <NullableNumberInput
            value={value.max_num_inference_steps}
            onChange={(v) => set("max_num_inference_steps", v)}
            placeholder="(unlimited)"
          />
        </Field>
      </Section>
    </div>
  );
}

function T2sForm({
  value,
  patch,
}: {
  value: T2sParams;
  patch: (next: T2sParams) => void;
}) {
  const set = <K extends keyof T2sParams>(k: K, v: T2sParams[K]) =>
    patch({ ...value, [k]: v });
  return (
    <div className="space-y-4">
      <Section title="Runtime">
        <Field label="Num streams">
          <NumberInput
            value={value.num_streams}
            min={0}
            onChange={(n) => set("num_streams", n)}
          />
        </Field>
      </Section>
      <Section
        title="Speaker"
        hint="Optional voice override. Leave both blank to use the model's default voice."
      >
        <Row>
          <Field label="Speaker name">
            <NullableStringInput
              value={value.speaker_name}
              onChange={(v) => set("speaker_name", v)}
              placeholder="(optional)"
            />
          </Field>
          <Field label="Speaker path">
            <NullableStringInput
              value={value.speaker_path}
              onChange={(v) => set("speaker_path", v)}
              placeholder="path/to/speaker.bin"
            />
          </Field>
        </Row>
      </Section>
    </div>
  );
}

function S2tForm({
  value,
  patch,
}: {
  value: S2tParams;
  patch: (next: S2tParams) => void;
}) {
  const set = <K extends keyof S2tParams>(k: K, v: S2tParams[K]) =>
    patch({ ...value, [k]: v });
  return (
    <div className="space-y-4">
      <Section title="Runtime">
        <Row>
          <Field label="Num streams">
            <NumberInput
              value={value.num_streams}
              min={0}
              onChange={(n) => set("num_streams", n)}
            />
          </Field>
          <ToggleField
            label="Word timestamps"
            hint="emits per-word start/end offsets"
            checked={value.enable_word_timestamps}
            onChange={(b) => set("enable_word_timestamps", b)}
          />
        </Row>
      </Section>
    </div>
  );
}

// ─── Tiny form primitives ──────────────────────────────────────────────────

const SELECT_CLASS =
  "w-full rounded-[4px] border border-tui-border bg-[var(--fluent-bg-subtle)] px-2 py-1 text-tui-fg outline-none transition-colors hover:bg-[var(--fluent-bg-subtle-hover)] focus:border-b-2 focus:border-b-tui-accent";

function Section({
  title,
  hint,
  children,
}: {
  title: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <section className="rounded-md border border-tui-border bg-[var(--fluent-bg-subtle)]/40 p-3">
      <header className="mb-2.5 flex items-baseline justify-between gap-3">
        <h3 className="text-[11px] font-semibold uppercase tracking-wide text-tui-fg-dim">
          {title}
        </h3>
        {hint && (
          <span className="truncate text-[10px] text-tui-fg-muted">{hint}</span>
        )}
      </header>
      <div className="space-y-3">{children}</div>
    </section>
  );
}

/**
 * Section that starts collapsed by default. Used to tuck NPU-only
 * advanced text-gen flags out of the way so the average user isn't
 * confronted with knobs they don't need.
 */
function CollapsibleSection({
  title,
  subtle,
  children,
}: {
  title: string;
  subtle?: boolean;
  children: React.ReactNode;
}) {
  return (
    <details
      className={
        "group rounded-md border border-tui-border " +
        (subtle ? "bg-[var(--fluent-bg-subtle)]/20" : "")
      }
    >
      <summary
        className={
          "flex cursor-pointer select-none items-center gap-2 px-3 py-2 " +
          "text-[11px] font-semibold uppercase tracking-wide text-tui-fg-dim " +
          "rounded-md transition-colors hover:bg-[var(--fluent-bg-subtle-hover)]"
        }
      >
        <svg
          width="12"
          height="12"
          viewBox="0 0 24 24"
          aria-hidden="true"
          className="shrink-0 text-tui-fg-muted transition-transform duration-150 ease-out group-open:rotate-90"
          {...ICON_STROKE}
        >
          <path d="m9 6 6 6-6 6" />
        </svg>
        <span>{title}</span>
      </summary>
      <div className="space-y-3 border-t border-tui-border p-3">{children}</div>
    </details>
  );
}

function Row({ children }: { children: React.ReactNode }) {
  return <div className="grid grid-cols-2 gap-3">{children}</div>;
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <label className="block space-y-1 text-[12px]">
      <div className="flex items-baseline justify-between gap-2">
        <span className="text-[11px] font-medium text-tui-fg-dim">{label}</span>
        {hint && (
          <span className="truncate text-[10px] text-tui-fg-muted">{hint}</span>
        )}
      </div>
      {children}
    </label>
  );
}

/**
 * Label + hint + checkbox laid out as a row so the toggle aligns with
 * sibling Field controls. Used for boolean knobs where the "Field" row
 * would feel oversized for a single checkbox.
 */
function ToggleField({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (b: boolean) => void;
}) {
  return (
    <label className="flex cursor-pointer items-start gap-2.5 rounded-[4px] border border-tui-border bg-[var(--fluent-bg-subtle)] px-2.5 py-2 text-[12px] transition-colors hover:bg-[var(--fluent-bg-subtle-hover)]">
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        className="mt-0.5 shrink-0 accent-tui-accent"
      />
      <span className="min-w-0 flex-1">
        <div className="text-[11px] font-medium text-tui-fg">{label}</div>
        {hint && <div className="text-[10px] text-tui-fg-muted">{hint}</div>}
      </span>
    </label>
  );
}

function NumberInput({
  value,
  min,
  step,
  onChange,
}: {
  value: number;
  min?: number;
  step?: number;
  onChange: (n: number) => void;
}) {
  return (
    <TuiInput
      type="number"
      value={value}
      min={min}
      step={step}
      onChange={(e) => {
        const v = Number(e.target.value);
        if (Number.isFinite(v)) onChange(v);
      }}
    />
  );
}

function NullableNumberInput({
  value,
  onChange,
  placeholder,
  step,
  float = false,
}: {
  value: number | null;
  onChange: (v: number | null) => void;
  placeholder?: string;
  step?: number;
  float?: boolean;
}) {
  // Track the raw string so the user can clear the field without losing
  // focus and so an in-progress digit doesn't get coerced mid-typing.
  const [raw, setRaw] = useState<string>(value == null ? "" : String(value));
  useEffect(() => {
    setRaw(value == null ? "" : String(value));
  }, [value]);
  return (
    <TuiInput
      type="number"
      value={raw}
      step={step}
      placeholder={placeholder}
      onChange={(e) => {
        const s = e.target.value;
        setRaw(s);
        if (s.trim() === "") {
          onChange(null);
          return;
        }
        const v = float ? parseFloat(s) : parseInt(s, 10);
        if (Number.isFinite(v)) onChange(v);
      }}
    />
  );
}

function NullableStringInput({
  value,
  onChange,
  placeholder,
}: {
  value: string | null;
  onChange: (v: string | null) => void;
  placeholder?: string;
}) {
  return (
    <TuiInput
      value={value ?? ""}
      placeholder={placeholder}
      onChange={(e) => {
        const s = e.target.value;
        onChange(s.trim() === "" ? null : s);
      }}
    />
  );
}

function NullableSelect<T extends string>({
  value,
  choices,
  onChange,
  placeholder,
}: {
  value: T | null;
  choices: readonly T[];
  onChange: (v: T | null) => void;
  placeholder: string;
}) {
  const opts = useMemo(() => choices, [choices]);
  return (
    <select
      value={value ?? ""}
      onChange={(e) =>
        onChange(e.target.value === "" ? null : (e.target.value as T))
      }
      className={SELECT_CLASS}
    >
      <option value="">{placeholder}</option>
      {opts.map((o) => (
        <option key={o} value={o}>
          {o}
        </option>
      ))}
    </select>
  );
}

/**
 * Device picker for per-model OpenVINO target override. Surfaces the
 * four built-in shortcuts ({@link DEVICE_CHOICES}) plus an `(inherit)`
 * option (= use the runtime ovms setting) and a `custom…` escape hatch
 * for free-form strings like `MULTI:CPU,GPU` or `HETERO:GPU,CPU`.
 */
function DeviceSelect({
  value,
  runtimeDefault,
  onChange,
}: {
  value: string | null;
  runtimeDefault: string;
  onChange: (v: string | null) => void;
}) {
  // "custom" mode is entered either explicitly by the user picking the
  // custom option, or implicitly when the stored value is non-null and
  // not one of the built-in shortcuts (e.g. a hand-edited `MULTI:...`).
  const isBuiltIn = value != null && DEVICE_CHOICES.includes(value);
  const [customMode, setCustomMode] = useState(value != null && !isBuiltIn);

  const selectValue = customMode ? "__custom" : (value ?? "");

  return (
    <div className="space-y-1">
      <select
        value={selectValue}
        onChange={(e) => {
          const v = e.target.value;
          if (v === "") {
            setCustomMode(false);
            onChange(null);
          } else if (v === "__custom") {
            setCustomMode(true);
            // Keep current value if it's already custom-shaped; otherwise
            // seed with the runtime default so the input isn't empty.
            if (value == null || DEVICE_CHOICES.includes(value)) {
              onChange(runtimeDefault);
            }
          } else if (COMPOUND_DEVICE_SEEDS[v]) {
            // HETERO / MULTI on their own aren't a valid device string;
            // flip into custom mode pre-seeded with a sensible suffix so
            // the user can adjust the device list inline.
            setCustomMode(true);
            onChange(COMPOUND_DEVICE_SEEDS[v]);
          } else {
            setCustomMode(false);
            onChange(v);
          }
        }}
        className={SELECT_CLASS}
      >
        <option value="">{`(inherit: ${runtimeDefault || "GPU"})`}</option>
        {DEVICE_CHOICES.map((d) => (
          <option key={d} value={d}>
            {COMPOUND_DEVICE_SEEDS[d] ? `${d}…` : d}
          </option>
        ))}
        <option value="__custom">custom…</option>
      </select>
      {customMode && (
        <TuiInput
          value={value ?? ""}
          placeholder="e.g. MULTI:GPU,CPU"
          onChange={(e) => {
            const s = e.target.value;
            onChange(s.trim() === "" ? null : s);
          }}
        />
      )}
    </div>
  );
}
