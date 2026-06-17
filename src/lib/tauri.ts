import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen, type Event as TauriEvent } from "@tauri-apps/api/event";

/**
 * Thin wrapper around Tauri's `invoke`.
 * In a non-Tauri context (e.g. plain `pnpm dev` in browser) it logs and resolves
 * with a sensible default so the UI doesn't explode while we iterate.
 */
export async function invoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) {
    console.warn(`[ipc] non-tauri context, stub invoke: ${cmd}`, args);
    return undefined as unknown as T;
  }
  return tauriInvoke<T>(cmd, args);
}

export async function on<T>(
  event: string,
  handler: (payload: T) => void,
): Promise<() => void> {
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) {
    console.warn(`[ipc] non-tauri context, stub listen: ${event}`);
    return () => {};
  }
  const unlisten = await listen<T>(event, (e: TauriEvent<T>) =>
    handler(e.payload),
  );
  return unlisten;
}

/** Shared event names — keep in sync with `src-tauri/src/events.rs`. */
export const Events = {
  ChatDelta: "chat://delta",
  ChatDone: "chat://done",
  ChatError: "chat://error",
  ChatRewrite: "chat://rewrite",
  ChatToolConfirm: "chat://tool-confirm",
  ModelDownloadProgress: "models://download-progress",
  OvmsLog: "ovms://log",
  OvmsStatus: "ovms://status",
  OvmsInstallProgress: "ovms://install-progress",
  TaskTick: "tasks://tick",
} as const;
