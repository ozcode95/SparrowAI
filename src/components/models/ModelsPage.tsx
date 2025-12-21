import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { PageContainer } from "../layout";
import { Card, Button, Input } from "../ui";
import { useAppStore } from "@/store";
import { useDebounce } from "@/hooks";
import { ModelDownloadDialog } from "./ModelDownloadDialog";
import { DownloadedModelsDialog } from "./DownloadedModelsDialog";
import { ModelInfo, SearchResult, DownloadProgress } from "@/types/models";
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
  } = useAppStore();
  const [searchResults, setSearchResults] = useState<string[]>([]);
  const [selectedModel, setSelectedModel] = useState<ModelInfo | null>(null);
  const [loadingModelInfo, setLoadingModelInfo] = useState<string | null>(null);
  const [modelInfoCache, setModelInfoCache] = useState<Map<string, ModelInfo>>(
    new Map()
  );
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
  }, []);

  const loadDownloadedModels = async () => {
    try {
      // Get the models directory path
      const homeDir = await invoke<string>("get_home_dir");
      const modelsPath = `${homeDir}\\.sparrow\\models`;

      // Check if directory exists and list models
      const models = await invoke<string[]>("list_directory_names", {
        path: modelsPath,
      }).catch(() => []);

      setDownloadedModels(models);
    } catch (error) {
      console.error("Failed to load downloaded models:", error);
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
        const { modelId, progress } = event.payload;
        setDownloadProgress(modelId, progress);

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
      alert(`Failed to fetch model info: ${error}`);
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
  };

  const handleDelete = async (modelId: string) => {
    if (!confirm(`Are you sure you want to delete ${modelId}?`)) {
      return;
    }

    try {
      const homeDir = await invoke<string>("get_home_dir");
      const modelPath = `${homeDir}\\.sparrow\\models\\${modelId}`;

      await invoke("delete_directory", { path: modelPath });
      removeDownloadedModel(modelId);
      await loadDownloadedModels();
    } catch (error) {
      console.error("Delete failed:", error);
      alert(`Failed to delete model: ${error}`);
    }
  };

  const handleOpenModelFolder = async (modelId: string) => {
    try {
      await invoke("open_model_folder", { modelId });
    } catch (error) {
      console.error("Failed to open model folder:", error);
    }
  };

  const handleLoadModel = async (modelId: string) => {
    try {
      await invoke("load_model", { modelId });
      setLoadedModel(modelId);
    } catch (error) {
      console.error("Failed to load model:", error);
      alert(`Failed to load model: ${error}`);
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
          <div className="flex-1 overflow-y-auto space-y-3 min-h-0">
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
              <Card className="p-8 text-center">
                <Search className="w-12 h-12 mx-auto mb-3 text-gray-400" />
                <p className="text-gray-600 dark:text-gray-400">
                  Search for OpenVINO models to get started
                </p>
              </Card>
            )}

            {searchResults.map((modelId) => {
              const downloading = isModelDownloading(modelId);
              const downloaded = isModelDownloaded(modelId);
              const progress = getDownloadProgress(modelId);
              const modelInfo = modelInfoCache.get(modelId);
              const isLoadingInfo = loadingModelInfo === modelId;
              const isSelected = selectedModel?.id === modelId;

              return (
                <Card
                  key={modelId}
                  className={`p-4 cursor-pointer transition-all hover:shadow-md ${
                    isSelected ? "ring-2 ring-primary" : ""
                  }`}
                  onClick={() => handleModelClick(modelId)}
                >
                  <div className="flex items-start justify-between gap-3">
                    <div className="flex-1 min-w-0">
                      <h3 className="font-semibold text-lg truncate">
                        {modelId}
                      </h3>
                      {isLoadingInfo && (
                        <div className="flex items-center gap-2 mt-2 text-sm text-gray-600">
                          <Loader2 className="w-4 h-4 animate-spin" />
                          <span>Loading info...</span>
                        </div>
                      )}
                      {modelInfo && modelInfo.pipeline_tag && (
                        <span className="inline-block px-2 py-1 mt-1 text-xs rounded-full bg-primary/10 text-primary">
                          {modelInfo.pipeline_tag}
                        </span>
                      )}
                    </div>

                    <div className="flex flex-col gap-2">
                      {downloaded ? (
                        <>
                          <div className="flex items-center gap-2 text-sm text-green-600">
                            <CheckCircle className="w-4 h-4" />
                            <span>Downloaded</span>
                          </div>
                          <Button
                            variant="outline"
                            size="sm"
                            onClick={(e) => {
                              e.stopPropagation();
                              handleDelete(modelId);
                            }}
                          >
                            <Trash2 className="w-4 h-4 mr-2" />
                            Delete
                          </Button>
                        </>
                      ) : downloading ? (
                        <div className="space-y-2">
                          <div className="flex items-center gap-2 text-sm text-primary">
                            <Loader2 className="w-4 h-4 animate-spin" />
                            <span>{progress}%</span>
                          </div>
                          <div className="w-32 h-2 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden">
                            <div
                              className="h-full bg-primary transition-all duration-300"
                              style={{ width: `${progress}%` }}
                            />
                          </div>
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
                        >
                          <Download className="w-4 h-4 mr-2" />
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
      />
    </PageContainer>
  );
};
