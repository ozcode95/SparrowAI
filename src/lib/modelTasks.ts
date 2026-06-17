/**
 * UI metadata for the six OVMS task categories.
 *
 * The Models page groups every local model into one of these buckets so
 * the user can see at a glance which capabilities they have covered
 * (chat / embeddings / rerank / image-gen / STT / TTS). Loaded models
 * report their `task` directly via `OvmsInfo.loaded_models`; for
 * not-yet-loaded local rows we fall back to a heuristic mapping from
 * the HuggingFace `pipeline_tag` captured at download time.
 */

import type { ModelTaskKind } from "./modelParams";

export interface TaskCategory {
  kind: ModelTaskKind;
  /** Short noun used in section headers and overview chips. */
  label: string;
  /** One-line elevator pitch shown when the category is empty. */
  blurb: string;
  /** Search hint pre-filled when the user clicks "Find a model". */
  searchHint: string;
  /** Inline SVG path data for the 16×16 icon (single `<path d=…/>`). */
  iconPath: string;
}

/**
 * Ordered list — drives the rendering order of the capability overview
 * and the local-models task sections. Text generation comes first
 * because it's the primary chat surface; the rest follow in roughly
 * declining ubiquity.
 */
export const TASK_CATEGORIES: readonly TaskCategory[] = [
  {
    kind: "text_generation",
    label: "Chat & completion",
    blurb: "Conversational LLM that answers prompts and uses tools.",
    searchHint: "Qwen",
    iconPath:
      "M21 12a8 8 0 0 1-11.6 7.1L3 21l1.9-6.4A8 8 0 1 1 21 12Z",
  },
  {
    kind: "embeddings",
    label: "Embeddings",
    blurb: "Vector encoder for semantic search and memory recall.",
    searchHint: "embedding",
    iconPath:
      "M4 7h16M4 12h10M4 17h7M18 14l3 3-3 3M21 17h-7",
  },
  {
    kind: "rerank",
    label: "Rerank",
    blurb: "Cross-encoder that re-orders retrieved documents by relevance.",
    searchHint: "reranker",
    iconPath:
      "M3 6h13M3 12h9M3 18h5M17 4v16m0 0-3-3m3 3 3-3",
  },
  {
    kind: "image_generation",
    label: "Image generation",
    blurb: "Diffusion model that turns text prompts into images.",
    searchHint: "stable diffusion",
    iconPath:
      "M4 5h16v14H4zM4 16l4-4 4 4 3-3 5 5M9 10a1.5 1.5 0 1 1-3 0 1.5 1.5 0 0 1 3 0Z",
  },
  {
    kind: "speech2text",
    label: "Speech → Text",
    blurb: "Whisper-style transcriber for voice notes and dictation.",
    searchHint: "whisper",
    iconPath:
      "M12 3a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3ZM5 11a7 7 0 0 0 14 0M12 18v3M8 21h8",
  },
  {
    kind: "text2speech",
    label: "Text → Speech",
    blurb: "Voice synthesizer for read-aloud and assistant responses.",
    searchHint: "text to speech",
    iconPath:
      "M3 10v4h3l5 4V6L6 10H3zM16 8a5 5 0 0 1 0 8M19 5a9 9 0 0 1 0 14",
  },
];

const CATEGORY_BY_KIND: Record<ModelTaskKind, TaskCategory> = Object.fromEntries(
  TASK_CATEGORIES.map((c) => [c.kind, c]),
) as Record<ModelTaskKind, TaskCategory>;

export function categoryFor(kind: ModelTaskKind): TaskCategory {
  return CATEGORY_BY_KIND[kind];
}

/**
 * Best-effort mapping from a HuggingFace `pipeline_tag` to one of our
 * six OVMS task buckets. Used to categorise local models that aren't
 * loaded yet (a loaded model reports its real `task` directly). When
 * the tag is unknown we default to `text_generation` because that's
 * the overwhelming majority of OpenVINO IR uploads on HF.
 */
export function taskKindFromPipelineTag(
  tag: string | null | undefined,
): ModelTaskKind {
  if (!tag) return "text_generation";
  switch (tag) {
    case "text-generation":
    case "text2text-generation":
    case "image-text-to-text":
    case "conversational":
      return "text_generation";
    case "feature-extraction":
    case "sentence-similarity":
      return "embeddings";
    case "text-classification":
    case "text-ranking":
      return "rerank";
    case "text-to-image":
    case "image-to-image":
      return "image_generation";
    case "automatic-speech-recognition":
      return "speech2text";
    case "text-to-speech":
    case "text-to-audio":
      return "text2speech";
    default:
      return "text_generation";
  }
}
