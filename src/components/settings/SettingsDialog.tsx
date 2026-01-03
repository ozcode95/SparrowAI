import React, { useState, useEffect } from "react";
import {
  Settings as SettingsIcon,
  MessageSquare,
  Sliders,
  Settings,
  Sparkles,
} from "lucide-react";
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
  const [activeTab, setActiveTab] = useState<"general" | "chat" | "advanced" | "features">(
    "general"
  );
  const [isLoadingAutostart, setIsLoadingAutostart] = useState(false);

  const { settingsDialogOpen, setSettingsDialogOpen } = useUI();
  const { settings, updateSettings, resetSettings } = useSettings();

  // Load settings from backend when dialog opens
  useEffect(() => {
    if (settingsDialogOpen) {
      loadSettings();
    }
  }, [settingsDialogOpen]);

  const loadSettings = async () => {
    try {
      const config = await invoke<typeof settings>("get_app_config");
      updateSettings(config);
    } catch (error) {
      console.error("Failed to load settings:", error);
    }
  };

  const handleAutostartToggle = async (checked: boolean) => {
    setIsLoadingAutostart(true);
    try {
      if (checked) {
        await invoke("enable_autostart");
      } else {
        await invoke("disable_autostart");
      }
      updateSettings({ enableAutostart: checked });
      // Save to backend config
      await invoke("update_app_config", {
        updates: { enable_autostart: checked },
      });
    } catch (error) {
      console.error("Failed to toggle autostart:", error);
      // Reload settings on error
      loadSettings();
    } finally {
      setIsLoadingAutostart(false);
    }
  };

  const handleClose = () => {
    setSettingsDialogOpen(false);
  };

  // Helper to update both local state and backend config
  const handleSettingChange = async (updates: Partial<typeof settings>) => {
    updateSettings(updates);
    try {
      // Convert camelCase to snake_case for backend
      const backendUpdates: any = {};
      Object.entries(updates).forEach(([key, value]) => {
        // Special handling for certain keys
        let snakeKey: string;
        if (key === "useRAG") {
          snakeKey = "use_rag";
        } else {
          snakeKey = key.replace(/([A-Z])/g, "_$1").toLowerCase();
        }
        backendUpdates[snakeKey] = value;
      });
      await invoke("update_app_config", { updates: backendUpdates });
    } catch (error) {
      console.error("Failed to save settings:", error);
    }
  };

  const tabs = [
    { id: "general" as const, label: "General", icon: Settings },
    { id: "chat" as const, label: "Chat", icon: MessageSquare },
    { id: "advanced" as const, label: "Advanced", icon: Sliders },
    { id: "features" as const, label: "Features", icon: Sparkles },
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
                <div className="flex items-start justify-between">
                  <div className="flex-1">
                    <div className="text-sm font-medium text-gray-900 dark:text-gray-100">
                      Launch on System Startup
                    </div>
                    <p className="mt-1 text-sm text-gray-600 dark:text-gray-400">
                      Automatically start SparrowAI when your computer boots up
                    </p>
                  </div>
                  <label className="relative inline-flex items-center cursor-pointer ml-4">
                    <input
                      type="checkbox"
                      checked={settings.enableAutostart}
                      onChange={(e) => handleAutostartToggle(e.target.checked)}
                      disabled={isLoadingAutostart}
                      className="sr-only peer"
                    />
                    <div className="w-11 h-6 bg-gray-200 peer-focus:outline-none peer-focus:ring-4 peer-focus:ring-accent-300 dark:peer-focus:ring-accent-800 rounded-full peer dark:bg-gray-700 peer-checked:after:translate-x-full rtl:peer-checked:after:-translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-0.5 after:start-0.5 after:bg-white after:border-gray-300 after:border after:rounded-full after:h-5 after:w-5 after:transition-all dark:border-gray-600 peer-checked:bg-accent-600 peer-disabled:opacity-50 peer-disabled:cursor-not-allowed"></div>
                  </label>
                </div>

                <div className={cn("flex items-start justify-between", !settings.enableAutostart && "opacity-50")}>
                  <div className="flex-1">
                    <div className="text-sm font-medium text-gray-900 dark:text-gray-100">
                      Start Minimized
                    </div>
                    <p className="mt-1 text-sm text-gray-600 dark:text-gray-400">
                      Launch the app minimized to system tray on startup
                    </p>
                  </div>
                  <label className="relative inline-flex items-center cursor-pointer ml-4">
                    <input
                      type="checkbox"
                      checked={settings.startMinimized}
                      onChange={(e) =>
                        handleSettingChange({
                          startMinimized: e.target.checked,
                        })
                      }
                      disabled={!settings.enableAutostart}
                      className="sr-only peer"
                    />
                    <div className="w-11 h-6 bg-gray-200 peer-focus:outline-none peer-focus:ring-4 peer-focus:ring-accent-300 dark:peer-focus:ring-accent-800 rounded-full peer dark:bg-gray-700 peer-checked:after:translate-x-full rtl:peer-checked:after:-translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-0.5 after:start-0.5 after:bg-white after:border-gray-300 after:border after:rounded-full after:h-5 after:w-5 after:transition-all dark:border-gray-600 peer-checked:bg-accent-600 peer-disabled:opacity-50 peer-disabled:cursor-not-allowed"></div>
                  </label>
                </div>
              </div>
            )}

            {/* Chat Tab */}
            {activeTab === "chat" && (
              <div className="space-y-6">
                <div className="flex items-start justify-between">
                  <div className="flex-1">
                    <div className="text-sm font-medium text-gray-900 dark:text-gray-100">
                      Include Conversation History
                    </div>
                    <p className="mt-1 text-sm text-gray-600 dark:text-gray-400">
                      Include previous messages in context for better responses
                    </p>
                  </div>
                  <label className="relative inline-flex items-center cursor-pointer ml-4">
                    <input
                      type="checkbox"
                      checked={settings.includeConversationHistory}
                      onChange={(e) =>
                        handleSettingChange({
                          includeConversationHistory: e.target.checked,
                        })
                      }
                      className="sr-only peer"
                    />
                    <div className="w-11 h-6 bg-gray-200 peer-focus:outline-none peer-focus:ring-4 peer-focus:ring-accent-300 dark:peer-focus:ring-accent-800 rounded-full peer dark:bg-gray-700 peer-checked:after:translate-x-full rtl:peer-checked:after:-translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-0.5 after:start-0.5 after:bg-white after:border-gray-300 after:border after:rounded-full after:h-5 after:w-5 after:transition-all dark:border-gray-600 peer-checked:bg-accent-600"></div>
                  </label>
                </div>

                <div>
                  <label className="mb-2 block text-sm font-medium text-gray-900 dark:text-gray-100">
                    System Prompt
                  </label>
                  <Textarea
                    value={settings.systemPrompt}
                    onChange={(e) =>
                      handleSettingChange({ systemPrompt: e.target.value })
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
                      handleSettingChange({
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
                      handleSettingChange({ topP: parseFloat(e.target.value) })
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
                        handleSettingChange({
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
                        handleSettingChange({
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

            {/* Features Tab */}
            {activeTab === "features" && (
              <div className="space-y-6 pb-6">
                <div className="flex items-start justify-between">
                  <div className="flex-1">
                    <div className="text-sm font-medium text-gray-900 dark:text-gray-100">
                      Image Captioning (Vision-to-Text)
                    </div>
                    <p className="mt-1 text-sm text-gray-600 dark:text-gray-400">
                      Enable vision-to-text model selection and image attachments in chat
                    </p>
                  </div>
                  <label className="relative inline-flex items-center cursor-pointer ml-4">
                    <input
                      type="checkbox"
                      checked={settings.enableImageCaptioning}
                      onChange={(e) =>
                        handleSettingChange({
                          enableImageCaptioning: e.target.checked,
                        })
                      }
                      className="sr-only peer"
                    />
                    <div className="w-11 h-6 bg-gray-200 peer-focus:outline-none peer-focus:ring-4 peer-focus:ring-accent-300 dark:peer-focus:ring-accent-800 rounded-full peer dark:bg-gray-700 peer-checked:after:translate-x-full rtl:peer-checked:after:-translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-0.5 after:start-0.5 after:bg-white after:border-gray-300 after:border after:rounded-full after:h-5 after:w-5 after:transition-all dark:border-gray-600 peer-checked:bg-accent-600"></div>
                  </label>
                </div>

                <div className="flex items-start justify-between">
                  <div className="flex-1">
                    <div className="text-sm font-medium text-gray-900 dark:text-gray-100">
                      Image Generation
                    </div>
                    <p className="mt-1 text-sm text-gray-600 dark:text-gray-400">
                      Show the Gallery page for generating images
                    </p>
                  </div>
                  <label className="relative inline-flex items-center cursor-pointer ml-4">
                    <input
                      type="checkbox"
                      checked={settings.enableImageGeneration}
                      onChange={(e) =>
                        handleSettingChange({
                          enableImageGeneration: e.target.checked,
                        })
                      }
                      className="sr-only peer"
                    />
                    <div className="w-11 h-6 bg-gray-200 peer-focus:outline-none peer-focus:ring-4 peer-focus:ring-accent-300 dark:peer-focus:ring-accent-800 rounded-full peer dark:bg-gray-700 peer-checked:after:translate-x-full rtl:peer-checked:after:-translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-0.5 after:start-0.5 after:bg-white after:border-gray-300 after:border after:rounded-full after:h-5 after:w-5 after:transition-all dark:border-gray-600 peer-checked:bg-accent-600"></div>
                  </label>
                </div>

                <div className="flex items-start justify-between">
                  <div className="flex-1">
                    <div className="text-sm font-medium text-gray-900 dark:text-gray-100">
                      RAG (Retrieval Augmented Generation)
                    </div>
                    <p className="mt-1 text-sm text-gray-600 dark:text-gray-400">
                      Enable document knowledge base, Documents page, and file attachments
                    </p>
                  </div>
                  <label className="relative inline-flex items-center cursor-pointer ml-4">
                    <input
                      type="checkbox"
                      checked={settings.enableRag}
                      onChange={(e) =>
                        handleSettingChange({
                          enableRag: e.target.checked,
                        })
                      }
                      className="sr-only peer"
                    />
                    <div className="w-11 h-6 bg-gray-200 peer-focus:outline-none peer-focus:ring-4 peer-focus:ring-accent-300 dark:peer-focus:ring-accent-800 rounded-full peer dark:bg-gray-700 peer-checked:after:translate-x-full rtl:peer-checked:after:-translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-0.5 after:start-0.5 after:bg-white after:border-gray-300 after:border after:rounded-full after:h-5 after:w-5 after:transition-all dark:border-gray-600 peer-checked:bg-accent-600"></div>
                  </label>
                </div>

                <div className="flex items-start justify-between">
                  <div className="flex-1">
                    <div className="text-sm font-medium text-gray-900 dark:text-gray-100">
                      Speech-to-Text
                    </div>
                    <p className="mt-1 text-sm text-gray-600 dark:text-gray-400">
                      Enable voice input capabilities (coming soon)
                    </p>
                  </div>
                  <label className="relative inline-flex items-center cursor-pointer ml-4">
                    <input
                      type="checkbox"
                      checked={settings.enableSpeechToText}
                      onChange={(e) =>
                        handleSettingChange({
                          enableSpeechToText: e.target.checked,
                        })
                      }
                      className="sr-only peer"
                    />
                    <div className="w-11 h-6 bg-gray-200 peer-focus:outline-none peer-focus:ring-4 peer-focus:ring-accent-300 dark:peer-focus:ring-accent-800 rounded-full peer dark:bg-gray-700 peer-checked:after:translate-x-full rtl:peer-checked:after:-translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-0.5 after:start-0.5 after:bg-white after:border-gray-300 after:border after:rounded-full after:h-5 after:w-5 after:transition-all dark:border-gray-600 peer-checked:bg-accent-600"></div>
                  </label>
                </div>

                <div className="flex items-start justify-between">
                  <div className="flex-1">
                    <div className="text-sm font-medium text-gray-900 dark:text-gray-100">
                      Text-to-Speech
                    </div>
                    <p className="mt-1 text-sm text-gray-600 dark:text-gray-400">
                      Enable voice output for AI responses (coming soon)
                    </p>
                  </div>
                  <label className="relative inline-flex items-center cursor-pointer ml-4">
                    <input
                      type="checkbox"
                      checked={settings.enableTextToSpeech}
                      onChange={(e) =>
                        handleSettingChange({
                          enableTextToSpeech: e.target.checked,
                        })
                      }
                      className="sr-only peer"
                    />
                    <div className="w-11 h-6 bg-gray-200 peer-focus:outline-none peer-focus:ring-4 peer-focus:ring-accent-300 dark:peer-focus:ring-accent-800 rounded-full peer dark:bg-gray-700 peer-checked:after:translate-x-full rtl:peer-checked:after:-translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-0.5 after:start-0.5 after:bg-white after:border-gray-300 after:border after:rounded-full after:h-5 after:w-5 after:transition-all dark:border-gray-600 peer-checked:bg-accent-600"></div>
                  </label>
                </div>
              </div>
            )}
          </div>
        </div>

        <DialogFooter>
          <div className="flex justify-between w-full">
            <Button variant="outline" onClick={resetSettings}>
              Reset to Defaults
            </Button>
            <Button variant="outline" onClick={handleClose}>
              Close
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};
