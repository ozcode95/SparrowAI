import { create } from "zustand";
import { invoke, on, Events } from "@/lib/tauri";

export type OvmsStatus =
  | "not_installed"
  | "installing"
  | "installed"
  | "starting"
  | "running"
  | "stopping"
  | "stopped"
  | "error";

export interface LoadedModel {
  id: string;
  /**
   * `text_generation` | `embeddings` | `rerank` | `image_generation` |
   * `text2speech` | `speech2text`. Lets the UI route by role when
   * showing the current OVMS lineup (one of each kind can be loaded at
   * once now).
   */
  task: string;
  /** `tool_parser` declared in the model's sidecar, if any. */
  tool_parser: string | null;
  /**
   * `LM` | `LM_CB` | `VLM` | `VLM_CB` | `AUTO`. Only set for
   * text-generation models. Surfaced so the UI / chat runner can tell
   * a VLM-served model apart from a plain text LM without re-reading
   * the on-disk sidecar.
   */
  pipeline_type?: string | null;
}

// ─── TFS inspection types ────────────────────────────────────────────
//
// Mirrors the wire shape of OVMS's TensorFlow-Serving-compatible REST
// endpoints. Kept here (instead of a separate module) so the Settings
// page's model inspector can import everything it needs from the same
// place as the rest of the OVMS surface.

/** `{ error_code, error_message }` block attached to each version row. */
export interface TfsModelStatusError {
  error_code: string;
  error_message: string;
}

/** Per-version state row from `GET /v1/models/<name>` or `/v1/config`. */
export interface TfsModelVersionStatus {
  version: string;
  /** OVMS-mirrored TFS states: `AVAILABLE` / `LOADING` / `UNAVAILABLE` / `END`. */
  state: string;
  status: TfsModelStatusError;
}

export interface TfsModelStatus {
  model_version_status: TfsModelVersionStatus[];
}

/** Aggregated status map keyed by model name. */
export type TfsConfigStatus = Record<string, TfsModelStatus>;

export interface OvmsInfo {
  installed_version: string | null;
  latest_version: string | null;
  status: OvmsStatus;
  pid: number | null;
  base_url: string;
  /**
   * Every model OVMS is currently serving, insertion-ordered. Empty
   * when OVMS is stopped. Add / remove cycles are applied live via
   * `ovms_load_model` / `ovms_remove_model` — OVMS doesn't need a
   * restart for either.
   */
  loaded_models: LoadedModel[];
  /**
   * Convenience: id of the most recently added text-generation model
   * (or any model if none of the loaded models are text-gen). Kept for
   * existing chat-header / Server-page code; prefer `loaded_models`
   * when you need to route by task kind.
   */
  loaded_model: string | null;
  last_error: string | null;
  /**
   * `tool_parser` configured on `loaded_model`'s sidecar (e.g.
   * `"hermes3"`, `"llama3"`). `null` when no model is loaded *or* the
   * loaded model has no parser configured — in the latter case OVMS
   * cannot extract structured `tool_calls`, so the chat runner falls
   * back to a text-only protocol and the Server page surfaces a
   * warning chip.
   */
  loaded_model_tool_parser: string | null;
}

export interface OvmsLogLine {
  ts: string;
  level: string;
  line: string;
}

export type InstallStage =
  | "fetch_release"
  | "download"
  | "extract"
  | "verify"
  | "done"
  | "error";

export interface InstallProgress {
  stage: InstallStage;
  message: string;
  bytes_done: number;
  bytes_total: number | null;
  percent: number;
}

