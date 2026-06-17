import type { ModelTaskKind } from "./modelParams";

/**
 * Manually curated set of OpenVINO IR models the UI can recommend
 * per task role.  Entries point to HuggingFace model ids that are
 * known to ship OpenVINO IR blobs (so the regular download + `graph.pbtxt`
 * pipeline works unchanged).
 *
 * Each entry is a suggestion — the user still picks which one to install,
 * and every entry can be opened with the Configure… modal for
 * fine‑tuning task / device overrides before download.
 */

export interface RecommendedModel {
  /** HuggingFace model id, e.g. `OpenVINO/Qwen2.5-7B-Instruct-int4-ov`. */
  hfId: string;
  /** Short name displayed on the card, e.g. "Qwen 2.5 7B". */
  displayName: string;
  /** One‑line description, e.g. "Fast 7B general‑purpose chat model". */
  blurb: string;
  /** Approximate on‑disk size (human‑readable hint). */
  sizeHint: string;
}

/** Map of task‑kind → recommended models in priority order. */
export const RECOMMENDED_BY_KIND: Record<
  ModelTaskKind,
  RecommendedModel[]
> = {
  text_generation: [
    {
      hfId: "OpenVINO/Qwen2.5-7B-Instruct-int4-ov",
      displayName: "Qwen 2.5 7B",
      blurb: "Strong all‑round 7B chat & reasoning model",
      sizeHint: "~4.4 GB",
    },
    {
      hfId: "OpenVINO/Phi-3.5-mini-instruct-int4-ov",
      displayName: "Phi‑3.5 Mini",
      blurb: "Compact 3.8B model, great on CPU / NPU",
      sizeHint: "~2.2 GB",
    },
    {
      hfId: "OpenVINO/Llama-3.2-3B-Instruct-int4-ov",
      displayName: "Llama 3.2 3B",
      blurb: "Meta's lightweight multi‑lingual chat model",
      sizeHint: "~2.0 GB",
    },
    {
      hfId: "OpenVINO/gemma-2-2b-it-int4-ov",
      displayName: "Gemma 2 2B",
      blurb: "Google's fast & compact instruction‑tuned model",
      sizeHint: "~1.4 GB",
    },
  ],
  embeddings: [
    {
      hfId: "OpenVINO/bge-small-en-v1.5-int4-ov",
      displayName: "BGE Small EN v1.5",
      blurb: "Tiny 384‑dim embedding model, great for RAG",
      sizeHint: "~35 MB",
    },
    {
      hfId: "OpenVINO/bge-large-en-v1.5-int4-ov",
      displayName: "BGE Large EN v1.5",
      blurb: "Higher‑quality 1024‑dim embeddings",
      sizeHint: "~340 MB",
    },
    {
      hfId: "OpenVINO/e5-large-v2-int4-ov",
      displayName: "E5 Large v2",
      blurb: "Microsoft's general‑purpose text embedder",
      sizeHint: "~340 MB",
    },
  ],
  rerank: [
    {
      hfId: "OpenVINO/bge-reranker-v2-m3-int4-ov",
      displayName: "BGE Reranker v2 M3",
      blurb: "Cross‑encoder re‑ranker, multi‑lingual",
      sizeHint: "~570 MB",
    },
    {
      hfId: "OpenVINO/bge-reranker-large-int4-ov",
      displayName: "BGE Reranker Large",
      blurb: "Higher accuracy English re‑ranker",
      sizeHint: "~330 MB",
    },
  ],
  image_generation: [
    {
      hfId: "OpenVINO/stable-diffusion-v1-5-int8-ov",
      displayName: "Stable Diffusion 1.5",
      blurb: "Classic text‑to‑image diffusion model",
      sizeHint: "~1.1 GB",
    },
    {
      hfId: "OpenVINO/LCM-Dreamshaper-v7-int4-ov",
      displayName: "LCM Dreamshaper v7",
      blurb: "Fast few‑step latent consistency model",
      sizeHint: "~2.0 GB",
    },
    {
      hfId: "OpenVINO/FLUX.1-schnell-int4-ov",
      displayName: "FLUX.1 Schnell",
      blurb: "High‑quality fast image generation",
      sizeHint: "~6.0 GB",
    },
  ],
  speech2text: [
    {
      hfId: "OpenVINO/whisper-tiny-int4-ov",
      displayName: "Whisper Tiny",
      blurb: "Lightweight multi‑lingual ASR, runs on CPU",
      sizeHint: "~40 MB",
    },
    {
      hfId: "OpenVINO/whisper-base-int4-ov",
      displayName: "Whisper Base",
      blurb: "Better accuracy than Tiny, still CPU‑friendly",
      sizeHint: "~75 MB",
    },
    {
      hfId: "OpenVINO/whisper-small-int4-ov",
      displayName: "Whisper Small",
      blurb: "Good balance of speed and transcription quality",
      sizeHint: "~250 MB",
    },
  ],
  text2speech: [
    {
      hfId: "OpenVINO/kokoro-v0.19-int4-ov",
      displayName: "Kokoro v0.19",
      blurb: "High‑quality neural TTS with multiple voices",
      sizeHint: "~80 MB",
    },
    {
      hfId: "OpenVINO/bark-small-int4-ov",
      displayName: "Bark Small",
      blurb: "Multi‑lingual TTS with expressive tones",
      sizeHint: "~450 MB",
    },
  ],
};

/** Flattened convenience list used by the global search popover. */
export const ALL_RECOMMENDED: (RecommendedModel & {
  kind: ModelTaskKind;
})[] = (Object.entries(RECOMMENDED_BY_KIND) as [ModelTaskKind, RecommendedModel[]][])
  .flatMap(([kind, models]) =>
    models.map((m) => ({ ...m, kind })),
  );
