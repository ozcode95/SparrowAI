import { create } from "zustand";
import { invoke } from "@/lib/tauri";

export type ProviderKind = "ovms" | "ollama" | "vllm";

/**
 * Optional per-layer sampling overrides. Every field is independently
 * optional: a `null` means "don't override at this layer", letting the
 * runner's precedence chain (conversation → provider → per-model
 * profile → upstream default) fall through to the next one.
 *
 * Same shape lives on a conversation row for the per-chat popover —
 * the editor component in the UI is reused for both surfaces.
 */
export interface SamplingConfig {
  /** Sampling temperature override. Typical range 0.0–2.0. */
  temperature: number | null;
  /** Nucleus-sampling cutoff override. Typical range 0.0–1.0. */
  top_p: number | null;
  /** Top-k cutoff override. Sent as an OpenAI-compat extension. */
  top_k: number | null;
}

export const EMPTY_SAMPLING: SamplingConfig = {
  temperature: null,
  top_p: null,
  top_k: null,
};

export interface ProviderConfig {
  id: string;
  kind: ProviderKind;
  name: string;
  base_url: string;
  enabled: boolean;
  api_key_ref?: string | null;
  /** Sampling defaults applied to every chat that uses this provider. */
  sampling: SamplingConfig;
}

/**
 * External MCP server configuration. Supports three transports:
 *
 * - `http` / `sse` — standard JSON-RPC over HTTP. Uses `url` + `headers`.
 * - `stdio`        — zero spawns `command` with `args` (plus any `env`)
 *                    and speaks JSON-RPC on stdin/stdout per call.
 *
 * Stdio-only fields are optional so existing settings files
 * (HTTP/SSE-only) keep loading without migration.
 */
export interface McpServerConfig {
  id: string;
  name: string;
  /** `http` | `sse` | `stdio`. */
  transport: string;
  /** Endpoint URL for http/sse. Empty for stdio. */
  url: string;
  /** Header tuples forwarded verbatim on every JSON-RPC POST (http/sse). */
  headers: [string, string][];
  /** Executable to spawn (stdio only). */
  command?: string;
  /** argv for `command` (stdio only). */
  args?: string[];
  /** Extra environment variables for the child process (stdio only). */
  env?: [string, string][];
  enabled: boolean;
}

export interface OvmsSettings {
  rest_port: number;
  grpc_port: number;
  /** Free-form OpenVINO device: `CPU`, `GPU`, `NPU`, `AUTO`, `MULTI:CPU,GPU`, ... */
  device: string;

  // ─── advanced server-side knobs (all optional) ───────────────────────
  /** `DEBUG` / `INFO` / `ERROR`. `null` keeps the OVMS default (INFO). */
  log_level: string | null;
  /** Optional path to a log file. */
  log_path: string | null;
  /** Optional model-compilation cache directory. Major startup speedup. */
  cache_dir: string | null;
  /** CORS `Access-Control-Allow-Origin`. */
  allowed_origins: string | null;
  /** Path to a file whose first line holds the API key for `/v3/*`. */
  api_key_file: string | null;
  /**
   * Escape hatch for any flag we don't model explicitly. Each entry is
   * forwarded verbatim as a separate argv token — the Settings UI splits
   * the user's textarea on whitespace so multi-token flags become two
   * entries (`--grpc_workers` then `4`).
   */
  extra_args: string[];
}

export interface Settings {
  active_provider_id: string | null;
  providers: ProviderConfig[];
  hf_token_set: boolean;
  default_model: string | null;
  thinking_enabled: boolean;
  agent_max_iterations: number;
  destructive_tool_confirm: boolean;
  ovms: OvmsSettings;
  /** Auto-install OVMS on startup and auto-start the default model if it's already downloaded. */
  auto_provision_ovms: boolean;
  /** Folder names under `~/.zero/skills/` to inject into the system prompt. */
  skills_enabled: string[];
  /**
   * Names of built-in tools (e.g. `fs.list`, `shell.exec`) the user has
   * globally disabled from the Tools page. Hidden from every chat's
   * catalog. Per-chat overrides live separately on the conversation row.
   */
  builtin_tools_disabled: string[];
  /**
   * When true, the chat runner ships only the `tools.list` built-in to
   * the LLM on the first round of each turn instead of the full tool
   * catalogue. The model must call `tools.list` to discover what else
   * is available; once it does, the runner expands the catalogue on
   * subsequent rounds in the same turn. Trades one extra round-trip
   * on tool-using turns for a much smaller initial-context cost.
   */
  lazy_tool_discovery: boolean;
  /** External MCP servers (HTTP/SSE) we can list / call tools on. */
  mcp_servers: McpServerConfig[];
}

interface SettingsState extends Settings {
  loaded: boolean;
  load: () => Promise<void>;
  save: (patch: Partial<Settings>) => Promise<void>;
}

const DEFAULTS: Settings = {
  active_provider_id: "ovms-local",
  providers: [
    {
      id: "ovms-local",
      kind: "ovms",
      name: "OVMS (local)",
      base_url: "http://127.0.0.1:8000/v3",
      enabled: true,
      sampling: { ...EMPTY_SAMPLING },
    },
  ],
  hf_token_set: false,
  default_model: null,
  thinking_enabled: true,
  agent_max_iterations: 8,
  destructive_tool_confirm: true,
  ovms: {
    rest_port: 8000,
    // 0 disables OVMS's gRPC interface. zero only talks REST, so we leave
    // gRPC off by default to avoid port collisions on 9000.
    grpc_port: 0,
    device: "GPU",
    log_level: null,
    log_path: null,
    cache_dir: null,
    allowed_origins: null,
    api_key_file: null,
    extra_args: [],
  },
  auto_provision_ovms: true,
  skills_enabled: [],
  // Default to a conservative built-in tool surface: filesystem mutation
  // (`fs.edit` / `fs.write`) and metadata probing (`fs.stat`), arbitrary
  // outbound HTTP (`http.fetch`), and clipboard I/O are off by default
  // so a fresh install can't surprise the user. They can be re-enabled
  // individually from the Tools page.
  builtin_tools_disabled: [
    "clipboard.read",
    "clipboard.write",
    "http.fetch",
    "fs.edit",
    "fs.write",
    "fs.stat",
  ],
  lazy_tool_discovery: true,
  mcp_servers: [],
};

export const useSettingsStore = create<SettingsState>((set, get) => ({
  ...DEFAULTS,
  loaded: false,
  load: async () => {
    try {
      const s = await invoke<Settings>("settings_load");
      if (s) set({ ...s, loaded: true });
      else set({ loaded: true });
    } catch (e) {
      console.error("settings_load failed", e);
      set({ loaded: true });
    }
  },
  save: async (patch) => {
    const next = { ...get(), ...patch };
    set(patch);
    try {
      await invoke("settings_save", { settings: next });
    } catch (e) {
      console.error("settings_save failed", e);
    }
  },
}));
