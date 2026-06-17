import { create } from "zustand";
import { invoke } from "@/lib/tauri";
import type { McpServerConfig } from "@/stores/settings";

export interface McpToolSchema {
  name: string;
  description: string;
  input_schema: unknown;
  destructive: boolean;
}

export interface McpServerSummary extends McpServerConfig {
  reachable: boolean | null;
  tool_count: number;
  last_error: string | null;
}

/**
 * Per-server probe state. Lives in the store so the Tools page can keep a
 * spinner / "down" badge per row without each Panel managing its own state.
 */
interface ProbeState {
  loading: boolean;
  tools: McpToolSchema[];
  error: string | null;
}

interface McpState {
  servers: McpServerSummary[];
  probes: Record<string, ProbeState>;
  builtins: McpToolSchema[];
  list: () => Promise<void>;
  upsert: (server: McpServerConfig) => Promise<void>;
  remove: (id: string) => Promise<void>;
  setEnabled: (id: string, enabled: boolean) => Promise<void>;
  probe: (id: string) => Promise<void>;
  call: (
    id: string,
    name: string,
    args: Record<string, unknown>,
  ) => Promise<{ content: string; is_error: boolean } | null>;
  listBuiltins: () => Promise<void>;
}

export const useMcpStore = create<McpState>((set, get) => ({
  servers: [],
  probes: {},
  builtins: [],
  list: async () => {
    try {
      const servers = (await invoke<McpServerSummary[]>("mcp_list_servers")) ?? [];
      set({ servers });
    } catch (e) {
      console.error("mcp_list_servers failed", e);
    }
  },
  upsert: async (server) => {
    await invoke("mcp_upsert_server", { server });
    await get().list();
  },
  remove: async (id) => {
    await invoke("mcp_delete_server", { id });
    set((s) => ({
      servers: s.servers.filter((srv) => srv.id !== id),
      probes: Object.fromEntries(
        Object.entries(s.probes).filter(([k]) => k !== id),
      ),
    }));
  },
  setEnabled: async (id, enabled) => {
    await invoke("mcp_set_enabled", { id, enabled });
    set((s) => ({
      servers: s.servers.map((srv) =>
        srv.id === id ? { ...srv, enabled } : srv,
      ),
    }));
  },
  probe: async (id) => {
    set((s) => ({
      probes: {
        ...s.probes,
        [id]: { loading: true, tools: s.probes[id]?.tools ?? [], error: null },
      },
    }));
    try {
      const tools = (await invoke<McpToolSchema[]>("mcp_list_tools", {
        serverId: id,
      })) ?? [];
      set((s) => ({
        probes: { ...s.probes, [id]: { loading: false, tools, error: null } },
      }));
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      set((s) => ({
        probes: {
          ...s.probes,
          [id]: { loading: false, tools: [], error: msg },
        },
      }));
    }
  },
  call: async (id, name, args) => {
    try {
      return await invoke<{ content: string; is_error: boolean }>(
        "mcp_call_tool",
        {
          serverId: id,
          name,
          arguments: args,
        },
      );
    } catch (e) {
      console.error("mcp_call_tool failed", e);
      return null;
    }
  },
  listBuiltins: async () => {
    try {
      const builtins = (await invoke<McpToolSchema[]>("mcp_list_builtins")) ?? [];
      set({ builtins });
    } catch (e) {
      console.error("mcp_list_builtins failed", e);
    }
  },
}));
