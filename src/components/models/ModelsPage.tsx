import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { logUserAction, logError, logDebug } from "@/lib/logger";
import { PageContainer } from "../layout";
import { Card, Button, Input } from "../ui";
import { useAppStore } from "@/store";
import type { ModelCategory } from "@/store/types";
import { useDebounce } from "@/hooks";
import {
  categorizeModel,
  getCategoryDisplayName,
  getCategoryColor,
} from "@/lib/modelUtils";
import { ModelDownloadDialog } from "./ModelDownloadDialog";
import { DownloadedModelsDialog } from "./DownloadedModelsDialog";
import {
  ModelInfo,
  SearchResult,
  DownloadProgress,
  ModelMetadata,
} from "@/types/models";
import {
  Search,
  Download,
  Trash2,
  HardDrive,
  Calendar,
  Heart,
  TrendingUp,
  CheckCircle,
  Loader2,
  Package,
} from "lucide-react";

const RECOMMENDED_MODELS = [
  "OpenVINO/Qwen3-8B-int4-ov",
  "OpenVINO/Qwen3-4B-int8-ov",
  "OpenVINO/Mistral-7B-Instruct-v0.3-int4-ov",
  "OpenVINO/Phi-4-mini-instruct-int4-ov",
  "OpenVINO/Qwen2.5-VL-7B-Instruct-int4-ov",
];

