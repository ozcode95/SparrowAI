import { useEffect, useMemo, useState } from "react";
import { Panel } from "@/components/tui/Panel";
import { TuiInput, TuiTextarea } from "@/components/tui/Input";
import { TuiButton } from "@/components/tui/Button";
import { useSkillsStore, type Skill } from "@/stores/skills";
import { useSettingsStore } from "@/stores/settings";
import { bytes } from "@/lib/format";

/**
 * Skills page. Each skill is a `SKILL.md` under `~/.zero/skills/<id>/`
 * with YAML frontmatter (`name`, `description`). The list view on the
 * left enumerates them; the editor on the right lets the user toggle,
 * rename, rewrite the body, or delete the skill.
 *
 * Toggling `enabled` updates `settings.skills_enabled` which the chat
 * runner reads every turn to assemble the system prompt.
 */
export function SkillsView() {
  const skills = useSkillsStore((s) => s.skills);
  const list = useSkillsStore((s) => s.list);
  const create = useSkillsStore((s) => s.create);
  const update = useSkillsStore((s) => s.update);
  const remove = useSkillsStore((s) => s.remove);
  const readSource = useSkillsStore((s) => s.readSource);
  const setEnabled = useSkillsStore((s) => s.setEnabled);

  const enabledIds = useSettingsStore((s) => s.skills_enabled);
  const saveSettings = useSettingsStore((s) => s.save);

  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [draft, setDraft] = useState<{
    id: string;
    name: string;
    description: string;
    body: string;
    isNew: boolean;
  } | null>(null);

  useEffect(() => {
    void list();
  }, [list]);

  const selected = useMemo(
    () => skills.find((s) => s.id === selectedId) ?? null,
    [skills, selectedId],
  );

  async function openExisting(s: Skill) {
    setSelectedId(s.id);
    const body = await readSource(s.id);
    // Strip the frontmatter so the editor shows just the prompt body.
    const trimmed = body
      .replace(/^---[\s\S]*?---\s*\n?/, "")
      .replace(/^\uFEFF/, "");
    setDraft({
      id: s.id,
      name: s.name,
      description: s.description ?? "",
      body: trimmed,
      isNew: false,
    });
  }

  function startNew() {
    setSelectedId(null);
    setDraft({
      id: "",
      name: "",
      description: "",
      body: "",
      isNew: true,
    });
  }

  async function save() {
    if (!draft) return;
    if (draft.isNew) {
      if (!draft.id.trim() || !draft.name.trim()) return;
      await create({
        id: draft.id.trim(),
        name: draft.name.trim(),
        description: draft.description.trim() || null,
        body: draft.body,
      });
      setSelectedId(draft.id.trim());
      setDraft((d) => (d ? { ...d, isNew: false } : null));
    } else {
      await update({
        id: draft.id,
        name: draft.name.trim(),
        description: draft.description.trim() || null,
        body: draft.body,
      });
    }
  }

  async function deleteSelected() {
    if (!selected) return;
    if (!confirm(`Delete skill "${selected.name}"? This removes the folder.`))
      return;
    await remove(selected.id);
    if (enabledIds.includes(selected.id)) {
      await saveSettings({
        skills_enabled: enabledIds.filter((id) => id !== selected.id),
      });
    }
    setSelectedId(null);
    setDraft(null);
  }

  async function deleteFromRow(s: Skill) {
    if (!confirm(`Delete skill "${s.name}"? This removes the folder.`)) return;
    await remove(s.id);
    if (enabledIds.includes(s.id)) {
      await saveSettings({
        skills_enabled: enabledIds.filter((id) => id !== s.id),
      });
    }
    // If the row we just nuked happened to be the one being edited,
    // close the editor so it doesn't keep showing a stale draft.
    if (selectedId === s.id) {
      setSelectedId(null);
      setDraft(null);
    }
  }

  async function toggle(s: Skill, on: boolean) {
    await setEnabled(s.id, on);
    const next = on
      ? Array.from(new Set([...enabledIds, s.id]))
      : enabledIds.filter((id) => id !== s.id);
    await saveSettings({ skills_enabled: next });
  }

  return (
    <div className="flex min-h-0 min-w-0 flex-1 gap-3">
      <Panel
        title="Skills"
        hint={`${skills.length} · ${enabledIds.length} on`}
        className="w-72 shrink-0"
        flush
      >
        <div className="border-b border-tui-border p-2">
          <TuiButton onClick={startNew} variant="primary" className="w-full">
            <span className="text-[14px] leading-none">+</span>
            New skill
          </TuiButton>
        </div>
        <ul className="flex-1 overflow-auto p-1">
          {skills.length === 0 && (
            <li className="px-2 py-3 text-center text-[11px] text-tui-fg-muted">
              No skills yet. Create one to teach zero project-specific
              conventions, persona, or tool preferences.
            </li>
          )}
          {skills.map((s) => {
            const on = enabledIds.includes(s.id);
            const active = s.id === selectedId;
            return (
              <li
                key={s.id}
                className={`group relative flex items-center justify-between gap-2 rounded-md px-2 py-1.5 text-[12px] transition-colors ${
                  active
                    ? "bg-[var(--fluent-bg-subtle-pressed)] text-tui-fg"
                    : "text-tui-fg-dim hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-fg"
                }`}
              >
                {active && (
                  <span
                    aria-hidden="true"
                    className="absolute left-0 top-1.5 bottom-1.5 w-[3px] rounded-full bg-tui-accent"
                  />
                )}
                <button
                  onClick={() => void openExisting(s)}
                  className="min-w-0 flex-1 pl-1.5 text-left"
                  title={s.description ?? s.path}
                >
                  <div className="flex items-center gap-1.5">
                    <span
                      aria-hidden="true"
                      className={`h-1.5 w-1.5 rounded-full ${on ? "bg-tui-accent" : "bg-tui-fg-muted"}`}
                    />
                    <span className="truncate font-medium">{s.name}</span>
                  </div>
                  <div className="truncate text-[11px] text-tui-fg-muted">
                    {s.id} · {bytes(s.body_bytes)}
                  </div>
                </button>
                <input
                  type="checkbox"
                  checked={on}
                  onChange={(e) => void toggle(s, e.target.checked)}
                  title={on ? "Disable" : "Enable"}
                  aria-label={on ? "Disable skill" : "Enable skill"}
                />
                <button
                  onClick={(e) => {
                    // Don't bubble into the row's open-editor button.
                    e.stopPropagation();
                    void deleteFromRow(s);
                  }}
                  className={
                    "shrink-0 rounded p-1 text-tui-fg-muted opacity-0 " +
                    "transition-opacity duration-150 " +
                    "hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-err " +
                    "group-hover:opacity-100 focus-visible:opacity-100"
                  }
                  title="Delete skill"
                  aria-label={`Delete skill ${s.name}`}
                >
                  <svg
                    width="12"
                    height="12"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.8"
                    strokeLinecap="round"
                  >
                    <path d="M18 6 6 18M6 6l12 12" />
                  </svg>
                </button>
              </li>
            );
          })}
        </ul>
      </Panel>

      <Panel
        title={
          draft ? (draft.isNew ? "New skill" : `Edit · ${draft.id}`) : "Editor"
        }
        hint={draft?.isNew ? "" : selected?.path}
        className="flex-1"
        flush
      >
        {!draft && (
          <div className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-center text-[12px] text-tui-fg-muted">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-[var(--fluent-bg-subtle)] text-tui-fg-muted">
              <svg
                width="20"
                height="20"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.75"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="M12 3 14.5 9l6 .5-4.6 4 1.5 6L12 16l-5.4 3.5 1.5-6L3.5 9.5 9.5 9 12 3Z" />
              </svg>
            </div>
            <p>Pick a skill on the left, or click “+ New skill”.</p>
            <p className="text-tui-fg-muted">
              Each skill lives at{" "}
              <code className="font-mono">
                ~/.zero/skills/&lt;id&gt;/SKILL.md
              </code>{" "}
              and is appended to the chat system prompt every turn when enabled.
            </p>
          </div>
        )}
        {draft && (
          <div className="flex flex-1 flex-col gap-3 overflow-auto p-3">
            <SkillField label="ID">
              <TuiInput
                value={draft.id}
                onChange={(e) =>
                  setDraft({ ...draft, id: e.target.value.trim() })
                }
                disabled={!draft.isNew}
                placeholder="python-helper"
              />
            </SkillField>
            <SkillField label="Name">
              <TuiInput
                value={draft.name}
                onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                placeholder="Python helper"
              />
            </SkillField>
            <SkillField label="Description">
              <TuiInput
                value={draft.description}
                onChange={(e) =>
                  setDraft({ ...draft, description: e.target.value })
                }
                placeholder="One-line summary surfaced in the picker."
              />
            </SkillField>
            <label className="flex flex-1 flex-col gap-1 text-[11px] font-medium text-tui-fg-dim">
              Body (markdown, appended to system prompt when enabled)
              <TuiTextarea
                value={draft.body}
                onChange={(e) => setDraft({ ...draft, body: e.target.value })}
                rows={16}
                className="flex-1 font-mono text-[12px]"
                placeholder={`Write the persona / conventions / tool guidance for this skill.\n\nExample:\nYou are a senior Python engineer. Prefer pathlib over os.path.\nAlways propose tests alongside new functions.`}
              />
            </label>
            <div className="flex justify-between gap-2 pt-1">
              <TuiButton
                variant="danger"
                onClick={() => void deleteSelected()}
                disabled={draft.isNew || !selected}
              >
                Delete
              </TuiButton>
              <div className="flex gap-2">
                <TuiButton
                  onClick={() => {
                    if (draft.isNew) {
                      setDraft(null);
                    } else if (selected) {
                      void openExisting(selected);
                    }
                  }}
                >
                  Reset
                </TuiButton>
                <TuiButton
                  variant="primary"
                  onClick={() => void save()}
                  disabled={
                    !draft.name.trim() || (draft.isNew && !draft.id.trim())
                  }
                >
                  {draft.isNew ? "Create" : "Save"}
                </TuiButton>
              </div>
            </div>
          </div>
        )}
      </Panel>
    </div>
  );
}

function SkillField({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <label className="flex flex-col gap-1 text-[11px] font-medium text-tui-fg-dim">
      {label}
      {children}
    </label>
  );
}
