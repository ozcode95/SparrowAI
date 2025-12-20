import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { PageContainer } from "../layout";
import { Card, Button, Input } from "../ui";
import { useAppStore } from "@/store";
import { useDebounce } from "@/hooks";
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

interface ModelInfo {
  id: string;
  author: string | null;
  sha: string | null;
  pipeline_tag: string | null;
  tags: string[];
  downloads: number | null;
  likes: number | null;
  created_at: string | null;
  last_modified: string | null;
}

interface SearchResult {
  models: ModelInfo[];
  total_count: number | null;
}

interface DownloadProgress {
  modelId: string;
  progress: number;
  currentFile: string;
  fileIndex: number;
  totalFiles: number;
}

export const ModelsPage = () => {
  const {
    searchQuery,
    setSearchQuery,
    isSearching,
    setIsSearching,
    isModelDownloading,
    getDownloadProgress,
    setDownloadProgress,
    downloadedModels,
    addDownloadedModel,
    removeDownloadedModel,
    isModelDownloaded,
    setDownloadedModels,
  } = useAppStore();
  const [searchResults, setSearchResults] = useState<ModelInfo[]>([]);
  const [selectedModel, setSelectedModel] = useState<ModelInfo | null>(null);
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
      handleSearch();
    } else {
      setSearchResults([]);
    }
  }, [debouncedSearch]);

  // Listen for download progress
  useEffect(() => {
    const unlisten = listen<DownloadProgress>("download-progress", (event) => {
      const { modelId, progress } = event.payload;
      setDownloadProgress(modelId, progress);
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, [setDownloadProgress]);

  const handleSearch = async () => {
    setIsSearching(true);
    try {
      const result = await invoke<SearchResult>("search_models", {
        query: debouncedSearch,
        limit: 10,
      });
      setSearchResults(result.models);
    } catch (error) {
      console.error("Search failed:", error);
      setSearchResults([]);
    } finally {
      setIsSearching(false);
    }
  };

  const handleDownload = async (modelId: string) => {
    try {
      await invoke("download_entire_model", {
        modelId,
        downloadPath: null,
      });
      addDownloadedModel(modelId);
      await loadDownloadedModels();
    } catch (error) {
      console.error("Download failed:", error);
      alert(`Failed to download model: ${error}`);
    }
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
    <PageContainer title="Models" description="Browse and manage AI models">
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6 h-full">
        {/* Search & Results Column */}
        <div className="lg:col-span-2 flex flex-col gap-4">
          {/* Search Input */}
          <Card className="p-4">
            <div className="relative">
              <Search className="absolute left-3 top-1/2 transform -translate-y-1/2 w-5 h-5 text-gray-400" />
              <Input
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder="Search OpenVINO models..."
                className="pl-10"
              />
              {isSearching && (
                <Loader2 className="absolute right-3 top-1/2 transform -translate-y-1/2 w-5 h-5 text-primary animate-spin" />
              )}
            </div>
          </Card>

          {/* Search Results */}
          <div className="flex-1 overflow-y-auto space-y-3">
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

            {searchResults.map((model) => {
              const downloading = isModelDownloading(model.id);
              const downloaded = isModelDownloaded(model.id);
              const progress = getDownloadProgress(model.id);

              return (
                <Card
                  key={model.id}
                  className={`p-4 cursor-pointer transition-all hover:shadow-md ${
                    selectedModel?.id === model.id ? "ring-2 ring-primary" : ""
                  }`}
                  onClick={() => setSelectedModel(model)}
                >
                  <div className="flex items-start justify-between gap-3">
                    <div className="flex-1 min-w-0">
                      <h3 className="font-semibold text-lg truncate">
                        {model.id}
                      </h3>
                      {model.pipeline_tag && (
                        <span className="inline-block px-2 py-1 mt-1 text-xs rounded-full bg-primary/10 text-primary">
                          {model.pipeline_tag}
                        </span>
                      )}

                      <div className="flex items-center gap-4 mt-2 text-sm text-gray-600 dark:text-gray-400">
                        <span className="flex items-center gap-1">
                          <TrendingUp className="w-4 h-4" />
                          {formatNumber(model.downloads)}
                        </span>
                        <span className="flex items-center gap-1">
                          <Heart className="w-4 h-4" />
                          {formatNumber(model.likes)}
                        </span>
                      </div>
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
                              handleDelete(model.id);
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
                            handleDownload(model.id);
                          }}
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

          {/* Downloaded Models */}
          <Card className="p-4">
            <h3 className="font-semibold mb-3 flex items-center gap-2">
              <CheckCircle className="w-5 h-5 text-green-600" />
              Downloaded Models ({downloadedModels.size})
            </h3>
            <div className="space-y-2 max-h-48 overflow-y-auto">
              {Array.from(downloadedModels).length === 0 ? (
                <p className="text-sm text-gray-600 dark:text-gray-400 text-center py-4">
                  No models downloaded yet
                </p>
              ) : (
                Array.from(downloadedModels).map((modelId) => (
                  <div
                    key={modelId}
                    className="flex items-center justify-between p-2 rounded hover:bg-gray-100 dark:hover:bg-gray-800"
                  >
                    <span className="text-sm truncate flex-1">{modelId}</span>
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={() => handleDelete(modelId)}
                    >
                      <Trash2 className="w-4 h-4" />
                    </Button>
                  </div>
                ))
              )}
            </div>
          </Card>
        </div>
      </div>
    </PageContainer>
  );
};
