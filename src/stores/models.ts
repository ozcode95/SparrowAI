import { create } from "zustand";
import { invoke, on, Events } from "@/lib/tauri";
import type { ModelDeployParams } from "@/lib/modelParams";

export interface HfModelSummary {
  id: string;
  author: string;
  downloads: number;
  likes: number;
  updated_at: string;
  tags: string[];
  pipeline_tag?: string | null;
  has_openvino_ir: boolean;
  total_size_bytes: number | null;
}

export interface LocalModel {
  id: string;
  path: string;
  bytes: number;
  added_at: string;
  hf_id: string | null;
  revision: string | null;
  files: number | null;
  /**
   * Number of files whose sha256 was recorded at download time (i.e.
   * verified against an HF-published LFS digest). `null` when the local
   * manifest pre-dates verification tracking — the next install/update will
   * backfill it. `0` means we have no integrity coverage at all.
   */
  verified_files?: number | null;
  /**
   * Upstream HuggingFace `pipeline_tag` captured at download time —
   * e.g. `text-generation`, `image-text-to-text`, `feature-extraction`.
   * The backend backfills this from the per-model sidecar for rows
   * that pre-date the column (final fallback: `text-generation`), so
   * callers can treat the absence of a value as "truly unknown" rather
   * than "legacy install".
   */
  pipeline_tag?: string | null;
}

export interface DownloadProgress {
  model_id: string;
  bytes_done: number;
  bytes_total: number | null;
  files_done: number;
  files_total: number;
  state:
    | "pending"
    | "downloading"
    | "verifying"
    | "done"
    | "cancelled"
    | "error";
  error?: string | null;
}

interface ModelsState {
  query: string;
  results: HfModelSummary[];
  searching: boolean;
  local: LocalModel[];
  downloads: Record<string, DownloadProgress>;
  setQuery: (q: string) => void;
  search: () => Promise<void>;
  refreshLocal: () => Promise<void>;
  download: (id: string, params?: ModelDeployParams) => Promise<void>;
  cancel: (id: string) => Promise<void>;
  remove: (id: string) => Promise<void>;
  update: (id: string) => Promise<void>;
  /**
   * Forget a download entry from local state. Used by the Models card
   * grid to evict failed / cancelled downloads on sight so the user
   * isn't left staring at a dead card — the actual on-disk artifacts
   * (if any) are still cleaned up by the backend.
   */
  dismissDownload: (id: string) => void;
  getParams: (id: string) => Promise<ModelDeployParams>;
  /**
   * Ask the backend to infer task / pipeline type / tool parser from
   * upstream HF metadata + `config.json`. Used by the pre-download
   * "configure…" modal so the user sees model-aware defaults instead
   * of plain text-generation values. Falls back to defaults on error.
   */
  suggestParams: (id: string) => Promise<ModelDeployParams>;
  setParams: (id: string, params: ModelDeployParams) => Promise<void>;
  bindEvents: () => Promise<() => void>;
}

export const useModelsStore = create<ModelsState>((set, get) => ({
  query: "",
  results: [],
  searching: false,
  local: [],
  downloads: {},

  setQuery: (q) => set({ query: q }),

  search: async () => {
    set({ searching: true });
    try {
      // The Models page is OpenVINO-only by product decision — the
      // runner can't load anything else — so we always scope the HF
      // query to that. Empty user input becomes a plain "OpenVINO"
      // search so the popover stays useful on first focus.
      const raw = get().query.trim();
      const scoped = raw ? `OpenVINO ${raw}` : "OpenVINO";
      const all =
        (await invoke<HfModelSummary[]>("models_search", {
          query: scoped,
        })) ?? [];
      // Defensive filter: drop anything the HF index returned without
      // an OpenVINO IR sibling. Cheap and protects us if the search
      // term ever drifts away from the OpenVINO namespace.
      const results = all.filter((m) => m.has_openvino_ir);
      set({ results, searching: false });
    } catch (e) {
      console.error("models_search failed", e);
      set({ searching: false });
    }
  },

  refreshLocal: async () => {
    try {
      const local = (await invoke<LocalModel[]>("models_list_local")) ?? [];
      set({ local });
    } catch (e) {
      console.error("models_list_local failed", e);
    }
  },

  download: async (id, params) => {
    try {
      await invoke("models_download", { modelId: id, params: params ?? null });
    } catch (e) {
      console.error("models_download failed", e);
    }
  },

  cancel: async (id) => {
    try {
      await invoke<boolean>("models_cancel", { modelId: id });
    } catch (e) {
      console.error("models_cancel failed", e);
    }
  },

  remove: async (id) => {
    await invoke("models_delete", { modelId: id });
    await get().refreshLocal();
  },

  update: async (id) => {
    try {
      await invoke("models_update", { modelId: id });
    } catch (e) {
      console.error("models_update failed", e);
    }
  },

  getParams: async (id) => {
    return await invoke<ModelDeployParams>("models_get_params", {
      modelId: id,
    });
  },

  suggestParams: async (id) => {
    return await invoke<ModelDeployParams>("models_suggest_params", {
      modelId: id,
    });
  },

  setParams: async (id, params) => {
    await invoke("models_set_params", { modelId: id, params });
  },

  dismissDownload: (id) => {
    set((s) => {
      if (!(id in s.downloads)) return s;
      const { [id]: _drop, ...rest } = s.downloads;
      return { downloads: rest };
    });
  },

  bindEvents: async () => {
    const off = await on<DownloadProgress>(
      Events.ModelDownloadProgress,
      (p) => {
        set((s) => {
          // Terminal failure states are evicted on arrival so the
          // Models card grid never has to render a dead "error" card.
          // The download invocation itself already surfaced the error
          // to the user via the result panel before it closed.
          if (p.state === "error" || p.state === "cancelled") {
            if (!(p.model_id in s.downloads)) return s;
            const { [p.model_id]: _drop, ...rest } = s.downloads;
            return { downloads: rest };
          }
          return { downloads: { ...s.downloads, [p.model_id]: p } };
        });
        if (p.state === "done") void get().refreshLocal();
      },
    );
    return off;
  },
}));
