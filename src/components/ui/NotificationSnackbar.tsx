import React, { useEffect } from "react";
import { X, CheckCircle, AlertCircle, Info, AlertTriangle } from "lucide-react";
import { cn } from "@/lib/utils";
import { useUI } from "@/store";

export const NotificationSnackbar: React.FC = () => {
  const { notification, clearNotification } = useUI();

  useEffect(() => {
    if (notification && notification.timeout) {
      const timer = setTimeout(() => {
        clearNotification();
      }, notification.timeout);
      return () => clearTimeout(timer);
    }
  }, [notification, clearNotification]);

  if (!notification) return null;

  const icons = {
    success: <CheckCircle className="h-5 w-5" />,
    error: <AlertCircle className="h-5 w-5" />,
    warning: <AlertTriangle className="h-5 w-5" />,
    info: <Info className="h-5 w-5" />,
  };

  const styles = {
    success:
      "bg-green-50 text-green-900 border-green-200 dark:bg-green-900/20 dark:text-green-100 dark:border-green-800",
    error:
      "bg-red-50 text-red-900 border-red-200 dark:bg-red-900/20 dark:text-red-100 dark:border-red-800",
    warning:
      "bg-yellow-50 text-yellow-900 border-yellow-200 dark:bg-yellow-900/20 dark:text-yellow-100 dark:border-yellow-800",
    info: "bg-blue-50 text-blue-900 border-blue-200 dark:bg-blue-900/20 dark:text-blue-100 dark:border-blue-800",
  };

  return (
    <div className="fixed bottom-4 right-4 z-50 animate-in">
      <div
        className={cn(
          "flex items-center gap-3 rounded-lg border px-4 py-3 shadow-lg",
          styles[notification.type]
        )}
      >
        {icons[notification.type]}
        <p className="text-sm font-medium">{notification.message}</p>
        <button
          onClick={clearNotification}
          className="ml-2 rounded-md p-1 hover:bg-black/10 dark:hover:bg-white/10"
        >
          <X className="h-4 w-4" />
        </button>
      </div>
    </div>
  );
};
