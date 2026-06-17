import { create } from "zustand";
import { invoke } from "@/lib/tauri";

export interface Skill {
  id: string;
  name: string;
  description: string | null;
  body_bytes: number;
  path: string;
}

interface SkillInput {
  id: string;
  name: string;
  description?: string | null;
  body?: string;
}

interface SkillsState {
  skills: Skill[];
  loading: boolean;
  list: () => Promise<void>;
  create: (input: SkillInput) => Promise<void>;
  update: (input: SkillInput) => Promise<void>;
  remove: (id: string) => Promise<void>;
  setEnabled: (id: string, enabled: boolean) => Promise<void>;
  readSource: (id: string) => Promise<string>;
}

/**
 * User-authored skills. Mirrors `crate::skills` on the Rust side: each skill
 * is a folder under `~/.zero/skills/<id>/` containing `SKILL.md`. Enabled
 * ids are stored in `settings.skills_enabled` (see `useSettingsStore`) so
 * the chat runner can find them every turn.
 */
export const useSkillsStore = create<SkillsState>((set) => ({
  skills: [],
  loading: false,
  list: async () => {
    set({ loading: true });
    try {
      const skills = (await invoke<Skill[]>("skills_list")) ?? [];
      set({ skills, loading: false });
    } catch (e) {
      console.error("skills_list failed", e);
      set({ loading: false });
    }
  },
  create: async (input) => {
    await invoke("skills_create", { input });
    const skills = (await invoke<Skill[]>("skills_list")) ?? [];
    set({ skills });
  },
  update: async (input) => {
    await invoke("skills_update", { input });
    const skills = (await invoke<Skill[]>("skills_list")) ?? [];
    set({ skills });
  },
  remove: async (id) => {
    await invoke("skills_delete", { id });
    set((s) => ({ skills: s.skills.filter((sk) => sk.id !== id) }));
  },
  setEnabled: async (id, enabled) => {
    await invoke("skills_set_enabled", { id, enabled });
  },
  readSource: async (id) => {
    return (await invoke<string>("skills_read_source", { id })) ?? "";
  },
}));
