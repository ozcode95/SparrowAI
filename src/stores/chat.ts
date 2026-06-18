import { create } from "zustand";
import { invoke, on, Events } from "@/lib/tauri";
import { useServerStore } from "@/stores/server";
import { useSettingsStore, type SamplingConfig } from "@/stores/settings";
import { EMPTY_SAMPLING } from "@/stores/settings";

/// Title used for freshly-created conversations until either the user
/// renames the chat manually or `send()` auto-derives one from the first
/// user message. Kept in a single place so the auto-rename check stays in
/// sync with `create()` and the backend default.
export const DEFAULT_CONVERSATION_TITLE = "New chat";

/// Maximum length (in characters) of an auto-derived conversation title.
/// Tuned to fit the sidebar row without truncation on a typical window
/// width while still being recognisable.
const AUTO_TITLE_MAX_CHARS = 48;

/// Derive a human-friendly conversation title from arbitrary message text.
/// Collapses whitespace, strips control characters, and truncates with a
/// trailing ellipsis when needed. Returns an empty string when the input
/// is blank — callers should treat that as "don't rename".
export function deriveConversationTitle(text: string): string {
  const cleaned = text.replace(/\s+/g, " ").trim();
  if (!cleaned) return "";
  if (cleaned.length <= AUTO_TITLE_MAX_CHARS) return cleaned;
  // Prefer cutting at the last word boundary inside the budget so titles
  // don't end mid-word; fall back to a hard cut for pathological input.
  const slice = cleaned.slice(0, AUTO_TITLE_MAX_CHARS);
  const lastSpace = slice.lastIndexOf(" ");
  const trimmed =
    lastSpace > AUTO_TITLE_MAX_CHARS / 2 ? slice.slice(0, lastSpace) : slice;
  return `${trimmed.trimEnd()}\u2026`;
}

export type Role = "system" | "user" | "assistant" | "tool";

export interface Message {
  id: string;
  conversation_id: string;
  role: Role;
  content: string;
  thinking?: string | null;
  created_at: string;
  attachments?: Attachment[];
  /**
   * Per-turn capability flags the composer attached to this user
   * message (web search, deep research, thinking opt-in, autonomous
   * loop). Only ever set on user rows; absent / `null` on every
   * other role. Legacy rows persisted before the column existed
   * also surface as missing here — the Rust runner falls back to
   * slash-prefix parsing in that case so old chats keep working.
   */
  turn_overrides?: TurnOverrides | null;
}

/**
 * Per-turn capability flags forwarded to the chat runner. Each field
 * maps to a previously slash-gated behaviour:
 *   - `web` → unlock `web.search` + `web.read_page` (was `/web`).
 *   - `research` → unlock `web.deep_research` + `web.read_page`
 *     (was `/research`).
 *   - `think` → opt **into** the model's reasoning trace for this
 *     turn. Default for every chat is *no thinking*; this flag is
 *     the per-turn opt-in (Gemma 4 gets its `<|think|>` control
 *     token; families without one just see no `/no_think`).
 *   - `loop` → autonomous-agent mode (was `/loop`).
 *
 * All fields are optional on the wire because the Rust side defaults
 * each missing flag to `false` — callers only need to send the keys
 * they want set.
 */
export interface TurnOverrides {
  web?: boolean;
  research?: boolean;
  think?: boolean;
  loop?: boolean;
}

export interface Attachment {
  kind: "image" | "audio" | "document";
  path: string;
  mime: string;
  bytes: number;
  /** Original filename the user picked. Surfaced in chips + bubbles. */
  name: string;
}

export interface Conversation {
  id: string;
  title: string;
  created_at: string;
  updated_at: string;
  model: string | null;
}

interface ChatDeltaPayload {
  conversation_id: string;
  message_id: string;
  delta: string;
  thinking?: boolean;
}

/// Payload for `chat://rewrite`. The handler *replaces* the message's
/// `content` with `content` rather than appending like a delta. Emitted
/// by the runner after it consumes a legacy ```tool_use``` fenced-JSON
/// block so the un-trimmed text already streamed into the bubble gets
/// scrubbed before the tool banners append on top.
interface ChatRewritePayload {
  conversation_id: string;
  message_id: string;
  content: string;
}

