import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  Button,
  Input,
} from "../ui";
import { useUI } from "@/store";
import { logError } from "@/lib/logger";
import type { Task, ActionType, TriggerTime } from "@/types/tasks";

interface CreateTaskDialogProps {
  open: boolean;
  onClose: () => void;
  onTaskCreated: () => void;
  editingTask?: Task | null;
}

interface McpServer {
  name: string;
  type: string;
}

interface McpTool {
  name: string;
  description?: string;
  inputSchema?: any;
}

interface BuiltinTool {
  name: string;
  description: string;
  input_schema: any;
}

export const CreateTaskDialog = ({
  open,
  onClose,
  onTaskCreated,
  editingTask,
}: CreateTaskDialogProps) => {
  const { showNotification } = useUI();
  const [taskName, setTaskName] = useState("");
  const [actionType, setActionType] = useState<
    "ShowNotification" | "RunMcpFunction"
  >("ShowNotification");

  // ShowNotification params
  const [notificationTitle, setNotificationTitle] = useState("");
  const [notificationMessage, setNotificationMessage] = useState("");

  // RunMcpFunction params
  const [toolSource, setToolSource] = useState<"builtin" | "mcp">("mcp");
  const [mcpServers, setMcpServers] = useState<McpServer[]>([]);
  const [selectedServer, setSelectedServer] = useState("");
  const [mcpTools, setMcpTools] = useState<McpTool[]>([]);
  const [builtinTools, setBuiltinTools] = useState<BuiltinTool[]>([]);
  const [selectedTool, setSelectedTool] = useState("");
  const [toolArguments, setToolArguments] = useState("{}");

  // Trigger settings
  const [triggerType, setTriggerType] = useState<
    "DateTime" | "Daily" | "Weekly" | "Monthly"
  >("Daily");
  const [triggerDateTime, setTriggerDateTime] = useState("");
  const [triggerTime, setTriggerTime] = useState("09:00");
  const [triggerDayOfWeek, setTriggerDayOfWeek] = useState(0); // 0=Sunday
  const [triggerDayOfMonth, setTriggerDayOfMonth] = useState(1); // 1-31
  const [autoDelete, setAutoDelete] = useState(false);

  const [submitting, setSubmitting] = useState(false);

  // Load MCP servers and builtin tools when dialog opens
  useEffect(() => {
    if (open && actionType === "RunMcpFunction") {
      loadMcpServers();
      loadBuiltinTools();
    }
  }, [open, actionType]);

  // Load tools when server is selected (for MCP)
  useEffect(() => {
    if (selectedServer && toolSource === "mcp") {
      loadMcpTools(selectedServer);
    }
  }, [selectedServer, toolSource]);

  const loadMcpServers = async () => {
    try {
      const servers = await invoke<McpServer[]>("get_mcp_servers");
      setMcpServers(servers);
    } catch (error) {
      logError("Failed to load MCP servers", error as Error);
    }
  };

  const loadBuiltinTools = async () => {
    try {
      const tools = await invoke<BuiltinTool[]>("get_builtin_tools");
      setBuiltinTools(tools);
    } catch (error) {
      logError("Failed to load builtin tools", error as Error);
    }
  };

  const loadMcpTools = async (serverName: string) => {
    try {
      const tools = await invoke<McpTool[]>("fetch_mcp_server_tools_details", {
        serverName,
      });
      setMcpTools(tools);
    } catch (error) {
      logError("Failed to load MCP tools", error as Error);
      setMcpTools([]);
    }
  };

  useEffect(() => {
    if (editingTask) {
      setTaskName(editingTask.name);

      // Set action type and params
      const action = editingTask.action_type;
      setActionType(action.type as any);

      switch (action.type) {
        case "ShowNotification":
          setNotificationTitle(action.title);
          setNotificationMessage(action.message);
          break;
        case "RunMcpFunction":
          // Check if it's a builtin tool (server_name === "builtin")
          if (action.server_name === "builtin") {
            setToolSource("builtin");
            setSelectedTool(action.tool_name);
          } else {
            setToolSource("mcp");
            setSelectedServer(action.server_name);
            setSelectedTool(action.tool_name);
          }
          setToolArguments(JSON.stringify(action.arguments, null, 2));
          break;
      }

      // Set trigger
      const trigger = editingTask.trigger_time;
      setTriggerType(trigger.type as any);

      switch (trigger.type) {
        case "DateTime":
          setTriggerDateTime(trigger.datetime);
          break;
        case "Daily":
          setTriggerTime(trigger.time);
          break;
        case "Weekly":
          setTriggerDayOfWeek(trigger.day_of_week);
          setTriggerTime(trigger.time);
          break;
        case "Monthly":
          setTriggerDayOfMonth(trigger.day_of_month);
          setTriggerTime(trigger.time);
          break;
      }
      
      setAutoDelete(editingTask.auto_delete || false);
    } else {
      resetForm();
    }
  }, [editingTask, open]);

  const resetForm = () => {
    setTaskName("");
    setActionType("ShowNotification");
    setNotificationTitle("");
    setNotificationMessage("");
    setToolSource("mcp");
    setSelectedServer("");
    setSelectedTool("");
    setToolArguments("{}");
    setTriggerType("Daily");
    // Pre-fill with current time for DateTime trigger
    const now = new Date();
    now.setMinutes(now.getMinutes() - now.getTimezoneOffset()); // Adjust for timezone
    setTriggerDateTime(now.toISOString().slice(0, 16)); // Format: YYYY-MM-DDTHH:mm
    setTriggerTime("09:00");
    setTriggerDayOfWeek(0);
    setTriggerDayOfMonth(1);
    setAutoDelete(false);
  };

  const handleSubmit = async () => {
    if (!taskName.trim()) {
      showNotification("Please enter a task name", "error");
      return;
    }

    try {
      setSubmitting(true);

      // Build action type
      let action: ActionType;
      switch (actionType) {
        case "ShowNotification":
          if (!notificationTitle.trim()) {
            showNotification("Please enter a notification title", "error");
            return;
          }
          if (!notificationMessage.trim()) {
            showNotification("Please enter a notification message", "error");
            return;
          }
          action = {
            type: "ShowNotification",
            title: notificationTitle,
            message: notificationMessage,
          };
          break;
        case "RunMcpFunction":
          if (toolSource === "mcp" && !selectedServer) {
            showNotification("Please select an MCP server", "error");
            return;
          }
          if (!selectedTool) {
            showNotification("Please select a tool", "error");
            return;
          }
          let args;
          try {
            args = JSON.parse(toolArguments);
          } catch (e) {
            showNotification("Invalid JSON in tool arguments", "error");
            return;
          }
          action = {
            type: "RunMcpFunction",
            server_name: toolSource === "builtin" ? "builtin" : selectedServer,
            tool_name: selectedTool,
            arguments: args,
          };
          break;
        default:
          throw new Error("Invalid action type");
      }

      // Build trigger time
      let trigger: TriggerTime;
      switch (triggerType) {
        case "DateTime":
          if (!triggerDateTime) {
            showNotification("Please select a date and time", "error");
            return;
          }
          trigger = {
            type: "DateTime",
            datetime: new Date(triggerDateTime).toISOString(),
          };
          break;
        case "Daily":
          if (!triggerTime) {
            showNotification("Please select a time", "error");
            return;
          }
          trigger = {
            type: "Daily",
            time: triggerTime,
          };
          break;
        case "Weekly":
          if (!triggerTime) {
            showNotification("Please select a time", "error");
            return;
          }
          trigger = {
            type: "Weekly",
            day_of_week: triggerDayOfWeek,
            time: triggerTime,
          };
          break;
        case "Monthly":
          if (!triggerTime) {
            showNotification("Please select a time", "error");
            return;
          }
          if (triggerDayOfMonth < 1 || triggerDayOfMonth > 31) {
            showNotification("Day of month must be between 1 and 31", "error");
            return;
          }
          trigger = {
            type: "Monthly",
            day_of_month: triggerDayOfMonth,
            time: triggerTime,
          };
          break;
        default:
          throw new Error("Invalid trigger type");
      }

      if (editingTask) {
        // Update existing task
        const updatedTask: Task = {
          ...editingTask,
          name: taskName,
          action_type: action,
          action_params: {},
          trigger_time: trigger,
          repeat_interval: undefined,
          auto_delete: autoDelete,
        };

        await invoke("update_task", { task: updatedTask });
        showNotification("Task updated successfully", "success", 3000);
      } else {
        // Create new task
        await invoke("create_task", {
          name: taskName,
          actionType: action,
          actionParams: {},
          triggerTime: trigger,
          repeatInterval: undefined,
          autoDelete: autoDelete,
        });
        showNotification("Task created successfully", "success", 3000);
      }

      onTaskCreated();
      onClose();
      resetForm();
    } catch (error) {
      logError("Failed to save task", error as Error);
      showNotification("Failed to save task", "error");
    } finally {
      setSubmitting(false);
    }
  };

  const weekDays = [
    { value: 0, label: "Sunday" },
    { value: 1, label: "Monday" },
    { value: 2, label: "Tuesday" },
    { value: 3, label: "Wednesday" },
    { value: 4, label: "Thursday" },
    { value: 5, label: "Friday" },
    { value: 6, label: "Saturday" },
  ];

  return (
    <Dialog open={open} onOpenChange={onClose}>
      <DialogContent className="w-[800px] max-w-[90vw] max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>
            {editingTask ? "Edit Task" : "Create New Task"}
          </DialogTitle>
        </DialogHeader>

        <div className="space-y-6 mt-4">
          {/* Task Name */}
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
              Task Name
            </label>
            <Input
              value={taskName}
              onChange={(e) => setTaskName(e.target.value)}
              placeholder="Enter task name"
            />
          </div>

          {/* Action Type */}
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
              Action Type
            </label>
            <select
              value={actionType}
              onChange={(e) => setActionType(e.target.value as any)}
              className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-800 text-gray-900 dark:text-white"
            >
              <option value="ShowNotification">Show Notification</option>
              <option value="RunMcpFunction">Run MCP Function</option>
            </select>
          </div>

          {/* Action Parameters */}
          {actionType === "ShowNotification" && (
            <div className="space-y-4">
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                  Notification Title
                </label>
                <Input
                  value={notificationTitle}
                  onChange={(e) => setNotificationTitle(e.target.value)}
                  placeholder="Enter notification title"
                />
              </div>
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                  Notification Message
                </label>
                <textarea
                  value={notificationMessage}
                  onChange={(e) => setNotificationMessage(e.target.value)}
                  placeholder="Enter notification message"
                  className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-800 text-gray-900 dark:text-white min-h-25"
                  rows={3}
                />
              </div>
            </div>
          )}

          {actionType === "RunMcpFunction" && (
            <div className="space-y-4">
              {/* Tool Source Selection */}
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                  Tool Source
                </label>
                <select
                  value={toolSource}
                  onChange={(e) => {
                    setToolSource(e.target.value as "builtin" | "mcp");
                    setSelectedServer("");
                    setSelectedTool("");
                    setToolArguments("{}");
                  }}
                  className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-800 text-gray-900 dark:text-white"
                >
                  <option value="mcp">MCP Server</option>
                  <option value="builtin">Built-in Functions</option>
                </select>
              </div>

              {/* MCP Server Selection */}
              {toolSource === "mcp" && (
                <>
                  <div>
                    <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                      MCP Server
                    </label>
                    <select
                      value={selectedServer}
                      onChange={(e) => {
                        setSelectedServer(e.target.value);
                        setSelectedTool("");
                      }}
                      className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-800 text-gray-900 dark:text-white truncate"
                    >
                      <option value="">Select a server...</option>
                      {mcpServers.map((server) => (
                        <option key={server.name} value={server.name}>
                          {server.name} ({server.type})
                        </option>
                      ))}
                    </select>
                  </div>

                  {selectedServer && (
                    <div>
                      <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                        MCP Tool
                      </label>
                      <select
                        value={selectedTool}
                        onChange={(e) => setSelectedTool(e.target.value)}
                        className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-800 text-gray-900 dark:text-white truncate"
                      >
                        <option value="">Select a tool...</option>
                        {mcpTools.map((tool) => (
                          <option key={tool.name} value={tool.name}>
                            {tool.name}
                            {tool.description ? ` - ${tool.description}` : ""}
                          </option>
                        ))}
                      </select>
                    </div>
                  )}
                </>
              )}

              {/* Builtin Tool Selection */}
              {toolSource === "builtin" && (
                <div>
                  <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                    Built-in Tool
                  </label>
                  <select
                    value={selectedTool}
                    onChange={(e) => setSelectedTool(e.target.value)}
                    className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-800 text-gray-900 dark:text-white truncate"
                  >
                    <option value="">Select a tool...</option>
                    {builtinTools.map((tool) => (
                      <option key={tool.name} value={tool.name}>
                        {tool.name} - {tool.description}
                      </option>
                    ))}
                  </select>
                </div>
              )}

              {/* Tool Arguments */}
              {selectedTool && (
                <div>
                  <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                    Tool Arguments (JSON)
                  </label>
                  <textarea
                    value={toolArguments}
                    onChange={(e) => setToolArguments(e.target.value)}
                    placeholder='{"arg1": "value1"}'
                    className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-800 text-gray-900 dark:text-white font-mono text-sm min-h-25"
                    rows={4}
                  />
                </div>
              )}
            </div>
          )}

          {/* Trigger Type & Parameters */}
          <div className="grid grid-cols-2 gap-4">
            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                When to Run
              </label>
              <select
                value={triggerType}
                onChange={(e) => setTriggerType(e.target.value as any)}
                className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-800 text-gray-900 dark:text-white"
              >
                <option value="DateTime">At specific date/time</option>
                <option value="Daily">Daily</option>
                <option value="Weekly">Weekly</option>
                <option value="Monthly">Monthly</option>
              </select>
            </div>

            {triggerType === "DateTime" && (
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                  Date and Time
                </label>
                <Input
                  type="datetime-local"
                  value={triggerDateTime}
                  onChange={(e) => setTriggerDateTime(e.target.value)}
                />
              </div>
            )}

            {triggerType === "Daily" && (
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                  Time (24-hour format)
                </label>
                <Input
                  type="time"
                  value={triggerTime}
                  onChange={(e) => setTriggerTime(e.target.value)}
                />
              </div>
            )}

            {(triggerType === "Weekly" || triggerType === "Monthly") && (
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                  Time (24-hour format)
                </label>
                <Input
                  type="time"
                  value={triggerTime}
                  onChange={(e) => setTriggerTime(e.target.value)}
                />
              </div>
            )}
          </div>

          {triggerType === "Weekly" && (
            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                Day of Week
              </label>
              <select
                value={triggerDayOfWeek}
                onChange={(e) => setTriggerDayOfWeek(parseInt(e.target.value))}
                className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-800 text-gray-900 dark:text-white"
              >
                {weekDays.map((day) => (
                  <option key={day.value} value={day.value}>
                    {day.label}
                  </option>
                ))}
              </select>
            </div>
          )}

          {triggerType === "Monthly" && (
            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                Day of Month (1-31)
              </label>
              <Input
                type="number"
                min="1"
                max="31"
                value={triggerDayOfMonth}
                onChange={(e) =>
                  setTriggerDayOfMonth(parseInt(e.target.value) || 1)
                }
              />
            </div>
          )}

          {/* Auto-delete toggle for one-time tasks */}
          {!editingTask?.repeat_interval && triggerType === "DateTime" && (
            <div className="flex items-center gap-3 p-4 bg-gray-50 dark:bg-gray-800/50 rounded-lg border border-gray-200 dark:border-gray-700">
              <input
                type="checkbox"
                id="auto-delete"
                checked={autoDelete}
                onChange={(e) => setAutoDelete(e.target.checked)}
                className="w-4 h-4 text-orange-600 bg-gray-100 border-gray-300 rounded focus:ring-orange-500 dark:focus:ring-orange-600 dark:ring-offset-gray-800 focus:ring-2 dark:bg-gray-700 dark:border-gray-600"
              />
              <label
                htmlFor="auto-delete"
                className="text-sm font-medium text-gray-700 dark:text-gray-300 cursor-pointer"
              >
                Auto-delete task after it runs (one-time tasks only)
              </label>
            </div>
          )}

          {/* Actions */}
          <div className="flex justify-end gap-3 pt-4 border-t">
            <Button variant="ghost" onClick={onClose} disabled={submitting}>
              Cancel
            </Button>
            <Button onClick={handleSubmit} disabled={submitting}>
              {submitting
                ? "Saving..."
                : editingTask
                ? "Update Task"
                : "Create Task"}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
};
