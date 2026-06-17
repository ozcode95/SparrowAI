import { useUiStore, type ViewId } from "@/stores/ui";
import { useChatStore } from "@/stores/chat";
import { relativeTime } from "@/lib/format";

interface NavItem {
  id: ViewId;
  label: string;
  /** Inline 20×20 Fluent-style line icon. */
  icon: React.ReactNode;
}

const stroke = {
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.5,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
};

const NAV: NavItem[] = [
  {
    id: "chat",
    label: "New chat",
    // Speech bubble + a plus mark so the row reads as an action
    // ("start a new chat") rather than a passive section link.
    icon: (
      <svg width="18" height="18" viewBox="0 0 24 24" {...stroke}>
        <path d="M21 12a8 8 0 0 1-11.6 7.15L4 21l1.85-5.4A8 8 0 1 1 21 12Z" />
        <path d="M12 9v6M9 12h6" />
      </svg>
    ),
  },
  {
    id: "models",
    label: "Models",
    icon: (
      <svg width="18" height="18" viewBox="0 0 24 24" {...stroke}>
        <path d="M12 3 3 7.5 12 12l9-4.5L12 3Z" />
        <path d="M3 12.5 12 17l9-4.5" />
        <path d="M3 17.5 12 22l9-4.5" />
      </svg>
    ),
  },
  {
    id: "tasks",
    label: "Tasks",
    icon: (
      <svg width="18" height="18" viewBox="0 0 24 24" {...stroke}>
        <rect x="4" y="4" width="16" height="16" rx="2" />
        <path d="m8 12 3 3 5-6" />
      </svg>
    ),
  },
  {
    id: "memory",
    label: "Memory",
    icon: (
      <svg width="18" height="18" viewBox="0 0 24 24" {...stroke}>
        <path d="M9 5a4 4 0 1 0-4 4v6a4 4 0 1 0 4 4 4 4 0 1 0 6 0 4 4 0 1 0 4-4V9a4 4 0 1 0-4-4 4 4 0 1 0-6 0Z" />
      </svg>
    ),
  },
  {
    id: "tools",
    label: "Tools",
    icon: (
      <svg width="18" height="18" viewBox="0 0 24 24" {...stroke}>
        <path d="M14.7 6.3a4 4 0 0 0-5.4 5.4L3 18v3h3l6.3-6.3a4 4 0 0 0 5.4-5.4l-2.6 2.6-2.4-2.4 2.6-2.6Z" />
      </svg>
    ),
  },
  {
    id: "skills",
    label: "Skills",
    icon: (
      <svg width="18" height="18" viewBox="0 0 24 24" {...stroke}>
        <path d="M12 3 14.5 9l6 .5-4.6 4 1.5 6L12 16l-5.4 3.5 1.5-6L3.5 9.5 9.5 9 12 3Z" />
      </svg>
    ),
  },
];

const FOOTER: NavItem[] = [
  {
    id: "settings",
    label: "Settings",
    icon: (
      <svg width="18" height="18" viewBox="0 0 24 24" {...stroke}>
        <circle cx="12" cy="12" r="3" />
        <path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3h.1a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8v.1a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1Z" />
      </svg>
    ),
  },
];

/**
 * Fluent NavigationView (compact rail variant). Items show as 36px-tall
 * rows with an icon and label. The active item gets an accent pill that
 * animates between rows — straight out of the Win11 Settings app.
 *
 * The conversation history list lives in the rail on every view except
 * Settings — keeping it visible while the user is on Models, Tasks,
 * Memory, Tools, or Skills lets them peek at recent chats and jump
 * back without losing context. Clicking a history row both routes the
 * user back to the Chat view and opens that conversation.
 */
export function Sidebar() {
  const view = useUiStore((s) => s.view);

  // History is hidden on Settings only — there's no chat affordance to
  // pair it with there, and Settings already uses the rail real-estate
  // visually with its own section nav.
  const showHistory = view !== "settings";

  return (
    <nav className="fluent-mica relative flex w-64 shrink-0 flex-col border-r border-tui-border">
      <ul className="flex flex-col px-2 pt-3">
        {NAV.map((item) => (
          <NavRow key={item.id} item={item} />
        ))}
      </ul>

      {showHistory && (
        <>
          <div className="mx-3 mt-2 mb-1 h-px bg-tui-border" />
          <ConversationsSection />
        </>
      )}

      <div className="flex-1" />

      <div className="px-2 pb-3">
        <div className="mx-1 my-1 h-px bg-tui-border" />
        {FOOTER.map((item) => (
          <NavRow key={item.id} item={item} />
        ))}
      </div>
    </nav>
  );
}

