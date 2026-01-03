import { useEffect, useState, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { Sidebar, AppLayout } from "./components/layout";
import { ChatPage } from "./components/chat";
import { ModelsPage } from "./components/models";
import { DocumentsPage } from "./components/documents";
import { SkillsPage } from "./components/skills";
import { McpPage } from "./components/mcp";
import { TasksPage } from "./components/tasks";
import { GalleryPage } from "./components/gallery";
import { SettingsDialog } from "./components/settings";
import {
  NotificationSnackbar,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  LoadingSpinner,
} from "./components/ui";

import { useDownloadedModels } from "./hooks";
import { useUI, useModels, useChat, useTheme } from "./store";
import { logInfo, logError, logDebug } from "./lib/logger";

function App() {
  const { currentPage, setCurrentPage, showNotification } = useUI();
  const { setIsOvmsRunning, getLoadedModels } = useModels();
  const { themeMode } = useTheme();
  const {
    setActiveChatSessionId,
    clearCurrentChatMessages,
    clearTemporarySession,
  } = useChat();

  const [initStatus, setInitStatus] = useState<any>(null);
  const [showInitDialog, setShowInitDialog] = useState(false);
  const ovmsNotificationShownRef = useRef(false);
  const modelLoadedRef = useRef(false);
  const initCompletedRef = useRef(false);

  useDownloadedModels();

  // Auto-connect MCP servers on app start
  useEffect(() => {
    const autoConnectMcpServers = async () => {
      try {
        logDebug("Auto-connecting MCP servers");
        const connected = await invoke<string[]>("auto_connect_mcp_servers");
        if (connected.length > 0) {
          logInfo("MCP servers auto-connected", { servers: connected });
        }
      } catch (error) {
        logError("Failed to auto-connect MCP servers", error as Error);
      }
    };
    autoConnectMcpServers();
  }, []);

  // Apply theme mode to DOM on mount and when it changes
  useEffect(() => {
    logDebug("Applying theme mode", { mode: themeMode });
    if (themeMode === "dark") {
      document.documentElement.classList.add("dark");
    } else {
      document.documentElement.classList.remove("dark");
    }
  }, [themeMode]);

  // Clear chat state on app startup (session will be created on first message)
  useEffect(() => {
    logDebug("Clearing chat state on startup");
    clearTemporarySession();
    clearCurrentChatMessages();
    setActiveChatSessionId(null);
  }, []);

  // Listen for task action events
  useEffect(() => {
    const unlistenNotification = listen<any>(
      "task-action-show-notification",
      (event) => {
        logInfo("Task action: Show notification", event.payload);
        const { title, message } = event.payload;
        showNotification(`${title}: ${message}`, "info", 5000);
      }
    );

    return () => {
      unlistenNotification.then((fn) => fn());
    };
  }, [showNotification]);

  // Monitor OVMS initialization status
  useEffect(() => {
    // Skip if already initialized
    if (initCompletedRef.current) {
      return;
    }

    let interval: NodeJS.Timeout | null = null;

    const checkInitStatus = async () => {
      try {
        const status: any = await invoke("get_initialization_status");
        setInitStatus(status);

        if (!status.is_complete && !status.has_error) {
          setShowInitDialog(true);
        }

        if (status.is_complete) {
          logInfo("OVMS initialization completed", { step: status.step });
          setIsOvmsRunning(true);
          initCompletedRef.current = true; // Mark as completed

          // Clear the interval once initialization is complete
          if (interval) {
            clearInterval(interval);
          }

          // Load model once when OVMS is ready
          if (!modelLoadedRef.current) {
            await getLoadedModels();
            modelLoadedRef.current = true;
          }

          // Close dialog and show notification after model is loaded
          setShowInitDialog(false);

          // Only show notification once using ref
          if (!ovmsNotificationShownRef.current) {
            showNotification("OVMS initialized successfully", "success", 3000);
            ovmsNotificationShownRef.current = true;
          }

          const event = new Event("ovms-initialization-complete");
          window.dispatchEvent(event);
        }

        if (status.has_error) {
          logError(
            "OVMS initialization failed",
            new Error(status.error_message || "Unknown error")
          );

          // Clear interval on error too
          if (interval) {
            clearInterval(interval);
          }

          setShowInitDialog(false);
          showNotification(
            `OVMS initialization failed: ${
              status.error_message || "Unknown error"
            }`,
            "error",
            8000
          );
        }
      } catch (error) {
        logError("Failed to check OVMS init status", error as Error);
      }
    };

    checkInitStatus();
    interval = setInterval(checkInitStatus, 2000);

    return () => {
      if (interval) {
        clearInterval(interval);
      }
    };
  }, []);

  // Listen for OVMS initialization updates
  useEffect(() => {
    const unlisten = listen<any>("ovms-init-progress", async (event) => {
      setInitStatus(event.payload);

      if (event.payload.is_complete && !event.payload.has_error) {
        // Skip if already completed
        if (initCompletedRef.current) {
          return;
        }

        logInfo("OVMS initialization event received", {
          progress: event.payload.progress,
        });
        setIsOvmsRunning(true);
        initCompletedRef.current = true; // Mark as completed

        // Load model once when OVMS is ready
        if (!modelLoadedRef.current) {
          await getLoadedModels();
          modelLoadedRef.current = true;
        }

        // Close dialog and show notification after model is loaded
        setShowInitDialog(false);

        // Only show notification once using ref
        if (!ovmsNotificationShownRef.current) {
          showNotification("OVMS initialized successfully", "success", 3000);
          ovmsNotificationShownRef.current = true;
        }

        const ovmsEvent = new Event("ovms-initialization-complete");
        window.dispatchEvent(ovmsEvent);
      }
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const renderPage = () => {
    switch (currentPage) {
      case "chat":
        return <ChatPage />;
      case "models":
        return <ModelsPage />;
      case "documents":
        return <DocumentsPage />;
      case "skills":
        return <SkillsPage />;
      case "mcp":
        return <McpPage />;
      case "tasks":
        return <TasksPage />;
      case "gallery":
        return <GalleryPage />;
      default:
        return <ChatPage />;
    }
  };

  return (
    <>
      <AppLayout
        sidebar={
          <Sidebar currentPage={currentPage} onPageChange={setCurrentPage} />
        }
      >
        {renderPage()}
      </AppLayout>

      <SettingsDialog />
      <NotificationSnackbar />

      {/* Initialization Dialog */}
      <Dialog open={showInitDialog} onOpenChange={setShowInitDialog}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Initializing OVMS</DialogTitle>
            <DialogDescription>
              {initStatus?.current_step || "Setting up the model server..."}
            </DialogDescription>
          </DialogHeader>

          <div className="flex flex-col items-center gap-4 py-6">
            <LoadingSpinner size="lg" />
            <p className="text-sm text-gray-600 dark:text-gray-400">
              This may take a few moments...
            </p>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}

export default App;
