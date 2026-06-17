import { create } from "zustand";

export type ViewId =
  | "chat"
  | "models"
  | "tasks"
  | "memory"
  | "tools"
  | "skills"
  | "settings";

interface UiState {
  view: ViewId;
  commandPaletteOpen: boolean;
  setView: (v: ViewId) => void;
  toggleCommandPalette: () => void;
  bindHotkeys: () => () => void;
}

export const useUiStore = create<UiState>((set, get) => ({
  view: "chat",
  commandPaletteOpen: false,

  setView: (v) => set({ view: v }),

  toggleCommandPalette: () =>
    set((s) => ({ commandPaletteOpen: !s.commandPaletteOpen })),

  bindHotkeys: () => {
    const handler = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;
      if (!mod) return;
      if (e.key === "k" || e.key === "K") {
        e.preventDefault();
        get().toggleCommandPalette();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  },
}));
