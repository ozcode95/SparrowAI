/**
 * TypeScript mirror of `src-tauri/src/hf/params.rs`.
 *
 * Mirrors the parameter surface of the upstream
 * `openvinotoolkit/model_server/demos/common/export_models/export_model.py`
 * tool so anyone familiar with that script can map zero settings 1:1.
 *
 * Persisted on disk as `<model_dir>/.zero_model.json` by the Rust side;
 * the wire shape matches this file. Keep enum string values in lockstep
 * with the Rust `#[serde(rename_all = "snake_case")]` discriminator.
 */

export type TextGenPipelineType = "LM" | "LM_CB" | "VLM" | "VLM_CB" | "AUTO";

export type ReasoningParser = "qwen3" | "gptoss" | "gemma4";

export type ToolParser =
  | "llama3"
  | "phi4"
  | "hermes3"
  | "mistral"
  | "qwen3coder"
  | "gptoss"
  | "devstral"
  | "lfm2"
  | "gemma4";

export type KvCachePrecision = "u8";

export type EmbeddingsPooling = "CLS" | "LAST" | "MEAN";

export interface TextGenParams {
  pipeline_type: TextGenPipelineType | null;
  enable_prefix_caching: boolean;
  cache_size: number;
  max_num_seqs: number;
  kv_cache_precision: KvCachePrecision | null;
  max_num_batched_tokens: number | null;
  dynamic_split_fuse: boolean;
  max_prompt_len: number | null;
  prompt_lookup_decoding: boolean;
  reasoning_parser: ReasoningParser | null;
  tool_parser: ToolParser | null;
  enable_tool_guided_generation: boolean;
}

export interface EmbeddingsParams {
  normalize: boolean;
  pooling: EmbeddingsPooling;
  truncate: boolean;
  num_streams: number;
}

export interface RerankParams {
  num_streams: number;
  max_doc_length: number;
}

export interface ImageGenParams {
  num_streams: number;
  resolution: string | null;
  guidance_scale: number | null;
  num_images_per_prompt: number | null;
  max_resolution: string | null;
  default_resolution: string | null;
  max_num_images_per_prompt: number | null;
  default_num_inference_steps: number | null;
  max_num_inference_steps: number | null;
}

export interface T2sParams {
  num_streams: number;
  speaker_name: string | null;
  speaker_path: string | null;
}

export interface S2tParams {
  num_streams: number;
  enable_word_timestamps: boolean;
}

export type ModelTaskKind =
  | "text_generation"
  | "embeddings"
  | "rerank"
  | "image_generation"
  | "text2speech"
  | "speech2text";

/**
 * Discriminated union matching `#[serde(tag = "kind")]` on the Rust side.
 * The flattened fields under each variant are the task's params struct.
 */
export type ModelTask =
  | ({ kind: "text_generation" } & TextGenParams)
  | ({ kind: "embeddings" } & EmbeddingsParams)
  | ({ kind: "rerank" } & RerankParams)
  | ({ kind: "image_generation" } & ImageGenParams)
  | ({ kind: "text2speech" } & T2sParams)
  | ({ kind: "speech2text" } & S2tParams);

export interface ModelDeployParams {
  version: number;
  /**
   * Per-model OpenVINO device override (`CPU`, `GPU`, `NPU`, `AUTO`,
   * `MULTI:CPU,GPU`, ...). `null` means inherit the runtime OVMS setting
   * configured on the Settings page. Honoured by the Rust graph renderer
   * when writing `graph.pbtxt`.
   */
  device: string | null;
  task: ModelTask;
}

// ─── Defaults ──────────────────────────────────────────────────────────────

export const defaultTextGen: TextGenParams = {
  pipeline_type: null,
  enable_prefix_caching: true,
  cache_size: 0,
  max_num_seqs: 256,
  kv_cache_precision: null,
  max_num_batched_tokens: null,
  dynamic_split_fuse: true,
  max_prompt_len: null,
  prompt_lookup_decoding: false,
  reasoning_parser: null,
  tool_parser: null,
  enable_tool_guided_generation: false,
};

