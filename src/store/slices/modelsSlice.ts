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
  loadedModel: null,
  loadedModels: [],
  loadedModelsByType: {
    text: null,
    "image-to-text": null,
    "image-gen": null,
    "speech-to-text": null,
    "text-to-speech": null,
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
  setLoadedModel: (modelId) => {
    logStateChange("models", "setLoadedModel", { modelId });
    set({ loadedModel: modelId });
  },

  setLoadedModels: (modelIds) => {
    logStateChange("models", "setLoadedModels", { count: modelIds.length });
    set({
      loadedModels: modelIds,
      loadedModel: modelIds.length > 0 ? modelIds[0] : null, // Set first as primary
    });
  },

  getLoadedModels: async () => {
    try {
      const modelNames: string[] = await invoke("get_loaded_models");
      logInfo("Loaded models from config", {
        count: modelNames.length,
        models: modelNames,
      });
      set({
        loadedModels: modelNames,
        loadedModel: modelNames.length > 0 ? modelNames[0] : null,
      });
      return modelNames;
    } catch (error) {
      logError("Failed to get loaded models", error as Error);
      return [];
    }
  },

  getLoadedModel: async () => {
    try {
      const modelNames: string[] = await invoke("get_loaded_models");

      if (modelNames.length > 0) {
        // Get first model from config
        const modelId = modelNames[0].startsWith("OpenVINO/")
          ? modelNames[0]
          : `OpenVINO/${modelNames[0]}`;
        logInfo("Model loaded from config", {
          modelId,
          total: modelNames.length,
        });
        set({ loadedModel: modelId, loadedModels: modelNames });
        return modelId;
      }

      set({ loadedModel: null, loadedModels: [] });
      return null;
    } catch (error) {
      logError("Failed to get loaded model", error as Error);
      set({ loadedModel: null, loadedModels: [] });
      return null;
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
