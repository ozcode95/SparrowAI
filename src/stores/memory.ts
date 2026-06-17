import { create } from "zustand";
import { invoke } from "@/lib/tauri";

/**
 * Persistent agent memory. Mirrors `crate::memory` on the Rust side: two
 * character-bounded markdown files under `~/.zero/memories/` (`MEMORY.md`
 * for the agent's personal notes; `USER.md` for user-profile facts). The
 * chat runner injects both as a frozen snapshot into the system prompt at
 * the start of every turn, and the agent curates them through the
 * built-in `memory` tool.
 *
 * The Memory page is just a friendlier surface onto the same files —
 * everything routes through the same `crate::memory` API the tool uses,
 * so capacity limits and substring-match rules apply equally whether the
 * model is editing or the user is editing.
 */

export type MemoryTarget = "memory" | "user";

export interface MemorySnapshot {
  target: MemoryTarget;
  entries: string[];
  used: number;
  limit: number;
  path: string;
}

export interface MemoryState {
  memory: MemorySnapshot;
  user: MemorySnapshot;
}

interface MemoryStore {
  state: MemoryState | null;
  loading: boolean;
  /** Last error from a write op, surfaced inline so the user can read it. */
  error: string | null;
  load: () => Promise<void>;
  add: (target: MemoryTarget, content: string) => Promise<void>;
  replace: (
    target: MemoryTarget,
    oldText: string,
    content: string,
  ) => Promise<void>;
  remove: (target: MemoryTarget, oldText: string) => Promise<void>;
  setRaw: (target: MemoryTarget, raw: string) => Promise<void>;
  clearError: () => void;
}

function emptySnapshot(target: MemoryTarget, limit: number): MemorySnapshot {
  return {
    target,
    entries: [],
    used: 0,
    limit,
    path: "",
  };
}

const FALLBACK_STATE: MemoryState = {
  // The same default caps the Rust side uses (`crate::memory::DEFAULT_*`).
  // Kept here so the UI can render a sane "0 / N" gauge before the first
  // `load()` resolves.
  memory: emptySnapshot("memory", 2200),
  user: emptySnapshot("user", 1375),
};

export const useMemoryStore = create<MemoryStore>((set, get) => ({
  state: null,
  loading: false,
  error: null,
  load: async () => {
    set({ loading: true });
    try {
      const state = (await invoke<MemoryState>("memory_load")) ?? FALLBACK_STATE;
      set({ state, loading: false, error: null });
    } catch (e) {
      console.error("memory_load failed", e);
      set({ loading: false, error: String(e) });
    }
  },
  add: async (target, content) => {
    try {
      const snap = await invoke<MemorySnapshot>("memory_add", {
        target,
        content,
      });
      mergeSnapshot(set, get, snap);
    } catch (e) {
      set({ error: String(e) });
      throw e;
    }
  },
  replace: async (target, oldText, content) => {
    try {
      const snap = await invoke<MemorySnapshot>("memory_replace", {
        target,
        oldText,
        content,
      });
      mergeSnapshot(set, get, snap);
    } catch (e) {
      set({ error: String(e) });
      throw e;
    }
  },
  remove: async (target, oldText) => {
    try {
      const snap = await invoke<MemorySnapshot>("memory_remove", {
        target,
        oldText,
      });
      mergeSnapshot(set, get, snap);
    } catch (e) {
      set({ error: String(e) });
      throw e;
    }
  },
  setRaw: async (target, raw) => {
    try {
      const snap = await invoke<MemorySnapshot>("memory_set_raw", {
        target,
        raw,
      });
      mergeSnapshot(set, get, snap);
    } catch (e) {
      set({ error: String(e) });
      throw e;
    }
  },
  clearError: () => set({ error: null }),
}));

/**
 * Splice an updated snapshot into the cached state without re-fetching
 * the half of the store that didn't change. We deliberately don't trust
 * any snapshot field other than `target` to identify which half it is —
 * the wire shape uses the same `target` enum the backend does.
 */
function mergeSnapshot(
  set: (
    partial: Partial<MemoryStore> | ((s: MemoryStore) => Partial<MemoryStore>),
  ) => void,
  get: () => MemoryStore,
  snap: MemorySnapshot,
) {
  const current = get().state ?? FALLBACK_STATE;
  const next: MemoryState =
    snap.target === "memory"
      ? { ...current, memory: snap }
      : { ...current, user: snap };
  set({ state: next, error: null });
}

/** % of capacity used. Capped at 100 so the progress bar never overflows. */
export function snapshotPercent(snap: MemorySnapshot): number {
  if (snap.limit <= 0) return 0;
  return Math.min(100, Math.round((snap.used / snap.limit) * 100));
}
