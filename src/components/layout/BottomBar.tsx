import { useServerStore } from "@/stores/server";
import { useChatStore } from "@/stores/chat";
import { StatusSpacer } from "@/components/tui/StatusBar";
import { ServerLogsPopover } from "@/components/server/ServerLogsPopover";

/**
 * Persistent status strip pinned to the bottom of the shell. Surfaces
 * the always-relevant runtime state (OVMS lifecycle + current
 * conversation) so it stays visible regardless of which page is in
 * front.
 *
 * Per-model load state used to live here too, but it's now surfaced
 * directly on each card on the Models page — the single "current
 * model" pill couldn't represent the fact that OVMS can now serve a
 * full lineup (one of each task kind) at the same time.
 */
export function BottomBar() {
  const info = useServerStore((s) => s.info);
  const streaming = useChatStore((s) => s.streamingMessageId);
  const activeChat = useChatStore((s) =>
    s.conversations.find((c) => c.id === s.activeId),
  );

  return (
    <footer
      className={
        "flex shrink-0 items-center gap-3 border-t border-tui-border " +
        "bg-tui-bg-elev px-4 py-1.5 text-[11px]"
      }
    >
      <div className="inline-flex items-center gap-2 text-tui-fg-dim">
        <span className="relative flex h-2 w-2">
          <span
            className={
              "absolute inline-flex h-full w-full rounded-full bg-tui-accent " +
              (streaming ? "animate-ping opacity-75" : "opacity-0")
            }
          />
          <span className="relative inline-flex h-2 w-2 rounded-full bg-tui-accent" />
        </span>
        <span className="font-semibold text-tui-fg">
          {streaming ? "Generating…" : "Ready"}
        </span>
        {activeChat && !streaming && (
          <span className="text-tui-fg-muted">
            · {activeChat.title || "Untitled"}
          </span>
        )}
      </div>
      <StatusSpacer />
      <ServerLogsPopover
        label="ovms"
        status={info?.status ?? "—"}
        tone={
          info?.status === "running"
            ? "ok"
            : info?.status === "error"
              ? "err"
              : "default"
        }
      />
    </footer>
  );
}
