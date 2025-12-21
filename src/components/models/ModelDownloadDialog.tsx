import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Dialog, Button, Input } from "../ui";
import { Download, Settings, X } from "lucide-react";
import { GraphGenerationParams, ModelTaskType } from "@/types/models";

interface ModelDownloadDialogProps {
  isOpen: boolean;
  onClose: () => void;
  modelId: string;
  onSuccess: () => void;
  setModelDownloading: (modelId: string, isDownloading: boolean) => void;
}

const TASK_TYPES: { value: ModelTaskType; label: string }[] = [
  { value: "text_generation", label: "Text Generation" },
  { value: "embeddings_ov", label: "Embeddings" },
  { value: "rerank_ov", label: "Reranking" },
  { value: "text2speech", label: "Text-to-Speech" },
  { value: "speech2text", label: "Speech-to-Text" },
  { value: "image_generation", label: "Image Generation" },
];

export const ModelDownloadDialog = ({
  isOpen,
  onClose,
  modelId,
  onSuccess,
  setModelDownloading,
}: ModelDownloadDialogProps) => {
  const [isDownloading, setIsDownloading] = useState(false);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [error, setError] = useState<string>("");
  const [taskType, setTaskType] = useState<ModelTaskType>("text_generation");

  // Graph generation parameters
  const [graphParams, setGraphParams] = useState<GraphGenerationParams>({
    task_type: "text_generation",
    target_device: "GPU",
    num_streams: 1,
    // Text generation defaults
    enable_prefix_caching: false,
    cache_size: 10,
    max_num_seqs: 256,
    // Embeddings defaults
    normalize: true,
  });

  const handleTaskTypeChange = (newType: ModelTaskType) => {
    setTaskType(newType);
    // Reset params with new task type
    const baseParams: GraphGenerationParams = {
      task_type: newType,
      target_device: graphParams.target_device || "GPU",
      num_streams: graphParams.num_streams || 1,
    };

    // Add task-specific defaults
    if (newType === "text_generation") {
      baseParams.enable_prefix_caching = false;
      baseParams.cache_size = 10;
      baseParams.max_num_seqs = 256;
    } else if (newType === "embeddings_ov") {
      baseParams.normalize = true;
    } else if (newType === "image_generation") {
      baseParams.default_resolution = "512x512";
      baseParams.max_resolution = "1024x1024";
    }

    setGraphParams(baseParams);
  };

  const handleDownload = async () => {
    setIsDownloading(true);
    setError("");

    // Mark model as downloading and close dialog immediately
    setModelDownloading(modelId, true);
    onClose();

    // Start download in background
    try {
      await invoke("download_entire_model", {
        modelId,
        downloadPath: null,
        graphParams: showAdvanced
          ? graphParams
          : { task_type: taskType, target_device: "GPU" },
      });

      onSuccess();
    } catch (err) {
      // If download fails, mark as no longer downloading
      setModelDownloading(modelId, false);
      console.error("Download failed:", err);
      // You could also emit a notification here if needed
    } finally {
      setIsDownloading(false);
    }
  };

  const updateParam = <K extends keyof GraphGenerationParams>(
    key: K,
    value: GraphGenerationParams[K]
  ) => {
    setGraphParams((prev) => ({ ...prev, [key]: value }));
  };

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <div className="p-6 bg-white dark:bg-gray-800 rounded-xl max-w-2xl w-full max-h-[90vh] overflow-y-auto">
        {/* Header */}
        <div className="flex items-start justify-between mb-6">
          <div className="flex items-center gap-3">
            <div className="p-2 bg-blue-100 dark:bg-blue-900/30 rounded-lg">
              <Download className="w-5 h-5 text-blue-600 dark:text-blue-400" />
            </div>
            <div>
              <h2 className="text-xl font-semibold text-gray-900 dark:text-white">
                Download Model
              </h2>
              <p className="text-sm text-gray-500 dark:text-gray-400 mt-1">
                {modelId}
              </p>
            </div>
          </div>
          <button
            onClick={onClose}
            disabled={isDownloading}
            className="text-gray-400 hover:text-gray-500 dark:hover:text-gray-300 disabled:opacity-50"
          >
            <X className="w-5 h-5" />
          </button>
        </div>

        {/* Task Type Selection */}
        <div className="mb-4">
          <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
            Model Task Type
          </label>
          <select
            value={taskType}
            onChange={(e) =>
              handleTaskTypeChange(e.target.value as ModelTaskType)
            }
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-800 text-gray-900 dark:text-white"
            disabled={isDownloading}
          >
            {TASK_TYPES.map((type) => (
              <option key={type.value} value={type.value}>
                {type.label}
              </option>
            ))}
          </select>
          <p className="text-xs text-gray-500 dark:text-gray-400 mt-1">
            Select the type that matches your model's purpose
          </p>
        </div>

        {/* Advanced Settings Toggle */}
        <div className="mb-4">
          <button
            onClick={() => setShowAdvanced(!showAdvanced)}
            className="flex items-center gap-2 text-sm text-blue-600 dark:text-blue-400 hover:underline"
          >
            <Settings className="w-4 h-4" />
            {showAdvanced ? "Hide" : "Show"} Advanced Graph Settings
          </button>
        </div>

        {/* Advanced Settings */}
        {showAdvanced && (
          <div className="mb-6 p-4 bg-gray-50 dark:bg-gray-900/50 rounded-lg space-y-4 max-h-96 overflow-y-auto">
            <h3 className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-3">
              Graph Generation Parameters
            </h3>

            {/* Common Parameters for all types */}
            <div className="grid grid-cols-2 gap-4">
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                  Target Device
                </label>
                <select
                  value={graphParams.target_device}
                  onChange={(e) => updateParam("target_device", e.target.value)}
                  className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-800 text-gray-900 dark:text-white text-sm"
                >
                  <option value="GPU">GPU</option>
                  <option value="CPU">CPU</option>
                  <option value="NPU">NPU</option>
                  <option value="AUTO">AUTO</option>
                </select>
              </div>

              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                  Number of Streams
                </label>
                <Input
                  type="number"
                  value={graphParams.num_streams}
                  onChange={(e) =>
                    updateParam("num_streams", parseInt(e.target.value) || 1)
                  }
                  min="1"
                  max="16"
                  className="text-sm"
                />
              </div>
            </div>

            {/* Text Generation Specific Parameters */}
            {taskType === "text_generation" && (
              <div className="border-t border-gray-200 dark:border-gray-700 pt-4">
                <h4 className="text-xs font-semibold text-gray-600 dark:text-gray-400 mb-3">
                  Text Generation Settings
                </h4>
                <div className="grid grid-cols-2 gap-4">
                  <div className="flex items-center">
                    <input
                      type="checkbox"
                      checked={graphParams.enable_prefix_caching || false}
                      onChange={(e) =>
                        updateParam("enable_prefix_caching", e.target.checked)
                      }
                      className="mr-2"
                    />
                    <label className="text-sm text-gray-700 dark:text-gray-300">
                      Enable Prefix Caching
                    </label>
                  </div>

                  <div>
                    <label className="block text-sm text-gray-700 dark:text-gray-300 mb-1">
                      Cache Size
                    </label>
                    <Input
                      type="number"
                      value={graphParams.cache_size || 10}
                      onChange={(e) =>
                        updateParam(
                          "cache_size",
                          parseInt(e.target.value) || 10
                        )
                      }
                      min="1"
                      className="text-sm"
                    />
                  </div>

                  <div>
                    <label className="block text-sm text-gray-700 dark:text-gray-300 mb-1">
                      Max Sequences
                    </label>
                    <Input
                      type="number"
                      value={graphParams.max_num_seqs || 256}
                      onChange={(e) =>
                        updateParam(
                          "max_num_seqs",
                          parseInt(e.target.value) || 256
                        )
                      }
                      min="1"
                      className="text-sm"
                    />
                  </div>

                  <div>
                    <label className="block text-sm text-gray-700 dark:text-gray-300 mb-1">
                      KV Cache Precision
                    </label>
                    <select
                      value={graphParams.kv_cache_precision || ""}
                      onChange={(e) =>
                        updateParam(
                          "kv_cache_precision",
                          e.target.value || undefined
                        )
                      }
                      className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-800 text-gray-900 dark:text-white text-sm"
                    >
                      <option value="">Default</option>
                      <option value="u8">u8</option>
                      <option value="f16">f16</option>
                      <option value="f32">f32</option>
                    </select>
                  </div>

                  <div>
                    <label className="block text-sm text-gray-700 dark:text-gray-300 mb-1">
                      Max Batched Tokens
                    </label>
                    <Input
                      type="number"
                      value={graphParams.max_num_batched_tokens || ""}
                      onChange={(e) =>
                        updateParam(
                          "max_num_batched_tokens",
                          parseInt(e.target.value) || undefined
                        )
                      }
                      placeholder="Optional"
                      className="text-sm"
                    />
                  </div>

                  <div className="flex items-center">
                    <input
                      type="checkbox"
                      checked={graphParams.dynamic_split_fuse ?? true}
                      onChange={(e) =>
                        updateParam("dynamic_split_fuse", e.target.checked)
                      }
                      className="mr-2"
                    />
                    <label className="text-sm text-gray-700 dark:text-gray-300">
                      Dynamic Split Fuse
                    </label>
                  </div>
                </div>
              </div>
            )}

            {/* Embeddings Specific Parameters */}
            {taskType === "embeddings_ov" && (
              <div className="border-t border-gray-200 dark:border-gray-700 pt-4">
                <h4 className="text-xs font-semibold text-gray-600 dark:text-gray-400 mb-3">
                  Embeddings Settings
                </h4>
                <div className="flex items-center gap-4">
                  <div className="flex items-center">
                    <input
                      type="checkbox"
                      checked={graphParams.normalize ?? true}
                      onChange={(e) =>
                        updateParam("normalize", e.target.checked)
                      }
                      className="mr-2"
                    />
                    <label className="text-sm text-gray-700 dark:text-gray-300">
                      Normalize Embeddings
                    </label>
                  </div>

                  <div className="flex items-center">
                    <input
                      type="checkbox"
                      checked={graphParams.truncate || false}
                      onChange={(e) =>
                        updateParam("truncate", e.target.checked)
                      }
                      className="mr-2"
                    />
                    <label className="text-sm text-gray-700 dark:text-gray-300">
                      Truncate Input
                    </label>
                  </div>
                </div>
              </div>
            )}

            {/* Image Generation Specific Parameters */}
            {taskType === "image_generation" && (
              <div className="border-t border-gray-200 dark:border-gray-700 pt-4">
                <h4 className="text-xs font-semibold text-gray-600 dark:text-gray-400 mb-3">
                  Image Generation Settings
                </h4>
                <div className="grid grid-cols-2 gap-4">
                  <div>
                    <label className="block text-sm text-gray-700 dark:text-gray-300 mb-1">
                      Default Resolution
                    </label>
                    <Input
                      type="text"
                      value={graphParams.default_resolution || ""}
                      onChange={(e) =>
                        updateParam(
                          "default_resolution",
                          e.target.value || undefined
                        )
                      }
                      placeholder="512x512"
                      className="text-sm"
                    />
                  </div>

                  <div>
                    <label className="block text-sm text-gray-700 dark:text-gray-300 mb-1">
                      Max Resolution
                    </label>
                    <Input
                      type="text"
                      value={graphParams.max_resolution || ""}
                      onChange={(e) =>
                        updateParam(
                          "max_resolution",
                          e.target.value || undefined
                        )
                      }
                      placeholder="1024x1024"
                      className="text-sm"
                    />
                  </div>

                  <div>
                    <label className="block text-sm text-gray-700 dark:text-gray-300 mb-1">
                      Guidance Scale
                    </label>
                    <Input
                      type="number"
                      step="0.1"
                      value={graphParams.guidance_scale || ""}
                      onChange={(e) =>
                        updateParam(
                          "guidance_scale",
                          e.target.value || undefined
                        )
                      }
                      placeholder="7.5"
                      className="text-sm"
                    />
                  </div>

                  <div>
                    <label className="block text-sm text-gray-700 dark:text-gray-300 mb-1">
                      Max Inference Steps
                    </label>
                    <Input
                      type="number"
                      value={graphParams.max_num_inference_steps || ""}
                      onChange={(e) =>
                        updateParam(
                          "max_num_inference_steps",
                          parseInt(e.target.value) || undefined
                        )
                      }
                      placeholder="50"
                      className="text-sm"
                    />
                  </div>
                </div>
              </div>
            )}

            {/* Rerank and Speech types use only common parameters */}
            {(taskType === "rerank_ov" ||
              taskType === "text2speech" ||
              taskType === "speech2text") && (
              <div className="border-t border-gray-200 dark:border-gray-700 pt-4">
                <p className="text-xs text-gray-500 dark:text-gray-400">
                  This model type uses the common parameters above (Target
                  Device and Number of Streams).
                </p>
              </div>
            )}
          </div>
        )}

        {/* Error Message */}
        {error && (
          <div className="mb-4 p-3 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg">
            <p className="text-sm text-red-600 dark:text-red-400">{error}</p>
          </div>
        )}

        {/* Actions */}
        <div className="flex justify-end gap-3">
          <Button variant="outline" onClick={onClose} disabled={isDownloading}>
            Cancel
          </Button>
          <Button onClick={handleDownload} disabled={isDownloading}>
            {isDownloading ? (
              <>
                <Download className="w-4 h-4 mr-2 animate-spin" />
                Downloading...
              </>
            ) : (
              <>
                <Download className="w-4 h-4 mr-2" />
                Download
              </>
            )}
          </Button>
        </div>
      </div>
    </Dialog>
  );
};
