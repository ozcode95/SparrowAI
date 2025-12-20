import React, { useState, useEffect } from "react";
import { Settings as SettingsIcon, RefreshCw } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/Dialog";
import { Button, LoadingSpinner } from "@/components/ui";

interface OvmsStatusDialogProps {
  open: boolean;
  onClose: () => void;
}

interface OvmsStatus {
  status: string;
  loaded_models: string[];
}

export const OvmsStatusDialog: React.FC<OvmsStatusDialogProps> = ({
  open,
  onClose,
}) => {
  const [ovmsStatus, setOvmsStatus] = useState<OvmsStatus | null>(null);
  const [checkingStatus, setCheckingStatus] = useState(false);
  const [statusError, setStatusError] = useState("");

  useEffect(() => {
    if (open) {
      handleCheckOvmsStatus();
    }
  }, [open]);

  const handleCheckOvmsStatus = async () => {
    setCheckingStatus(true);
    setStatusError("");
    setOvmsStatus(null);

    try {
      const ovmsStatusResponse: any = await invoke("check_ovms_status");
      setOvmsStatus(ovmsStatusResponse);
    } catch (error) {
      console.error("OvmsStatusDialog: Failed to check OVMS status:", error);
      setStatusError(String(error));
    } finally {
      setCheckingStatus(false);
    }
  };

  const handleGetModelDetails = async (modelName: string) => {
    try {
      const response: any = await invoke("get_ovms_model_metadata", {
        modelName,
      });
      const metadataData = JSON.parse(response);
      alert(
        `Model Details for ${modelName}:\n\n${JSON.stringify(
          metadataData,
          null,
          2
        )}`
      );
    } catch (error) {
      console.error("OvmsStatusDialog: Failed to get model metadata:", error);
      alert(`Failed to get details for ${modelName}:\n${error}`);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onClose}>
      <DialogContent onClose={onClose} className="max-w-2xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <SettingsIcon className="h-5 w-5" />
            {checkingStatus ? "Checking OVMS Status..." : "OVMS Server Status"}
          </DialogTitle>
          <DialogDescription>
            View the status of the OpenVINO Model Server
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-4">
          {checkingStatus && (
            <div className="flex items-center gap-3 rounded-lg border border-gray-200 bg-gray-50 p-4 dark:border-gray-800 dark:bg-gray-900">
              <LoadingSpinner size="sm" />
              <span className="text-sm text-gray-700 dark:text-gray-300">
                Checking OVMS server status...
              </span>
            </div>
          )}

          {statusError && (
            <div className="rounded-lg border border-red-200 bg-red-50 p-4 dark:border-red-800 dark:bg-red-900/20">
              <p className="text-sm font-medium text-red-900 dark:text-red-100">
                Error: {statusError}
              </p>
              <p className="mt-2 text-sm text-red-700 dark:text-red-200">
                Make sure OVMS server is running on localhost:1114
              </p>
            </div>
          )}

          {ovmsStatus && (
            <div className="rounded-lg border border-green-200 bg-green-50 p-4 dark:border-green-800 dark:bg-green-900/20">
              <p className="text-sm font-medium text-green-900 dark:text-green-100">
                OVMS Server is running!
              </p>
              <div className="mt-3 space-y-2">
                <p className="text-sm text-green-800 dark:text-green-200">
                  <strong>Status:</strong> {ovmsStatus.status}
                </p>
                <p className="text-sm text-green-800 dark:text-green-200">
                  <strong>
                    Loaded Models ({ovmsStatus.loaded_models?.length || 0}):
                  </strong>
                </p>
                {!ovmsStatus.loaded_models ||
                ovmsStatus.loaded_models.length === 0 ? (
                  <p className="ml-4 text-sm text-green-700 dark:text-green-300">
                    No models currently loaded
                  </p>
                ) : (
                  <div className="ml-4 space-y-1">
                    {ovmsStatus.loaded_models.map((modelName, idx) => (
                      <button
                        key={idx}
                        onClick={() => handleGetModelDetails(modelName)}
                        className="text-sm text-green-700 hover:text-green-900 hover:underline dark:text-green-300 dark:hover:text-green-100"
                      >
                        • {modelName}
                      </button>
                    ))}
                  </div>
                )}
              </div>
            </div>
          )}
        </div>

        <DialogFooter>
          <Button
            variant="outline"
            onClick={handleCheckOvmsStatus}
            disabled={checkingStatus}
          >
            <RefreshCw className="h-4 w-4" />
            Refresh Status
          </Button>
          <Button onClick={onClose}>Close</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};
