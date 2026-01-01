import type { ModelCategory } from "@/store/types";

/**
 * Categorizes a model based on its ID/name
 * Returns null for embedding/reranker models
 */
export const categorizeModel = (modelId: string): ModelCategory | null => {
  const lowerModelId = modelId.toLowerCase();

  if (lowerModelId.includes("reranker")) {
    return "reranker";
  }

  if (lowerModelId.includes("embedding")) {
    return "embedding";
  }

  if (
    lowerModelId.includes("flux") ||
    lowerModelId.includes("stable-diffusion") ||
    lowerModelId.includes("sd-") ||
    lowerModelId.includes("image-generation") ||
    lowerModelId.includes("imagegen")
  ) {
    return "image-gen";
  }

  if (
    lowerModelId.includes("vision") ||
    lowerModelId.includes("llava") ||
    lowerModelId.includes("minicpm-v") ||
    lowerModelId.includes("phi-3-vision") ||
    lowerModelId.includes("image-to-text") ||
    lowerModelId.includes("vl-")
  ) {
    return "image-to-text";
  }

  if (
    lowerModelId.includes("whisper") ||
    lowerModelId.includes("speech-to-text") ||
    lowerModelId.includes("speech2text") ||
    lowerModelId.includes("stt")
  ) {
    return "speech-to-text";
  }

  if (
    lowerModelId.includes("tts") ||
    lowerModelId.includes("text-to-speech") ||
    lowerModelId.includes("text2speech") ||
    lowerModelId.includes("speecht5") ||
    lowerModelId.includes("bark")
  ) {
    return "text-to-speech";
  }

  return "text";
};

/**
 * Gets display name for a model category
 */
export const getCategoryDisplayName = (category: ModelCategory): string => {
  const names: Record<ModelCategory, string> = {
    text: "Text",
    "image-to-text": "Vision",
    "image-gen": "Image Gen",
    "speech-to-text": "STT",
    "text-to-speech": "TTS",
    embedding: "Embedding",
    reranker: "Reranker",
  };
  return names[category];
};

/**
 * Gets full display name for a model category
 */
export const getCategoryFullName = (category: ModelCategory): string => {
  const names: Record<ModelCategory, string> = {
    text: "Text Generation",
    "image-to-text": "Vision (Image-to-Text)",
    "image-gen": "Image Generation",
    "speech-to-text": "Speech-to-Text",
    "text-to-speech": "Text-to-Speech",
    embedding: "Embedding Models",
    reranker: "Reranker Models",
  };
  return names[category];
};

/**
 * Gets Tailwind CSS color classes for a model category
 */
export const getCategoryColor = (category: ModelCategory): string => {
  const colors: Record<ModelCategory, string> = {
    text: "bg-blue-100 dark:bg-blue-900/30 text-blue-700 dark:text-blue-300",
    "image-to-text":
      "bg-purple-100 dark:bg-purple-900/30 text-purple-700 dark:text-purple-300",
    "image-gen":
      "bg-pink-100 dark:bg-pink-900/30 text-pink-700 dark:text-pink-300",
    "speech-to-text":
      "bg-green-100 dark:bg-green-900/30 text-green-700 dark:text-green-300",
    "text-to-speech":
      "bg-orange-100 dark:bg-orange-900/30 text-orange-700 dark:text-orange-300",
    embedding:
      "bg-cyan-100 dark:bg-cyan-900/30 text-cyan-700 dark:text-cyan-300",
    reranker:
      "bg-indigo-100 dark:bg-indigo-900/30 text-indigo-700 dark:text-indigo-300",
  };
  return colors[category];
};