export const ModelsPage = () => {
  const {
    searchQuery,
    setSearchQuery,
    isSearching,
    setIsSearching,
    isModelDownloading,
    hasAnyDownloading,
    setModelDownloading,
    getDownloadProgress,
    setDownloadProgress,
    downloadedModels,
    addDownloadedModel,
    removeDownloadedModel,
    isModelDownloaded,
    setDownloadedModels,
    loadedModel,
    setLoadedModel,
    loadedModelsByType,
    setLoadedModelByType,
  } = useAppStore();
  const [searchResults, setSearchResults] = useState<string[]>([]);
  const [selectedModel, setSelectedModel] = useState<ModelInfo | null>(null);
  const [, setLoadingModelInfo] = useState<string | null>(null);
  const [modelInfoCache, setModelInfoCache] = useState<Map<string, ModelInfo>>(
    new Map()
  );
  const [modelMetadata, setModelMetadata] = useState<
    Record<string, ModelMetadata>
  >({});
  const [currentLimit, setCurrentLimit] = useState(10);
  const [hasMoreResults, setHasMoreResults] = useState(false);
  const [downloadDialogOpen, setDownloadDialogOpen] = useState(false);
  const [selectedModelForDownload, setSelectedModelForDownload] =
    useState<string>("");
  const [downloadedModelsDialogOpen, setDownloadedModelsDialogOpen] =
    useState(false);
  const debouncedSearch = useDebounce(searchQuery, 500);

  // Load downloaded models on mount
  useEffect(() => {
    loadDownloadedModels();
    loadModelMetadata();
  }, []);

  const loadDownloadedModels = async () => {
    try {
      // Use check_downloaded_models which now reads from metadata
      const models = await invoke<string[]>("check_downloaded_models", {
        downloadPath: null,
      });

      setDownloadedModels(models);
      logDebug("Loaded downloaded models from metadata", {
        count: models.length,
      });
    } catch (error) {
      logError("Failed to load downloaded models", error as Error);
    }
  };

  const loadModelMetadata = async () => {
    try {
      const metadata = await invoke<Record<string, ModelMetadata>>(
        "get_all_model_metadata"
      );
      setModelMetadata(metadata);
      logDebug("Loaded model metadata", {
        count: Object.keys(metadata).length,
      });
    } catch (error) {
      logError("Failed to load model metadata", error as Error);
    }
  };

  // Search models when debounced search changes
  useEffect(() => {
    if (debouncedSearch.trim()) {
      setCurrentLimit(10); // Reset to initial limit
      handleSearch(10);
    } else {
      setSearchResults([]);
      setHasMoreResults(false);
    }
  }, [debouncedSearch]);

  // Listen for download progress
  useEffect(() => {
    const unlistenDownload = listen<DownloadProgress>(
      "download-progress",
      (event) => {
        const { modelId, progress, currentFile } = event.payload;
        console.log("Download progress:", { modelId, progress, currentFile });
        setDownloadProgress(modelId, progress, currentFile || "");

        // Check if download is complete (100%)
        if (progress >= 100) {
          // Mark model as no longer downloading
          setModelDownloading(modelId, false);
          // Reload downloaded models list after a short delay to ensure file system is updated
          setTimeout(() => {
            loadDownloadedModels();
          }, 500);
        }
      }
    );

    return () => {
      unlistenDownload.then((fn) => fn());
    };
  }, [setDownloadProgress, setModelDownloading]);

  const handleSearch = async (limit?: number) => {
    setIsSearching(true);
    const searchLimit = limit || currentLimit;
    console.log("Searching with limit:", searchLimit);
    try {
      const result = await invoke<SearchResult>("search_models", {
        query: debouncedSearch,
        limit: searchLimit,
      });
      console.log("Search results:", result.model_ids.length, "models");
      console.log(
        "Has more?",
        result.model_ids.length === searchLimit && searchLimit < 100
      );
      setSearchResults(result.model_ids);
      // Show load more if we got exactly what we asked for (likely more available)
      // or if search limit is less than 100 and we have results
      setHasMoreResults(
        result.model_ids.length === searchLimit && searchLimit < 100
      );
    } catch (error) {
      console.error("Search failed:", error);
      setSearchResults([]);
      setHasMoreResults(false);
    } finally {
      setIsSearching(false);
    }
  };

  const handleLoadMore = () => {
    const newLimit = currentLimit + 10;
    setCurrentLimit(newLimit);
    handleSearch(newLimit);
  };

  const handleModelClick = async (modelId: string) => {
    // Check cache first
    const cached = modelInfoCache.get(modelId);
    if (cached) {
      setSelectedModel(cached);
      return;
    }

    // Fetch model info
    setLoadingModelInfo(modelId);
    try {
      const modelInfo = await invoke<ModelInfo>("get_model_info", {
        modelId,
      });
      setModelInfoCache((prev) => new Map(prev).set(modelId, modelInfo));
      setSelectedModel(modelInfo);
    } catch (error) {
      console.error("Failed to fetch model info:", error);
    } finally {
      setLoadingModelInfo(null);
    }
  };

  const handleDownload = (modelId: string) => {
    setSelectedModelForDownload(modelId);
    setDownloadDialogOpen(true);
  };

  const handleDownloadSuccess = async () => {
    addDownloadedModel(selectedModelForDownload);
    setModelDownloading(selectedModelForDownload, false);
    await loadDownloadedModels();
    await loadModelMetadata(); // Reload metadata to get the new model's info
  };

  const handleDelete = async (modelId: string) => {
    try {
      const homeDir = await invoke<string>("get_home_dir");
      const modelPath = `${homeDir}\\.sparrow\\models\\${modelId}`;

      await invoke("delete_directory", { path: modelPath });
      removeDownloadedModel(modelId);
      await loadDownloadedModels();
    } catch (error) {
      console.error("Delete failed:", error);
    }
  };

  const handleOpenModelFolder = async (modelId: string) => {
    try {
      await invoke("open_model_folder", { modelId });
      logUserAction("Open model folder", { modelId });
    } catch (error) {
      logError("Failed to open model folder", error as Error, { modelId });
    }
  };

  const handleLoadModel = async (modelId: string, modelType: ModelCategory) => {
    try {
      await invoke("load_model", { modelId });
      setLoadedModelByType(modelType, modelId);

      // Also set as the primary loaded model if it's a text model
      if (modelType === "text") {
        setLoadedModel(modelId);
      }
    } catch (error) {
      console.error("Failed to load model:", error);
    }
  };

  const formatNumber = (num: number | null) => {
    if (!num) return "0";
    if (num >= 1000000) return `${(num / 1000000).toFixed(1)}M`;
    if (num >= 1000) return `${(num / 1000).toFixed(1)}K`;
    return num.toString();
  };

  const formatDate = (dateStr: string | null) => {
    if (!dateStr) return "N/A";
    try {
      const date = new Date(dateStr);
      return date.toLocaleDateString();
    } catch {
      return "N/A";
    }
  };

  return (
    <PageContainer
      title="Models"
      description="Browse and manage AI models"
      actions={
        <Button
          variant="outline"
          onClick={() => setDownloadedModelsDialogOpen(true)}
          className="gap-2"
        >
          <CheckCircle className="w-4 h-4" />
          Downloaded Models ({downloadedModels.size})
        </Button>
      }
    >
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6 h-full">
        {/* Search & Results Column */}
        <div className="lg:col-span-2 flex flex-col gap-4 min-h-0">
          {/* Search Input */}
          <Card className="p-4">
            <div className="relative">
              <Search className="absolute left-3 top-1/2 transform -translate-y-1/2 w-5 h-5 text-gray-400" />
              <Input
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder={
                  hasAnyDownloading()
                    ? "Download in progress..."
                    : "Search OpenVINO models..."
                }
                className="pl-10"
                disabled={hasAnyDownloading()}
              />
              {isSearching && (
                <Loader2 className="absolute right-3 top-1/2 transform -translate-y-1/2 w-5 h-5 text-primary animate-spin" />
              )}
            </div>
          </Card>

          {/* Search Results */}
          <div className="flex-1 overflow-y-auto space-y-3 min-h-0 px-2 py-1">
            {searchResults.length === 0 &&
              !isSearching &&
              debouncedSearch.trim() && (
                <Card className="p-8 text-center">
                  <Package className="w-12 h-12 mx-auto mb-3 text-gray-400" />
                  <p className="text-gray-600 dark:text-gray-400">
                    No models found for "{debouncedSearch}"
                  </p>
                </Card>
              )}

            {searchResults.length === 0 && !debouncedSearch.trim() && (
              <>
                <div className="mb-8">
                  <h3 className="text-lg font-semibold mb-2">
                    Recommended Models
                  </h3>
                  <p className="text-sm text-gray-600 dark:text-gray-400">
                    Popular OpenVINO models to get started
                  </p>
                </div>
                {RECOMMENDED_MODELS.map((modelId) => {
                  const downloading = isModelDownloading(modelId);
                  const downloaded = isModelDownloaded(modelId);
                  const progressData = getDownloadProgress(modelId);
                  const progress = progressData.progress;
                  const currentFile = progressData.currentFile;
                  const modelInfo = modelInfoCache.get(modelId);

                  const isSelected = selectedModel?.id === modelId;
                  const category = categorizeModel(modelId);

                  return (
                    <Card
                      key={modelId}
                      className={`p-3 cursor-pointer transition-all hover:shadow-md ${
                        isSelected ? "ring-1 ring-primary" : ""
                      }`}
                      onClick={() => handleModelClick(modelId)}
                    >
                      <div className="flex items-start justify-between gap-2">
                        <div className="flex-1 min-w-0">
                          <div className="flex items-center gap-2">
                            <h3 className="font-semibold text-base truncate">
                              {modelId}
                            </h3>
                            {downloaded && (
                              <CheckCircle className="w-4 h-4 text-green-600 dark:text-green-400 shrink-0" />
                            )}
                          </div>
                          <div className="flex items-center gap-1.5 mt-1">
                            {category && (
                              <span
                                className={`inline-block px-2 py-0.5 text-xs rounded-full ${getCategoryColor(
                                  category
                                )}`}
                              >
                                {getCategoryDisplayName(category)}
                              </span>
                            )}
                            {modelInfo && modelInfo.pipeline_tag && (
                              <span className="inline-block px-2 py-0.5 text-xs rounded-full bg-gray-100 dark:bg-gray-800 text-gray-700 dark:text-gray-300">
                                {modelInfo.pipeline_tag}
                              </span>
                            )}
                          </div>
                        </div>

                        <div className="flex flex-col gap-1">
                          {downloaded ? (
                            <Button
                              variant="primary"
                              size="sm"
                              onClick={(e) => {
                                e.stopPropagation();
                                handleDelete(modelId);
                              }}
                              className="bg-red-600 hover:bg-red-700 dark:bg-red-600 dark:hover:bg-red-700 w-28"
                            >
                              <Trash2 className="w-3 h-3 mr-1" />
                              Delete
                            </Button>
                          ) : downloading ? (
                            <div className="space-y-2 min-w-[120px]">
                              <div className="flex items-center gap-1 text-xs text-primary">
                                <Loader2 className="w-3 h-3 animate-spin" />
                                <span>{progress}%</span>
                              </div>
                              <div className="w-full h-1.5 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden">
                                <div
                                  className="h-full bg-accent-600 dark:bg-accent-500 transition-all duration-300"
                                  style={{ width: `${progress}%` }}
                                />
                              </div>
                              {currentFile && (
                                <p
                                  className="text-xs text-gray-500 dark:text-gray-400 truncate max-w-[120px]"
                                  title={currentFile}
                                >
                                  {currentFile.split("/").pop()}
                                </p>
                              )}
                            </div>
                          ) : (
                            <Button
                              variant="primary"
                              size="sm"
                              onClick={(e) => {
                                e.stopPropagation();
                                handleDownload(modelId);
                              }}
                              disabled={hasAnyDownloading()}
                              className="w-28"
                            >
                              <Download className="w-3 h-3 mr-1" />
                              Download
                            </Button>
                          )}
                        </div>
                      </div>
                    </Card>
                  );
                })}
              </>
            )}

            {searchResults.map((modelId) => {
              const downloading = isModelDownloading(modelId);
              const downloaded = isModelDownloaded(modelId);
              const progressData = getDownloadProgress(modelId);
              const progress = progressData.progress;
              const currentFile = progressData.currentFile;
              const modelInfo = modelInfoCache.get(modelId);

              const isSelected = selectedModel?.id === modelId;
              const category = categorizeModel(modelId);

              return (
                <Card
                  key={modelId}
                  className={`p-3 cursor-pointer transition-all hover:shadow-md ${
                    isSelected ? "ring-1 ring-primary" : ""
                  }`}
                  onClick={() => handleModelClick(modelId)}
                >
                  <div className="flex items-start justify-between gap-2">
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <h3 className="font-semibold text-base truncate">
                          {modelId}
                        </h3>
                        {downloaded && (
                          <CheckCircle className="w-4 h-4 text-green-600 dark:text-green-400 shrink-0" />
                        )}
                      </div>
                      <div className="flex items-center gap-1.5 mt-1">
                        {category && (
                          <span
                            className={`inline-block px-2 py-0.5 text-xs rounded-full ${getCategoryColor(
                              category
                            )}`}
                          >
                            {getCategoryDisplayName(category)}
                          </span>
                        )}
                        {modelInfo && modelInfo.pipeline_tag && (
                          <span className="inline-block px-2 py-0.5 text-xs rounded-full bg-gray-100 dark:bg-gray-800 text-gray-700 dark:text-gray-300">
                            {modelInfo.pipeline_tag}
                          </span>
                        )}
                      </div>
                    </div>

                    <div className="flex flex-col gap-1">
                      {downloaded ? (
                        <Button
                          variant="primary"
                          size="sm"
                          onClick={(e) => {
                            e.stopPropagation();
                            handleDelete(modelId);
                          }}
                          className="bg-red-600 hover:bg-red-700 dark:bg-red-600 dark:hover:bg-red-700 w-28"
                        >
                          <Trash2 className="w-3 h-3 mr-1" />
                          Delete
                        </Button>
                      ) : downloading ? (
                        <div className="space-y-1 min-w-[120px]">
                          <div className="flex items-center gap-1 text-xs text-primary">
                            <Loader2 className="w-3 h-3 animate-spin" />
                            <span>{progress}%</span>
                          </div>
                          <div className="w-full h-1.5 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden">
                            <div
                              className="h-full bg-accent-600 dark:bg-accent-500 transition-all duration-300"
                              style={{ width: `${progress}%` }}
                            />
                          </div>
                          {currentFile && (
                            <p
                              className="text-xs text-gray-500 dark:text-gray-400 truncate max-w-[120px]"
                              title={currentFile}
                            >
                              {currentFile.split("/").pop()}
                            </p>
                          )}
                        </div>
                      ) : (
                        <Button
                          variant="primary"
                          size="sm"
                          onClick={(e) => {
                            e.stopPropagation();
                            handleDownload(modelId);
                          }}
                          disabled={hasAnyDownloading()}
                          className="w-28"
                        >
                          <Download className="w-3 h-3 mr-1" />
                          Download
                        </Button>
                      )}
                    </div>
                  </div>
                </Card>
              );
            })}

            {/* Results count */}
            {searchResults.length > 0 && (
              <div className="text-center text-sm text-gray-500 dark:text-gray-400 py-2">
                Showing {searchResults.length} model
                {searchResults.length !== 1 ? "s" : ""}
              </div>
            )}

            {/* Load More Button */}
            {searchResults.length > 0 && hasMoreResults && !isSearching && (
              <div className="flex justify-center pt-2">
                <Button
                  variant="outline"
                  onClick={handleLoadMore}
                  disabled={isSearching}
                  className="w-full"
                >
                  {isSearching ? (
                    <>
                      <Loader2 className="w-4 h-4 mr-2 animate-spin" />
                      Loading...
                    </>
                  ) : (
                    <>Load More Models</>
                  )}
                </Button>
              </div>
            )}
          </div>
        </div>

        {/* Details Column */}
        <div className="flex flex-col gap-4">
          <Card className="p-6 flex-1">
            {selectedModel ? (
              <div className="space-y-4">
                <div>
                  <h2 className="text-xl font-bold mb-2">{selectedModel.id}</h2>
                  {selectedModel.author && (
                    <p className="text-sm text-gray-600 dark:text-gray-400">
                      by {selectedModel.author}
                    </p>
                  )}
                </div>

                {selectedModel.pipeline_tag && (
                  <div>
                    <h3 className="text-sm font-semibold mb-1">Pipeline</h3>
                    <span className="inline-block px-3 py-1 text-sm rounded-full bg-primary/10 text-primary">
                      {selectedModel.pipeline_tag}
                    </span>
                  </div>
                )}

                <div className="grid grid-cols-2 gap-4">
                  <div>
                    <h3 className="text-sm font-semibold mb-1 flex items-center gap-2">
                      <TrendingUp className="w-4 h-4" />
                      Downloads
                    </h3>
                    <p className="text-lg">
                      {formatNumber(selectedModel.downloads)}
                    </p>
                  </div>
                  <div>
                    <h3 className="text-sm font-semibold mb-1 flex items-center gap-2">
                      <Heart className="w-4 h-4" />
                      Likes
                    </h3>
                    <p className="text-lg">
                      {formatNumber(selectedModel.likes)}
                    </p>
                  </div>
                </div>

                <div>
                  <h3 className="text-sm font-semibold mb-1 flex items-center gap-2">
                    <Calendar className="w-4 h-4" />
                    Created
                  </h3>
                  <p className="text-sm text-gray-600 dark:text-gray-400">
                    {formatDate(selectedModel.created_at)}
                  </p>
                </div>

                <div>
                  <h3 className="text-sm font-semibold mb-1 flex items-center gap-2">
                    <Calendar className="w-4 h-4" />
                    Last Modified
                  </h3>
                  <p className="text-sm text-gray-600 dark:text-gray-400">
                    {formatDate(selectedModel.last_modified)}
                  </p>
                </div>

                {selectedModel.tags && selectedModel.tags.length > 0 && (
                  <div>
                    <h3 className="text-sm font-semibold mb-2">Tags</h3>
                    <div className="flex flex-wrap gap-2">
                      {selectedModel.tags.slice(0, 10).map((tag) => (
                        <span
                          key={tag}
                          className="px-2 py-1 text-xs rounded bg-gray-100 dark:bg-gray-800 text-gray-700 dark:text-gray-300"
                        >
                          {tag}
                        </span>
                      ))}
                    </div>
                  </div>
                )}

                {selectedModel.collections &&
                  selectedModel.collections.length > 0 && (
                    <div>
                      <h3 className="text-sm font-semibold mb-2">
                        Collections
                      </h3>
                      <div className="flex flex-wrap gap-2">
                        {selectedModel.collections.map((collection) => (
                          <span
                            key={collection}
                            className="px-2 py-1 text-xs rounded bg-blue-100 dark:bg-blue-900 text-blue-700 dark:text-blue-300"
                          >
                            {collection}
                          </span>
                        ))}
                      </div>
                    </div>
                  )}

                {selectedModel.siblings &&
                  selectedModel.siblings.length > 0 && (
                    <div>
                      <h3 className="text-sm font-semibold mb-2">
                        Files ({selectedModel.siblings.length})
                      </h3>
                      <div className="max-h-48 overflow-y-auto space-y-1 text-xs">
                        {selectedModel.siblings.map((sibling) => (
                          <div
                            key={sibling.rfilename}
                            className="px-2 py-1 rounded bg-gray-50 dark:bg-gray-800 text-gray-700 dark:text-gray-300 font-mono"
                          >
                            {sibling.rfilename}
                          </div>
                        ))}
                      </div>
                    </div>
                  )}
              </div>
            ) : (
              <div className="flex flex-col items-center justify-center h-full text-center">
                <HardDrive className="w-16 h-16 mb-4 text-gray-400" />
                <p className="text-gray-600 dark:text-gray-400">
                  Select a model to view details
                </p>
              </div>
            )}
          </Card>
        </div>
      </div>

      <ModelDownloadDialog
        isOpen={downloadDialogOpen}
        onClose={() => setDownloadDialogOpen(false)}
        modelId={selectedModelForDownload}
        onSuccess={handleDownloadSuccess}
        setModelDownloading={setModelDownloading}
      />

      <DownloadedModelsDialog
        isOpen={downloadedModelsDialogOpen}
        onClose={() => setDownloadedModelsDialogOpen(false)}
        downloadedModels={downloadedModels}
        onDelete={handleDelete}
        onOpenFolder={handleOpenModelFolder}
        onLoadModel={handleLoadModel}
        loadedModel={loadedModel}
        loadedModelsByType={loadedModelsByType}
        modelMetadata={modelMetadata}
      />
    </PageContainer>
  );
};
