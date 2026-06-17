import { forwardRef, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { convertFileSrc } from "@tauri-apps/api/core";
import { Panel } from "@/components/tui/Panel";
import { TuiTextarea } from "@/components/tui/Input";
import { TuiButton } from "@/components/tui/Button";
import { Spinner } from "@/components/tui/Spinner";
import { Markdown } from "@/components/tui/Markdown";
import {
  useChatStore,
  type Attachment,
  type ChatErrorInfo,
  type Message,
  type ToolConfirmRequest,
  type TurnOverrides,
} from "@/stores/chat";
import { useUiStore } from "@/stores/ui";
import { useModelsStore } from "@/stores/models";
import { useServerStore } from "@/stores/server";
import { useMcpStore, type McpToolSchema } from "@/stores/mcp";
import {
  useSettingsStore,
  EMPTY_SAMPLING,
  type SamplingConfig,
} from "@/stores/settings";
import { useSkillsStore } from "@/stores/skills";
import {
  SamplingEditor,
  summariseSampling,
  samplingOverrideCount,
} from "@/components/SamplingEditor";
import { bytes } from "@/lib/format";
import {
  formatSamplingDefault,
  modelSamplingDefaults,
} from "@/lib/modelProfile";
import { invoke } from "@/lib/tauri";

// Stable empty fallbacks so Zustand selectors return cached references
// when there is no active conversation or no entry yet for the active id.
// Returning a fresh `[]` / `{}` on each snapshot read causes
// useSyncExternalStore to detect a "changed" snapshot every render,
// which triggers an infinite update loop.
const EMPTY_MESSAGES: Message[] = [];
const EMPTY_ERRORS: Record<string, ChatErrorInfo> = {};

/**
 * Built-in slash commands the user can prefix their message with.
 *
 * Kept in one place so the autocomplete popover above the chat input
 * and any future help surfaces stay in sync. The actual semantics live
 * elsewhere:
 *   * `/loop` is detected by the Rust runner (`is_loop_prefixed`) and
 *     flips that turn into autonomous agent mode.
 *   * `/no_think` can be used by the user to manually disable thinking
 *     for that turn (Qwen3, gpt-oss, Gemma 4 thinking variants). For
 *     Qwen3/Qwen3.5 models specifically, the backend automatically passes
 *     {"enable_thinking": false} via chat_template_kwargs when thinking
 *     is not enabled, so manual use of /no_think is rarely needed.
 *   * `/web` and `/research` are detected by the runner and unlock the
 *     built-in web tools for that turn — they're hidden from the model
 *     otherwise so unprompted browsing can't happen.
 *
 * All four also have a structured equivalent the composer's `+` menu
 * writes to `TurnOverrides`. New invocations should prefer the menu;
 * the slash forms remain a documented fallback for power users.
 */
interface SlashCommand {
  name: string;
  hint: string;
  desc: string;
}
const SLASH_COMMANDS: SlashCommand[] = [
  {
    name: "/loop",
    hint: "agent mode",
    desc: "Let the assistant chain multiple tool calls autonomously this turn.",
  },
  {
    name: "/no_think",
    hint: "skip thinking",
    desc: "Disable model thinking for this turn (Qwen3, gpt-oss, Gemma 4…).",
  },
  {
    name: "/web",
    hint: "web search",
    desc: "Allow the assistant to call web.search (and web.read_page) this turn.",
  },
  {
    name: "/research",
    hint: "deep research",
    desc: "Allow web.deep_research (and web.read_page) for a multi-source research pass this turn.",
  },
];
const EMPTY_SKILL_IDS: string[] = [];
const EMPTY_TOOL_CONFIRMS: Record<string, ToolConfirmRequest> = {};
const EMPTY_DISABLED_TOOLS: string[] = [];
const EMPTY_DISABLED_BUILTINS: string[] = [];

// ─── Per-turn composer presets ( + menu toggles ) ──────────────
//
// User-friendly surface over the slash-command capability flags. Each
// preset is a turn-scoped toggle the user can flip from the composer
// `+` menu; when active, `onSend` prepends the matching slash command
// to the outgoing message and then clears the set, so the unlock
// applies exactly to that one turn (matches the runner's existing
// single-turn slash semantics).
//
// `inverse: true` flips the semantics for presets whose *default*
// state is the slash command being applied — the "Thinking" toggle
// uses this so that all chats default to `/no_think` (no thinking
// trace) and the user explicitly opts *into* thinking for a turn by
// flipping the toggle on. In that mode "in the active set" means
// "suppress the slash command" rather than "emit it".
type TurnPreset = "web" | "research" | "think";
interface TurnPresetSpec {
  key: TurnPreset;
  /** Canonical slash command the runner detects (`/web`, `/research`, `/no_think`). */
  command: string;
  /** Menu / chip label, e.g. "Search". */
  label: string;
  /** One-liner shown under the label in the menu. */
  hint: string;
  /** Inline SVG glyph rendered next to the label. */
  icon: React.ReactNode;
  /**
   * When `true`, the slash command is the *default* for every turn
   * and toggling the preset *on* suppresses it. When `false`
   * (default), the slash command is opt-in: toggling on prepends
   * it, toggling off leaves the message untouched.
   */
  inverse?: boolean;
}
const GLOBE_ICON = (
  <svg
    width="14"
    height="14"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.75"
    strokeLinecap="round"
    strokeLinejoin="round"
  >
    <circle cx="12" cy="12" r="9" />
    <path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18" />
  </svg>
);
const RESEARCH_ICON = (
  <svg
    width="14"
    height="14"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.75"
    strokeLinecap="round"
    strokeLinejoin="round"
  >
    <circle cx="11" cy="11" r="7" />
    <path d="m20 20-3.5-3.5M11 8v6M8 11h6" />
  </svg>
);
const THINKING_ICON = (
  <svg
    width="14"
    height="14"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.75"
    strokeLinecap="round"
    strokeLinejoin="round"
  >
    <path d="M9 18h6M10 21h4" />
    <path d="M12 3a6 6 0 0 0-4 10.5c.7.7 1 1.5 1 2.5h6c0-1 .3-1.8 1-2.5A6 6 0 0 0 12 3z" />
  </svg>
);
const TURN_PRESETS: TurnPresetSpec[] = [
  {
    key: "web",
    command: "/web",
    label: "Search",
    hint: "Search the web",
    icon: GLOBE_ICON,
  },
  {
    key: "research",
    command: "/research",
    label: "Deep research",
    hint: "Multi-source web research pass",
    icon: RESEARCH_ICON,
  },
  {
    key: "think",
    command: "/no_think",
    label: "Thinking",
    hint: "Let the model show its reasoning trace this turn",
    icon: THINKING_ICON,
    // Default behaviour is `/no_think` for every turn — toggling this
    // preset on lifts the suppression so the model can think out loud.
    inverse: true,
  },
];

export function ChatView() {
  const activeId = useChatStore((s) => s.activeId);
  const activeConv = useChatStore((s) =>
    s.activeId
      ? (s.conversations.find((c) => c.id === s.activeId) ?? null)
      : null,
  );
  const messages = useChatStore((s) =>
    s.activeId ? (s.messages[s.activeId] ?? EMPTY_MESSAGES) : EMPTY_MESSAGES,
  );
  const errorsByMsg = useChatStore((s) =>
    s.activeId ? (s.errors[s.activeId] ?? EMPTY_ERRORS) : EMPTY_ERRORS,
  );
  const streamingId = useChatStore((s) => s.streamingMessageId);
  const create = useChatStore((s) => s.create);
  const send = useChatStore((s) => s.send);
  const cancel = useChatStore((s) => s.cancel);
  const retry = useChatStore((s) => s.retry);
  const dismissError = useChatStore((s) => s.dismissError);
  const setModel = useChatStore((s) => s.setModel);
  const pendingModel = useChatStore((s) => s.pendingModel);
  const setPendingModel = useChatStore((s) => s.setPendingModel);
  const toolConfirms = useChatStore(
    (s) => s.toolConfirms ?? EMPTY_TOOL_CONFIRMS,
  );
  const resolveToolConfirm = useChatStore((s) => s.resolveToolConfirm);
  // Render the oldest pending confirm so a model that emits two in a row
  // doesn't surprise the user with overlapping modals.
  const pendingConfirm = useMemo(() => {
    const entries = Object.values(toolConfirms);
    return entries.length > 0 ? entries[0] : null;
  }, [toolConfirms]);

  const localModels = useModelsStore((s) => s.local);
  const ovmsLoaded = useServerStore((s) => s.info?.loaded_model ?? null);
  // OVMS can serve more than one model at a time (text-gen + embeddings,
  // or two text-gen models the user has both staged). `loaded_model` is
  // just the convenience "primary" pointer — for the banner's pending
  // logic we need the full set so we can tell that the conversation's
  // target model is *already* serving even when it isn't the primary.
  //
  // We subscribe to the raw `loaded_models` array (referentially stable
  // across store snapshots; the server store only swaps it when an
  // ovms://status event arrives) and check membership directly inside
  // `swapPending` below — deriving a fresh ids array in the selector
  // would return a new reference every render and trip React 18's
  // "getSnapshot should be cached" infinite-loop guard.
  const ovmsLoadedModels = useServerStore((s) => s.info?.loaded_models);
  const ovmsStatus = useServerStore((s) => s.info?.status ?? null);
  const ovmsLastError = useServerStore((s) => s.info?.last_error ?? null);
  const activeProviderKind = useSettingsStore((s) => {
    const p = s.providers.find((x) => x.id === s.active_provider_id);
    return p?.kind ?? null;
  });

  const enabledSkillIds = useSettingsStore(
    (s) => s.skills_enabled ?? EMPTY_SKILL_IDS,
  );
  const disabledBuiltinNames = useSettingsStore(
    (s) => s.builtin_tools_disabled ?? EMPTY_DISABLED_BUILTINS,
  );
  // Derive the typed list from a *stable* slice (`mcp_servers`) via
  // useMemo. Returning the filtered+mapped array directly from the
  // selector would allocate a new reference on every snapshot read,
  // which causes the useSyncExternalStore loop the comment at the top
  // of this file warns about.
  const mcpServersRaw = useSettingsStore((s) => s.mcp_servers);
  const allSkills = useSkillsStore((s) => s.skills);
  const listSkills = useSkillsStore((s) => s.list);

  // ─── Tool catalog ────────────────────────────────────────────────
  // Driven from the same source the chat runner uses:
  //   - `useMcpStore.builtins` for the in-process tool registry
  //   - `useMcpStore.probes[id].tools` for each enabled MCP server
  // The header popover lets the user disable any individual tool just
  // for this conversation — the backend persists the per-chat override
  // via `chat_set_disabled_tools`.
  const builtins = useMcpStore((s) => s.builtins);
  const listBuiltins = useMcpStore((s) => s.listBuiltins);
  const probes = useMcpStore((s) => s.probes);
  const probeMcp = useMcpStore((s) => s.probe);
  const disabledTools = useChatStore(
    (s) =>
      (activeId ? s.disabledTools[activeId] : null) ?? EMPTY_DISABLED_TOOLS,
  );
  const loadDisabledTools = useChatStore((s) => s.loadDisabledTools);
  const setToolDisabled = useChatStore((s) => s.setToolDisabled);

  // Per-conversation sampling override. Loaded lazily so opening the
  // popover is the trigger for the first DB read; until then the
  // header chip just shows "using defaults".
  const samplingOverride = useChatStore((s) =>
    activeId ? (s.sampling[activeId] ?? null) : null,
  );
  const loadSampling = useChatStore((s) => s.loadSampling);
  const setSampling = useChatStore((s) => s.setSampling);
  // The active provider's defaults are what an empty conversation
  // override falls through to — surface them in the popover as
  // placeholders so the user sees what they're inheriting.
  const activeProviderSampling = useSettingsStore((s) => {
    const p = s.providers.find((x) => x.id === s.active_provider_id);
    return p?.sampling ?? null;
  });

  const [draft, setDraft] = useState("");
  const [pending, setPending] = useState<Attachment[]>([]);
  const [attaching, setAttaching] = useState(false);
  // Per-turn capability toggles surfaced through the composer `+`
  // menu. Mirrors the `/web` / `/research` slash commands the runner
  // already understands — we just prepend the appropriate prefix in
  // `onSend` and clear the set so the unlock stays scoped to one turn
  // (the same single-turn semantics typing the slash command by hand
  // already gives). Stored as a `Set` so toggling is O(1) and the
  // rendered chip order is deterministic via the `TURN_PRESETS` list.
  const [turnPresets, setTurnPresets] = useState<Set<TurnPreset>>(
    () => new Set(),
  );
  function toggleTurnPreset(key: TurnPreset) {
    setTurnPresets((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }
  // Slash-command popover state. `slashDismissed` lets Escape close the
  // menu without us also having to wipe the leading "/" the user just
  // typed — it auto-resets whenever the draft transitions away from
  // starting with a slash.
  const [slashIdx, setSlashIdx] = useState(0);
  const [slashDismissed, setSlashDismissed] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    scrollRef.current?.scrollTo({
      top: scrollRef.current.scrollHeight,
      behavior: "smooth",
    });
  }, [messages.length, streamingId, activeId]);

  useEffect(() => {
    // Cheap one-shot — keeps the "skills: N on" indicator current even
    // when the user toggles skills from the Skills page.
    void listSkills();
  }, [listSkills]);

  useEffect(() => {
    // Mirror the Tools page's lazy load of the built-in registry so the
    // header popover has names + descriptions ready before the user opens
    // it. Cheap and idempotent.
    void listBuiltins();
  }, [listBuiltins]);

  useEffect(() => {
    // Probe each enabled MCP server once on mount so its tool catalog
    // lands in the Tools popover automatically — otherwise the user
    // would have to click `Probe` per-server before they could see (or
    // toggle) any external tools. `probe` populates a cache, so this is
    // cheap on repeat visits and tolerant of slow/unreachable servers.
    for (const srv of mcpServersRaw?.filter((m) => m.enabled) ?? []) {
      if (!probes[srv.id]) void probeMcp(srv.id);
    }
    // We deliberately leave `probes` out of the dep array: it changes
    // every time we write into it, which would re-trigger this effect
    // and probe in a loop.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mcpServersRaw, probeMcp]);

  useEffect(() => {
    // Lazily hydrate the per-conversation tool-disable list so the
    // header chip's count reflects any saved override the moment the
    // chat opens.
    if (activeId) void loadDisabledTools(activeId);
  }, [activeId, loadDisabledTools]);

  useEffect(() => {
    // Same lazy-hydrate trick for the sampling override so the chip
    // shows the saved-override badge as soon as the chat opens.
    if (activeId) void loadSampling(activeId);
  }, [activeId, loadSampling]);

  /**
   * Open the OS file picker, copy each chosen file into the conversation's
   * attachment store, and append the resulting metadata to `pending`.
   * The backend returns the persisted path/mime/size so the runner can
   * later read the bytes back when building the multimodal request.
   */
  async function attachFiles() {
    if (attaching) return;
    try {
      setAttaching(true);
      const convId = activeId ?? (await create());
      if (!convId) return;
      const picked = await openDialog({
        multiple: true,
        filters: [
          {
            name: "Images",
            extensions: ["png", "jpg", "jpeg", "gif", "webp", "bmp"],
          },
          {
            name: "Documents",
            extensions: [
              "md",
              "txt",
              "json",
              "csv",
              "log",
              "py",
              "rs",
              "ts",
              "tsx",
              "js",
              "html",
              "yaml",
              "yml",
              "toml",
              "pdf",
            ],
          },
          { name: "All", extensions: ["*"] },
        ],
      });
      if (!picked) return;
      const paths = Array.isArray(picked) ? picked : [picked];
      const next: Attachment[] = [];
      for (const p of paths) {
        try {
          const att = await invoke<Attachment>("attachments_save", {
            conversationId: convId,
            sourcePath: p,
          });
          if (att) next.push(att);
        } catch (e) {
          console.error("attachments_save failed", p, e);
        }
      }
      setPending((cur) => [...cur, ...next]);
    } finally {
      setAttaching(false);
    }
  }

  async function dropAttachment(att: Attachment) {
    setPending((cur) => cur.filter((a) => a.path !== att.path));
    try {
      await invoke("attachments_delete", { path: att.path });
    } catch (e) {
      console.error("attachments_delete failed", e);
    }
  }

  async function onSend() {
    const txt = draft.trim();
    if (!txt && pending.length === 0) return;
    const atts = pending;
    // Build the structured overrides payload from the active preset
    // set. The Rust runner reads these directly off the persisted
    // user message (replaces the old `/web` / `/research` /
    // `/no_think` text-prefix scan). `inverse` presets like
    // "Thinking" map to *not* including the slash command in the
    // first place — here that means: `think: true` when the toggle
    // is in the set, `think: false` (default) otherwise.
    const overrides: TurnOverrides = {
      web: turnPresets.has("web"),
      research: turnPresets.has("research"),
      think: turnPresets.has("think"),
    };
    setDraft("");
    setPending([]);
    setTurnPresets(new Set());
    await send(txt, atts, overrides);
  }

  // ─── slash-command autocomplete ────────────────────────────────────
  // We only consider the very first whitespace-delimited token. If that
  // token starts with `/`, it's a candidate — we filter the registry by
  // prefix and surface matches in a popover above the textarea.
  const slashMatch = useMemo(() => {
    if (!draft.startsWith("/")) return null;
    const firstToken = draft.split(/\s/, 1)[0];
    // Once the user has typed a space or newline after the command, the
    // marker is "committed" and we get out of their way.
    if (
      firstToken.length !== draft.length &&
      /\s/.test(draft[firstToken.length])
    ) {
      return null;
    }
    const q = firstToken.slice(1).toLowerCase();
    const matches = SLASH_COMMANDS.filter((c) =>
      c.name.slice(1).toLowerCase().startsWith(q),
    );
    if (matches.length === 0) return null;
    return { token: firstToken, matches };
  }, [draft]);

  // Reset the suppression flag whenever the leading slash is gone so
  // typing "/" again re-opens the menu.
  useEffect(() => {
    if (!draft.startsWith("/")) setSlashDismissed(false);
  }, [draft]);

  // Clamp the highlighted index whenever the filtered list shrinks.
  useEffect(() => {
    if (!slashMatch) return;
    if (slashIdx >= slashMatch.matches.length) setSlashIdx(0);
  }, [slashMatch, slashIdx]);

  const slashOpen = !!slashMatch && !slashDismissed;

  function pickSlash(cmd: SlashCommand) {
    // Replace the first token with the canonical command name and keep
    // the rest of the draft untouched. Append a trailing space so the
    // caret sits ready for the user's actual message.
    const firstToken = draft.split(/\s/, 1)[0];
    const rest = draft.slice(firstToken.length);
    const sep = rest.length === 0 || /^\s/.test(rest) ? "" : " ";
    setDraft(`${cmd.name} ${rest.replace(/^\s+/, "")}${sep}`);
    setSlashDismissed(true);
    // Keep focus on the textarea after picking via mouse so the user
    // can keep typing without an extra click.
    requestAnimationFrame(() => textareaRef.current?.focus());
  }

  function onKey(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (slashOpen && slashMatch) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSlashIdx((i) => (i + 1) % slashMatch.matches.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setSlashIdx(
          (i) =>
            (i - 1 + slashMatch.matches.length) % slashMatch.matches.length,
        );
        return;
      }
      if (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey)) {
        e.preventDefault();
        pickSlash(slashMatch.matches[slashIdx] ?? slashMatch.matches[0]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setSlashDismissed(true);
        return;
      }
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void onSend();
    }
  }

  const effectiveModel =
    activeConv?.model ?? pendingModel ?? ovmsLoaded ?? null;
  const modelOptions = (() => {
    const seen = new Set<string>();
    const out: string[] = [];
    for (const m of localModels) {
      const id = m.hf_id ?? m.id;
      if (id && !seen.has(id)) {
        seen.add(id);
        out.push(id);
      }
    }
    if (ovmsLoaded && !seen.has(ovmsLoaded)) out.push(ovmsLoaded);
    return out;
  })();

  const activeSkillLabels = allSkills
    .filter((s) => enabledSkillIds.includes(s.id))
    .map((s) => s.name);

  // Build the flat "tools available to this chat" list the header
  // popover renders. Built-ins always show; for each enabled MCP server
  // we attach whatever the last probe returned (or an empty list — the
  // popover can re-probe on demand). Tool key matches the backend
  // catalog format: `<server_id>::<tool_name>`.
  const availableToolGroups = useMemo(() => {
    const groups: ChatToolGroup[] = [];
    const disabledBuiltinSet = new Set(disabledBuiltinNames);
    const enabledBuiltins = builtins.filter(
      (t) => !disabledBuiltinSet.has(t.name),
    );
    if (enabledBuiltins.length > 0) {
      groups.push({
        serverId: "builtin",
        serverName: "Built-in",
        kind: "builtin",
        probeLoading: false,
        probeError: null,
        tools: enabledBuiltins,
      });
    }
    for (const srv of mcpServersRaw?.filter((m) => m.enabled) ?? []) {
      const probe = probes[srv.id];
      groups.push({
        serverId: srv.id,
        serverName: srv.name || srv.id,
        kind: "mcp",
        probeLoading: !!probe?.loading,
        probeError: probe?.error ?? null,
        tools: probe?.tools ?? [],
      });
    }
    return groups;
  }, [builtins, disabledBuiltinNames, mcpServersRaw, probes]);

  const totalAvailableTools = availableToolGroups.reduce(
    (n, g) => n + g.tools.length,
    0,
  );
  const disabledForChatCount = useMemo(
    () => disabledTools.length,
    [disabledTools],
  );

  // Banner state. We only ever surface this for the local OVMS provider
  // because vLLM / ollama / remote endpoints don't have a single-loaded-
  // model constraint, so a mismatch wouldn't mean anything to the user.
  //
  // "Pending" is membership-based, not equality-based: OVMS can serve
  // multiple models concurrently, so the chat runner will short-circuit
  // the stage step when the target is already in `loaded_models` —
  // even if it isn't the convenience `loaded_model` pointer. Comparing
  // against the pointer (as the original code did) left the banner
  // stuck on "will be loaded on next send (currently <other>)" after
  // the user picked an already-staged model from the dropdown.
  const isOvmsProvider = activeProviderKind === "ovms";
  const swapInFlight =
    isOvmsProvider &&
    !!effectiveModel &&
    (ovmsStatus === "starting" || ovmsStatus === "stopping");
  const swapPending =
    isOvmsProvider &&
    !!effectiveModel &&
    ovmsStatus === "running" &&
    !ovmsLoadedModels?.some((m) => m.id === effectiveModel);
  const swapError = isOvmsProvider && ovmsStatus === "error" && !!ovmsLastError;

  return (
    <div className="flex min-h-0 min-w-0 flex-1">
      <Panel
        action={
          <ChatHeaderControls
            modelValue={activeConv?.model ?? pendingModel ?? ""}
            modelOptions={modelOptions}
            ovmsLoaded={ovmsLoaded}
            disabled={!!streamingId}
            onModelChange={async (v) => {
              // No active conversation yet — stash the pick locally so
              // we don't materialize an empty chat row in the sidebar
              // (and a corresponding DB record) just because the user
              // is browsing models. The pin gets applied to the real
              // conversation when `create()` runs on first send.
              if (!activeId) {
                setPendingModel(v || null);
                return;
              }
              await setModel(v || null);
            }}
            skillLabels={activeSkillLabels}
            toolGroups={availableToolGroups}
            totalAvailableTools={totalAvailableTools}
            disabledToolKeys={disabledTools}
            disabledForChatCount={disabledForChatCount}
            onProbeMcp={(id) => void probeMcp(id)}
            onToggleTool={async (key, disabled) => {
              // Auto-create the conversation on first interaction so a
              // user who opens the Tools popover before sending anything
              // can still pin their per-chat overrides. Mirrors the same
              // "create on demand" pattern used by the model picker and
              // the attach-files button above.
              const convId = activeId ?? (await create());
              if (!convId) return;
              await setToolDisabled(convId, key, disabled);
            }}
          />
        }
        className="flex-1"
        flush
      >
        <ServerStatusBanner
          inFlight={swapInFlight}
          pending={swapPending}
          errored={swapError}
          target={effectiveModel}
          loaded={ovmsLoaded}
          lastError={ovmsLastError}
        />
        <div
          ref={scrollRef}
          className="allow-select flex-1 space-y-4 overflow-auto p-4"
        >
          {messages.length === 0 && <ChatEmptyState />}
          {messages.map((m) => {
            const err = errorsByMsg[m.id];
            return (
              <MessageBubble
                key={m.id}
                role={m.role}
                streaming={m.id === streamingId}
              >
                {m.thinking && (
                  <details className="mb-2 rounded-md border border-tui-border bg-[var(--fluent-bg-subtle)] text-[11px] text-tui-fg-muted">
                    <summary className="cursor-pointer select-none px-2 py-1 text-tui-fg-dim hover:bg-[var(--fluent-bg-subtle-hover)]">
                      Thinking
                    </summary>
                    <div className="whitespace-pre-wrap border-t border-tui-border px-3 py-2">
                      {m.thinking}
                    </div>
                  </details>
                )}
                {m.role === "assistant" ? (
                  <AssistantContent text={m.content} />
                ) : (
                  // User / system / tool messages render as plain text.
                  // Markdown rendering would silently strip backslash
                  // escapes (`\.` → `.`, `\*` → `*`, …), corrupting
                  // Windows paths like `C:\Users\David\.zero\logs`
                  // visually even though the stored content and the
                  // version sent to the LLM stay correct. Plain-text
                  // rendering with `whitespace-pre-wrap` matches the
                  // "thinking" details block above and the convention
                  // of every other chat UI (ChatGPT, Claude, …).
                  <div className="whitespace-pre-wrap break-words">
                    {m.content}
                  </div>
                )}
                {m.attachments && m.attachments.length > 0 && (
                  <AttachmentList atts={m.attachments} />
                )}
                {err && activeId && (
                  <ChatErrorBanner
                    info={err}
                    onRetry={
                      err.retryable && m.id !== streamingId
                        ? () => void retry(m.id)
                        : undefined
                    }
                    onDismiss={() => dismissError(activeId, m.id)}
                  />
                )}
              </MessageBubble>
            );
          })}
        </div>

        <div className="border-t border-tui-border p-3">
          {(pending.length > 0 || turnPresets.size > 0) && (
            <div className="mb-2 flex flex-wrap gap-1.5">
              {TURN_PRESETS.filter((p) => turnPresets.has(p.key)).map((p) => (
                <TurnPresetChip
                  key={p.key}
                  preset={p}
                  onRemove={() => toggleTurnPreset(p.key)}
                />
              ))}
              {pending.map((a) => (
                <AttachmentChip
                  key={a.path}
                  att={a}
                  onRemove={() => void dropAttachment(a)}
                />
              ))}
            </div>
          )}
          <div className="flex items-center gap-2">
            <ChatComposerAddMenu
              onAttach={() => void attachFiles()}
              attaching={attaching}
              disabled={!!streamingId}
              activePresets={turnPresets}
              onTogglePreset={toggleTurnPreset}
            />
            <div
              className="relative flex-1"
              style={{
                height:
                  Math.min(8, Math.max(1, draft.split("\n").length)) * 20 + 12,
              }}
            >
              {slashOpen && slashMatch && (
                <SlashCommandMenu
                  matches={slashMatch.matches}
                  selected={Math.min(slashIdx, slashMatch.matches.length - 1)}
                  onPick={pickSlash}
                  onHover={setSlashIdx}
                />
              )}
              <TuiTextarea
                ref={textareaRef}
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={onKey}
                rows={Math.min(8, Math.max(1, draft.split("\n").length))}
                placeholder="Ask anything…"
                style={{
                  display: "block",
                  boxSizing: "border-box",
                  margin: 0,
                  lineHeight: "20px",
                  height: "100%",
                }}
              />
            </div>
            <ChatComposerSamplingButton
              samplingOverride={samplingOverride}
              providerSampling={activeProviderSampling}
              modelId={effectiveModel || null}
              disabled={!!streamingId}
              onChange={async (next) => {
                // Same "create on demand" rule the header chips use —
                // a user who tweaks sampling before sending anything
                // still gets a real conversation row to pin it on.
                const convId = activeId ?? (await create());
                if (!convId) return;
                await setSampling(convId, next);
              }}
            />
            {streamingId ? (
              <button
                type="button"
                onClick={() => void cancel()}
                title="Stop"
                aria-label="Stop"
                className={
                  "relative inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-[6px] " +
                  "border border-tui-border bg-[var(--fluent-bg-subtle)] text-tui-err " +
                  "transition-colors duration-150 ease-out " +
                  "hover:bg-[rgba(255,153,164,0.10)] hover:border-tui-err/40 " +
                  "active:bg-[rgba(255,153,164,0.16)] " +
                  "disabled:opacity-50 disabled:cursor-not-allowed " +
                  "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-tui-accent"
                }
              >
                <Spinner size="sm" />
              </button>
            ) : (
              <button
                type="button"
                onClick={() => void onSend()}
                disabled={!draft.trim() && pending.length === 0}
                title="Send"
                aria-label="Send"
                className={
                  "relative inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-[6px] " +
                  "border border-transparent bg-tui-accent-dim text-white " +
                  "shadow-[var(--fluent-shadow-2)] " +
                  "transition-[background-color,box-shadow] duration-150 ease-out " +
                  "hover:bg-[var(--fluent-accent-hover)] hover:shadow-[var(--fluent-shadow-4)] " +
                  "active:bg-[var(--fluent-accent-pressed)] active:scale-[0.98] " +
                  "disabled:opacity-40 disabled:cursor-not-allowed disabled:active:scale-100 " +
                  "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-tui-accent"
                }
              >
                <svg
                  width="14"
                  height="14"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <path d="M5 12h14M13 6l6 6-6 6" />
                </svg>
              </button>
            )}
          </div>
        </div>
      </Panel>
      {pendingConfirm && (
        <ToolConfirmModal
          request={pendingConfirm}
          onAllow={() => void resolveToolConfirm(pendingConfirm.call_id, true)}
          onDeny={() => void resolveToolConfirm(pendingConfirm.call_id, false)}
        />
      )}
    </div>
  );
}

function ChatHeaderControls({
  modelValue,
  modelOptions,
  ovmsLoaded,
  disabled,
  onModelChange,
  skillLabels,
  toolGroups,
  totalAvailableTools,
  disabledToolKeys,
  disabledForChatCount,
  onProbeMcp,
  onToggleTool,
}: {
  modelValue: string;
  modelOptions: string[];
  ovmsLoaded: string | null;
  disabled: boolean;
  onModelChange: (v: string) => void;
  skillLabels: string[];
  toolGroups: ChatToolGroup[];
  totalAvailableTools: number;
  disabledToolKeys: string[];
  disabledForChatCount: number;
  onProbeMcp: (serverId: string) => void;
  onToggleTool: (key: string, disabled: boolean) => void;
}) {
  const setView = useUiStore((s) => s.setView);
  const [open, setOpen] = useState<"skills" | "tools" | null>(null);
  const skillsRef = useRef<HTMLButtonElement>(null);
  const toolsRef = useRef<HTMLButtonElement>(null);

  // Close popovers on outside click / Escape.
  useEffect(() => {
    if (!open) return;
    function onPointer(e: MouseEvent) {
      const target = e.target as Node;
      if (skillsRef.current?.contains(target)) return;
      if (toolsRef.current?.contains(target)) return;
      // Anything tagged with the popover marker is also "inside".
      if ((target as Element)?.closest?.("[data-chat-popover]")) return;
      setOpen(null);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(null);
    }
    window.addEventListener("mousedown", onPointer);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onPointer);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  // When the conversation has no pinned model we fall back to whatever
  // OVMS already has loaded so the dropdown still shows something
  // meaningful. `ovmsLoaded` is pushed into `modelOptions` upstream so
  // the native select can render it as a real selected row without
  // needing a dedicated sentinel option.
  const fallbackLabel = ovmsLoaded ?? null;
  const selectedValue = modelValue || fallbackLabel || "";
  const noSelection = !selectedValue;

  return (
    <div className="flex items-center gap-1.5 text-[11px]">
      <select
        value={selectedValue}
        onChange={(e) => onModelChange(e.target.value)}
        disabled={disabled}
        title={
          modelValue
            ? `Pinned: ${modelValue}`
            : fallbackLabel
              ? `Loaded: ${fallbackLabel}`
              : "No model available — add one on the Models page"
        }
        className={
          "max-w-[220px] truncate rounded-[6px] " +
          "border border-tui-border bg-[var(--fluent-bg-subtle)] " +
          "py-[3px] pl-2.5 pr-7 text-[11px] text-tui-fg " +
          "transition-[background-color,border-color] duration-150 ease-out " +
          "hover:bg-[var(--fluent-bg-subtle-hover)] " +
          "focus:outline-none focus:border-b-tui-accent " +
          "disabled:opacity-60"
        }
      >
        {noSelection && (
          <option value="" disabled hidden>
            {modelOptions.length === 0 ? "No models" : "Select a model"}
          </option>
        )}
        {modelOptions.map((id) => (
          <option key={id} value={id}>
            {id}
          </option>
        ))}
      </select>

      <HeaderChip
        ref={skillsRef}
        label="Skills"
        count={skillLabels.length}
        active={open === "skills"}
        onClick={() => setOpen(open === "skills" ? null : "skills")}
        title={skillLabels.join(", ") || "no skills enabled"}
      />
      <HeaderChip
        ref={toolsRef}
        label="Tools"
        count={Math.max(0, totalAvailableTools - disabledForChatCount)}
        active={open === "tools"}
        onClick={() => setOpen(open === "tools" ? null : "tools")}
        title={
          disabledForChatCount > 0
            ? `${disabledForChatCount} disabled for this chat`
            : `${totalAvailableTools} tools available to the model`
        }
      />

      {open === "skills" && (
        <HeaderPopover
          anchor={skillsRef.current}
          title="Enabled skills"
          emptyHint="No skills enabled. Open the Skills page to author and toggle them."
          items={skillLabels.map((label, i) => ({
            key: `${i}-${label}`,
            label,
          }))}
          manageLabel="Manage skills"
          onManage={() => {
            setView("skills");
            setOpen(null);
          }}
          onClose={() => setOpen(null)}
        />
      )}
      {open === "tools" && (
        <ChatToolsPopover
          anchor={toolsRef.current}
          groups={toolGroups}
          disabledKeys={disabledToolKeys}
          onProbe={onProbeMcp}
          onToggle={onToggleTool}
          onManage={() => {
            setView("tools");
            setOpen(null);
          }}
          onClose={() => setOpen(null)}
        />
      )}
    </div>
  );
}

// ─── Composer add-menu ( + icon, left of textarea ) ──────────────
//
// Win11-style "new actions" popover that opens above a `+` icon at
// the left of the chat input. Exposes:
//   * "Add photos & files" — calls the existing `attachFiles` flow.
//   * Per-turn capability toggles (web search, deep research) —
//     surfaced as togglable rows that mirror the runner's `/web` and
//     `/research` slash commands. The slash prefix is auto-prepended
//     in `onSend` so the user never has to type the command.
function ChatComposerAddMenu({
  onAttach,
  attaching,
  disabled,
  activePresets,
  onTogglePreset,
}: {
  onAttach: () => void;
  attaching: boolean;
  disabled: boolean;
  activePresets: Set<TurnPreset>;
  onTogglePreset: (key: TurnPreset) => void;
}) {
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  // The `+` glyph reads as a tiny status indicator too: when any
  // preset is on, the button picks up the accent color so the user
  // can see at a glance that a turn-scoped unlock is active even
  // when the menu is closed and the chip row has scrolled out.
  const presetCount = activePresets.size;

  useEffect(() => {
    if (!open) return;
    function onPointer(e: MouseEvent) {
      if (containerRef.current?.contains(e.target as Node)) return;
      setOpen(false);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    window.addEventListener("mousedown", onPointer);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onPointer);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div ref={containerRef} className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        disabled={disabled || attaching}
        aria-expanded={open}
        aria-haspopup="menu"
        title="Add files and more"
        aria-label="Add files and more"
        className={
          "relative inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-[6px] " +
          "border border-tui-border bg-[var(--fluent-bg-subtle)] text-tui-fg-muted " +
          "transition-colors duration-150 ease-out " +
          "hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-fg " +
          "disabled:opacity-50 disabled:hover:bg-[var(--fluent-bg-subtle)] " +
          (open || presetCount > 0
            ? "border-tui-accent/40 bg-tui-selection text-tui-accent"
            : "")
        }
      >
        {attaching ? (
          <Spinner size="sm" />
        ) : (
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
          >
            <path d="M12 5v14M5 12h14" />
          </svg>
        )}
      </button>

      {open && (
        <div
          role="menu"
          data-chat-popover
          className={
            "fluent-acrylic absolute left-0 bottom-full z-40 mb-2 " +
            "flex w-64 flex-col overflow-hidden " +
            "rounded-[10px] border border-tui-border " +
            "shadow-[var(--fluent-shadow-16)]"
          }
          style={{
            animation: "fluent-view-in 160ms var(--fluent-curve-decel) both",
          }}
          onClick={(e) => e.stopPropagation()}
        >
          <ul className="py-1">
            <li>
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setOpen(false);
                  onAttach();
                }}
                className={
                  "flex w-full items-center gap-2.5 px-3 py-2 text-[12px] text-tui-fg " +
                  "transition-colors duration-150 ease-out " +
                  "hover:bg-[var(--fluent-bg-subtle-hover)]"
                }
              >
                <svg
                  width="14"
                  height="14"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.75"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  className="text-tui-fg-muted"
                >
                  <path d="M21 11.5 11.6 21a5.5 5.5 0 0 1-7.8-7.8L13 4a4 4 0 0 1 5.7 5.7L9.2 19.2a2.5 2.5 0 0 1-3.5-3.5L14 7.4" />
                </svg>
                Add photos &amp; files
              </button>
            </li>
            <li
              aria-hidden="true"
              className="my-1 border-t border-tui-border"
            />
            {TURN_PRESETS.map((p) => {
              const active = activePresets.has(p.key);
              return (
                <li key={p.key}>
                  <button
                    type="button"
                    role="menuitemcheckbox"
                    aria-checked={active}
                    onClick={() => onTogglePreset(p.key)}
                    title={
                      p.inverse
                        ? `${p.hint} (default is ${p.command} — toggle on to suppress it)`
                        : `${p.hint} — maps to ${p.command}`
                    }
                    className={
                      "flex w-full items-center gap-2.5 px-3 py-2 text-[12px] " +
                      "transition-colors duration-150 ease-out " +
                      "hover:bg-[var(--fluent-bg-subtle-hover)] " +
                      (active ? "text-tui-accent" : "text-tui-fg")
                    }
                  >
                    <span
                      className={
                        active ? "text-tui-accent" : "text-tui-fg-muted"
                      }
                      aria-hidden="true"
                    >
                      {p.icon}
                    </span>
                    <span className="flex-1 text-left">{p.label}</span>
                    {active && (
                      <svg
                        width="12"
                        height="12"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        strokeWidth="2.25"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        aria-hidden="true"
                      >
                        <path d="m5 12 5 5L20 7" />
                      </svg>
                    )}
                  </button>
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </div>
  );
}

// Removable pill that surfaces an active per-turn preset alongside
// the attachment chips. Clicking the × turns the preset back off so
// the user has a single place to undo a mis-toggle without re-opening
// the `+` menu.
function TurnPresetChip({
  preset,
  onRemove,
}: {
  preset: TurnPresetSpec;
  onRemove: () => void;
}) {
  return (
    <span
      className={
        "inline-flex items-center gap-1.5 rounded-full border px-2 py-[2px] text-[11px] font-medium " +
        "border-tui-accent/40 bg-tui-selection text-tui-accent"
      }
      title={
        preset.inverse
          ? `${preset.hint} — default ${preset.command} suppressed for next turn`
          : `${preset.hint} — active for the next turn (${preset.command})`
      }
    >
      <span aria-hidden="true" className="text-tui-accent">
        {preset.icon}
      </span>
      {preset.label}
      <button
        type="button"
        onClick={onRemove}
        aria-label={`Disable ${preset.label}`}
        className="-mr-1 rounded-full p-[1px] text-tui-accent/70 hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-accent"
      >
        <svg
          width="10"
          height="10"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2.5"
          strokeLinecap="round"
        >
          <path d="M18 6 6 18M6 6l12 12" />
        </svg>
      </button>
    </span>
  );
}

// ─── Composer sampling button ( gear icon, right of textarea ) ───
//
// Replaces the old "Sampling" header chip with a compact gear icon
// sitting next to the Send button. Clicking it pops the same
// `ChatSamplingPopover` editor used in the header — anchored above
// the icon (`placement="top"`) so the panel doesn't slide off the
// bottom of the viewport.
function ChatComposerSamplingButton({
  samplingOverride,
  providerSampling,
  modelId,
  disabled,
  onChange,
}: {
  samplingOverride: SamplingConfig | null;
  providerSampling: SamplingConfig | null;
  modelId: string | null;
  disabled: boolean;
  onChange: (next: SamplingConfig) => void | Promise<void>;
}) {
  const setView = useUiStore((s) => s.setView);
  const [open, setOpen] = useState(false);
  const buttonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    function onPointer(e: MouseEvent) {
      const target = e.target as Node;
      if (buttonRef.current?.contains(target)) return;
      if ((target as Element)?.closest?.("[data-chat-popover]")) return;
      setOpen(false);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    window.addEventListener("mousedown", onPointer);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onPointer);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const overrideCount = samplingOverrideCount(
    samplingOverride ?? EMPTY_SAMPLING,
  );
  const titleText =
    samplingOverride && overrideCount > 0
      ? `Sampling override: ${summariseSampling(samplingOverride)}`
      : "Sampling — using provider / model defaults";

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        onClick={() => setOpen((v) => !v)}
        disabled={disabled}
        aria-expanded={open}
        aria-haspopup="dialog"
        title={titleText}
        aria-label="Sampling settings"
        className={
          "relative inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-[6px] " +
          "border border-tui-border bg-[var(--fluent-bg-subtle)] text-tui-fg-muted " +
          "transition-colors duration-150 ease-out " +
          "hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-fg " +
          "disabled:opacity-50 disabled:hover:bg-[var(--fluent-bg-subtle)] " +
          (open ? "border-tui-accent/40 bg-tui-selection text-tui-accent" : "")
        }
      >
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.75"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <circle cx="12" cy="12" r="3" />
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33h.01a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51h.01a1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v.01a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
        </svg>
        {overrideCount > 0 && (
          <span
            aria-hidden="true"
            className="absolute -right-0.5 -top-0.5 h-2 w-2 rounded-full bg-tui-accent ring-2 ring-tui-bg"
          />
        )}
      </button>
      {open && (
        <ChatSamplingPopover
          anchor={buttonRef.current}
          value={samplingOverride ?? EMPTY_SAMPLING}
          providerSampling={providerSampling}
          modelId={modelId}
          placement="top"
          onChange={(next) => void onChange(next)}
          onManage={() => {
            setView("settings");
            setOpen(false);
          }}
          onClose={() => setOpen(false)}
        />
      )}
    </>
  );
}

const HeaderChip = forwardRef<
  HTMLButtonElement,
  {
    label: string;
    count: number;
    active: boolean;
    onClick: () => void;
    title?: string;
  }
>(function HeaderChip({ label, count, active, onClick, title }, ref) {
  return (
    <button
      ref={ref}
      type="button"
      onClick={onClick}
      aria-expanded={active}
      title={title}
      className={
        "inline-flex items-center gap-1.5 rounded-full border px-2 py-[3px] text-[11px] font-medium " +
        "transition-colors duration-150 ease-out " +
        (active
          ? "border-tui-accent/40 bg-tui-selection text-tui-accent"
          : count > 0
            ? "border-tui-border bg-[var(--fluent-bg-subtle)] text-tui-fg hover:bg-[var(--fluent-bg-subtle-hover)]"
            : "border-tui-border bg-[var(--fluent-bg-subtle)] text-tui-fg-muted hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-fg")
      }
    >
      <span
        aria-hidden="true"
        className={
          "h-1.5 w-1.5 rounded-full " +
          (count > 0 ? "bg-tui-accent" : "bg-tui-fg-muted")
        }
      />
      {label}
      <span
        className={
          "min-w-[1.25rem] rounded-full px-1 text-center text-[10px] " +
          (count > 0
            ? "bg-tui-accent-dim/30 text-tui-fg"
            : "bg-[var(--fluent-bg-subtle-pressed)] text-tui-fg-muted")
        }
      >
        {count || 0}
      </span>
    </button>
  );
});

function HeaderPopover({
  anchor,
  title,
  emptyHint,
  items,
  manageLabel,
  onManage,
  onClose,
}: {
  anchor: HTMLElement | null;
  title: string;
  emptyHint: string;
  items: { key: string; label: string }[];
  manageLabel: string;
  onManage: () => void;
  onClose: () => void;
}) {
  // Re-measure the anchor on open / resize / scroll so the popover
  // stays glued under the chip even when the user moves the window.
  const [pos, setPos] = useState<{ top: number; right: number } | null>(null);
  useEffect(() => {
    if (!anchor) return;
    function reposition() {
      const r = anchor!.getBoundingClientRect();
      setPos({
        top: r.bottom + 6,
        right: Math.max(8, window.innerWidth - r.right),
      });
    }
    reposition();
    window.addEventListener("resize", reposition);
    window.addEventListener("scroll", reposition, true);
    return () => {
      window.removeEventListener("resize", reposition);
      window.removeEventListener("scroll", reposition, true);
    };
  }, [anchor]);

  if (!anchor || !pos) return null;

  return createPortal(
    <div
      role="dialog"
      data-chat-popover
      style={{
        position: "fixed",
        top: pos.top,
        right: pos.right,
        zIndex: 60,
        animation: "fluent-view-in 160ms var(--fluent-curve-decel) both",
      }}
      className={
        "w-72 overflow-hidden rounded-[10px] " +
        "fluent-acrylic border border-tui-border " +
        "shadow-[var(--fluent-shadow-16)]"
      }
      onClick={(e) => e.stopPropagation()}
    >
      <div className="flex items-center justify-between gap-2 border-b border-tui-border px-3 py-2">
        <span className="text-[11px] font-semibold uppercase tracking-wide text-tui-fg-muted">
          {title}
        </span>
        <span className="text-[10px] text-tui-fg-muted">{items.length}</span>
      </div>
      {items.length === 0 ? (
        <p className="px-3 py-3 text-[11px] text-tui-fg-muted">{emptyHint}</p>
      ) : (
        <ul className="max-h-64 overflow-auto py-1">
          {items.map((it) => (
            <li
              key={it.key}
              className="flex items-center gap-2 px-3 py-1.5 text-[12px] text-tui-fg"
            >
              <span
                aria-hidden="true"
                className="h-1.5 w-1.5 shrink-0 rounded-full bg-tui-accent"
              />
              <span className="truncate">{it.label}</span>
            </li>
          ))}
        </ul>
      )}
      <div className="flex items-center justify-between gap-2 border-t border-tui-border px-3 py-2">
        <button
          type="button"
          onClick={onManage}
          className="rounded-[4px] px-1.5 py-0.5 text-[11px] font-medium text-tui-accent hover:bg-[var(--fluent-bg-subtle-hover)]"
        >
          {manageLabel} →
        </button>
        <button
          type="button"
          onClick={onClose}
          className="rounded-[4px] px-1.5 py-0.5 text-[11px] text-tui-fg-muted hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-fg"
        >
          Close
        </button>
      </div>
    </div>,
    document.body,
  );
}

// ─── Tools popover ( per-conversation tool overrides ) ───────────
//
// Renders the live tool catalog grouped by server (built-in first, then
// each enabled MCP server) with a checkbox per tool. Toggling a row
// flips the corresponding entry in the per-conversation
// `disabled_tools` set the runner subtracts from its catalog. Disabling
// a tool here ONLY affects this chat — the global enable on the Tools
// page is unchanged. MCP groups show a Probe button when no catalog
// has been fetched yet so the user can pull live names + descriptions
// without leaving the chat.

interface ChatToolGroup {
  serverId: string;
  serverName: string;
  kind: "builtin" | "mcp";
  probeLoading: boolean;
  probeError: string | null;
  tools: McpToolSchema[];
}

function toolKey(serverId: string, toolName: string): string {
  return `${serverId}::${toolName}`;
}

function ChatToolsPopover({
  anchor,
  groups,
  disabledKeys,
  onProbe,
  onToggle,
  onManage,
  onClose,
}: {
  anchor: HTMLElement | null;
  groups: ChatToolGroup[];
  disabledKeys: string[];
  onProbe: (serverId: string) => void;
  onToggle: (key: string, disabled: boolean) => void;
  onManage: () => void;
  onClose: () => void;
}) {
  // Same anchor-following positioning trick as `HeaderPopover`.
  const [pos, setPos] = useState<{ top: number; right: number } | null>(null);
  useEffect(() => {
    if (!anchor) return;
    function reposition() {
      const r = anchor!.getBoundingClientRect();
      setPos({
        top: r.bottom + 6,
        right: Math.max(8, window.innerWidth - r.right),
      });
    }
    reposition();
    window.addEventListener("resize", reposition);
    window.addEventListener("scroll", reposition, true);
    return () => {
      window.removeEventListener("resize", reposition);
      window.removeEventListener("scroll", reposition, true);
    };
  }, [anchor]);

  const disabledSet = useMemo(() => new Set(disabledKeys), [disabledKeys]);
  const totalTools = groups.reduce((n, g) => n + g.tools.length, 0);
  const enabledTools = totalTools - disabledKeys.length;

  if (!anchor || !pos) return null;

  return createPortal(
    <div
      role="dialog"
      data-chat-popover
      style={{
        position: "fixed",
        top: pos.top,
        right: pos.right,
        zIndex: 60,
        animation: "fluent-view-in 160ms var(--fluent-curve-decel) both",
      }}
      className={
        "w-96 max-w-[92vw] overflow-hidden rounded-[10px] " +
        "fluent-acrylic border border-tui-border " +
        "shadow-[var(--fluent-shadow-16)]"
      }
      onClick={(e) => e.stopPropagation()}
    >
      <div className="flex items-center justify-between gap-2 border-b border-tui-border px-3 py-2">
        <span className="text-[11px] font-semibold uppercase tracking-wide text-tui-fg-muted">
          Tools for this chat
        </span>
        <span className="text-[10px] text-tui-fg-muted">
          {enabledTools}/{totalTools} on
        </span>
      </div>
      <p className="border-b border-tui-border px-3 py-2 text-[11px] text-tui-fg-dim">
        Toggle a tool off to hide it from the model on this chat only. The
        global enable on the Tools page is unchanged.
      </p>
      {totalTools === 0 ? (
        <div className="px-3 py-3 text-[11px] text-tui-fg-muted">
          No tools available. Configure built-ins or add an MCP server on the
          Tools page.
        </div>
      ) : (
        <ul className="max-h-[60vh] overflow-auto">
          {groups.map((group) => (
            <li key={group.serverId} className="border-b border-tui-border">
              <div className="flex items-baseline justify-between gap-2 bg-[rgba(255,255,255,0.022)] px-3 py-1.5">
                <div className="flex items-baseline gap-2">
                  <span className="text-[11px] font-semibold uppercase tracking-wide text-tui-fg-dim">
                    {group.serverName}
                  </span>
                  <span className="text-[10px] text-tui-fg-muted">
                    {group.tools.length} tool
                    {group.tools.length === 1 ? "" : "s"}
                  </span>
                </div>
                {group.kind === "mcp" && group.tools.length === 0 && (
                  <button
                    type="button"
                    onClick={() => onProbe(group.serverId)}
                    className="rounded-[4px] border border-tui-border px-1.5 py-0.5 text-[10px] text-tui-fg-dim hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-fg"
                  >
                    {group.probeLoading ? "Probing…" : "Probe"}
                  </button>
                )}
              </div>
              {group.probeError && (
                <div className="border-t border-tui-border bg-[rgba(255,153,164,0.06)] px-3 py-1.5 text-[10px] text-tui-err">
                  {group.probeError}
                </div>
              )}
              {group.tools.length === 0 ? (
                <p className="px-3 py-2 text-[11px] text-tui-fg-muted">
                  {group.kind === "mcp"
                    ? "No catalog cached. Probe to populate."
                    : "No tools registered."}
                </p>
              ) : (
                <ul>
                  {group.tools.map((t) => {
                    const key = toolKey(group.serverId, t.name);
                    const off = disabledSet.has(key);
                    return (
                      <li
                        key={t.name}
                        className={
                          "flex items-start gap-2 border-t border-tui-border px-3 py-1.5" +
                          (off ? " opacity-60" : "")
                        }
                      >
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-2">
                            <span
                              className={
                                "truncate font-medium " +
                                (off
                                  ? "text-tui-fg-muted line-through"
                                  : "text-tui-accent")
                              }
                            >
                              {t.name}
                            </span>
                            {t.destructive && (
                              <span className="rounded-[3px] border border-tui-warn/40 bg-[rgba(252,225,0,0.10)] px-1 text-[9px] text-tui-warn">
                                destructive
                              </span>
                            )}
                          </div>
                          {t.description && (
                            <div className="truncate text-[11px] text-tui-fg-dim">
                              {t.description}
                            </div>
                          )}
                        </div>
                        <TuiButton
                          variant={off ? "primary" : "danger"}
                          onClick={() => onToggle(key, !off)}
                          aria-label={`${off ? "Enable" : "Disable"} ${t.name} for this chat`}
                        >
                          {off ? "Enable" : "Disable"}
                        </TuiButton>
                      </li>
                    );
                  })}
                </ul>
              )}
            </li>
          ))}
        </ul>
      )}
      <div className="flex items-center justify-between gap-2 border-t border-tui-border px-3 py-2">
        <button
          type="button"
          onClick={onManage}
          className="rounded-[4px] px-1.5 py-0.5 text-[11px] font-medium text-tui-accent hover:bg-[var(--fluent-bg-subtle-hover)]"
        >
          Manage tools →
        </button>
        <button
          type="button"
          onClick={onClose}
          className="rounded-[4px] px-1.5 py-0.5 text-[11px] text-tui-fg-muted hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-fg"
        >
          Close
        </button>
      </div>
    </div>,
    document.body,
  );
}

// ─── Sampling popover ( per-conversation sampling override ) ────
//
// Renders the same `SamplingEditor` the Settings → Providers card
// uses, but writes to the per-conversation `sampling` column instead
// of the provider config. Empty fields here mean "fall through to the
// provider's sampling defaults" — we surface the provider's resolved
// value as a placeholder so the user sees what they're inheriting.

function ChatSamplingPopover({
  anchor,
  value,
  providerSampling,
  modelId,
  onChange,
  onManage,
  onClose,
  placement = "bottom",
}: {
  anchor: HTMLElement | null;
  value: SamplingConfig;
  /**
   * Provider-level fallback shown as input placeholders. `null` when
   * the provider config hasn't loaded yet (extremely brief window at
   * cold start) — we fall back to the per-model profile number in
   * that case (same source the chat runner uses on the wire).
   */
  providerSampling: SamplingConfig | null;
  /**
   * Effective model for this chat (pinned override or the OVMS-loaded
   * fallback). Drives the "and-finally" placeholder when neither the
   * conversation nor the provider has a value for a field.
   */
  modelId: string | null;
  onChange: (next: SamplingConfig) => void;
  onManage: () => void;
  onClose: () => void;
  /**
   * Where to render the popover relative to its anchor. `"bottom"`
   * (default) keeps the original chat-header behaviour; `"top"` flips
   * the panel above the trigger so the gear icon next to the Send
   * button doesn't push it off-screen at the composer.
   */
  placement?: "top" | "bottom";
}) {
  // Same anchor-following positioning trick as `HeaderPopover` /
  // `ChatToolsPopover` — keeps the popover glued to the trigger when
  // the user moves or resizes the window. We store either a `top`
  // (for the default below-anchor placement) or a `bottom`
  // (for the above-anchor placement used at the composer) so the
  // popover can grow upward without first measuring its own height.
  const [pos, setPos] = useState<
    { top: number; right: number } | { bottom: number; right: number } | null
  >(null);
  useEffect(() => {
    if (!anchor) return;
    function reposition() {
      const r = anchor!.getBoundingClientRect();
      const right = Math.max(8, window.innerWidth - r.right);
      if (placement === "top") {
        setPos({
          bottom: Math.max(8, window.innerHeight - r.top + 6),
          right,
        });
      } else {
        setPos({ top: r.bottom + 6, right });
      }
    }
    reposition();
    window.addEventListener("resize", reposition);
    window.addEventListener("scroll", reposition, true);
    return () => {
      window.removeEventListener("resize", reposition);
      window.removeEventListener("scroll", reposition, true);
    };
  }, [anchor, placement]);

  if (!anchor || !pos) return null;

  // Build per-field placeholder strings that show the next-layer-down
  // value (provider override → falls through to per-model profile if
  // unset). Keeps the popover honest about what "blank" actually means.
  const profileDefaults = modelSamplingDefaults(modelId);
  function placeholder(field: keyof SamplingConfig): string {
    const v = providerSampling?.[field];
    if (v != null) return `provider: ${v}`;
    return formatSamplingDefault(profileDefaults[field]);
  }

  const overrideCount = samplingOverrideCount(value);

  return createPortal(
    <div
      role="dialog"
      data-chat-popover
      style={{
        position: "fixed",
        ...pos,
        zIndex: 60,
        animation: "fluent-view-in 160ms var(--fluent-curve-decel) both",
      }}
      className={
        "w-[22rem] max-w-[92vw] overflow-hidden rounded-[10px] " +
        "fluent-acrylic border border-tui-border " +
        "shadow-[var(--fluent-shadow-16)]"
      }
      onClick={(e) => e.stopPropagation()}
    >
      <div className="flex items-center justify-between gap-2 border-b border-tui-border px-3 py-2">
        <span className="text-[11px] font-semibold uppercase tracking-wide text-tui-fg-muted">
          Sampling for this chat
        </span>
        <span className="text-[10px] text-tui-fg-muted">
          {overrideCount === 0
            ? "no override"
            : `${overrideCount} override${overrideCount === 1 ? "" : "s"}`}
        </span>
      </div>
      <p className="border-b border-tui-border px-3 py-2 text-[11px] text-tui-fg-dim">
        Overrides apply to this chat only. Blank fields fall back to the
        provider defaults, and then to the per-model recommendation.
      </p>
      <div className="px-3 py-3">
        <SamplingEditor
          value={value}
          onChange={onChange}
          temperaturePlaceholder={placeholder("temperature")}
          topPPlaceholder={placeholder("top_p")}
          topKPlaceholder={placeholder("top_k")}
        />
      </div>
      <div className="flex items-center justify-between gap-2 border-t border-tui-border px-3 py-2">
        <button
          type="button"
          onClick={onManage}
          className="rounded-[4px] px-1.5 py-0.5 text-[11px] font-medium text-tui-accent hover:bg-[var(--fluent-bg-subtle-hover)]"
        >
          Provider defaults →
        </button>
        <button
          type="button"
          onClick={onClose}
          className="rounded-[4px] px-1.5 py-0.5 text-[11px] text-tui-fg-muted hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-fg"
        >
          Close
        </button>
      </div>
    </div>,
    document.body,
  );
}

function SlashCommandMenu({
  matches,
  selected,
  onPick,
  onHover,
}: {
  matches: SlashCommand[];
  selected: number;
  onPick: (cmd: SlashCommand) => void;
  onHover: (idx: number) => void;
}) {
  // Anchored to the relatively-positioned wrapper around the textarea
  // so the menu floats just above the input. `mb-1.5` gives it a hair
  // of breathing room above the field; `shadow-16` lifts it above the
  // pending-attachments strip without needing a portal.
  return (
    <div
      role="listbox"
      aria-label="Slash commands"
      className={
        "fluent-mica absolute right-0 bottom-full left-0 z-30 mb-1.5 " +
        "overflow-hidden rounded-[8px] border border-tui-border " +
        "shadow-[var(--fluent-shadow-16)]"
      }
    >
      <div className="flex items-center justify-between gap-2 border-b border-tui-border px-3 py-1.5 text-[10px] uppercase tracking-wide text-tui-fg-muted">
        <span>Slash commands</span>
        <span className="normal-case tracking-normal">
          ↑↓ navigate · ⏎ / Tab select · Esc dismiss
        </span>
      </div>
      <ul className="max-h-64 overflow-auto py-1">
        {matches.map((cmd, i) => {
          const active = i === selected;
          return (
            <li key={cmd.name} className="list-none">
              <button
                type="button"
                role="option"
                aria-selected={active}
                onMouseDown={(e) => {
                  // Stop the textarea from losing focus before our
                  // click handler runs — a mousedown on a button
                  // outside the textarea blurs the textarea by
                  // default, which would close the menu before pick.
                  e.preventDefault();
                  onPick(cmd);
                }}
                onMouseEnter={() => onHover(i)}
                className={
                  "flex w-full items-center gap-3 px-3 py-1.5 text-left text-[12px] " +
                  "transition-colors duration-100 ease-out " +
                  (active
                    ? "bg-[var(--fluent-bg-subtle-selected)] text-tui-fg"
                    : "text-tui-fg-dim hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-fg")
                }
              >
                <span className="font-mono text-[12px] font-semibold text-tui-fg">
                  {cmd.name}
                </span>
                <span className="shrink-0 rounded-[3px] border border-tui-border bg-[var(--fluent-bg-subtle)] px-1.5 py-px text-[10px] text-tui-fg-muted">
                  {cmd.hint}
                </span>
                <span className="truncate text-[11px] text-tui-fg-muted">
                  {cmd.desc}
                </span>
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

function ChatEmptyState() {
  return (
    <div className="mx-auto flex h-full max-w-md flex-col items-center justify-center text-center">
      <div className="mb-5 flex h-14 w-14 items-center justify-center rounded-2xl bg-tui-accent-dim text-white shadow-[var(--fluent-shadow-8)]">
        <svg
          width="26"
          height="26"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.75"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path d="M21 12a8 8 0 0 1-11.6 7.15L4 21l1.85-5.4A8 8 0 1 1 21 12Z" />
        </svg>
      </div>
      <h2
        className="text-[20px] font-semibold text-tui-fg"
        style={{ fontFamily: "var(--font-display)" }}
      >
        What’s on your mind today?
      </h2>
    </div>
  );
}

function MessageBubble({
  role,
  children,
  streaming,
}: {
  role: string;
  children: React.ReactNode;
  streaming?: boolean;
}) {
  const tag = {
    user: {
      label: "You",
      color: "text-tui-accent",
      bubble: "border border-tui-accent/30 bg-tui-selection text-tui-fg",
      avatar: "bg-tui-accent-dim text-white shadow-[var(--fluent-shadow-2)]",
      align: "items-end",
      avatarGlyph: (
        <svg
          width="12"
          height="12"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          strokeLinecap="round"
        >
          <circle cx="12" cy="8" r="4" />
          <path d="M4 21a8 8 0 0 1 16 0" />
        </svg>
      ),
    },
    assistant: {
      label: "Assistant",
      color: "text-tui-fg",
      bubble:
        "border border-tui-border bg-[var(--fluent-bg-subtle)] text-tui-fg",
      avatar: "bg-tui-bg-elev text-tui-accent border border-tui-border",
      align: "items-start",
      avatarGlyph: (
        <svg
          width="12"
          height="12"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.6"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path d="M4 5h6v6H4zM14 5h6v6h-6zM4 13h6v6H4zM14 13h6v6h-6z" />
        </svg>
      ),
    },
    system: {
      label: "System",
      color: "text-tui-warn",
      bubble:
        "border border-tui-warn/30 bg-[rgba(252,225,0,0.06)] text-tui-fg-dim",
      avatar:
        "bg-[rgba(252,225,0,0.10)] text-tui-warn border border-tui-warn/30",
      align: "items-start",
      avatarGlyph: <span className="text-[11px]">!</span>,
    },
    tool: {
      label: "Tool",
      color: "text-tui-fg-dim",
      bubble:
        "border border-tui-border bg-[var(--fluent-bg-subtle)] text-tui-fg-dim",
      avatar: "bg-tui-bg-elev text-tui-fg-dim border border-tui-border",
      align: "items-start",
      avatarGlyph: (
        <svg
          width="12"
          height="12"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.6"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path d="M14.7 6.3a4 4 0 0 0-5.4 5.4L3 18v3h3l6.3-6.3a4 4 0 0 0 5.4-5.4l-2.6 2.6-2.4-2.4 2.6-2.6Z" />
        </svg>
      ),
    },
  }[role] ?? {
    label: role,
    color: "text-tui-fg",
    bubble: "border border-tui-border bg-[var(--fluent-bg-subtle)] text-tui-fg",
    avatar: "bg-tui-bg-elev text-tui-fg-dim border border-tui-border",
    align: "items-start",
    avatarGlyph: <span className="text-[10px]">·</span>,
  };

  const isUser = role === "user";

  return (
    <div
      className={`flex flex-col gap-1 text-[13px] ${isUser ? "items-end" : tag.align}`}
    >
      <div
        className={`flex items-center gap-1.5 ${isUser ? "flex-row-reverse" : ""}`}
      >
        <span
          className={`flex h-5 w-5 items-center justify-center rounded-full ${tag.avatar}`}
          aria-hidden="true"
        >
          {tag.avatarGlyph}
        </span>
        <span className={`text-[11px] font-semibold ${tag.color}`}>
          {tag.label}
        </span>
      </div>
      <div
        className={
          `max-w-[88%] rounded-[10px] px-3.5 py-2.5 leading-relaxed shadow-[var(--fluent-shadow-2)] ${tag.bubble} ` +
          (streaming ? "fluent-typing" : "")
        }
      >
        {children}
      </div>
    </div>
  );
}

/**
 * Inline preview for attachments stored on a sent message. Images get a
 * small thumbnail (via Tauri's `convertFileSrc` for the local-file
 * protocol); docs render as a paperclip + filename chip.
 */
function AttachmentList({ atts }: { atts: Attachment[] }) {
  return (
    <div className="mt-2 flex flex-wrap gap-2">
      {atts.map((a) => {
        if (a.kind === "image") {
          const src = (() => {
            try {
              return convertFileSrc(a.path);
            } catch {
              return null;
            }
          })();
          return (
            <div
              key={a.path}
              className="flex flex-col items-start gap-0.5"
              title={`${a.name} · ${bytes(a.bytes)}`}
            >
              {src ? (
                <img
                  src={src}
                  alt={a.name}
                  className="max-h-40 max-w-[14rem] rounded-md border border-tui-border bg-tui-bg object-contain"
                />
              ) : (
                <div className="rounded-md border border-tui-border bg-[var(--fluent-bg-subtle)] px-2 py-1 text-[11px] text-tui-fg-muted">
                  {a.name}
                </div>
              )}
              <span className="text-[10px] text-tui-fg-muted">
                {a.name} · {bytes(a.bytes)}
              </span>
            </div>
          );
        }
        return (
          <div
            key={a.path}
            className="inline-flex items-center gap-1.5 rounded-md border border-tui-border bg-[var(--fluent-bg-subtle)] px-2 py-1 text-[11px] text-tui-fg-dim"
            title={`${a.path} · ${bytes(a.bytes)}`}
          >
            <svg
              width="12"
              height="12"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.75"
              strokeLinecap="round"
              strokeLinejoin="round"
              className="text-tui-fg-muted"
            >
              <path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z" />
              <path d="M14 3v6h6" />
            </svg>
            <span className="max-w-[24ch] truncate">{a.name}</span>
            <span className="text-[10px] text-tui-fg-muted">
              {bytes(a.bytes)}
            </span>
          </div>
        );
      })}
    </div>
  );
}

/**
 * Removable chip rendered above the input for files queued for the
 * next send. Image chips show a small thumbnail; doc chips just show
 * the filename.
 */
function AttachmentChip({
  att,
  onRemove,
}: {
  att: Attachment;
  onRemove: () => void;
}) {
  const src =
    att.kind === "image"
      ? (() => {
          try {
            return convertFileSrc(att.path);
          } catch {
            return null;
          }
        })()
      : null;
  return (
    <div
      className="group inline-flex items-center gap-1.5 rounded-md border border-tui-border bg-[var(--fluent-bg-subtle)] py-1 pl-1 pr-2 text-[11px] text-tui-fg-dim"
      title={att.name}
    >
      {src ? (
        <img
          src={src}
          alt={att.name}
          className="h-6 w-6 rounded object-cover"
        />
      ) : (
        <span className="flex h-6 w-6 items-center justify-center rounded bg-[var(--fluent-bg-subtle-hover)] text-tui-fg-muted">
          <svg
            width="12"
            height="12"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.75"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z" />
            <path d="M14 3v6h6" />
          </svg>
        </span>
      )}
      <span className="max-w-[18ch] truncate">{att.name}</span>
      <span className="text-[10px] text-tui-fg-muted">{bytes(att.bytes)}</span>
      <button
        onClick={onRemove}
        className="flex h-4 w-4 items-center justify-center rounded text-tui-fg-muted transition-colors hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-err"
        title="Remove"
        aria-label="Remove attachment"
      >
        <svg
          width="10"
          height="10"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2.5"
          strokeLinecap="round"
        >
          <path d="M18 6 6 18M6 6l12 12" />
        </svg>
      </button>
    </div>
  );
}

function ChatErrorBanner({
  info,
  onRetry,
  onDismiss,
}: {
  info: import("@/stores/chat").ChatErrorInfo;
  onRetry?: () => void;
  onDismiss: () => void;
}) {
  const headline = (() => {
    switch (info.kind) {
      case "ovms_not_running":
        return "OVMS is not running";
      case "no_active_provider":
        return "no active provider";
      case "no_model_selected":
        return "no model selected";
      case "upstream_unreachable":
        return `cannot reach ${info.provider_kind ?? "upstream"}`;
      case "upstream_http":
        return "upstream rejected the request";
      case "unsupported_provider":
        return "unsupported provider kind";
      default:
        return "chat failed";
    }
  })();
  return (
    <div
      className="mt-2 rounded-md border border-tui-err/30 bg-[rgba(255,153,164,0.06)] px-3 py-2 text-[12px]"
      role="alert"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 font-semibold text-tui-err">
            <svg
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <circle cx="12" cy="12" r="10" />
              <path d="M12 8v4M12 16h.01" />
            </svg>
            {headline}
          </div>
          {info.hint && <div className="mt-1 text-tui-fg-dim">{info.hint}</div>}
          <details className="mt-2 text-[11px] text-tui-fg-muted">
            <summary className="cursor-pointer select-none">Details</summary>
            <div className="mt-1 whitespace-pre-wrap break-words rounded border border-tui-border bg-[var(--fluent-bg-subtle)] p-2 font-mono text-[11px] text-tui-fg-dim">
              {info.error}
              {info.base_url && `\n\nbase_url: ${info.base_url}`}
              {info.provider_kind && `\nprovider: ${info.provider_kind}`}
              {`\nkind: ${info.kind}`}
            </div>
          </details>
        </div>
        <div className="flex shrink-0 flex-col items-end gap-1">
          {onRetry && (
            <TuiButton variant="primary" onClick={onRetry}>
              Retry
            </TuiButton>
          )}
          <button
            onClick={onDismiss}
            className="rounded px-2 py-0.5 text-[11px] text-tui-fg-muted transition-colors hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-fg"
            title="Dismiss"
          >
            Dismiss
          </button>
        </div>
      </div>
    </div>
  );
}

/**
 * Inline banner shown above the message list whenever the local OVMS
 * server is doing something the user cares about for *this* conversation:
 *
 * - `inFlight`: OVMS is mid-swap (`starting` / `stopping`). Show the
 *   target model + a spinner so the user knows their next reply is going
 *   to be a bit slow rather than wondering why nothing is happening.
 * - `pending`: OVMS is running, but the loaded model isn't the one the
 *   chat header is pinned to. Sending will trigger an auto-swap.
 * - `errored`: OVMS bailed out (`status === "error"`). Show the upstream
 *   message and point at the Server page.
 *
 * The component renders nothing when none of the above is true, so it's
 * safe to mount unconditionally.
 */
function ServerStatusBanner({
  inFlight,
  pending,
  errored,
  target,
  loaded,
  lastError,
}: {
  inFlight: boolean;
  pending: boolean;
  errored: boolean;
  target: string | null;
  loaded: string | null;
  lastError: string | null;
}) {
  if (errored) {
    return (
      <div
        className="border-b border-tui-err/30 bg-[rgba(255,153,164,0.06)] px-3 py-2 text-[12px]"
        role="alert"
      >
        <div className="flex items-center gap-2">
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            className="text-tui-err"
          >
            <circle cx="12" cy="12" r="10" />
            <path d="M12 8v4M12 16h.01" />
          </svg>
          <span className="font-semibold text-tui-err">OVMS error</span>
          <span className="text-tui-fg-dim">
            {lastError ?? "unknown error"}
          </span>
          <span className="text-tui-fg-muted">
            — open the Server page to recover
          </span>
        </div>
      </div>
    );
  }
  if (inFlight) {
    return (
      <div
        className="flex items-center gap-2 border-b border-tui-border bg-[var(--fluent-bg-subtle)] px-3 py-2 text-[12px] text-tui-fg-dim"
        role="status"
        aria-live="polite"
      >
        <Spinner size="sm" />
        <span>
          Loading model <span className="text-tui-accent">{target ?? "…"}</span>
          {loaded && loaded !== target ? (
            <span className="text-tui-fg-muted"> (was {loaded})</span>
          ) : null}
          … first response may take a moment.
        </span>
      </div>
    );
  }
  if (pending) {
    return (
      <div
        className="flex items-center gap-2 border-b border-tui-border bg-[var(--fluent-bg-subtle)] px-3 py-2 text-[12px] text-tui-fg-dim"
        role="status"
      >
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          className="text-tui-accent"
        >
          <circle cx="12" cy="12" r="10" />
          <path d="M12 16v-4M12 8h.01" />
        </svg>
        <span>
          Model <span className="text-tui-accent">{target}</span> will be loaded
          on next send
          {loaded ? (
            <span className="text-tui-fg-muted"> (currently {loaded})</span>
          ) : null}
        </span>
      </div>
    );
  }
  return null;
}

// ─── Assistant content parser + renderer ──────────────────────────────
//
// The chat runner inlines tool-call, tool-result, and per-round
// thinking markers into the streamed assistant text using fixed
// templates from `render_tool_call_banner`, `render_result_block`, and
// `render_thinking_block` in `src-tauri/src/chat/runner.rs`:
//
//     [tool call: SERVER/TOOL]\n```json\n{...}\n```
//     [tool result]\n```\n...\n```
//     [thinking]\n...\n[/thinking]
//
// We parse those out and render them as collapsible cards so a chatty
// tool doesn't blow out the chat bubble visually, and so the reasoning
// for each agent round sits *next to* the tool calls it produced
// instead of being collapsed into one accumulated block at the top of
// the turn. Anything between (or outside) the markers renders as
// normal whitespace-preserving text.

type Segment =
  | { kind: "text"; text: string }
  | { kind: "call"; server: string; tool: string; args: string }
  | { kind: "result"; body: string }
  | { kind: "thinking"; body: string };

/**
 * Split assistant text into a sequence of text / tool-call / tool-result
 * / thinking segments. Robust to partial / streaming input: an
 * unterminated fence (no closing ``` or `[/thinking]`) is rendered with
 * whatever body has arrived so far so the user sees something while
 * the model is still typing.
 */
function parseAssistantContent(text: string): Segment[] {
  const segments: Segment[] = [];
  let cursor = 0;
  while (cursor < text.length) {
    const callIdx = text.indexOf("[tool call:", cursor);
    const resultIdx = text.indexOf("[tool result]", cursor);
    const thinkIdx = text.indexOf("[thinking]", cursor);
    // Pick whichever marker comes first; ties are broken by the natural
    // order they're checked in (matches the runner's own emission order:
    // thinking → tool call → tool result).
    let kind: "call" | "result" | "thinking" | null = null;
    let idx = -1;
    const candidates: Array<{
      kind: "call" | "result" | "thinking";
      idx: number;
    }> = [];
    if (callIdx !== -1) candidates.push({ kind: "call", idx: callIdx });
    if (resultIdx !== -1) candidates.push({ kind: "result", idx: resultIdx });
    if (thinkIdx !== -1) candidates.push({ kind: "thinking", idx: thinkIdx });
    for (const c of candidates) {
      if (idx === -1 || c.idx < idx) {
        idx = c.idx;
        kind = c.kind;
      }
    }
    if (kind === null) {
      segments.push({ kind: "text", text: text.slice(cursor) });
      break;
    }
    if (idx > cursor) {
      segments.push({ kind: "text", text: text.slice(cursor, idx) });
    }
    if (kind === "call") {
      const closeBracket = text.indexOf("]", idx);
      if (closeBracket === -1) {
        segments.push({ kind: "text", text: text.slice(idx) });
        break;
      }
      const header = text.slice(idx + "[tool call: ".length, closeBracket);
      const slash = header.indexOf("/");
      const server = slash === -1 ? "" : header.slice(0, slash);
      const tool = slash === -1 ? header : header.slice(slash + 1);

      const fenceOpen = text.indexOf("```json\n", closeBracket);
      if (fenceOpen === -1) {
        // Header present but no body yet — render as a streaming card with
        // empty args so the user sees "calling…" right away.
        segments.push({ kind: "call", server, tool, args: "" });
        cursor = closeBracket + 1;
        continue;
      }
      const argsStart = fenceOpen + "```json\n".length;
      const fenceClose = text.indexOf("\n```", argsStart);
      const args =
        fenceClose === -1
          ? text.slice(argsStart)
          : text.slice(argsStart, fenceClose);
      segments.push({ kind: "call", server, tool, args });
      cursor = fenceClose === -1 ? text.length : fenceClose + "\n```".length;
    } else if (kind === "result") {
      const headerEnd = idx + "[tool result]".length;
      const fenceOpen = text.indexOf("```\n", headerEnd);
      if (fenceOpen === -1) {
        segments.push({ kind: "result", body: "" });
        cursor = headerEnd;
        continue;
      }
      const bodyStart = fenceOpen + "```\n".length;
      const fenceClose = text.indexOf("\n```", bodyStart);
      const body =
        fenceClose === -1
          ? text.slice(bodyStart)
          : text.slice(bodyStart, fenceClose);
      segments.push({ kind: "result", body });
      cursor = fenceClose === -1 ? text.length : fenceClose + "\n```".length;
    } else {
      // thinking — `[thinking]\n…\n[/thinking]`. While the round is
      // still streaming we won't have seen the closing tag yet; render
      // the partial body so the user can read the reasoning live.
      const bodyStart = idx + "[thinking]".length;
      const close = text.indexOf("[/thinking]", bodyStart);
      const body =
        close === -1
          ? text.slice(bodyStart).replace(/^\n/, "").replace(/\n+$/, "")
          : text.slice(bodyStart, close).replace(/^\n/, "").replace(/\n+$/, "");
      segments.push({ kind: "thinking", body });
      cursor = close === -1 ? text.length : close + "[/thinking]".length;
    }
  }
  // Merge adjacent text segments so we don't render a useless extra <div>
  // for the tiny whitespace between markers.
  const merged: Segment[] = [];
  for (const s of segments) {
    const last = merged[merged.length - 1];
    if (s.kind === "text" && last && last.kind === "text") {
      last.text += s.text;
    } else {
      merged.push(s);
    }
  }
  return merged;
}

function AssistantContent({ text }: { text: string }) {
  const segments = useMemo(() => parseAssistantContent(text), [text]);
  return (
    <div className="space-y-1">
      {segments.map((s, i) => {
        if (s.kind === "text") {
          if (!s.text) return null;
          return (
            <div key={i}>
              <Markdown>{s.text}</Markdown>
            </div>
          );
        }
        // A tool result that immediately follows a tool call belongs to
        // it visually — collapse the gap so the pair reads as one
        // operation instead of two free-floating cards.
        const prev = i > 0 ? segments[i - 1] : null;
        const flushWithPrev =
          (s.kind === "result" && prev?.kind === "call") ||
          (s.kind === "call" && prev?.kind === "call");
        const pairClass = flushWithPrev ? "-mt-1" : "";
        if (s.kind === "thinking") {
          // Inline per-round reasoning. Styled like the message-level
          // "Thinking" panel at the top of the bubble (muted, small,
          // collapsible) so the two stay visually consistent, but
          // anchored next to the tool calls of *this* round.
          return (
            <details
              key={i}
              className="overflow-hidden rounded-md border border-tui-border bg-[var(--fluent-bg-subtle)] text-[11px] text-tui-fg-muted"
            >
              <summary className="flex cursor-pointer select-none items-center gap-2 px-2 py-1 text-tui-fg-dim hover:bg-[var(--fluent-bg-subtle-hover)]">
                <svg
                  width="12"
                  height="12"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.75"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <path d="M12 3a7 7 0 0 0-4 12.7V18a2 2 0 0 0 2 2h4a2 2 0 0 0 2-2v-2.3A7 7 0 0 0 12 3Z" />
                  <path d="M9 22h6" />
                </svg>
                Thinking
              </summary>
              <div className="whitespace-pre-wrap border-t border-tui-border px-3 py-2">
                {s.body}
              </div>
            </details>
          );
        }
        if (s.kind === "call") {
          return (
            <details
              key={i}
              className={`overflow-hidden rounded-md border border-tui-border bg-[var(--fluent-bg-subtle)] text-[12px] ${pairClass}`}
            >
              <summary className="flex cursor-pointer select-none items-center gap-2 px-2 py-1 text-tui-accent hover:bg-[var(--fluent-bg-subtle-hover)]">
                <svg
                  width="12"
                  height="12"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.75"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <path d="M14.7 6.3a4 4 0 0 0-5.4 5.4L3 18v3h3l6.3-6.3a4 4 0 0 0 5.4-5.4l-2.6 2.6-2.4-2.4 2.6-2.6Z" />
                </svg>
                <span className="text-tui-fg-dim">Calling</span>
                <span className="text-tui-fg">
                  {s.server || "?"}
                  <span className="text-tui-fg-muted">/</span>
                  {s.tool}
                </span>
              </summary>
              {s.args && (
                <pre className="overflow-x-auto border-t border-tui-border px-3 py-2 font-mono text-[11px] text-tui-fg-dim">
                  {s.args}
                </pre>
              )}
            </details>
          );
        }
        // tool result
        return (
          <details
            key={i}
            className={`overflow-hidden rounded-md border border-tui-border bg-[var(--fluent-bg-subtle)] text-[12px] ${pairClass}`}
          >
            <summary className="flex cursor-pointer select-none items-center gap-2 px-2 py-1 text-tui-fg-dim hover:bg-[var(--fluent-bg-subtle-hover)]">
              <svg
                width="12"
                height="12"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="M9 14 4 9l5-5" />
                <path d="M20 20v-7a4 4 0 0 0-4-4H4" />
              </svg>
              Tool result
            </summary>
            <pre className="overflow-x-auto whitespace-pre-wrap border-t border-tui-border px-3 py-2 font-mono text-[11px] text-tui-fg">
              {s.body}
            </pre>
          </details>
        );
      })}
    </div>
  );
}

// ─── Destructive-tool confirm modal ──────────────────────────────────
//
// The chat runner blocks the streaming task while a destructive tool is
// awaiting confirmation. We render this on top of the chat as a focused
// modal because the user can't usefully do anything else with the chat
// view until they decide.

function ToolConfirmModal({
  request,
  onAllow,
  onDeny,
}: {
  request: ToolConfirmRequest;
  onAllow: () => void;
  onDeny: () => void;
}) {
  const argsText = useMemo(() => {
    try {
      return JSON.stringify(request.arguments, null, 2);
    } catch {
      return String(request.arguments ?? "");
    }
  }, [request.arguments]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/40 pt-20"
      role="dialog"
      aria-modal="true"
      aria-labelledby="tool-confirm-title"
      onClick={onDeny}
    >
      <div
        className="w-[560px] max-w-[92vw] overflow-hidden rounded-xl border border-tui-border bg-tui-bg-elev shadow-[var(--fluent-shadow-16)]"
        onClick={(e) => e.stopPropagation()}
      >
        <header
          id="tool-confirm-title"
          className="flex items-center justify-between gap-3 border-b border-tui-border px-4 py-3"
        >
          <div className="flex items-center gap-2">
            <span className="flex h-7 w-7 items-center justify-center rounded-md bg-[rgba(255,153,164,0.12)] text-tui-err">
              <svg
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="m12 3 10 18H2L12 3Z" />
                <path d="M12 10v4M12 18h.01" />
              </svg>
            </span>
            <span className="text-[14px] font-semibold text-tui-fg">
              Destructive tool requested
            </span>
          </div>
          <span className="text-[11px] text-tui-fg-muted">
            {request.server_id}
          </span>
        </header>
        <div className="space-y-3 px-4 py-3 text-[12px]">
          <div className="text-tui-fg">
            The model wants to call{" "}
            <code className="rounded bg-[var(--fluent-bg-subtle)] px-1 py-px font-mono text-tui-accent">
              {request.tool}
            </code>{" "}
            on{" "}
            <code className="rounded bg-[var(--fluent-bg-subtle)] px-1 py-px font-mono text-tui-fg">
              {request.server_name || request.server_id}
            </code>
            .
          </div>
          {request.description && (
            <div className="text-tui-fg-dim">{request.description}</div>
          )}
          <details
            className="rounded-md border border-tui-border bg-[var(--fluent-bg-subtle)] text-[11px]"
            open
          >
            <summary className="cursor-pointer select-none px-3 py-1.5 text-tui-fg-dim hover:bg-[var(--fluent-bg-subtle-hover)]">
              Arguments
            </summary>
            <pre className="max-h-[40vh] overflow-auto whitespace-pre-wrap border-t border-tui-border px-3 py-2 font-mono text-tui-fg-dim">
              {argsText}
            </pre>
          </details>
          <div className="text-[11px] text-tui-fg-muted">
            Toggle <code className="font-mono">destructive_tool_confirm</code>{" "}
            off in Settings to stop being prompted for tools like this one.
          </div>
        </div>
        <footer className="flex justify-end gap-2 border-t border-tui-border px-4 py-3">
          <TuiButton onClick={onDeny}>Deny</TuiButton>
          <TuiButton variant="danger" onClick={onAllow}>
            Allow once
          </TuiButton>
        </footer>
      </div>
    </div>
  );
}
