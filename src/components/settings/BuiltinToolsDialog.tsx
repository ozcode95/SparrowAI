import React, { useState, useEffect } from "react";
import { Wrench, X } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/Dialog";
import { Button } from "@/components/ui";

interface BuiltinToolsConfig {
  tools: Record<string, boolean>;
}

interface BuiltinToolsDialogProps {
  open: boolean;
  onClose: () => void;
}

export const BuiltinToolsDialog: React.FC<BuiltinToolsDialogProps> = ({
  open,
  onClose,
}) => {
  const [config, setConfig] = useState<BuiltinToolsConfig | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (open) {
      loadConfig();
    }
  }, [open]);

  const loadConfig = async () => {
    try {
      const loadedConfig = await invoke<BuiltinToolsConfig>(
        "get_builtin_tools_config"
      );
      setConfig(loadedConfig);
    } catch (error) {
      console.error("Failed to load builtin tools config:", error);
    }
  };

  const handleToggle = async (toolName: string, enabled: boolean) => {
    if (!config) return;

    // Update local state immediately
    setConfig({
      ...config,
      tools: {
        ...config.tools,
        [toolName]: enabled,
      },
    });

    // Save to backend
    try {
      await invoke("set_builtin_tool_enabled", {
        toolName,
        enabled,
      });
    } catch (error) {
      console.error("Failed to update tool status:", error);
      // Revert on error
      loadConfig();
    }
  };

  const handleEnableAll = async () => {
    if (!config) return;
    setLoading(true);
    try {
      const updatedTools: Record<string, boolean> = {};
      Object.keys(config.tools).forEach((key) => {
        updatedTools[key] = true;
      });
      const newConfig = { tools: updatedTools };
      await invoke("save_builtin_tools_config", { config: newConfig });
      setConfig(newConfig);
    } catch (error) {
      console.error("Failed to enable all tools:", error);
    } finally {
      setLoading(false);
    }
  };

  const handleDisableAll = async () => {
    if (!config) return;
    setLoading(true);
    try {
      const updatedTools: Record<string, boolean> = {};
      Object.keys(config.tools).forEach((key) => {
        updatedTools[key] = false;
      });
      const newConfig = { tools: updatedTools };
      await invoke("save_builtin_tools_config", { config: newConfig });
      setConfig(newConfig);
    } catch (error) {
      console.error("Failed to disable all tools:", error);
    } finally {
      setLoading(false);
    }
  };

  // Format tool name for display
  const formatToolName = (name: string) => {
    return name
      .split("_")
      .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
      .join(" ");
  };

  if (!config) {
    return null;
  }

  const sortedTools = Object.keys(config.tools).sort();

  return (
    <Dialog open={open} onOpenChange={onClose}>
      <DialogContent
        onClose={onClose}
        className="max-w-2xl max-h-[80vh] overflow-hidden flex flex-col"
      >
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Wrench className="h-5 w-5" />
            Built-in Tools Configuration
          </DialogTitle>
        </DialogHeader>

        <div className="flex-1 overflow-y-auto pr-2">
          <div className="mb-4 flex gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={handleEnableAll}
              disabled={loading}
            >
              Enable All
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={handleDisableAll}
              disabled={loading}
            >
              Disable All
            </Button>
          </div>

          <div className="space-y-2">
            {sortedTools.map((toolName) => (
              <div
                key={toolName}
                className="flex items-center justify-between p-3 rounded-lg border border-gray-200 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800 transition-colors"
              >
                <div className="flex-1">
                  <div className="font-medium text-sm text-gray-900 dark:text-gray-100">
                    {formatToolName(toolName)}
                  </div>
                  <div className="text-xs text-gray-500 dark:text-gray-400 font-mono">
                    {toolName}
                  </div>
                </div>
                <label className="relative inline-flex items-center cursor-pointer">
                  <input
                    type="checkbox"
                    checked={config.tools[toolName]}
                    onChange={(e) => handleToggle(toolName, e.target.checked)}
                    className="sr-only peer"
                  />
                  <div className="w-11 h-6 bg-gray-200 peer-focus:outline-none peer-focus:ring-4 peer-focus:ring-accent-300 dark:peer-focus:ring-accent-800 rounded-full peer dark:bg-gray-700 peer-checked:after:translate-x-full rtl:peer-checked:after:-translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-0.5 after:start-0.5 after:bg-white after:border-gray-300 after:border after:rounded-full after:h-5 after:w-5 after:transition-all dark:border-gray-600 peer-checked:bg-accent-600"></div>
                </label>
              </div>
            ))}
          </div>
        </div>

        <div className="flex justify-end mt-4 pt-4 border-t border-gray-200 dark:border-gray-700">
          <Button variant="outline" onClick={onClose}>
            Close
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
};
