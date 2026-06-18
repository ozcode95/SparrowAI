import { useServerStore } from "@/stores/server";
import type { OvmsStatus } from "@/stores/server";
import { useChatStore } from "@/stores/chat";
import { StatusSpacer } from "@/components/tui/StatusBar";
import { Spinner } from "@/components/tui/Spinner";
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
  const loadingModelIds = useServerStore((s) => s.loadingModelIds);
  const streaming = useChatStore((s) => s.streamingMessageId);
  const activeChat = useChatStore((s) =>
    s.conversations.find((c) => c.id === s.activeId),
  );

  // Prefer the "loading model" indicator over the generic Ready state
  // so the bottom bar reflects exactly what the chat composer and the
  // Models page cards are reporting — without this, the user sees
  // "Ready" while their composer is greyed out, which is contradictory.
  const loadingId = loadingModelIds.values().next().value as string | undefined;
  const isLoadingModel = !streaming && !!loadingId;

  // Map the raw OVMS lifecycle status to the headline the bar should
  // show. We only land on "Ready" when OVMS is actually `running` —
  // the bar used to fall back to "Ready" for `starting` / `stopping` /
  // `installing` too, which contradicted the rest of the UI (composer
  // disabled, Models cards busy) and let the user fire off a chat
  // before MediaPipe graphs finished loading. Streaming and per-model
  // load take precedence so an in-flight `loadModel` invocation still
  // wins over the lifecycle label.
  const ovmsStatus: OvmsStatus | null = info?.status ?? null;
  const ovmsTransitional =
    ovmsStatus === "starting" ||
    ovmsStatus === "stopping" ||
    ovmsStatus === "installing";
  const ovmsLabel = (() => {
    switch (ovmsStatus) {
      case "starting":
        return "Starting OVMS…";
      case "stopping":
        return "Stopping OVMS…";
      case "installing":
        return "Installing OVMS…";
      case "stopped":
        return "OVMS stopped";
      case "installed":
        return "OVMS idle";
      case "not_installed":
        return "OVMS not installed";
      case "error":
        return "OVMS error";
      case "running":
      default:
        return "Ready";
    }
  })();
  const isReady = ovmsStatus === "running" && !streaming && !isLoadingModel;
  const showSpinner = isLoadingModel || ovmsTransitional;

  return (
    <footer
      className={
        "flex shrink-0 items-center gap-3 border-t border-tui-border " +
        "bg-tui-bg-elev px-4 py-1.5 text-[11px]"
      }
    >
      <div className="inline-flex items-center gap-2 text-tui-fg-dim">
        {showSpinner ? (
          <Spinner size="sm" />
        ) : (
          <span className="relative flex h-2 w-2">
            <span
              className={
                "absolute inline-flex h-full w-full rounded-full bg-tui-accent " +
                (streaming ? "animate-ping opacity-75" : "opacity-0")
              }
            />
            <span
              className={
                "relative inline-flex h-2 w-2 rounded-full " +
                (isReady
                  ? "bg-tui-accent"
                  : ovmsStatus === "error"
                    ? "bg-tui-err"
                    : "bg-tui-fg-muted")
              }
            />
          </span>
        )}
        <span className="font-semibold text-tui-fg">
          {streaming
            ? "Generating…"
            : isLoadingModel
              ? "Loading model"
              : ovmsLabel}
        </span>
        {isLoadingModel && loadingId && (
          <span className="text-tui-fg-muted">· {loadingId}</span>
        )}
        {activeChat && !streaming && !isLoadingModel && !ovmsTransitional && (
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
