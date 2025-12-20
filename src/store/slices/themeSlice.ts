import { StateCreator } from "zustand";
import type { AppState, ThemeSlice } from "../types";

export const createThemeSlice: StateCreator<AppState, [], [], ThemeSlice> = (
  set
) => ({
  themeMode: "dark",
  themeColor: "orange",

  setThemeMode: (mode) => set({ themeMode: mode }),
  setThemeColor: (color) => set({ themeColor: color }),
  toggleThemeMode: () =>
    set((state) => ({
      themeMode: state.themeMode === "dark" ? "light" : "dark",
    })),
});
