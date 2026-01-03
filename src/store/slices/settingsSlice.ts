import { StateCreator } from "zustand";
import type { AppState, SettingsSlice, Settings } from "../types";
import { logStateChange } from "../../lib/logger";

const defaultSettings: Settings = {
  includeConversationHistory: true,
  systemPrompt: "You're an AI assistant that provides helpful responses.",
  temperature: 0.7,
  topP: 1.0,
  seed: null,
  maxTokens: 2048,
  maxCompletionTokens: null,
  useRAG: false,
  enableAutostart: false,
  startMinimized: true,
};

export const createSettingsSlice: StateCreator<
  AppState,
  [],
  [],
  SettingsSlice
> = (set) => ({
  settings: defaultSettings,

  updateSettings: (newSettings) => {
    logStateChange("settings", "updateSettings", {
      keys: Object.keys(newSettings),
    });
    set((state) => ({
      settings: { ...state.settings, ...newSettings },
    }));
  },

  resetSettings: () => set({ settings: defaultSettings }),

  updateSetting: (key, value) => {
    logStateChange("settings", "updateSetting", { key, value });
    set((state) => ({
      settings: { ...state.settings, [key]: value },
    }));
  },
});
