import { create } from "zustand";
import { invoke } from "@/lib/tauri";

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
}));
