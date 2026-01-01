import { Dialog } from "../ui";
import { Button } from "../ui";
import {
  CheckCircle,
  Trash2,
  FolderOpen,
  X,
  Play,
  Loader2,
} from "lucide-react";
import type { ModelMetadata } from "@/types/models";
import type { ModelCategory, LoadedModelsByType } from "@/store/types";
import {
  categorizeModel,
  getCategoryFullName,
  getCategoryDisplayName,
  getCategoryColor,
} from "@/lib/modelUtils";

interface DownloadedModelsDialogProps {
  isOpen: boolean;
  onClose: () => void;
  downloadedModels: Set<string>;
  onDelete: (modelId: string) => void;
  onOpenFolder: (modelId: string) => void;
  onLoadModel?: (modelId: string, modelType: ModelCategory) => void;
  loadedModelsByType?: LoadedModelsByType;
  modelMetadata?: Record<string, ModelMetadata>;
  loadingModelId?: string;
  deletingModelId?: string;
}

export const DownloadedModelsDialog = ({
  isOpen,
  onClose,
  downloadedModels,
  onDelete,
  onOpenFolder,
  onLoadModel,
  loadedModelsByType,
  loadingModelId,
  deletingModelId,
}: DownloadedModelsDialogProps) => {
  // Group models by category
  const modelsByCategory: Record<ModelCategory, string[]> = {
    text: [],
    "image-to-text": [],
    "image-gen": [],
    "speech-to-text": [],
    "text-to-speech": [],
    embedding: [],
    reranker: [],
  };

  Array.from(downloadedModels).forEach((modelId) => {
    const category = categorizeModel(modelId);
    if (category) {
      modelsByCategory[category].push(modelId);
    }
  });

  const isModelLoaded = (modelId: string, category: ModelCategory): boolean => {
    if (!loadedModelsByType) return false;
    console.log(loadedModelsByType, category, modelId);
    return loadedModelsByType[category] === modelId;
  };
  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && onClose()}>
      <div className="bg-white dark:bg-gray-800 rounded-xl max-w-2xl w-full p-6 h-[80vh] max-h-[80vh] flex flex-col">
        <div className="flex items-center justify-between mb-6">
          <div className="flex items-center gap-3">
            <div className="p-2 rounded-lg bg-green-500/10">
              <CheckCircle className="w-6 h-6 text-green-600" />
            </div>
            <div>
              <h2 className="text-xl font-semibold text-gray-900 dark:text-gray-100">
                Downloaded Models
              </h2>
              <p className="text-sm text-gray-600 dark:text-gray-400">
                {downloadedModels.size}{" "}
                {downloadedModels.size === 1 ? "model" : "models"} downloaded
              </p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="p-1 text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200 rounded transition-colors"
          >
            <X className="w-5 h-5" />
          </button>
        </div>

        <div className="flex-1 overflow-y-auto min-h-0 pr-2">
          {Array.from(downloadedModels).length === 0 ? (
            <div className="flex flex-col items-center justify-center py-12 text-center">
              <CheckCircle className="w-16 h-16 text-gray-300 dark:text-gray-600 mb-4" />
              <p className="text-gray-600 dark:text-gray-400 mb-2">
                No models downloaded yet
              </p>
              <p className="text-sm text-gray-500 dark:text-gray-500">
                Export models from the search results to get started
              </p>
            </div>
          ) : (
            <div className="space-y-6">
              {(
                Object.entries(modelsByCategory) as [ModelCategory, string[]][]
              ).map(
                ([category, models]) =>
                  models.length > 0 && (
                    <div key={category}>
                      <h3 className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-3 px-1">
                        {getCategoryFullName(category)}
                      </h3>
                      <div className="space-y-2">
                        {models.map((modelId) => {
                          const loaded = isModelLoaded(modelId, category);
                          return (
                            <div
                              key={modelId}
                              className="flex items-center justify-between p-4 rounded-lg bg-gray-50 dark:bg-gray-700/50 hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors"
                            >
                              <div className="flex-1 min-w-0 mr-4">
                                <p className="text-sm font-medium text-gray-900 dark:text-gray-100 truncate">
                                  {modelId}
                                </p>
                                <div className="flex items-center gap-2 mt-0.5">
                                  {/* <p className="text-xs text-gray-500 dark:text-gray-400">
                                    {modelId.split("/")[0]} organization
                                  </p> */}
                                  <span
                                    className={`inline-block px-2 py-0.5 text-xs rounded-full ${getCategoryColor(
                                      category
                                    )}`}
                                  >
                                    {getCategoryDisplayName(category)}
                                  </span>
                                  {loaded && (
                                    <span className="flex items-center gap-1 text-xs text-green-600 dark:text-green-400">
                                      <span className="w-1.5 h-1.5 bg-green-500 rounded-full"></span>
                                      Loaded
                                    </span>
                                  )}
                                </div>
                              </div>
                              <div className="flex items-center gap-2">
                                {onLoadModel && (
                                  <Button
                                    variant="ghost"
                                    size="sm"
                                    onClick={() =>
                                      onLoadModel(modelId, category)
                                    }
                                    disabled={
                                      loaded || loadingModelId === modelId
                                    }
                                    className="shrink-0"
                                  >
                                    {loadingModelId === modelId ? (
                                      <>
                                        <Loader2 className="w-4 h-4 mr-1 animate-spin" />
                                        Loading...
                                      </>
                                    ) : (
                                      <>
                                        <Play className="w-4 h-4 mr-1" />
                                        {loaded ? "Loaded" : "Load"}
                                      </>
                                    )}
                                  </Button>
                                )}
                                <Button
                                  variant="ghost"
                                  size="sm"
                                  onClick={() => onOpenFolder(modelId)}
                                  className="shrink-0"
                                >
                                  <FolderOpen className="w-4 h-4 mr-1" />
                                  Open
                                </Button>
                                <Button
                                  variant="ghost"
                                  size="sm"
                                  onClick={() => onDelete(modelId)}
                                  disabled={deletingModelId === modelId}
                                  className="shrink-0 text-red-600 hover:text-red-700 dark:text-red-400 dark:hover:text-red-300"
                                >
                                  {deletingModelId === modelId ? (
                                    <>
                                      <Loader2 className="w-4 h-4 mr-1 animate-spin" />
                                      Deleting...
                                    </>
                                  ) : (
                                    <>
                                      <Trash2 className="w-4 h-4 mr-1" />
                                      Delete
                                    </>
                                  )}
                                </Button>
                              </div>
                            </div>
                          );
                        })}
                      </div>
                    </div>
                  )
              )}
            </div>
          )}
        </div>

        <div className="mt-6 pt-4 border-t border-gray-200 dark:border-gray-700">
          <Button onClick={onClose} variant="outline" className="w-full">
            Close
          </Button>
        </div>
      </div>
    </Dialog>
  );
};