function NavRow({ item }: { item: NavItem }) {
  const view = useUiStore((s) => s.view);
  const setView = useUiStore((s) => s.setView);
  const newChat = useChatStore((s) => s.newChat);
  const streaming = useChatStore((s) => s.streamingMessageId);
  const activeId = useChatStore((s) => s.activeId);
  const active = view === item.id;

  // The Chat rail item doubles as the "new chat" primary action —
  // clicking it both jumps to the Chat view *and* resets the active
  // conversation so the user lands on the empty composer. The header's
  // dedicated "New chat" button used to live in the panel; now this row
  // owns that responsibility so the rail is the single source of truth
  // for "start something new". We keep the click a no-op mid-stream so
  // a stray click can't drop the user out of a turn they can't recover.
  const isNewChat = item.id === "chat";
  const newChatDisabled = isNewChat && !!streaming;

  function handleClick() {
    if (isNewChat) {
      if (newChatDisabled) return;
      // Only reset if there's actually something to reset — clicking on
      // an already-empty new chat canvas should be a true no-op so we
      // don't churn the composer state out from under the user.
      if (activeId !== null) newChat();
      setView("chat");
      return;
    }
    setView(item.id);
  }

  return (
    <li className="list-none">
      <button
        onClick={handleClick}
        aria-current={active ? "page" : undefined}
        disabled={newChatDisabled}
        title={
          newChatDisabled
            ? "Wait for the assistant to finish before starting a new chat"
            : undefined
        }
        className={
          "group relative flex w-full items-center gap-3 rounded-[6px] " +
          "px-2.5 py-2 text-left text-[13px] " +
          "transition-[background-color,color] duration-150 ease-out " +
          (newChatDisabled
            ? "cursor-not-allowed text-tui-fg-muted opacity-60"
            : active
              ? "bg-[var(--fluent-bg-subtle-selected)] text-tui-fg"
              : "text-tui-fg-dim hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-fg active:bg-[var(--fluent-bg-subtle-pressed)]")
        }
      >
        <span
          aria-hidden="true"
          className={
            "absolute left-0 top-1/2 -translate-y-1/2 w-[3px] rounded-full bg-tui-accent " +
            "transition-all duration-200 ease-out " +
            (active ? "h-4 opacity-100" : "h-0 opacity-0")
          }
        />
        <span
          className={
            "flex h-5 w-5 shrink-0 items-center justify-center transition-colors " +
            (active
              ? "text-tui-accent"
              : "text-tui-fg-dim group-hover:text-tui-fg")
          }
        >
          {item.icon}
        </span>
        <span className="flex-1 truncate font-medium">{item.label}</span>
      </button>
    </li>
  );
}

/**
 * Conversation history nested inside the sidebar. Rendered only on the
 * Chat view — gives the chat page its full width and surfaces history
 * everywhere the rail is visible. The "New chat" button used to live
 * at the top of this list; it now lives in the chat panel header so
 * the rail can dedicate its full vertical real estate to history.
 */
function ConversationsSection() {
  const conversations = useChatStore((s) => s.conversations);
  const activeId = useChatStore((s) => s.activeId);
  const open = useChatStore((s) => s.open);
  const remove = useChatStore((s) => s.remove);
  const view = useUiStore((s) => s.view);
  const setView = useUiStore((s) => s.setView);

  // Highlighting only makes sense while the user is *on* the Chat view
  // — a row that's "active" on the Tasks page would look like a
  // navigation target rather than a state indicator.
  const onChatView = view === "chat";

  async function openAndFocus(id: string) {
    if (!onChatView) setView("chat");
    await open(id);
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center justify-between gap-2 px-3 pt-2 pb-1">
        <span className="text-[10px] font-semibold uppercase tracking-wide text-tui-fg-muted">
          History
        </span>
        <span className="text-[10px] text-tui-fg-muted">
          {conversations.length}
        </span>
      </div>

      <ul className="mt-1 flex-1 overflow-auto px-2 pb-1">
        {conversations.length === 0 && (
          <li className="px-2 py-3 text-center text-[11px] text-tui-fg-muted">
            No chats yet.
          </li>
        )}
        {conversations.map((c) => {
          const active = onChatView && c.id === activeId;
          return (
            <li key={c.id} className="list-none">
              <div
                className={
                  "group relative my-px flex items-center gap-1 rounded-[6px] " +
                  "px-2 py-1.5 text-[12px] " +
                  "transition-[background-color,color] duration-150 ease-out " +
                  (active
                    ? "bg-[var(--fluent-bg-subtle-selected)] text-tui-fg"
                    : "text-tui-fg-dim hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-fg")
                }
              >
                <span
                  aria-hidden="true"
                  className={
                    "absolute left-0 top-1/2 -translate-y-1/2 w-[3px] rounded-full bg-tui-accent " +
                    "transition-all duration-200 ease-out " +
                    (active ? "h-3.5 opacity-100" : "h-0 opacity-0")
                  }
                />
                <button
                  onClick={() => void openAndFocus(c.id)}
                  className="min-w-0 flex-1 pl-1.5 text-left"
                  title={c.title}
                >
                  <div className="truncate font-medium">
                    {c.title || "Untitled"}
                  </div>
                  <div className="truncate text-[10px] text-tui-fg-muted">
                    {relativeTime(c.updated_at)}
                  </div>
                </button>
                <button
                  onClick={() => void remove(c.id)}
                  className={
                    "shrink-0 rounded p-1 text-tui-fg-muted opacity-0 " +
                    "transition-opacity duration-150 " +
                    "hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-err " +
                    "group-hover:opacity-100 focus-visible:opacity-100"
                  }
                  title="Delete"
                  aria-label="Delete conversation"
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
              </div>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
