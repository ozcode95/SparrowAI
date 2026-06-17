import { useEffect } from "react";
import { useUiStore } from "@/stores/ui";
import { useChatStore } from "@/stores/chat";
import { useServerStore } from "@/stores/server";
import { useModelsStore } from "@/stores/models";
import { TitleBar } from "./TitleBar";
import { Sidebar } from "./Sidebar";
import { BottomBar } from "./BottomBar";
import { ChatView } from "@/pages/Chat";
import { ModelsView } from "@/pages/Models";
import { TasksView } from "@/pages/Tasks";
import { MemoryView } from "@/pages/Memory";
import { ToolsView } from "@/pages/Tools";
import { SkillsView } from "@/pages/Skills";
import { SettingsView } from "@/pages/Settings";
import { CommandPalette } from "@/components/tui/CommandPalette";

/** Human-readable label shown in the page header for each view.
 *
 * Only Settings still renders a page header; every other view owns its
 * own chrome (Chat / Models surface their controls in a panel header,
 * the rest now use a compact in-page title row to match). Keeping the
 * map around so the Settings header — and any future view that opts
 * in — stays driven by one source of truth.
 */
const VIEW_META: Record<
  ReturnType<typeof useUiStore.getState>["view"],
  { title: string; subtitle: string }
> = {
  chat: { title: "Chat", subtitle: "Conversations with your local agent" },
  models: { title: "Models", subtitle: "Browse, download, and configure" },
  tasks: { title: "Tasks", subtitle: "Scheduled and on-demand agents" },
  memory: { title: "Memory", subtitle: "Long-term knowledge store" },
  tools: { title: "Tools", subtitle: "MCP servers and built-in tools" },
  skills: { title: "Skills", subtitle: "Reusable prompt packages" },
  settings: {
    title: "Settings",
    subtitle: "Providers, OVMS, and preferences",
  },
};

export function Shell() {
  const view = useUiStore((s) => s.view);
  const cmdOpen = useUiStore((s) => s.commandPaletteOpen);

  const bindChat = useChatStore((s) => s.bindEvents);
  const listChat = useChatStore((s) => s.list);
  const bindServer = useServerStore((s) => s.bindEvents);
  const refreshServer = useServerStore((s) => s.refresh);
  const bindModels = useModelsStore((s) => s.bindEvents);
  const refreshLocalModels = useModelsStore((s) => s.refreshLocal);

  useEffect(() => {
    // React StrictMode double-invokes effects in dev. The event bindings
    // here are async, so the original cleanup couldn't see the listener
    // returned by the first mount before the second mount registered a
    // *second* listener — leaving one orphaned forever. Result: every
    // `chat://delta` ran twice and the assistant's output came out as
    // "HelloHello!!". Track cancellation in the closure so any listener
    // that resolves after cleanup is torn down immediately.
    let cancelled = false;
    let offChat: (() => void) | undefined;
    let offServer: (() => void) | undefined;
    let offModels: (() => void) | undefined;
    void (async () => {
      const a = await bindChat();
      const b = await bindServer();
      const c = await bindModels();
      if (cancelled) {
        a();
        b();
        c();
        return;
      }
      offChat = a;
      offServer = b;
      offModels = c;
    })();
    // Hydrate the conversation list so the sidebar populates, but
    // intentionally do NOT auto-open the most recent chat — the app
    // should always boot into a fresh "new chat" state. The header
    // controls (model picker, skills, tools) all render fine without
    // an active conversation, and any first interaction (typing,
    // attaching, picking a model) lazily creates one.
    void listChat();
    void refreshServer();
    // Prefetch the local models list so the chat model dropdown is
    // populated on first render instead of only after the Models page
    // has been visited.
    void refreshLocalModels();
    return () => {
      cancelled = true;
      offChat?.();
      offServer?.();
      offModels?.();
    };
  }, [
    bindChat,
    bindServer,
    bindModels,
    listChat,
    refreshServer,
    refreshLocalModels,
  ]);

  const meta = VIEW_META[view];

  return (
    <div className="flex h-full w-full flex-col bg-tui-bg text-tui-fg">
      <TitleBar />
      <div className="flex min-h-0 flex-1">
        <Sidebar />
        <main className="flex min-h-0 min-w-0 flex-1 flex-col">
          {/* page header — Settings is the only view that still uses
              the global title row; every other surface either owns its
              own panel chrome (Chat, Models) or renders its title
              inline so the layout feels uniform across the app. */}
          {view === "settings" && (
            <div className="flex shrink-0 items-baseline gap-3 px-5 pt-4 pb-2">
              <h1
                className="text-[22px] font-semibold leading-tight text-tui-fg"
                style={{ fontFamily: "var(--font-display)" }}
              >
                {meta.title}
              </h1>
              <span className="text-[12px] text-tui-fg-muted">
                {meta.subtitle}
              </span>
            </div>
          )}

          {/* page surface */}
          <div
            key={view}
            className={
              "fluent-view flex min-h-0 min-w-0 flex-1 flex-col gap-3 px-5 pb-4 pt-4"
            }
          >
            {view === "chat" && <ChatView />}
            {view === "models" && <ModelsView />}
            {view === "tasks" && <TasksView />}
            {view === "memory" && <MemoryView />}
            {view === "tools" && <ToolsView />}
            {view === "skills" && <SkillsView />}
            {view === "settings" && <SettingsView />}
          </div>
        </main>
      </div>
      <BottomBar />
      {cmdOpen && <CommandPalette />}
    </div>
  );
}
