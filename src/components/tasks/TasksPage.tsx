import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Clock,
  Plus,
  Trash2,
  Play,
  Pause,
  Calendar,
  RefreshCw,
  CheckCircle,
  Edit,
  History,
} from "lucide-react";
import { PageContainer } from "../layout";
import { Button, Card } from "../ui";
import { useUI } from "@/store";
import { logError, logDebug } from "@/lib/logger";
import type { Task, TaskExecutionLog } from "@/types/tasks";
import { CreateTaskDialog } from "./CreateTaskDialog";
import { TaskLogsDialog } from "./TaskLogsDialog";

export const TasksPage = () => {
  const { showNotification } = useUI();
  const [tasks, setTasks] = useState<Task[]>([]);
  const [loading, setLoading] = useState(true);
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [editingTask, setEditingTask] = useState<Task | null>(null);
  const [logsDialogTask, setLogsDialogTask] = useState<Task | null>(null);

  useEffect(() => {
    loadTasks();

    // Listen for task execution events
    const unlisten = listen<TaskExecutionLog>("task-executed", (event) => {
      logDebug("Task executed event received", event.payload);
      const log = event.payload;

      if (log.status === "Success") {
        showNotification(
          `Task executed successfully: ${log.message || ""}`,
          "success",
          4000
        );
      } else if (log.status === "Failed") {
        showNotification(
          `Task failed: ${log.error || "Unknown error"}`,
          "error",
          5000
        );
      }

      // Reload tasks to update last_run and next_run
      loadTasks();
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const loadTasks = async () => {
    try {
      setLoading(true);
      const result = await invoke<Task[]>("get_tasks");
      logDebug("Tasks loaded", { count: result.length });
      setTasks(result);
    } catch (error) {
      logError("Failed to load tasks", error as Error);
      showNotification("Failed to load tasks", "error");
    } finally {
      setLoading(false);
    }
  };

  const handleToggleTask = async (taskId: string) => {
    try {
      const updatedTask = await invoke<Task>("toggle_task", { taskId });
      setTasks((prev) => prev.map((t) => (t.id === taskId ? updatedTask : t)));
      showNotification(
        `Task ${updatedTask.enabled ? "enabled" : "disabled"}`,
        "success",
        3000
      );
    } catch (error) {
      logError("Failed to toggle task", error as Error);
      showNotification("Failed to toggle task", "error");
    }
  };

  const handleDeleteTask = async (taskId: string) => {
    try {
      await invoke("delete_task", { taskId });
      setTasks((prev) => prev.filter((t) => t.id !== taskId));
      showNotification("Task deleted", "success", 3000);
    } catch (error) {
      logError("Failed to delete task", error as Error);
      showNotification("Failed to delete task", "error");
    }
  };

  const handleExecuteTask = async (taskId: string) => {
    try {
      await invoke("execute_task_manually", { taskId });
      showNotification("Task execution started", "info", 3000);
    } catch (error) {
      logError("Failed to execute task", error as Error);
      showNotification("Failed to execute task", "error");
    }
  };

  const formatTriggerTime = (trigger: Task["trigger_time"]) => {
    switch (trigger.type) {
      case "Daily":
        return `Daily at ${trigger.time}`;
      case "Weekly":
        const weekDays = [
          "Sunday",
          "Monday",
          "Tuesday",
          "Wednesday",
          "Thursday",
          "Friday",
          "Saturday",
        ];
        return `Weekly on ${weekDays[trigger.day_of_week]} at ${trigger.time}`;
      case "Monthly":
        return `Monthly on day ${trigger.day_of_month} at ${trigger.time}`;
      case "DateTime":
        return new Date(trigger.datetime).toLocaleString();
      case "EveryNMinutes":
        return `Every ${trigger.minutes} minute${
          trigger.minutes !== 1 ? "s" : ""
        }`;
      case "EveryNHours":
        return `Every ${trigger.hours} hour${trigger.hours !== 1 ? "s" : ""}`;
      default:
        return "Unknown";
    }
  };

  const formatRepeatInterval = (task: Task) => {
    // For Daily/Weekly/Monthly/EveryN triggers, they inherently repeat
    const triggerType = task.trigger_time.type;
    if (
      ["Daily", "Weekly", "Monthly", "EveryNMinutes", "EveryNHours"].includes(
        triggerType
      )
    ) {
      return "Repeating";
    }

    // For DateTime triggers, check repeat_interval
    if (!task.repeat_interval) return "Once";
    return `Every ${
      task.repeat_interval.value
    } ${task.repeat_interval.unit.toLowerCase()}`;
  };

  const formatActionType = (action: Task["action_type"]) => {
    switch (action.type) {
      case "ShowNotification":
        return `Notification: "${action.title}"`;
      case "RunMcpFunction":
        return `MCP: ${action.server_name}/${action.tool_name}`;
      default:
        return "Unknown action";
    }
  };

  const getActionIcon = (action: Task["action_type"]) => {
    switch (action.type) {
      case "ShowNotification":
        return "🔔";
      case "RunMcpFunction":
        return "🔧";
      default:
        return "❓";
    }
  };

  return (
    <PageContainer>
      <div className="space-y-6">
        {/* Header */}
        <div className="flex items-center justify-between">
          <div>
            <h1 className="text-3xl font-bold text-gray-900 dark:text-white">
              Scheduled Tasks
            </h1>
            <p className="text-gray-600 dark:text-gray-400 mt-1">
              Automate actions with scheduled tasks
            </p>
          </div>
          <Button
            onClick={() => {
              setEditingTask(null);
              setCreateDialogOpen(true);
            }}
            className="flex items-center gap-2"
          >
            <Plus className="h-4 w-4" />
            Create Task
          </Button>
        </div>

        {/* Tasks List */}
        {loading ? (
          <div className="flex items-center justify-center py-12">
            <RefreshCw className="h-8 w-8 animate-spin text-gray-400" />
          </div>
        ) : tasks.length === 0 ? (
          <Card className="p-12 text-center">
            <Clock className="h-16 w-16 mx-auto text-gray-400 mb-4" />
            <h3 className="text-lg font-semibold text-gray-900 dark:text-white mb-2">
              No tasks yet
            </h3>
            <p className="text-gray-600 dark:text-gray-400 mb-4">
              Create your first scheduled task to automate actions
            </p>
            <Button
              onClick={() => {
                setEditingTask(null);
                setCreateDialogOpen(true);
              }}
            >
              <Plus className="h-4 w-4 mr-2" />
              Create Task
            </Button>
          </Card>
        ) : (
          <div className="grid gap-4">
            {tasks.map((task) => (
              <Card
                key={task.id}
                className={`p-6 transition-all ${
                  task.enabled
                    ? "border-l-4 border-l-orange-500 dark:border-l-orange-400"
                    : "opacity-60"
                }`}
              >
                <div className="flex items-start justify-between">
                  <div className="flex-1">
                    <div className="flex items-center gap-3 mb-2">
                      <span className="text-2xl">
                        {getActionIcon(task.action_type)}
                      </span>
                      <div>
                        <h3 className="text-lg font-semibold text-gray-900 dark:text-white">
                          {task.name}
                        </h3>
                        <p className="text-sm text-gray-600 dark:text-gray-400">
                          {formatActionType(task.action_type)}
                        </p>
                      </div>
                    </div>

                    <div className="grid grid-cols-2 gap-4 mt-4 text-sm">
                      <div className="flex items-center gap-2">
                        <Calendar className="h-4 w-4 text-gray-400" />
                        <span className="text-gray-600 dark:text-gray-400">
                          {formatTriggerTime(task.trigger_time)}
                        </span>
                      </div>
                      <div className="flex items-center gap-2">
                        <RefreshCw className="h-4 w-4 text-gray-400" />
                        <span className="text-gray-600 dark:text-gray-400">
                          {formatRepeatInterval(task)}
                        </span>
                      </div>
                      {task.next_run && (
                        <div className="flex items-center gap-2">
                          <Clock className="h-4 w-4 text-gray-400" />
                          <span className="text-gray-600 dark:text-gray-400">
                            Next: {new Date(task.next_run).toLocaleString()}
                          </span>
                        </div>
                      )}
                      {task.last_run && (
                        <div className="flex items-center gap-2">
                          <CheckCircle className="h-4 w-4 text-green-500" />
                          <span className="text-gray-600 dark:text-gray-400">
                            Last: {new Date(task.last_run).toLocaleString()}
                          </span>
                        </div>
                      )}
                    </div>

                    <div className="mt-3 flex items-center gap-2 text-xs text-gray-500 dark:text-gray-400">
                      <span>Runs: {task.run_count}</span>
                      <span>•</span>
                      <span>
                        Created:{" "}
                        {new Date(task.created_at).toLocaleDateString()}
                      </span>
                      {task.auto_delete && !task.repeat_interval && (
                        <>
                          <span>•</span>
                          <span className="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-orange-100 text-orange-800 dark:bg-orange-900/30 dark:text-orange-400">
                            Auto-delete
                          </span>
                        </>
                      )}
                    </div>
                  </div>

                  <div className="flex items-center gap-2">
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => handleExecuteTask(task.id)}
                      title="Run now"
                    >
                      <Play className="h-4 w-4" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setLogsDialogTask(task)}
                      title="View logs"
                    >
                      <History className="h-4 w-4" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => {
                        setEditingTask(task);
                        setCreateDialogOpen(true);
                      }}
                      title="Edit"
                    >
                      <Edit className="h-4 w-4" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => handleToggleTask(task.id)}
                      title={task.enabled ? "Disable" : "Enable"}
                    >
                      {task.enabled ? (
                        <Pause className="h-4 w-4" />
                      ) : (
                        <Play className="h-4 w-4" />
                      )}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => handleDeleteTask(task.id)}
                      title="Delete"
                      className="text-red-600 hover:text-red-700 dark:text-red-400"
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </div>
                </div>
              </Card>
            ))}
          </div>
        )}
      </div>

      {/* Create/Edit Dialog */}
      <CreateTaskDialog
        open={createDialogOpen}
        onClose={() => {
          setCreateDialogOpen(false);
          setEditingTask(null);
        }}
        onTaskCreated={loadTasks}
        editingTask={editingTask}
      />

      {/* Logs Dialog */}
      {logsDialogTask && (
        <TaskLogsDialog
          task={logsDialogTask}
          open={true}
          onClose={() => setLogsDialogTask(null)}
        />
      )}
    </PageContainer>
  );
};