interface ChatDonePayload {
  conversation_id: string;
  message_id: string;
  cancelled?: boolean;
}

/// Stable string tags emitted by `chat://error`. Keep in sync with
/// `ChatErrorKind` in `src-tauri/src/chat/runner.rs`.
export type ChatErrorKind =
  | "no_active_provider"
  | "unsupported_provider"
  | "no_model_selected"
  | "ovms_not_running"
  | "upstream_unreachable"
  | "upstream_http"
  | "other";

interface ChatErrorPayload {
  conversation_id: string;
  message_id: string;
  error: string;
  kind?: ChatErrorKind;
  hint?: string;
  provider_kind?: string;
  base_url?: string;
  retryable?: boolean;
}

/// Diagnostics stored against a failed assistant message. Surfaced in the
/// chat bubble by `Chat.tsx` so the user can see a one-line hint + retry
/// without digging through the raw error text.
export interface ChatErrorInfo {
  message_id: string;
  error: string;
  kind: ChatErrorKind;
  hint?: string;
  provider_kind?: string;
  base_url?: string;
  retryable: boolean;
}

/// In-flight destructive-tool confirmation prompt. The runner blocks the
/// stream waiting for `chat_tool_confirm(call_id, allow)` so we render a
/// modal until the user makes a choice (or the chat is cancelled).
export interface ToolConfirmRequest {
  conversation_id: string;
  message_id: string;
  call_id: string;
  server_id: string;
  server_name: string;
  tool: string;
  description: string;
  arguments: unknown;
  destructive: boolean;
}

interface SendResult {
  user: Message;
  assistant: Message;
}

interface ChatState {
  conversations: Conversation[];
  activeId: string | null;
  messages: Record<string, Message[]>;
  streamingMessageId: string | null;
  loading: boolean;
  /// Per-conversation map of the *latest* error for each assistant message.
  /// Cleared when a retry succeeds (first delta lands) or when the user
  /// dismisses it.
  errors: Record<string, Record<string, ChatErrorInfo>>;
  /// Outstanding destructive-tool confirm prompts, keyed by call_id.
  /// Insertion order is preserved by the underlying Record (each new
  /// confirm gets a fresh key) so the modal can render the oldest one.
  toolConfirms: Record<string, ToolConfirmRequest>;
  /// Per-conversation tool disable list. Entries are catalog keys of the
  /// form `<server_id>::<tool_name>` and represent tools the user turned
  /// off in the chat-header tools popover (overriding the global enable
  /// on the Tools page). Loaded lazily on demand and persisted via
  /// `chat_set_disabled_tools`.
  disabledTools: Record<string, string[]>;
  /// Per-conversation sampling override. `null` means "loaded and empty"
  /// (the runner will fall through to the provider + profile defaults);
  /// `undefined` means "not loaded yet". Loaded lazily on first popover
  /// open and persisted via `chat_set_sampling`.
  sampling: Record<string, SamplingConfig>;
  /// Model the user picked from the chat header *before* a conversation
  /// exists (the new "+ New chat" flow defers the DB write until the
  /// first message). Materialized onto the freshly-created conversation
  /// inside `create()` and cleared on `newChat` / `open` / first apply.
  pendingModel: string | null;

  list: () => Promise<void>;
  open: (id: string) => Promise<void>;
  /// Drop the active conversation (without touching the DB) so the chat
  /// panel renders the empty "new chat" state. The actual DB-backed
  /// conversation is created lazily on the first message send.
  newChat: () => void;
  create: (title?: string) => Promise<string>;
  remove: (id: string) => Promise<void>;
  rename: (id: string, title: string) => Promise<void>;
  setModel: (model: string | null) => Promise<void>;
  setPendingModel: (model: string | null) => Promise<void>;
  send: (
    text: string,
    attachments?: Attachment[],
    overrides?: TurnOverrides,
  ) => Promise<void>;
  cancel: () => Promise<void>;
  retry: (messageId: string) => Promise<void>;
  dismissError: (conversationId: string, messageId: string) => void;
  resolveToolConfirm: (callId: string, allow: boolean) => Promise<void>;
  loadDisabledTools: (conversationId: string) => Promise<void>;
  setToolDisabled: (
    conversationId: string,
    key: string,
    disabled: boolean,
  ) => Promise<void>;
  loadSampling: (conversationId: string) => Promise<void>;
  setSampling: (
    conversationId: string,
    sampling: SamplingConfig,
  ) => Promise<void>;

