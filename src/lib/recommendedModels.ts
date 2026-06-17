import type { ModelTaskKind } from "./modelParams";

/**
 * A model recommendation returned by the backend `system_recommend_models`
 * command.  The backend ranks entries by hardware fit (not displayed to
 * the user — the list is already sorted best‑first).
 *
 * For `text_generation` the catalog is enriched with live HuggingFace
 * search results (int8 qwen3/gemma4/phi3.5); all other categories use a
 * curated static list scored against the local system specs.
 */

export interface RecommendedModel {
  /** Exact HuggingFace model id, e.g. `OpenVINO/Qwen3.5-4B-int8-ov`. */
  hfId: string;
  /** One‑line description shown below the name. */
  blurb: string;
  /** Approximate on‑disk size (human‑readable). */
  sizeHint: string;
}

/**
 * Map returned by the backend: task‑kind → ranked recommendations.
 * Use `RECOMMENDED_BY_KIND` as the static fallback when the backend
 * hasn't responded yet.
 */
export type RecommendationMap = Partial<
  Record<ModelTaskKind, RecommendedModel[]>
>;

/** Static fallback — shown while the backend is fetching. */
export const RECOMMENDED_BY_KIND: Record<ModelTaskKind, RecommendedModel[]> = {
  text_generation: [
    // The backend will replace these with live HF results.
  ],
  embeddings: [],
  rerank: [],
  image_generation: [],
  speech2text: [],
  text2speech: [],
};

/** Flattened convenience list for the global search popover. */
export const ALL_RECOMMENDED: (RecommendedModel & {
  kind: ModelTaskKind;
})[] = (
  Object.entries(RECOMMENDED_BY_KIND) as [ModelTaskKind, RecommendedModel[]][]
).flatMap(([kind, models]) => models.map((m) => ({ ...m, kind })));
