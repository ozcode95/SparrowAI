import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { PageContainer } from "../layout";
import { Card, Button } from "../ui";
import { ModelDownloadDialog } from "./ModelDownloadDialog";
import {
  Upload,
  File,
  Trash2,
  FileText,
  Loader2,
  ChevronDown,
  ChevronRight,
} from "lucide-react";

interface FileInfo {
  file_path: string;
  file_name: string;
  file_type: string;
  chunk_count: number;
  created_at: number;
}

interface Document {
  id: string;
  title: string;
  content: string;
  file_type: string;
  file_path: string;
  chunk_index: number | null;
  metadata: Record<string, string>;
  embedding: number[] | null;
  created_at: number;
}

export const DocumentsPage = () => {
  const [files, setFiles] = useState<FileInfo[]>([]);
  const [expandedFiles, setExpandedFiles] = useState<Set<string>>(new Set());
  const [fileChunks, setFileChunks] = useState<Record<string, Document[]>>({});
  const [isUploading, setIsUploading] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [showModelDownload, setShowModelDownload] = useState(false);
  const [modelsChecked, setModelsChecked] = useState(false);

  useEffect(() => {
    checkBGEModels();
  }, []);

  const checkBGEModels = async () => {
    if (modelsChecked) return;

    try {
      const modelsExist = await invoke<boolean>("check_bge_models_exist", {
        downloadPath: null,
      });

      if (!modelsExist) {
        setShowModelDownload(true);
      } else {
        // Models exist, load files
        loadFiles();
      }
      setModelsChecked(true);
    } catch (error) {
      console.error("Failed to check BGE models:", error);
      // Continue anyway, user can still try to use the features
      loadFiles();
      setModelsChecked(true);
    }
  };

  const handleModelsDownloaded = () => {
    // Models have been downloaded, now load files
    loadFiles();
  };

  useEffect(() => {
    if (modelsChecked && !showModelDownload) {
      loadFiles();
    }
  }, [modelsChecked, showModelDownload]);

  const loadFiles = async () => {
    setIsLoading(true);
    try {
      const result = await invoke<FileInfo[]>("get_all_files");
      setFiles(result);
    } catch (error) {
      console.error("Failed to load files:", error);
    } finally {
      setIsLoading(false);
    }
  };

  const handleFileUpload = async () => {
    try {
      const selected = await open({
        multiple: true,
        filters: [
          {
            name: "Documents",
            extensions: ["pdf", "docx", "xlsx", "xls"],
          },
        ],
      });

      if (!selected || (Array.isArray(selected) && selected.length === 0)) {
        return;
      }

      const filePaths = Array.isArray(selected) ? selected : [selected];
      setIsUploading(true);

      for (const filePath of filePaths) {
        try {
          // Process document into chunks
          const documents = await invoke<Document[]>("process_document", {
            filePath,
          });

          // Create embeddings for each chunk
          const embeddedDocs: Document[] = [];
          for (const doc of documents) {
            const embedding = await invoke<number[]>(
              "create_document_embeddings",
              {
                text: doc.content,
              }
            );

            embeddedDocs.push({
              ...doc,
              embedding,
            });
          }

          // Store documents in vector store
          await invoke("store_documents", {
            documents: embeddedDocs,
          });
        } catch (error) {
          console.error(`Failed to process ${filePath}:`, error);
        }
      }

      await loadFiles();
    } catch (error) {
      console.error("Upload failed:", error);
    } finally {
      setIsUploading(false);
    }
  };

  const handleDeleteFile = async (filePath: string) => {
    if (
      !confirm(
        `Are you sure you want to delete all chunks from ${filePath
          .split("\\")
          .pop()}?`
      )
    ) {
      return;
    }

    try {
      await invoke("delete_file_by_path", { filePath });
      await loadFiles();

      // Remove from expanded files and chunks cache
      setExpandedFiles((prev) => {
        const newSet = new Set(prev);
        newSet.delete(filePath);
        return newSet;
      });

      setFileChunks((prev) => {
        const newChunks = { ...prev };
        delete newChunks[filePath];
        return newChunks;
      });
    } catch (error) {
      console.error("Delete failed:", error);
      alert(`Failed to delete file: ${error}`);
    }
  };

  const toggleFileExpansion = async (filePath: string) => {
    const newExpanded = new Set(expandedFiles);

    if (newExpanded.has(filePath)) {
      newExpanded.delete(filePath);
    } else {
      newExpanded.add(filePath);

      // Load chunks if not already loaded
      if (!fileChunks[filePath]) {
        try {
          const chunks = await invoke<Document[]>("get_file_chunks", {
            filePath,
          });

          setFileChunks((prev) => ({
            ...prev,
            [filePath]: chunks,
          }));
        } catch (error) {
          console.error("Failed to load chunks:", error);
        }
      }
    }

    setExpandedFiles(newExpanded);
  };

  const formatDate = (timestamp: number) => {
    return new Date(timestamp).toLocaleDateString();
  };

  const getFileIcon = (fileType: string) => {
    switch (fileType.toLowerCase()) {
      case "pdf":
        return <FileText className="w-5 h-5 text-red-500" />;
      case "docx":
        return <FileText className="w-5 h-5 text-blue-500" />;
      case "xlsx":
      case "xls":
        return <FileText className="w-5 h-5 text-green-500" />;
      default:
        return <File className="w-5 h-5 text-gray-500" />;
    }
  };

  return (
    <PageContainer
      title="Documents"
      description="Manage your document knowledge base"
    >
      <div className="flex flex-col h-full gap-4">
        {/* Upload Section */}
        <Card className="p-4">
          <div className="flex items-center justify-between">
            <div>
              <h2 className="text-lg font-semibold mb-1">Document Library</h2>
              <p className="text-sm text-gray-600 dark:text-gray-400">
                {files.length} {files.length === 1 ? "document" : "documents"}{" "}
                indexed
              </p>
            </div>
            <Button
              onClick={handleFileUpload}
              disabled={isUploading}
              className="flex items-center gap-2"
            >
              {isUploading ? (
                <>
                  <Loader2 className="w-5 h-5 animate-spin" />
                  Processing...
                </>
              ) : (
                <>
                  <Upload className="w-5 h-5" />
                  Upload Documents
                </>
              )}
            </Button>
          </div>
        </Card>

        {/* Files List */}
        <div className="flex-1 overflow-y-auto space-y-2">
          {isLoading ? (
            <Card className="p-8 text-center">
              <Loader2 className="w-12 h-12 mx-auto mb-3 text-primary animate-spin" />
              <p className="text-gray-600 dark:text-gray-400">
                Loading documents...
              </p>
            </Card>
          ) : files.length === 0 ? (
            <Card className="p-8 text-center">
              <Upload className="w-12 h-12 mx-auto mb-3 text-gray-400" />
              <h3 className="text-lg font-semibold mb-2">No documents yet</h3>
              <p className="text-gray-600 dark:text-gray-400 mb-4">
                Upload PDF, DOCX, or Excel files to build your knowledge base
              </p>
              <Button onClick={handleFileUpload}>
                <Upload className="w-5 h-5 mr-2" />
                Upload Your First Document
              </Button>
            </Card>
          ) : (
            files.map((file) => {
              const isExpanded = expandedFiles.has(file.file_path);
              const chunks = fileChunks[file.file_path] || [];

              return (
                <Card key={file.file_path} className="overflow-hidden">
                  {/* File Header */}
                  <div className="flex items-center justify-between p-4 hover:bg-gray-50 dark:hover:bg-gray-800 transition-colors">
                    <div
                      className="flex items-center gap-3 flex-1 cursor-pointer"
                      onClick={() => toggleFileExpansion(file.file_path)}
                    >
                      <button className="text-gray-500 hover:text-gray-700 dark:hover:text-gray-300">
                        {isExpanded ? (
                          <ChevronDown className="w-5 h-5" />
                        ) : (
                          <ChevronRight className="w-5 h-5" />
                        )}
                      </button>
                      {getFileIcon(file.file_type)}
                      <div className="flex-1 min-w-0">
                        <h3 className="font-medium truncate">
                          {file.file_name}
                        </h3>
                        <p className="text-sm text-gray-600 dark:text-gray-400">
                          {file.chunk_count}{" "}
                          {file.chunk_count === 1 ? "chunk" : "chunks"} •{" "}
                          {formatDate(file.created_at)}
                        </p>
                      </div>
                    </div>
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={(e) => {
                        e.stopPropagation();
                        handleDeleteFile(file.file_path);
                      }}
                    >
                      <Trash2 className="w-5 h-5" />
                    </Button>
                  </div>

                  {/* Chunks List */}
                  {isExpanded && (
                    <div className="border-t border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-900">
                      {chunks.length === 0 ? (
                        <div className="p-4 text-center text-gray-600 dark:text-gray-400">
                          <Loader2 className="w-6 h-6 mx-auto mb-2 animate-spin" />
                          Loading chunks...
                        </div>
                      ) : (
                        <div className="divide-y divide-gray-200 dark:divide-gray-700 max-h-96 overflow-y-auto">
                          {chunks.map((chunk, idx) => (
                            <div key={chunk.id} className="p-4 space-y-2">
                              <div className="flex items-start justify-between gap-2">
                                <span className="text-xs font-medium text-gray-500 dark:text-gray-400">
                                  Chunk{" "}
                                  {chunk.chunk_index !== null
                                    ? chunk.chunk_index + 1
                                    : idx + 1}
                                </span>
                                <span className="text-xs text-gray-400">
                                  {chunk.content.length} chars
                                </span>
                              </div>
                              <p className="text-sm text-gray-700 dark:text-gray-300 line-clamp-3">
                                {chunk.content}
                              </p>
                            </div>
                          ))}
                        </div>
                      )}
                    </div>
                  )}
                </Card>
              );
            })
          )}
        </div>
      </div>

      {/* Model Download Dialog */}
      <ModelDownloadDialog
        isOpen={showModelDownload}
        onClose={() => setShowModelDownload(false)}
        onSuccess={handleModelsDownloaded}
      />
    </PageContainer>
  );
};