interface ServerState {
  info: OvmsInfo | null;
  logs: OvmsLogLine[];
  installProgress: InstallProgress | null;
  /**
   * Model IDs whose `ovms_load_model` invocation is currently
   * in-flight. Shared so the chat header banner, the Models page
   * cards, and the bottom bar can all reflect the same staging state
   * regardless of which page kicked off the load.
   */
  loadingModelIds: Set<string>;
  refresh: () => Promise<void>;
  install: () => Promise<void>;
  update: () => Promise<void>;
  start: (modelId: string) => Promise<void>;
  stop: () => Promise<void>;
  loadModel: (id: string) => Promise<void>;
  /**
   * Drop a single model from the live serving set without bouncing
   * OVMS. Other loaded models keep serving uninterrupted.
   */
  removeModel: (id: string) => Promise<void>;
  /** Stop OVMS entirely. Persisted serving set is preserved — next
   * launch restores it. */
  unloadModel: () => Promise<void>;
  /** `GET /v1/config` passthrough — aggregated TFS status of every
   * loaded model. Used by the Settings model inspector. */
  getConfigStatus: () => Promise<TfsConfigStatus>;
  /** `GET /v1/models/<id>` passthrough — single-model TFS status
   * block. Used by the bottom-bar status popover. */
  getModelStatus: (id: string) => Promise<TfsModelStatus>;
  /** `GET /v1/models/<id>/metadata` passthrough — raw TFS metadata JSON
   * (signature_def with input/output tensor shapes & dtypes). */
  getModelMetadata: (id: string) => Promise<unknown>;
  bindEvents: () => Promise<() => void>;
  clearLogs: () => void;
}

export const useServerStore = create<ServerState>((set, get) => ({
  info: null,
  logs: [],
  installProgress: null,
  loadingModelIds: new Set<string>(),

  refresh: async () => {
    try {
      const info = await invoke<OvmsInfo>("ovms_info");
      if (info) set({ info });
    } catch (e) {
      console.error("ovms_info failed", e);
    }
  },

  install: async () => {
    set({ installProgress: null });
    try {
      await invoke("ovms_install");
    } catch (e) {
      console.error("ovms_install failed", e);
    } finally {
      await get().refresh();
    }
  },

  update: async () => {
    try {
      await invoke("ovms_update");
    } catch (e) {
      console.error("ovms_update failed", e);
    } finally {
      await get().refresh();
    }
  },

  start: async (modelId) => {
    await invoke("ovms_start", { modelId });
    await get().refresh();
  },

  stop: async () => {
    await invoke("ovms_stop");
    await get().refresh();
  },

  loadModel: async (id) => {
    // Track the in-flight load so every surface (chat header banner,
    // Models page card, bottom bar) can render the same "loading"
    // affordance regardless of which page started it.
    set((s) => {
      const next = new Set(s.loadingModelIds);
      next.add(id);
      return { loadingModelIds: next };
    });
    try {
      await invoke("ovms_load_model", { modelId: id });
      await get().refresh();
    } finally {
      set((s) => {
        if (!s.loadingModelIds.has(id)) return s;
        const next = new Set(s.loadingModelIds);
        next.delete(id);
        return { loadingModelIds: next };
      });
    }
  },

  removeModel: async (id) => {
    await invoke("ovms_remove_model", { modelId: id });
    await get().refresh();
  },

  unloadModel: async () => {
    await invoke("ovms_unload_model");
    await get().refresh();
  },

  getConfigStatus: async () => {
    return await invoke<TfsConfigStatus>("ovms_config_status");
  },

  getModelStatus: async (id) => {
    return await invoke<TfsModelStatus>("ovms_model_status", { modelId: id });
  },

  getModelMetadata: async (id) => {
    return await invoke<unknown>("ovms_model_metadata", { modelId: id });
  },

  bindEvents: async () => {
    const offLog = await on<OvmsLogLine>(Events.OvmsLog, (line) => {
      set((s) => ({ logs: [...s.logs.slice(-500), line] }));
    });
    const offStatus = await on<OvmsInfo>(Events.OvmsStatus, (info) => {
      set({ info });
    });
    const offProgress = await on<InstallProgress>(
      Events.OvmsInstallProgress,
      (p) => {
        set({ installProgress: p });
        if (p.stage === "done") {
          // Auto-dismiss the progress strip a moment after success.
          setTimeout(() => {
            const cur = get().installProgress;
            if (cur && cur.stage === "done") set({ installProgress: null });
          }, 2500);
        }
      },
    );
    return () => {
      offLog();
      offStatus();
      offProgress();
    };
  },

  clearLogs: () => set({ logs: [] }),
}));
