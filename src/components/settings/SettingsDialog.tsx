import React, { useState, useEffect } from "react";
import { Settings as SettingsIcon, MessageSquare, Sliders, Settings } from "lucide-react";
import { useUI, useSettings } from "@/store";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/Dialog";
import { Button, Input, Textarea } from "@/components/ui";
import { cn } from "@/lib/utils";
import { invoke } from "@tauri-apps/api/core";

export const SettingsDialog: React.FC = () => {
  const [activeTab, setActiveTab] = useState<"general" | "chat" | "advanced">("general");
  const [isLoadingAutostart, setIsLoadingAutostart] = useState(false);

  const { settingsDialogOpen, setSettingsDialogOpen } = useUI();
  const { settings, updateSettings } = useSettings();

  // Load current autostart status when dialog opens
  useEffect(() => {
    if (settingsDialogOpen) {
      loadAutostartStatus();
    }
  }, [settingsDialogOpen]);

  const loadAutostartStatus = async () => {
    try {
      const enabled = await invoke<boolean>('is_autostart_enabled');
      updateSettings({ enableAutostart: enabled });
    } catch (error) {
      console.error('Failed to load autostart status:', error);
    }
  };

  const handleAutostartToggle = async (checked: boolean) => {
    setIsLoadingAutostart(true);
    try {
      if (checked) {
        await invoke('enable_autostart');
      } else {
        await invoke('disable_autostart');
      }
      updateSettings({ enableAutostart: checked });
    } catch (error) {
      console.error('Failed to toggle autostart:', error);
      // Revert on error
      loadAutostartStatus();
    } finally {
      setIsLoadingAutostart(false);
    }
  };

  const handleClose = () => {
    setSettingsDialogOpen(false);
  };

  const tabs = [
    { id: "general" as const, label: "General", icon: Settings },
    { id: "chat" as const, label: "Chat", icon: MessageSquare },
    { id: "advanced" as const, label: "Advanced", icon: Sliders },
  ];

  return (
    <Dialog open={settingsDialogOpen} onOpenChange={handleClose}>
      <DialogContent
        onClose={handleClose}
        className="max-w-3xl max-h-[80vh] overflow-hidden flex flex-col"
      >
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <SettingsIcon className="h-5 w-5" />
            Settings
          </DialogTitle>
        </DialogHeader>

        <div className="flex flex-1 gap-6 overflow-hidden mt-7">
          {/* Tabs */}
          <div className="flex w-48 flex-col gap-1">
            {tabs.map((tab) => {
              const Icon = tab.icon;
              return (
                <button
                  key={tab.id}
                  onClick={() => setActiveTab(tab.id)}
                  className={cn(
                    "flex items-center gap-3 rounded-lg px-2 py-2.5 text-sm font-medium transition-colors",
                    activeTab === tab.id
                      ? "bg-accent-100 text-accent-900 dark:bg-accent-900/20 dark:text-accent-100"
                      : "text-gray-700 hover:bg-gray-100 dark:text-gray-300 dark:hover:bg-gray-800"
                  )}
                >
                  <Icon className="h-4 w-4" />
                  {tab.label}
                </button>
              );
            })}
          </div>

          {/* Content */}
          <div className="flex-1 overflow-y-auto pr-2 w-[500px] min-h-[400px]">
            {/* General Tab */}
            {activeTab === "general" && (
              <div className="space-y-6">
                <div>
                  <label className="flex items-center gap-3">
                    <input
                      type="checkbox"
                      checked={settings.enableAutostart}
                      onChange={(e) => handleAutostartToggle(e.target.checked)}
                      disabled={isLoadingAutostart}
                      className="h-4 w-4 rounded text-accent-600 disabled:opacity-50"
                    />
                    <span className="text-sm font-medium text-gray-900 dark:text-gray-100">
                      Launch on System Startup
                    </span>
                  </label>
                  <p className="ml-7 mt-1 text-sm text-gray-600 dark:text-gray-400">
                    Automatically start SparrowAI when your computer boots up
                  </p>
                </div>

                <div className={cn(!settings.enableAutostart && "opacity-50")}>
                  <label className="flex items-center gap-3">
                    <input
                      type="checkbox"
                      checked={settings.startMinimized}
                      onChange={(e) =>
                        updateSettings({ startMinimized: e.target.checked })
                      }
                      disabled={!settings.enableAutostart}
                      className="h-4 w-4 rounded text-accent-600 disabled:opacity-50 disabled:cursor-not-allowed"
                    />
                    <span className="text-sm font-medium text-gray-900 dark:text-gray-100">
                      Start Minimized
                    </span>
                  </label>
                  <p className="ml-7 mt-1 text-sm text-gray-600 dark:text-gray-400">
                    Launch the app minimized to system tray on startup
                  </p>
                </div>
              </div>
            )}

            {/* Chat Tab */}
            {activeTab === "chat" && (
              <div className="space-y-6">
                <div>
                  <label className="flex items-center gap-3">
                    <input
                      type="checkbox"
                      checked={settings.includeConversationHistory}
                      onChange={(e) =>
                        updateSettings({
                          includeConversationHistory: e.target.checked,
                        })
                      }
                      className="h-4 w-4 rounded text-accent-600"
                    />
                    <span className="text-sm font-medium text-gray-900 dark:text-gray-100">
                      Include Conversation History
                    </span>
                  </label>
                  <p className="ml-7 mt-1 text-sm text-gray-600 dark:text-gray-400">
                    Include previous messages in context for better responses
                  </p>
                </div>

                <div>
                  <label className="flex items-center gap-3">
                    <input
                      type="checkbox"
                      checked={settings.useRAG}
                      onChange={(e) =>
                        updateSettings({ useRAG: e.target.checked })
                      }
                      className="h-4 w-4 rounded text-accent-600"
                    />
                    <span className="text-sm font-medium text-gray-900 dark:text-gray-100">
                      Use RAG (Retrieval Augmented Generation)
                    </span>
                  </label>
                  <p className="ml-7 mt-1 text-sm text-gray-600 dark:text-gray-400">
                    Use document knowledge base to enhance responses
                  </p>
                </div>

                <div>
                  <label className="mb-2 block text-sm font-medium text-gray-900 dark:text-gray-100">
                    System Prompt
                  </label>
                  <Textarea
                    value={settings.systemPrompt}
                    onChange={(e) =>
                      updateSettings({ systemPrompt: e.target.value })
                    }
                    rows={4}
                    placeholder="Enter system prompt..."
                  />
                  <p className="mt-1 text-sm text-gray-600 dark:text-gray-400">
                    Instructions for the AI assistant's behavior
                  </p>
                </div>
              </div>
            )}

            {/* Advanced Tab */}
            {activeTab === "advanced" && (
              <div className="space-y-6">
                <div>
                  <label className="mb-2 block text-sm font-medium text-gray-900 dark:text-gray-100">
                    Temperature: {settings.temperature}
                  </label>
                  <input
                    type="range"
                    min="0"
                    max="2"
                    step="0.1"
                    value={settings.temperature}
                    onChange={(e) =>
                      updateSettings({
                        temperature: parseFloat(e.target.value),
                      })
                    }
                    className="w-full"
                  />
                  <p className="mt-1 text-sm text-gray-600 dark:text-gray-400">
                    Controls randomness in responses (0 = focused, 2 = creative)
                  </p>
                </div>

                <div>
                  <label className="mb-2 block text-sm font-medium text-gray-900 dark:text-gray-100">
                    Top P: {settings.topP}
                  </label>
                  <input
                    type="range"
                    min="0"
                    max="1"
                    step="0.05"
                    value={settings.topP}
                    onChange={(e) =>
                      updateSettings({ topP: parseFloat(e.target.value) })
                    }
                    className="w-full"
                  />
                  <p className="mt-1 text-sm text-gray-600 dark:text-gray-400">
                    Nucleus sampling threshold
                  </p>
                </div>

                <div className="grid grid-cols-2 gap-4">
                  <div>
                    <label className="mb-2 block text-sm font-medium text-gray-900 dark:text-gray-100">
                      Seed (Optional)
                    </label>
                    <Input
                      type="number"
                      value={settings.seed || ""}
                      onChange={(e) =>
                        updateSettings({
                          seed: e.target.value
                            ? parseInt(e.target.value)
                            : null,
                        })
                      }
                      placeholder="Random"
                    />
                  </div>

                  <div>
                    <label className="mb-2 block text-sm font-medium text-gray-900 dark:text-gray-100">
                      Max Tokens (Optional)
                    </label>
                    <Input
                      type="number"
                      value={settings.maxTokens || ""}
                      onChange={(e) =>
                        updateSettings({
                          maxTokens: e.target.value
                            ? parseInt(e.target.value)
                            : null,
                        })
                      }
                      placeholder="Default"
                    />
                  </div>
                </div>
              </div>
            )}
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={handleClose}>
            Close
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};
