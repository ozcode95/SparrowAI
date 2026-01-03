import React, { useEffect, useState } from "react";
import {
  MessageSquare,
  Search,
  FileText,
  Puzzle,
  Settings,
  Plus,
  ChevronLeft,
  ChevronRight,
  Trash2,
  Info,
  Clock,
  Image,
  Sparkles,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { useUI, useModels, useChat, type PageType } from "@/store";
import { Button } from "@/components/ui";
import { cn } from "@/lib/utils";
import { OvmsStatusDialog } from "../settings";

interface MenuItem {
  id: PageType;
  label: string;
  icon: React.ReactNode;
}

interface SidebarProps {
  currentPage: PageType;
  onPageChange: (page: PageType) => void;
}

export const Sidebar: React.FC<SidebarProps> = ({
  currentPage,
  onPageChange,
}) => {
  const {
    sidebarCollapsed: isCollapsed,
    toggleSidebar,
    setSettingsDialogOpen,
    showNotification,
  } = useUI();

  const { loadedModels, getLoadedModels } = useModels();

  const {
    chatSessions,
    activeChatSessionId,
    setChatSessions,
    setActiveChatSessionId,
    updateChatSession,
    removeChatSession,
    temporarySession,
    clearTemporarySession,
    clearCurrentChatMessages,
  } = useChat();

  const [, setLoadingChatSessions] = useState(false);
  const [ovmsStatusDialogOpen, setOvmsStatusDialogOpen] = useState(false);

  const menuItems: MenuItem[] = [
    {
      id: "models",
      label: "Models",
      icon: <Search className="h-5 w-5" />,
    },
    {
      id: "gallery",
      label: "Gallery",
      icon: <Image className="h-5 w-5" />,
    },
    {
      id: "documents",
      label: "Documents",
      icon: <FileText className="h-5 w-5" />,
    },
    {
      id: "skills",
      label: "Skills",
      icon: <Sparkles className="h-5 w-5" />,
    },
    {
      id: "mcp",
      label: "MCP",
      icon: <Puzzle className="h-5 w-5" />,
    },
    {
      id: "tasks",
      label: "Tasks",
      icon: <Clock className="h-5 w-5" />,
    },
  ];

  const createNewChat = async () => {
    try {
      // Clear current session - new one will be created on first message
      clearTemporarySession();
      setActiveChatSessionId(null);
      clearCurrentChatMessages();
      onPageChange("chat");
      // No notification needed when switching to new chat
    } catch (error) {
      console.error("Sidebar: Failed to create new chat:", error);
      showNotification("Failed to create new chat", "error", 3000);
    }
  };

  const loadChatSessions = async () => {
    try {
      setLoadingChatSessions(true);
      const result: any = await invoke("get_chat_sessions");
      setChatSessions(result.sessions || {});

      // Don't auto-load the last session - always start with a clean slate
      // User can manually select a previous session from the sidebar if needed
    } catch (error) {
      console.error("Sidebar: Failed to load chat sessions:", error);
      showNotification("Failed to load chat sessions", "error");
    } finally {
      setLoadingChatSessions(false);
    }
  };

  useEffect(() => {
    loadChatSessions();

    // Listen for new chat session creation
    const handleSessionCreated = () => {
      loadChatSessions();
    };

    window.addEventListener("chat-session-created", handleSessionCreated);

    return () => {
      window.removeEventListener("chat-session-created", handleSessionCreated);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const selectChatSession = async (sessionId: string) => {
    try {
      await invoke("set_active_chat_session", { sessionId });
      setActiveChatSessionId(sessionId);
      if (temporarySession && temporarySession.id !== sessionId) {
        clearTemporarySession();
      }

      const session = chatSessions[sessionId];
      if (session && !session.model_id) {
        const models = await getLoadedModels();
        const firstModel = models.length > 0 ? models[0] : null;
        if (firstModel) {
          const modelId = firstModel.startsWith("OpenVINO/")
            ? firstModel
            : `OpenVINO/${firstModel}`;
          try {
            await invoke("update_chat_session", {
              sessionId: sessionId,
              title: null,
              modelId: modelId,
            });
            updateChatSession(sessionId, { model_id: modelId });
          } catch (error) {
            console.error("Failed to update session with loaded model:", error);
          }
        }
      }

      onPageChange("chat");
    } catch (error) {
      console.error("Sidebar: Failed to select chat session:", error);
      showNotification("Failed to load chat session", "error");
    }
  };

  const deleteChat = async (sessionId: string, e: React.MouseEvent) => {
    e.stopPropagation();

    try {
      await invoke("delete_chat_session", { sessionId });
      removeChatSession(sessionId);
      showNotification("Chat deleted", "success", 3000);

      if (activeChatSessionId === sessionId) {
        await createNewChat();
      }
    } catch (error) {
      console.error("Sidebar: Failed to delete chat:", error);
      showNotification("Failed to delete chat", "error");
    }
  };

  const chatSessionsArray = Object.values(chatSessions).sort(
    (a, b) => b.updated_at - a.updated_at
  );

  return (
    <>
      <div
        className={cn(
          "flex h-screen flex-col border-r border-gray-200 bg-white transition-all duration-300 dark:border-gray-800 dark:bg-gray-950",
          isCollapsed ? "w-16" : "w-60"
        )}
      >
        {/* Header */}
        <div className="flex h-16 items-center justify-between border-b border-gray-200 px-4 dark:border-gray-800">
          {!isCollapsed && (
            <img src="/white_logo_xbg.png" alt="SparrowAI" className="h-8" />
          )}
          <Button
            variant="ghost"
            size="icon"
            onClick={toggleSidebar}
            className="shrink-0"
          >
            {isCollapsed ? (
              <ChevronRight className="h-5 w-5" />
            ) : (
              <ChevronLeft className="h-5 w-5" />
            )}
          </Button>
        </div>

        {/* Main Content */}
        <div className="flex-1 overflow-y-auto">
          {/* New Chat Button */}
          <div className="p-2">
            <Button
              variant="outline"
              className={cn(
                "w-full justify-start",
                isCollapsed && "justify-center px-0"
              )}
              onClick={createNewChat}
            >
              <Plus className="h-5 w-5" />
              {!isCollapsed && <span className="ml-2">New Chat</span>}
            </Button>
          </div>

          {/* Menu Items */}
          <div className="space-y-1 px-2 mb-3">
            {menuItems.map((item) => (
              <button
                key={item.id}
                onClick={() => onPageChange(item.id)}
                className={cn(
                  "flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm font-medium transition-colors",
                  currentPage === item.id
                    ? "bg-accent-100 text-accent-900 dark:bg-accent-900/20 dark:text-accent-100"
                    : "text-gray-700 hover:bg-gray-100 dark:text-gray-300 dark:hover:bg-gray-800",
                  isCollapsed && "justify-center"
                )}
              >
                {item.icon}
                {!isCollapsed && <span>{item.label}</span>}
              </button>
            ))}
          </div>

          {/* Divider */}
          {!isCollapsed && chatSessionsArray.length > 0 && (
            <div className="mx-2 mb-2 border-t border-gray-200 dark:border-gray-800" />
          )}

          {/* Chat Sessions */}
          {!isCollapsed && (
            <div className="space-y-0.5 px-2">
              {chatSessionsArray.map((session) => {
                const isActive =
                  session.id === activeChatSessionId ||
                  session.id === temporarySession?.id;

                return (
                  <div
                    key={session.id}
                    className={cn(
                      "group flex items-center gap-2 rounded-lg px-3 py-1.5 text-xs transition-colors cursor-pointer",
                      isActive
                        ? "bg-accent-100 text-accent-900 dark:bg-accent-900/20 dark:text-accent-100"
                        : "hover:bg-gray-100 dark:hover:bg-gray-800"
                    )}
                    onClick={() => selectChatSession(session.id)}
                  >
                    <MessageSquare className="h-3.5 w-3.5 shrink-0" />
                    <span className="flex-1 truncate pl-1">
                      {session.title}
                    </span>
                    <button
                      onClick={(e) => deleteChat(session.id, e)}
                      className="opacity-0 group-hover:opacity-100 rounded p-0.5 hover:bg-gray-200 dark:hover:bg-gray-700"
                    >
                      <Trash2 className="h-3 w-3" />
                    </button>
                  </div>
                );
              })}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="border-t border-gray-200 p-2 dark:border-gray-800">
          <Button
            variant="ghost"
            className={cn(
              "w-full justify-start",
              isCollapsed && "justify-center px-0"
            )}
            onClick={() => {
              getLoadedModels(); // Refresh model status on click
              setOvmsStatusDialogOpen(true);
            }}
          >
            <Info className="h-5 w-5" />
            {!isCollapsed && <span className="ml-2">Status</span>}
          </Button>
          <Button
            variant="ghost"
            className={cn(
              "w-full justify-start",
              isCollapsed && "justify-center px-0"
            )}
            onClick={() => setSettingsDialogOpen(true)}
          >
            <Settings className="h-5 w-5" />
            {!isCollapsed && <span className="ml-2">Settings</span>}
          </Button>
        </div>
      </div>

      <OvmsStatusDialog
        open={ovmsStatusDialogOpen}
        onClose={() => setOvmsStatusDialogOpen(false)}
      />
    </>
  );
};
