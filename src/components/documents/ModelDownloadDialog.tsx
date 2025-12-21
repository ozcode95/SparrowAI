import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Dialog } from "../ui";
import { Download, AlertCircle, CheckCircle2, X } from "lucide-react";
import { GraphGenerationParams } from "@/types/models";

interface ModelDownloadDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onSuccess: () => void;
}

interface DownloadProgress {
  modelId: string;
  currentFile: string;
  currentFileIndex: number;
  totalFiles: number;
  downloadedBytes: number;
  totalBytes: number;
  currentFileDownloaded: number;
  currentFileTotal: number;
}

export const ModelDownloadDialog = ({
  isOpen,
  onClose,
  onSuccess,
}: ModelDownloadDialogProps) => {
  const [isDownloading, setIsDownloading] = useState(false);
  const [currentModel, setCurrentModel] = useState<string>("");
  const [progress, setProgress] = useState<DownloadProgress | null>(null);
  const [error, setError] = useState<string>("");
  const [completedModels, setCompletedModels] = useState<Set<string>>(
    new Set()
  );

  const BGE_MODELS = [
    { id: "OpenVINO/bge-base-en-v1.5-int8-ov", name: "BGE Embedding Model" },
    { id: "OpenVINO/bge-reranker-base-int8-ov", name: "BGE Reranker Model" },
  ];

  const handleDownload = async () => {
    setIsDownloading(true);
    setError("");
    setCompletedModels(new Set());

    // Set up progress listener
    const unlisten = await listen<DownloadProgress>(
      "model-download-progress",
      (event) => {
        setProgress(event.payload);
      }
    );

    try {
      for (const model of BGE_MODELS) {
        setCurrentModel(model.name);
        setProgress(null);

        // Use default graph params optimized for embeddings/reranker models
        const graphParams: GraphGenerationParams = {
          target_device: "CPU",
          num_streams: 1,
          normalize: true, // Important for embeddings
        };

        await invoke("download_entire_model", {
          modelId: model.id,
          downloadPath: null, // Use default path
          graphParams,
        });

        setCompletedModels((prev) => new Set([...prev, model.id]));
      }

      // All models downloaded successfully
      unlisten();
      onSuccess();
      onClose();
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      setError(`Failed to download models: ${errorMessage}`);
      setIsDownloading(false);
      unlisten();
    }
  };

  const formatBytes = (bytes: number) => {
    if (bytes === 0) return "0 B";
    const k = 1024;
    const sizes = ["B", "KB", "MB", "GB"];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return `${(bytes / Math.pow(k, i)).toFixed(2)} ${sizes[i]}`;
  };

  const getProgressPercentage = () => {
    if (!progress || progress.totalBytes === 0) return 0;
    return Math.round((progress.downloadedBytes / progress.totalBytes) * 100);
  };

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => !isDownloading && !open && onClose()}
    >
      <div className="p-6 bg-white dark:bg-gray-800 rounded-xl max-w-lg w-full relative">
        {/* Close button (disabled during download) */}
        {!isDownloading && (
          <button
            onClick={onClose}
            className="absolute top-4 right-4 p-1 text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200 rounded transition-colors"
          >
            <X className="w-5 h-5" />
          </button>
        )}

        <div className="flex items-center gap-3 mb-4">
          <div className="p-2 rounded-lg bg-blue-500/10">
            <Download className="w-6 h-6 text-blue-500" />
          </div>
          <div>
            <h2 className="text-xl font-semibold text-gray-900 dark:text-gray-100">
              Download Required Models
            </h2>
            <p className="text-sm text-gray-600 dark:text-gray-400">
              RAG features require embedding and reranker models
            </p>
          </div>
        </div>

        {!isDownloading && !error && (
          <div className="space-y-4">
            <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded-lg p-4">
              <div className="flex gap-3">
                <AlertCircle className="w-5 h-5 text-blue-600 dark:text-blue-400 shrink-0 mt-0.5" />
                <div className="text-sm text-blue-800 dark:text-blue-200">
                  <p className="font-medium mb-2">
                    The following models will be downloaded:
                  </p>
                  <ul className="list-disc list-inside space-y-1 ml-2">
                    {BGE_MODELS.map((model) => (
                      <li key={model.id}>{model.name}</li>
                    ))}
                  </ul>
                  <p className="mt-3">
                    These models enable document search and retrieval
                    capabilities. The download may take several minutes
                    depending on your connection.
                  </p>
                </div>
              </div>
            </div>

            <div className="flex gap-3 justify-end">
              <button
                onClick={onClose}
                className="px-4 py-2 text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={handleDownload}
                className="px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-lg transition-colors flex items-center gap-2"
              >
                <Download className="w-4 h-4" />
                Download Models
              </button>
            </div>
          </div>
        )}

        {isDownloading && (
          <div className="space-y-4">
            <div className="space-y-3">
              {BGE_MODELS.map((model) => {
                const isCompleted = completedModels.has(model.id);
                const isCurrent = currentModel === model.name;

                return (
                  <div
                    key={model.id}
                    className="flex items-center gap-3 p-3 rounded-lg bg-gray-50 dark:bg-gray-800"
                  >
                    {isCompleted ? (
                      <CheckCircle2 className="w-5 h-5 text-green-500 shrink-0" />
                    ) : isCurrent ? (
                      <div className="w-5 h-5 border-2 border-blue-500 border-t-transparent rounded-full animate-spin shrink-0" />
                    ) : (
                      <div className="w-5 h-5 border-2 border-gray-300 dark:border-gray-600 rounded-full shrink-0" />
                    )}
                    <div className="flex-1 min-w-0">
                      <p className="text-sm font-medium text-gray-900 dark:text-gray-100">
                        {model.name}
                      </p>
                      {isCurrent && progress && (
                        <p className="text-xs text-gray-600 dark:text-gray-400 mt-1">
                          {progress.currentFile} ({progress.currentFileIndex}/
                          {progress.totalFiles}) -{" "}
                          {formatBytes(progress.downloadedBytes)}/
                          {formatBytes(progress.totalBytes)}
                        </p>
                      )}
                    </div>
                    {isCurrent && progress && (
                      <span className="text-sm font-medium text-blue-600 dark:text-blue-400">
                        {getProgressPercentage()}%
                      </span>
                    )}
                  </div>
                );
              })}
            </div>

            {progress && (
              <div className="w-full bg-gray-200 dark:bg-gray-700 rounded-full h-2">
                <div
                  className="bg-blue-600 h-2 rounded-full transition-all duration-300"
                  style={{ width: `${getProgressPercentage()}%` }}
                />
              </div>
            )}

            <p className="text-sm text-gray-600 dark:text-gray-400 text-center">
              Please wait while the models are being downloaded...
            </p>
          </div>
        )}

        {error && (
          <div className="space-y-4">
            <div className="bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg p-4">
              <div className="flex gap-3">
                <AlertCircle className="w-5 h-5 text-red-600 dark:text-red-400 shrink-0 mt-0.5" />
                <div className="text-sm text-red-800 dark:text-red-200">
                  <p className="font-medium mb-1">Download Failed</p>
                  <p>{error}</p>
                </div>
              </div>
            </div>

            <div className="flex gap-3 justify-end">
              <button
                onClick={onClose}
                className="px-4 py-2 text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg transition-colors"
              >
                Close
              </button>
              <button
                onClick={handleDownload}
                className="px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-lg transition-colors flex items-center gap-2"
              >
                <Download className="w-4 h-4" />
                Retry
              </button>
            </div>
          </div>
        )}
      </div>
    </Dialog>
  );
};
