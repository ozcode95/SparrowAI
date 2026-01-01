import { StateCreator } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { AppState, ModelsSlice } from "../types";
import { logStateChange, logError, logInfo } from "../../lib/logger";

export const createModelsSlice: StateCreator<AppState, [], [], ModelsSlice> = (
  set,
  get
) => ({
  searchQuery: "",
  searchResults: [],
  selectedModel: null,
  isSearching: false,
  downloadingModels: new Set(),
  downloadProgress: {},
  downloadedModels: new Set(),
  isOvmsRunning: false,
  loadedModels: [],
  loadedModelsByType: {
    text: null,
    "image-to-text": null,
    "image-gen": null,
    "speech-to-text": null,
    "text-to-speech": null,
    embedding: null,
    reranker: null,
  },

  setSearchQuery: (query) => set({ searchQuery: query }),
  setSearchResults: (results) => set({ searchResults: results }),
  setSelectedModel: (model) => set({ selectedModel: model }),
  setIsSearching: (isSearching) => set({ isSearching }),

  clearSearch: () =>
    set({
      searchQuery: "",
      searchResults: [],
      selectedModel: null,
    }),

  setModelDownloading: (modelId, isDownloading) => {
    logStateChange("models", "setModelDownloading", { modelId, isDownloading });
    set((state) => {
      const newDownloadingModels = new Set(state.downloadingModels);
      if (isDownloading) {
        newDownloadingModels.add(modelId);
      } else {
        newDownloadingModels.delete(modelId);
      }
      return { downloadingModels: newDownloadingModels };
    });
  },

  isModelDownloading: (modelId) => get().downloadingModels.has(modelId),
  hasAnyDownloading: () => get().downloadingModels.size > 0,
  isModelDownloaded: (modelId) => get().downloadedModels.has(modelId),

  setDownloadProgress: (modelId, progress, currentFile = "") =>
    set((state) => ({
      downloadProgress: {
        ...state.downloadProgress,
        [modelId]: { progress, currentFile },
      },
    })),

  getDownloadProgress: (modelId) =>
    get().downloadProgress[modelId] || { progress: 0, currentFile: "" },

  addDownloadedModel: (modelId) =>
    set((state) => {
      const newDownloadedModels = new Set(state.downloadedModels);
      newDownloadedModels.add(modelId);
      return { downloadedModels: newDownloadedModels };
    }),

  removeDownloadedModel: (modelId) =>
    set((state) => {
      const newDownloadedModels = new Set(state.downloadedModels);
      newDownloadedModels.delete(modelId);
      return { downloadedModels: newDownloadedModels };
    }),

  setDownloadedModels: (modelIds) =>
    set({ downloadedModels: new Set(modelIds) }),

  setIsOvmsRunning: (isRunning) => set({ isOvmsRunning: isRunning }),

  setLoadedModels: (modelIds) => {
    logStateChange("models", "setLoadedModels", { count: modelIds.length });
    set({
      loadedModels: modelIds,
    });
  },

  getLoadedModels: async () => {
    try {
      const modelNames: string[] = await invoke("get_loaded_models");
      logInfo("Loaded models from config", {
        count: modelNames.length,
        models: modelNames,
      });

      // Get metadata to categorize models by type
      try {
        const metadata = await invoke<
          Record<
            string,
            { model_id: string; model_type: string; pipeline_tag: string }
          >
        >("get_all_model_metadata");

        // Map model types from metadata to loadedModelsByType
        const loadedByType: Record<string, string | null> = {
          text: null,
          "image-to-text": null,
          "image-gen": null,
          "speech-to-text": null,
          "text-to-speech": null,
          embedding: null,
          reranker: null,
        };

        // Categorize each loaded model
        modelNames.forEach((modelName) => {
          // Try with OpenVINO/ prefix first (metadata key format)
          const fullModelId = `OpenVINO/${modelName}`;
          let modelMetadata = metadata[fullModelId];

          // If not found, try without prefix
          if (!modelMetadata) {
            modelMetadata = metadata[modelName];
          }

          if (modelMetadata) {
            const modelType = modelMetadata.model_type;
            // Map backend model types to frontend categories
            // Use full model ID for storage
            const modelIdToStore = fullModelId;

            if (modelType === "text") {
              loadedByType.text = modelIdToStore;
            } else if (
              modelType === "vision" ||
              modelType === "image-to-text"
            ) {
              loadedByType["image-to-text"] = modelIdToStore;
            } else if (modelType === "embedding") {
              loadedByType.embedding = modelIdToStore;
            } else if (modelType === "reranker") {
              loadedByType.reranker = modelIdToStore;
            } else if (
              modelType === "image" ||
              modelType === "image-generation"
            ) {
              loadedByType["image-gen"] = modelIdToStore;
            }
          }
        });

        logInfo("Categorized loaded models by type", { loadedByType });

        set({
          loadedModels: modelNames,
          loadedModelsByType: loadedByType as any,
        });
      } catch (metadataError) {
        logError(
          "Failed to get model metadata for categorization",
          metadataError as Error
        );
        // Still set loadedModels even if metadata fails
        set({
          loadedModels: modelNames,
        });
      }

      return modelNames;
    } catch (error) {
      logError("Failed to get loaded models", error as Error);
      return [];
    }
  },

  setLoadedModelByType: (modelType, modelId) => {
    logStateChange("models", "setLoadedModelByType", { modelType, modelId });
    set((state) => ({
      loadedModelsByType: {
        ...state.loadedModelsByType,
        [modelType]: modelId,
      },
    }));
  },

  getLoadedModelByType: (modelType) => {
    return get().loadedModelsByType[modelType];
  },
});
