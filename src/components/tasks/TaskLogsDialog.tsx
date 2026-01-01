import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  Button,
} from "../ui";
import { CheckCircle, XCircle, AlertCircle, RefreshCw } from "lucide-react";
import { logError } from "@/lib/logger";
import type { Task, TaskExecutionLog } from "@/types/tasks";

interface TaskLogsDialogProps {
  task: Task;
  open: boolean;
  onClose: () => void;
}

export const TaskLogsDialog = ({
  task,
  open,
  onClose,
}: TaskLogsDialogProps) => {
  const [logs, setLogs] = useState<TaskExecutionLog[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (open) {
      loadLogs();
    }
  }, [open, task.id]);

  const loadLogs = async () => {
    try {
      setLoading(true);
      const result = await invoke<TaskExecutionLog[]>("get_task_logs", {
        taskId: task.id,
      });
      setLogs(
        result.sort(
          (a, b) =>
            new Date(b.executed_at).getTime() -
            new Date(a.executed_at).getTime()
        )
      );
    } catch (error) {
      logError("Failed to load task logs", error as Error);
    } finally {
      setLoading(false);
    }
  };

  const getStatusIcon = (status: TaskExecutionLog["status"]) => {
    switch (status) {
      case "Success":
        return <CheckCircle className="h-5 w-5 text-green-500" />;
      case "Failed":
        return <XCircle className="h-5 w-5 text-red-500" />;
      case "Skipped":
        return <AlertCircle className="h-5 w-5 text-yellow-500" />;
    }
  };

  const getStatusColor = (status: TaskExecutionLog["status"]) => {
    switch (status) {
      case "Success":
        return "text-green-600 dark:text-green-400";
      case "Failed":
        return "text-red-600 dark:text-red-400";
      case "Skipped":
        return "text-yellow-600 dark:text-yellow-400";
    }
  };

  return (
    <Dialog open={open} onOpenChange={onClose}>
      <DialogContent className="max-w-3xl max-h-[80vh]">
        <DialogHeader>
          <DialogTitle>Task Execution Logs: {task.name}</DialogTitle>
        </DialogHeader>

        <div className="mt-4">
          {loading ? (
            <div className="flex items-center justify-center py-8">
              <RefreshCw className="h-8 w-8 animate-spin text-gray-400" />
            </div>
          ) : logs.length === 0 ? (
            <div className="text-center py-8">
              <AlertCircle className="h-12 w-12 mx-auto text-gray-400 mb-3" />
              <p className="text-gray-600 dark:text-gray-400">
                No execution logs yet
              </p>
            </div>
          ) : (
            <div className="space-y-3 max-h-[500px] overflow-y-auto">
              {logs.map((log, index) => (
                <div
                  key={index}
                  className="p-4 border border-gray-200 dark:border-gray-700 rounded-lg"
                >
                  <div className="flex items-start gap-3">
                    {getStatusIcon(log.status)}
                    <div className="flex-1">
                      <div className="flex items-center justify-between mb-1">
                        <span
                          className={`font-semibold ${getStatusColor(
                            log.status
                          )}`}
                        >
                          {log.status}
                        </span>
                        <span className="text-sm text-gray-500 dark:text-gray-400">
                          {new Date(log.executed_at).toLocaleString()}
                        </span>
                      </div>
                      {log.message && (
                        <p className="text-sm text-gray-700 dark:text-gray-300 mt-2">
                          {log.message}
                        </p>
                      )}
                      {log.error && (
                        <p className="text-sm text-red-600 dark:text-red-400 mt-2 font-mono bg-red-50 dark:bg-red-900/20 p-2 rounded">
                          {log.error}
                        </p>
                      )}
                    </div>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>

        <div className="flex justify-end gap-3 pt-4 border-t mt-4">
          <Button variant="ghost" onClick={loadLogs}>
            <RefreshCw className="h-4 w-4 mr-2" />
            Refresh
          </Button>
          <Button onClick={onClose}>Close</Button>
        </div>
      </DialogContent>
    </Dialog>
  );
};