  bindEvents: () => Promise<() => void>;
}

/// Stage `model` on the local OVMS instance and evict the prior
/// text-generation lineup. Shared by `setModel` (per-conversation pin)
/// and `setPendingModel` (header pick before a conversation exists) so
/// both code paths give the user the same "loading…" banner behaviour
/// as the Models page's explicit Load button.
///
/// Best-effort — the chat runner has the same fallback at send-time, so
/// failures here just log and let the runner surface the real error on
/// the next request.
async function swapOvmsTextGen(model: string): Promise<void> {
  const settings = useSettingsStore.getState();
  const provider = settings.providers.find(
    (p) => p.id === settings.active_provider_id,
  );
  if (!provider || provider.kind !== "ovms") return;
  const server = useServerStore.getState();
  const info = server.info;
  // Nothing to do when OVMS isn't ready (not installed, mid-install) —
  // the Server page is the right place for the user to react.
  if (!info) return;
  if (info.status === "not_installed" || info.status === "installing") return;

  // Snapshot which text-generation models OVMS is already serving so
  // we can evict them *after* the new one is up. Non-text-gen models
  // (embeddings, rerank, image, …) are left alone — they serve
  // different chat roles and the user may rely on them concurrently.
  //
  // We capture this *before* the load so the swap stays deterministic
  // even if the live `loaded_models` array shifts mid-flight (a
  // background `ovms://status` event could otherwise race the eviction).
  const previousTextGen = (info.loaded_models ?? [])
    .filter((m) => m.task === "text_generation" && m.id !== model)
    .map((m) => m.id);

  // If the target is already the only text-gen model loaded, there's
  // nothing to swap.
  if (
    previousTextGen.length === 0 &&
    info.loaded_models?.some((m) => m.id === model)
  ) {
    return;
  }

  try {
    // Load the new model first so there's no window where the chat
    // runner sees zero text-gen models available.
    if (!info.loaded_models?.some((m) => m.id === model)) {
      await server.loadModel(model);
    }
    // Evict the prior text-gen lineup. Failures here are best-effort
    // — the new model is already serving, so the chat works either
    // way and a leftover loaded model just wastes VRAM until the user
    // restarts OVMS.
    for (const oldId of previousTextGen) {
      try {
        await server.removeModel(oldId);
      } catch (e) {
        console.warn(`ovms_remove_model(${oldId}) failed`, e);
      }
    }
  } catch (e) {
    console.warn("eager ovms model swap failed (will retry on send)", e);
  }
}

