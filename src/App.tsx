import { useEffect, useState, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { Sidebar, AppLayout } from "./components/layout";
import { ChatPage } from "./components/chat";
import { ModelsPage } from "./components/models";
import { DocumentsPage } from "./components/documents";
import { McpPage } from "./components/mcp";
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

function App() {
  const { currentPage, setCurrentPage, showNotification } = useUI();
  const { setIsOvmsRunning } = useModels();
  const { themeMode } = useTheme();
  const {
    setActiveChatSessionId,
    clearCurrentChatMessages,
    clearTemporarySession,
    setTemporarySession,
  } = useChat();

  const [initStatus, setInitStatus] = useState<any>(null);
  const [showInitDialog, setShowInitDialog] = useState(false);
  const ovmsNotificationShownRef = useRef(false);

  useDownloadedModels();

  // Apply theme mode to DOM on mount and when it changes
  useEffect(() => {
    if (themeMode === "dark") {
      document.documentElement.classList.add("dark");
    } else {
      document.documentElement.classList.remove("dark");
    }
  }, [themeMode]);

  // Create a new chat session on app startup
  useEffect(() => {
    const createNewChatOnStartup = async () => {
      try {
        clearTemporarySession();
        clearCurrentChatMessages();

        await new Promise((resolve) => setTimeout(resolve, 50));

        const newSession: any = await invoke("create_temporary_chat_session", {
          title: "New Chat",
        });

        setTemporarySession(newSession);
        setActiveChatSessionId(newSession.id);
        clearCurrentChatMessages();
      } catch (error) {
        console.error("Failed to create new chat session on startup:", error);
      }
    };

    createNewChatOnStartup();
  }, []);

  // Monitor OVMS initialization status
  useEffect(() => {
    const checkInitStatus = async () => {
      try {
        const status: any = await invoke("get_initialization_status");
        setInitStatus(status);

        if (!status.is_complete && !status.has_error) {
          setShowInitDialog(true);
        }

        if (status.is_complete) {
          setIsOvmsRunning(true);
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
        console.error("Failed to check init status:", error);
      }
    };

    checkInitStatus();
    const interval = setInterval(checkInitStatus, 2000);

    return () => clearInterval(interval);
  }, []);

  // Listen for OVMS initialization updates
  useEffect(() => {
    const unlisten = listen<any>("ovms-init-progress", (event) => {
      setInitStatus(event.payload);

      if (event.payload.is_complete && !event.payload.has_error) {
        setIsOvmsRunning(true);
        setShowInitDialog(false);
        
        // Only show notification once using ref
        if (!ovmsNotificationShownRef.current) {
          showNotification("OVMS initialized successfully", "success", 3000);
          ovmsNotificationShownRef.current = true;
        }
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
      case "mcp":
        return <McpPage />;
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