export const defaultEmbeddings: EmbeddingsParams = {
  normalize: true,
  pooling: "CLS",
  truncate: false,
  num_streams: 1,
};

export const defaultRerank: RerankParams = {
  num_streams: 1,
  max_doc_length: 16000,
};

export const defaultImageGen: ImageGenParams = {
  num_streams: 0,
  resolution: null,
  guidance_scale: null,
  num_images_per_prompt: null,
  max_resolution: null,
  default_resolution: null,
  max_num_images_per_prompt: null,
  default_num_inference_steps: null,
  max_num_inference_steps: null,
};

export const defaultT2s: T2sParams = {
  num_streams: 0,
  speaker_name: null,
  speaker_path: null,
};

export const defaultS2t: S2tParams = {
  num_streams: 0,
  enable_word_timestamps: false,
};

export const defaultDeployParams: ModelDeployParams = {
  version: 1,
  device: null,
  task: { kind: "text_generation", ...defaultTextGen },
};

/** Return a fresh default `ModelTask` payload for the given kind. */
export function defaultTaskFor(kind: ModelTaskKind): ModelTask {
  switch (kind) {
    case "text_generation":
      return { kind, ...defaultTextGen };
    case "embeddings":
      return { kind, ...defaultEmbeddings };
    case "rerank":
      return { kind, ...defaultRerank };
    case "image_generation":
      return { kind, ...defaultImageGen };
    case "text2speech":
      return { kind, ...defaultT2s };
    case "speech2text":
      return { kind, ...defaultS2t };
  }
}

export const TASK_KIND_LABELS: Record<ModelTaskKind, string> = {
  text_generation: "text generation (chat / completion)",
  embeddings: "embeddings",
  rerank: "rerank",
  image_generation: "image generation",
  text2speech: "text → speech",
  speech2text: "speech → text",
};

export const REASONING_PARSER_CHOICES: ReasoningParser[] = [
  "qwen3",
  "gptoss",
  "gemma4",
];

export const TOOL_PARSER_CHOICES: ToolParser[] = [
  "llama3",
  "phi4",
  "hermes3",
  "mistral",
  "qwen3coder",
  "gptoss",
  "devstral",
  "lfm2",
  "gemma4",
];

export const PIPELINE_TYPE_CHOICES: TextGenPipelineType[] = [
  "AUTO",
  "LM",
  "LM_CB",
  "VLM",
  "VLM_CB",
];

export const POOLING_CHOICES: EmbeddingsPooling[] = ["CLS", "LAST", "MEAN"];

/**
 * Built-in OpenVINO device shortcuts surfaced in the UI. Mirrors the set
 * accepted by upstream `export_model.py --target_device` (`CPU`, `GPU`,
 * `NPU`, `HETERO`) plus `AUTO` and `MULTI` from the OVMS docs. Free-form
 * strings (`MULTI:CPU,GPU`, `HETERO:GPU,CPU`, ...) are still allowed via
 * the custom input — `MULTI` / `HETERO` pre-seed the editor with a sane
 * starting suffix.
 */
export const DEVICE_CHOICES: readonly string[] = [
  "CPU",
  "GPU",
  "NPU",
  "AUTO",
  "HETERO",
  "MULTI",
];

/**
 * Devices that aren't usable on their own — they need a `:DEV1,DEV2`
 * suffix at minimum (e.g. `MULTI:GPU,CPU`). When the user picks one of
 * these from the shortcut list, the device editor auto-flips into custom
 * mode pre-seeded with a sensible suffix.
 */
export const COMPOUND_DEVICE_SEEDS: Readonly<Record<string, string>> = {
  HETERO: "HETERO:GPU,CPU",
  MULTI: "MULTI:GPU,CPU",
};
