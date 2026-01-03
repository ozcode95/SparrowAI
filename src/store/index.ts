import { create } from "zustand";
import { persist, createJSONStorage } from "zustand/middleware";
import { useShallow } from "zustand/react/shallow";
import { createThemeSlice } from "./slices/themeSlice";
import { createUiSlice } from "./slices/uiSlice";
import { createModelsSlice } from "./slices/modelsSlice";
import { createSettingsSlice } from "./slices/settingsSlice";
import { createChatSlice } from "./slices/chatSlice";
import { createGallerySlice } from "./slices/gallerySlice";
import type { AppState } from "./types";

export const useAppStore = create<AppState>()(
  persist(
    (...a) => ({
      ...createThemeSlice(...a),
      ...createUiSlice(...a),
      ...createModelsSlice(...a),
      ...createSettingsSlice(...a),
      ...createChatSlice(...a),
      ...createGallerySlice(...a),
    }),
    {
      name: "sparrow-app-state",
      storage: createJSONStorage(() => localStorage),
      partialize: (state) => ({
        themeMode: state.themeMode,
        themeColor: state.themeColor,
        sidebarCollapsed: state.sidebarCollapsed,
        settings: state.settings,
        downloadedModels: Array.from(state.downloadedModels),
      }),
      merge: (persistedState: any, currentState) => {
        return {
          ...currentState,
          ...persistedState,
          downloadedModels: new Set(persistedState?.downloadedModels || []),
          downloadingModels: new Set(),
          downloadProgress: {},
          notification: null,
          settingsDialogOpen: false,
          selectedModel: null,
          isSearching: false,
          isOvmsRunning: false,
        };
      },
    }
  )
);

// Convenient hooks for specific slices
export const useTheme = () =>
  useAppStore(
    useShallow((state) => ({
      themeMode: state.themeMode,
      themeColor: state.themeColor,
      setThemeMode: state.setThemeMode,
      setThemeColor: state.setThemeColor,
      toggleThemeMode: state.toggleThemeMode,
    }))
  );

export const useUI = () =>
  useAppStore(
    useShallow((state) => ({
      sidebarCollapsed: state.sidebarCollapsed,
      settingsDialogOpen: state.settingsDialogOpen,
      currentPage: state.currentPage,
      notification: state.notification,
      setSidebarCollapsed: state.setSidebarCollapsed,
      toggleSidebar: state.toggleSidebar,
      setSettingsDialogOpen: state.setSettingsDialogOpen,
      setCurrentPage: state.setCurrentPage,
      showNotification: state.showNotification,
      clearNotification: state.clearNotification,
    }))
  );

export const useModels = () =>
  useAppStore(
    useShallow((state) => ({
      searchQuery: state.searchQuery,
      searchResults: state.searchResults,
      selectedModel: state.selectedModel,
      isSearching: state.isSearching,
      downloadingModels: state.downloadingModels,
      downloadProgress: state.downloadProgress,
      downloadedModels: state.downloadedModels,
      isOvmsRunning: state.isOvmsRunning,
      setSearchQuery: state.setSearchQuery,
      setSearchResults: state.setSearchResults,
      setSelectedModel: state.setSelectedModel,
      setIsSearching: state.setIsSearching,
      clearSearch: state.clearSearch,
      setModelDownloading: state.setModelDownloading,
      isModelDownloading: state.isModelDownloading,
      isModelDownloaded: state.isModelDownloaded,
      setDownloadProgress: state.setDownloadProgress,
      getDownloadProgress: state.getDownloadProgress,
      addDownloadedModel: state.addDownloadedModel,
      removeDownloadedModel: state.removeDownloadedModel,
      setDownloadedModels: state.setDownloadedModels,
      setIsOvmsRunning: state.setIsOvmsRunning,
      loadedModels: state.loadedModels,
      setLoadedModels: state.setLoadedModels,
      getLoadedModels: state.getLoadedModels,
      loadedModelsByType: state.loadedModelsByType,
      setLoadedModelByType: state.setLoadedModelByType,
      getLoadedModelByType: state.getLoadedModelByType,
    }))
  );

export const useSettings = () =>
  useAppStore(
    useShallow((state) => ({
      settings: state.settings,
      updateSettings: state.updateSettings,
      resetSettings: state.resetSettings,
      updateSetting: state.updateSetting,
    }))
  );

export const useChat = () =>
  useAppStore(
    useShallow((state) => ({
      chatSessions: state.chatSessions,
      activeChatSessionId: state.activeChatSessionId,
      currentChatMessages: state.currentChatMessages,
      temporarySession: state.temporarySession,
      isStreaming: state.isStreaming,
      setChatSessions: state.setChatSessions,
      setActiveChatSessionId: state.setActiveChatSessionId,
      setCurrentChatMessages: state.setCurrentChatMessages,
      setTemporarySession: state.setTemporarySession,
      addChatSession: state.addChatSession,
      updateChatSession: state.updateChatSession,
      removeChatSession: state.removeChatSession,
      addMessageToCurrentChat: state.addMessageToCurrentChat,
      clearCurrentChatMessages: state.clearCurrentChatMessages,
      clearTemporarySession: state.clearTemporarySession,
      getActiveSession: state.getActiveSession,
      getChatSessionsArray: state.getChatSessionsArray,
      getRecentChatSessions: state.getRecentChatSessions,
    }))
  );

export const useGallery = () =>
  useAppStore(
    useShallow((state) => ({
      generatedImages: state.generatedImages,
      isGenerating: state.isGenerating,
      currentGeneratingImage: state.currentGeneratingImage,
      setGeneratedImages: state.setGeneratedImages,
      addGeneratedImage: state.addGeneratedImage,
      setIsGenerating: state.setIsGenerating,
      setCurrentGeneratingImage: state.setCurrentGeneratingImage,
      clearGallery: state.clearGallery,
    }))
  );

// Export types
export type * from "./types";
