import { useUI } from "@/store";

export const useNotification = () => {
  const { showNotification, clearNotification, notification } = useUI();

  const showSuccess = (message: string, timeout: number = 5000) => {
    showNotification(message, "success", timeout);
  };

  const showError = (message: string, timeout: number = 8000) => {
    showNotification(message, "error", timeout);
  };

  const showWarning = (message: string, timeout: number = 6000) => {
    showNotification(message, "warning", timeout);
  };

  const showInfo = (message: string, timeout: number = 5000) => {
    showNotification(message, "info", timeout);
  };

  return {
    notification,
    showSuccess,
    showError,
    showWarning,
    showInfo,
    clearNotification,
  };
};
