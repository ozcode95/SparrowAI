import { StateCreator } from "zustand";
import type { AppState, UISlice } from "../types";

export const createUiSlice: StateCreator<AppState, [], [], UISlice> = (
  set
) => ({
  sidebarCollapsed: false,
  settingsDialogOpen: false,
  currentPage: "chat",
  notification: null,

  setSidebarCollapsed: (collapsed) => set({ sidebarCollapsed: collapsed }),
  toggleSidebar: () =>
    set((state) => ({ sidebarCollapsed: !state.sidebarCollapsed })),
  setSettingsDialogOpen: (open) => set({ settingsDialogOpen: open }),
  setCurrentPage: (page) => set({ currentPage: page }),

  showNotification: (message, type = "info", timeout = null) =>
    set({
      notification: { message, type, timestamp: Date.now(), timeout },
    }),
  clearNotification: () => set({ notification: null }),
});
