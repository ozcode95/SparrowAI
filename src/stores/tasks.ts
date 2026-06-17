import { create } from "zustand";
import { invoke } from "@/lib/tauri";

export type TaskTrigger =
  | { kind: "cron"; expr: string }
  | { kind: "interval"; seconds: number }
  | { kind: "manual" }
  | { kind: "once"; at: string }
  /**
   * Fires once each time the app launches. Handled out-of-band by the
   * Rust scheduler (`run_startup_pass`); never appears as "due" during
   * the periodic tick, and never persists a `last_run_at` baseline.
   */
  | { kind: "startup" };

/**
 * Discriminated union mirroring `TaskAction` on the Rust side
 * (`src-tauri/src/tasks/mod.rs`). Keep the `kind` strings and field
 * names in lockstep — they round-trip through the same JSON column.
 */
export type TaskAction =
  | {
      kind: "command";
      program: string;
      args: string[];
      cwd?: string | null;
    }
  | {
      kind: "script";
      path: string;
      interpreter?: string | null;
      cwd?: string | null;
    }
  | { kind: "notify"; title: string; body: string }
  | { kind: "prompt"; prompt: string; notify: boolean };

export type TaskActionKind = TaskAction["kind"];

export interface Task {
  id: string;
  name: string;
  description: string;
  action: TaskAction;
  trigger: TaskTrigger;
  enabled: boolean;
  last_run_at: string | null;
  last_status: "ok" | "error" | "running" | null;
  created_at: string;
}

interface TasksState {
  tasks: Task[];
  /** Last `tasks_run_now` outcome, keyed by task id — drives transient
   *  status hints in the UI without re-fetching the whole list. */
  lastRunMessage: Record<string, { ok: boolean; message: string }>;
  list: () => Promise<void>;
  create: (
    t: Omit<Task, "id" | "created_at" | "last_run_at" | "last_status">,
  ) => Promise<string>;
  update: (t: Task) => Promise<void>;
  remove: (id: string) => Promise<void>;
  runNow: (id: string) => Promise<void>;
  setEnabled: (id: string, enabled: boolean) => Promise<void>;
}

export const useTasksStore = create<TasksState>((set, get) => ({
  tasks: [],
  lastRunMessage: {},
  list: async () => {
    try {
      const tasks = (await invoke<Task[]>("tasks_list")) ?? [];
      set({ tasks });
    } catch (e) {
      console.error("tasks_list failed", e);
    }
  },
  create: async (t) => {
    const id = await invoke<string>("tasks_create", { task: t });
    await get().list();
    return id;
  },
  update: async (t) => {
    await invoke("tasks_update", { task: t });
    await get().list();
  },
  remove: async (id) => {
    await invoke("tasks_delete", { id });
    await get().list();
  },
  runNow: async (id) => {
    try {
      const message = (await invoke<string>("tasks_run_now", { id })) ?? "ok";
      set((s) => ({
        lastRunMessage: { ...s.lastRunMessage, [id]: { ok: true, message } },
      }));
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      set((s) => ({
        lastRunMessage: { ...s.lastRunMessage, [id]: { ok: false, message } },
      }));
    } finally {
      await get().list();
    }
  },
  setEnabled: async (id, enabled) => {
    await invoke("tasks_set_enabled", { id, enabled });
    await get().list();
  },
}));

/** Default payload for each action variant — used when the user switches
 *  the action type in the new-task dialog so we don't have to keep one
 *  bit of state per field across types. */
export function defaultAction(kind: TaskActionKind): TaskAction {
  switch (kind) {
    case "command":
      return { kind: "command", program: "", args: [], cwd: null };
    case "script":
      return { kind: "script", path: "", interpreter: null, cwd: null };
    case "notify":
      return { kind: "notify", title: "", body: "" };
    case "prompt":
      return { kind: "prompt", prompt: "", notify: true };
  }
}
