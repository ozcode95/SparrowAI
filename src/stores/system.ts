import { create } from "zustand";
import { invoke } from "@/lib/tauri";
import type { RecommendationMap } from "@/lib/recommendedModels";

export interface GpuInfo {
  name: string;
  vendor: string;
  vram_mb: number | null;
  kind: "integrated" | "discrete" | "unknown";
}

export interface NpuInfo {
  name: string;
  vendor: string;
}

export interface SystemSpecs {
  os: string;
  os_version: string;
  arch: string;
  cpu_brand: string;
  cpu_vendor: string;
  cpu_physical_cores: number;
  cpu_logical_cores: number;
  ram_total_mb: number;
  ram_usable_mb: number;
  gpus: GpuInfo[];
  npus: NpuInfo[];
  probed_at: string;
}

interface SystemState {
  specs: SystemSpecs | null;
  loading: boolean;
  error: string | null;
  probe: (force?: boolean) => Promise<void>;
  /**
   * Fetch spec-ranked model recommendations for every task category.
   * Returns a map of task‑kind → model list, sorted best‑fit‑first.
   * Falls back to a static catalog when offline.
   */
  recommendModels: () => Promise<RecommendationMap>;
}

export const useSystemStore = create<SystemState>((set) => ({
  specs: null,
  loading: false,
  error: null,
  probe: async (force = false) => {
    set({ loading: true, error: null });
    try {
      const specs = await invoke<SystemSpecs>("system_probe", { force });
      set({ specs, loading: false });
    } catch (e) {
      set({ error: String(e), loading: false });
    }
  },
  recommendModels: async () => {
    try {
      const map = await invoke<RecommendationMap>("system_recommend_models");
      return map ?? {};
    } catch (e) {
      console.error("system_recommend_models failed", e);
      return {};
    }
  },
}));