export const useChatStore = create<ChatState>((set, get) => ({
  conversations: [],
  activeId: null,
  messages: {},
  streamingMessageId: null,
  loading: false,
  errors: {},
  toolConfirms: {},
  disabledTools: {},
  sampling: {},
  pendingModel: null,

  list: async () => {
    set({ loading: true });
    try {
      const conversations =
        (await invoke<Conversation[]>("chat_list_conversations")) ?? [];
      set({ conversations, loading: false });
    } catch (e) {
      console.error("chat_list_conversations failed", e);
      set({ loading: false });
    }
  },

  open: async (id) => {
    set({ activeId: id, pendingModel: null });
    if (get().messages[id]) return;
    try {
      const msgs =
        (await invoke<Message[]>("chat_list_messages", {
          conversationId: id,
        })) ?? [];
      set((s) => ({ messages: { ...s.messages, [id]: msgs } }));
    } catch (e) {
      console.error("chat_list_messages failed", e);
    }
  },

  newChat: () => {
    // Reset to the empty "about to start a new chat" state without
    // hitting the backend. A real Conversation row only gets created on
    // the first `send()` (or any other action that needs a conv id),
    // which keeps abandoned drafts out of the sidebar and the DB.
    set({ activeId: null, pendingModel: null });
  },

  create: async (title) => {
    const resolvedTitle = title ?? DEFAULT_CONVERSATION_TITLE;
    const id = await invoke<string>("chat_create_conversation", {
      title: resolvedTitle,
    });
    if (!id) {
      // Backend refused — still reconcile the list so the UI doesn't go
      // stale, then bail without flipping `activeId`.
      await get().list();
      return id;
    }
    // Eagerly seed local state so the sidebar updates and the chat
    // panel snaps to an empty fresh view *before* the follow-up
    // `chat_list_conversations` round-trip resolves. Without this, the
    // user can click "+ New chat" and keep staring at the previous
    // conversation's messages until the reconciliation lands — and if
    // anything in that chain slows down (or a concurrent `send` is
    // mid-flight), the new chat never visibly takes over.
    const now = new Date().toISOString();
    set((s) => {
      const exists = s.conversations.some((c) => c.id === id);
      const conv: Conversation = {
        id,
        title: resolvedTitle,
        model: null,
        created_at: now,
        updated_at: now,
      };
      return {
        activeId: id,
        conversations: exists ? s.conversations : [conv, ...s.conversations],
        // Force an empty buffer for the new id so the messages selector
        // can never fall back to a stale entry from a previous chat.
        messages: { ...s.messages, [id]: [] },
      };
    });
    // Materialize any model the user picked from the header *before*
    // the conversation existed. Done here (rather than in every caller
    // that lazy-creates) so the pin survives whichever path triggered
    // the create — send, attach, tool toggle, etc.
    const pending = get().pendingModel;
    if (pending) {
      try {
        await get().setModel(pending);
        // Only clear pendingModel after setModel succeeds so the bottom
        // bar can display it during the async update.
        set({ pendingModel: null });
      } catch (e) {
        console.warn("applying pending model on create failed", e);
        // Still clear on error to avoid stale state.
        set({ pendingModel: null });
      }
    }

    // Reconcile with server-truth in the background. We don't await it
    // because the eager seed above is already enough to make the UI
    // correct; the refresh just fills in the canonical `updated_at` /
    // ordering once the backend has had a chance to commit.
    //
    // This MUST run *after* the `setModel` await above. Otherwise the
    // `chat_list_conversations` SELECT can race against the
    // `chat_set_model` UPDATE and ship back a stale row whose `model`
    // is still `null`, clobbering the just-pinned value and leaving
    // the bottom-bar "model" pill blank for the rest of the chat.
    void get().list();

    return id;
  },

  remove: async (id) => {
    await invoke("chat_delete_conversation", { conversationId: id });
    set((s) => {
      const { [id]: _drop, ...rest } = s.messages;
      return {
        messages: rest,
        activeId: s.activeId === id ? null : s.activeId,
      };
    });
    await get().list();
  },

  rename: async (id, title) => {
    const trimmed = title.trim();
    const normalized =
      trimmed.length > 0 ? trimmed : DEFAULT_CONVERSATION_TITLE;
    // Mirror locally first so the sidebar updates instantly; the backend
    // round-trip just persists the change.
    set((s) => ({
      conversations: s.conversations.map((c) =>
        c.id === id ? { ...c, title: normalized } : c,
      ),
    }));
    try {
      await invoke("chat_set_title", {
        conversationId: id,
        title: normalized,
      });
    } catch (e) {
      console.error("chat_set_title failed", e);
      // Don't roll back — the user sees the desired title in the sidebar.
      // A follow-up `list()` (e.g. on next create) will reconcile if the
      // backend really did reject the change.
    }
  },

  setPendingModel: async (model) => {
    set({ pendingModel: model });
    // Trigger the same OVMS swap that `setModel` runs once a
    // conversation exists. The user picked a model from the chat
    // header — they expect it to start loading immediately, the same
    // way the Models page "Load" button behaves. Without this, a fresh
    // "+ New chat" would silently defer the load until the first
    // `send()`, leaving the bottom-bar pill blank in the meantime.
    if (model) await swapOvmsTextGen(model);
  },

  setModel: async (model) => {
    const convId = get().activeId;
    if (!convId) return;
    await invoke("chat_set_model", { conversationId: convId, model });
    // Reflect the change locally without a round-trip.
    set((s) => ({
      conversations: s.conversations.map((c) =>
        c.id === convId ? { ...c, model } : c,
      ),
    }));
    if (model) await swapOvmsTextGen(model);
  },

  send: async (text, attachments, overrides) => {
    const convId = get().activeId ?? (await get().create());
    if (!convId) return;

    // Capture pre-send state so we can decide whether to auto-title the
    // chat after the user's first message lands. We deliberately read
    // *before* the splice below so a freshly-created chat (zero messages,
    // default title) is correctly recognised as eligible.
    const stateBefore = get();
    const convBefore = stateBefore.conversations.find((c) => c.id === convId);
    const msgsBefore = stateBefore.messages[convId] ?? [];
    const isFirstUserMessage = !msgsBefore.some((m) => m.role === "user");
    const titleIsDefault =
      (convBefore?.title ?? "").trim() === DEFAULT_CONVERSATION_TITLE;

    // Only ship `overrides` to the backend when at least one flag is
    // actually on so the wire stays clean for the common case (every
    // plain prompt). The Rust side treats a missing payload as the
    // all-false default.
    const hasOverride =
      !!overrides &&
      (overrides.web === true ||
        overrides.research === true ||
        overrides.think === true ||
        overrides.loop === true);

    const res = await invoke<SendResult>("chat_send_message", {
      conversationId: convId,
      content: text,
      attachments: attachments ?? [],
      overrides: hasOverride ? overrides : null,
    });
    if (!res) return;
    // Optimistically splice both turns in. The runner will then stream deltas
    // into the assistant row via `chat://delta`.
    set((s) => {
      const list = s.messages[convId] ?? [];
      return {
        messages: {
          ...s.messages,
          [convId]: [...list, res.user, res.assistant],
        },
        streamingMessageId: res.assistant.id,
      };
    });

    // Auto-rename the conversation from its first user message so the
    // sidebar stops showing a generic "New chat" row the moment the chat
    // actually has a topic. We only do this when the title is still the
    // default placeholder — a user who already renamed the chat (or one
    // seeded with a custom title via `create(title)`) is left alone.
    if (isFirstUserMessage && titleIsDefault) {
      const derived = deriveConversationTitle(text);
      if (derived) void get().rename(convId, derived);
    }
  },

  cancel: async () => {
    const id = get().streamingMessageId;
    if (!id) return;
    await invoke("chat_cancel", { messageId: id });
  },

  retry: async (messageId) => {
    // Optimistically clear the error + content so the bubble looks fresh
    // before the backend has had a chance to spin a new stream.
    set((s) => {
      const next: Record<string, Record<string, ChatErrorInfo>> = {
        ...s.errors,
      };
      for (const convId of Object.keys(next)) {
        if (next[convId][messageId]) {
          const { [messageId]: _drop, ...rest } = next[convId];
          next[convId] = rest;
        }
      }
      const updatedMessages: Record<string, Message[]> = { ...s.messages };
      for (const convId of Object.keys(updatedMessages)) {
        const list = updatedMessages[convId];
        const idx = list.findIndex((m) => m.id === messageId);
        if (idx !== -1) {
          const copy = list.slice();
          copy[idx] = { ...copy[idx], content: "", thinking: null };
          updatedMessages[convId] = copy;
        }
      }
      return {
        errors: next,
        messages: updatedMessages,
        streamingMessageId: messageId,
      };
    });
    try {
      await invoke("chat_retry", { messageId });
    } catch (e) {
      console.error("chat_retry failed", e);
      set((s) =>
        s.streamingMessageId === messageId ? { streamingMessageId: null } : s,
      );
    }
  },

  dismissError: (conversationId, messageId) => {
    set((s) => {
      const conv = s.errors[conversationId];
      if (!conv || !conv[messageId]) return s;
      const { [messageId]: _drop, ...rest } = conv;
      return { errors: { ...s.errors, [conversationId]: rest } };
    });
  },

  resolveToolConfirm: async (callId, allow) => {
    // Drop the prompt optimistically so the modal closes immediately even
    // if the IPC round-trip takes a moment. Backend resolution is
    // idempotent (returns `false` on unknown ids) so a duplicate click is
    // harmless.
    set((s) => {
      if (!s.toolConfirms[callId]) return s;
      const { [callId]: _drop, ...rest } = s.toolConfirms;
      return { toolConfirms: rest };
    });
    try {
      await invoke("chat_tool_confirm", { callId, allow });
    } catch (e) {
      console.error("chat_tool_confirm failed", e);
    }
  },

  loadDisabledTools: async (conversationId) => {
    if (get().disabledTools[conversationId]) return;
    try {
      const keys =
        (await invoke<string[]>("chat_get_disabled_tools", {
          conversationId,
        })) ?? [];
      set((s) => ({
        disabledTools: { ...s.disabledTools, [conversationId]: keys },
      }));
    } catch (e) {
      console.error("chat_get_disabled_tools failed", e);
      // Treat failures as "no overrides" so the popover still renders.
      set((s) =>
        s.disabledTools[conversationId]
          ? s
          : {
              disabledTools: { ...s.disabledTools, [conversationId]: [] },
            },
      );
    }
  },

  setToolDisabled: async (conversationId, key, disabled) => {
    const current = get().disabledTools[conversationId] ?? [];
    const next = disabled
      ? current.includes(key)
        ? current
        : [...current, key]
      : current.filter((k) => k !== key);
    if (next === current) return;
    set((s) => ({
      disabledTools: { ...s.disabledTools, [conversationId]: next },
    }));
    try {
      await invoke("chat_set_disabled_tools", {
        conversationId,
        keys: next,
      });
    } catch (e) {
      console.error("chat_set_disabled_tools failed", e);
      // Roll back the optimistic update on failure so the UI stays
      // consistent with what the backend will actually apply.
      set((s) => ({
        disabledTools: { ...s.disabledTools, [conversationId]: current },
      }));
    }
  },

  loadSampling: async (conversationId) => {
    if (get().sampling[conversationId]) return;
    try {
      const cfg = (await invoke<SamplingConfig>("chat_get_sampling", {
        conversationId,
      })) ?? { ...EMPTY_SAMPLING };
      set((s) => ({
        sampling: { ...s.sampling, [conversationId]: cfg },
      }));
    } catch (e) {
      console.error("chat_get_sampling failed", e);
      // Treat failures as "no override" so the popover renders the
      // provider/profile defaults instead of erroring out.
      set((s) =>
        s.sampling[conversationId]
          ? s
          : {
              sampling: {
                ...s.sampling,
                [conversationId]: { ...EMPTY_SAMPLING },
              },
            },
      );
    }
  },

  setSampling: async (conversationId, sampling) => {
    const current = get().sampling[conversationId] ?? { ...EMPTY_SAMPLING };
    // Optimistic local update so the popover slider feels live; rolled
    // back on backend failure (same pattern as `setToolDisabled`).
    set((s) => ({
      sampling: { ...s.sampling, [conversationId]: sampling },
    }));
    try {
      await invoke("chat_set_sampling", { conversationId, sampling });
    } catch (e) {
      console.error("chat_set_sampling failed", e);
      set((s) => ({
        sampling: { ...s.sampling, [conversationId]: current },
      }));
    }
  },

  bindEvents: async () => {
    const offDelta = await on<ChatDeltaPayload>(Events.ChatDelta, (p) => {
      set((s) => {
        const list = s.messages[p.conversation_id] ?? [];
        const idx = list.findIndex((m) => m.id === p.message_id);
        let next: Message[];
        if (idx === -1) {
          // No placeholder yet (e.g. another window opened the chat after
          // streaming started) — synthesize one.
          next = [
            ...list,
            {
              id: p.message_id,
              conversation_id: p.conversation_id,
              role: "assistant",
              content: p.thinking ? "" : p.delta,
              thinking: p.thinking ? p.delta : null,
              created_at: new Date().toISOString(),
            },
          ];
        } else {
          next = list.slice();
          const cur = next[idx];
          next[idx] = p.thinking
            ? { ...cur, thinking: (cur.thinking ?? "") + p.delta }
            : { ...cur, content: cur.content + p.delta };
        }
        // First delta after a retry clears any stale error for the same
        // message id so the hint banner disappears on its own.
        const convErrs = s.errors[p.conversation_id];
        const errors =
          convErrs && convErrs[p.message_id]
            ? (() => {
                const { [p.message_id]: _drop, ...rest } = convErrs;
                return { ...s.errors, [p.conversation_id]: rest };
              })()
            : s.errors;
        return {
          messages: { ...s.messages, [p.conversation_id]: next },
          streamingMessageId: p.message_id,
          errors,
        };
      });
    });

    const offDone = await on<ChatDonePayload>(Events.ChatDone, (p) => {
      set((s) =>
        s.streamingMessageId === p.message_id
          ? { streamingMessageId: null }
          : s,
      );
    });

    // `chat://rewrite` replaces the live streaming buffer with a
    // canonical version. Used by the runner to scrub legacy fenced
    // tool-call JSON that already streamed into the bubble before it
    // was recognised as a protocol marker, and to splice per-round
    // reasoning into `content` as an inline `[thinking] … [/thinking]`
    // block. Either way the live `thinking` field is reset to null:
    // for the legacy-fence case there was no separate reasoning to
    // begin with, and for the inline-thinking case the text has just
    // been moved into `content` (leaving it in `thinking` too would
    // render the same paragraph twice). If the message isn't in the
    // local cache (rare — same window must be open) we synthesise a
    // row with the canonical content.
    const offRewrite = await on<ChatRewritePayload>(Events.ChatRewrite, (p) => {
      set((s) => {
        const list = s.messages[p.conversation_id] ?? [];
        const idx = list.findIndex((m) => m.id === p.message_id);
        let next: Message[];
        if (idx === -1) {
          next = [
            ...list,
            {
              id: p.message_id,
              conversation_id: p.conversation_id,
              role: "assistant",
              content: p.content,
              thinking: null,
              created_at: new Date().toISOString(),
            },
          ];
        } else {
          next = list.slice();
          next[idx] = { ...next[idx], content: p.content, thinking: null };
        }
        return { messages: { ...s.messages, [p.conversation_id]: next } };
      });
    });

    const offErr = await on<ChatErrorPayload>(Events.ChatError, (p) => {
      console.error("chat error", p);
      set((s) => {
        const list = s.messages[p.conversation_id] ?? [];
        const idx = list.findIndex((m) => m.id === p.message_id);
        let next = list;
        if (idx !== -1) {
          next = list.slice();
          const cur = next[idx];
          next[idx] = {
            ...cur,
            content: cur.content || `[error] ${p.error}`,
          };
        }
        const info: ChatErrorInfo = {
          message_id: p.message_id,
          error: p.error,
          kind: p.kind ?? "other",
          hint: p.hint,
          provider_kind: p.provider_kind,
          base_url: p.base_url,
          retryable: p.retryable ?? false,
        };
        const convErrs = s.errors[p.conversation_id] ?? {};
        return {
          messages: { ...s.messages, [p.conversation_id]: next },
          streamingMessageId:
            s.streamingMessageId === p.message_id ? null : s.streamingMessageId,
          errors: {
            ...s.errors,
            [p.conversation_id]: { ...convErrs, [p.message_id]: info },
          },
        };
      });
    });

    const offConfirm = await on<ToolConfirmRequest>(
      Events.ChatToolConfirm,
      (p) => {
        set((s) => ({
          toolConfirms: { ...s.toolConfirms, [p.call_id]: p },
        }));
      },
    );

    // When a chat-level cancellation clears the streaming flag, drop any
    // confirms tied to that message so we don't leave a stale modal
    // around — the backend has already torn down the pending sender.
    const offDoneClearConfirms = await on<ChatDonePayload>(
      Events.ChatDone,
      (p) => {
        set((s) => {
          const next = { ...s.toolConfirms };
          let changed = false;
          for (const [key, req] of Object.entries(next)) {
            if (req.message_id === p.message_id) {
              delete next[key];
              changed = true;
            }
          }
          return changed ? { toolConfirms: next } : s;
        });
      },
    );

    return () => {
      offDelta();
      offDone();
      offRewrite();
      offErr();
      offConfirm();
      offDoneClearConfirms();
    };
  },
}));
