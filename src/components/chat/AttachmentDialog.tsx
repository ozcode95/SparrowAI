import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "../ui";
import { Button, Input } from "../ui";
import { File, Upload, X, Search, Loader2 } from "lucide-react";
import { logError, logInfo } from "@/lib/logger";

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

interface AttachmentInfo {
  file_path: string;
  file_name: string;
  file_type: string;
}

interface AttachmentDialogProps {
  isOpen: boolean;
  onClose: (open: boolean) => void;
  onAttach: (attachments: AttachmentInfo[]) => void;
  currentAttachments: AttachmentInfo[];
}

export const AttachmentDialog = ({
  isOpen,
  onClose,
  onAttach,
  currentAttachments,
}: AttachmentDialogProps) => {
  const [files, setFiles] = useState<FileInfo[]>([]);
  const [selectedFiles, setSelectedFiles] = useState<Set<string>>(new Set());
  const [searchQuery, setSearchQuery] = useState("");
  const [isLoading, setIsLoading] = useState(false);
  const [isUploading, setIsUploading] = useState(false);

  useEffect(() => {
    if (isOpen) {
      loadFiles();
      // Pre-select currently attached files
      const currentPaths = new Set(currentAttachments.map((a) => a.file_path));
      setSelectedFiles(currentPaths);
    }
  }, [isOpen, currentAttachments]);

  const loadFiles = async () => {
    setIsLoading(true);
    try {
      const result = await invoke<FileInfo[]>("get_all_files");
      setFiles(result);
      logInfo(`Loaded ${result.length} files for attachment selection`);
    } catch (error) {
      logError("Failed to load files", error as Error);
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

          // Create embeddings for all chunks at once
          const embeddedDocs = await invoke<Document[]>(
            "create_document_embeddings",
            {
              documents,
            }
          );

          // Store documents in vector store
          await invoke("store_documents", {
            documents: embeddedDocs,
          });

          logInfo(`Successfully processed and stored ${filePath}`);

          // Auto-select the newly uploaded file
          setSelectedFiles((prev) => new Set(prev).add(filePath));
        } catch (error) {
          logError(`Failed to process ${filePath}`, error as Error);
          console.error(`Failed to process ${filePath}:`, error);
          alert(`Failed to process ${filePath.split("\\").pop()}: ${error}`);
        }
      }

      await loadFiles();
    } catch (error) {
      console.error("Upload failed:", error);
    } finally {
      setIsUploading(false);
    }
  };

  const toggleFileSelection = (filePath: string) => {
    setSelectedFiles((prev) => {
      const newSet = new Set(prev);
      if (newSet.has(filePath)) {
        newSet.delete(filePath);
      } else {
        newSet.add(filePath);
      }
      return newSet;
    });
  };

  const handleAttach = () => {
    const attachments: AttachmentInfo[] = files
      .filter((f) => selectedFiles.has(f.file_path))
      .map((f) => ({
        file_path: f.file_path,
        file_name: f.file_name,
        file_type: f.file_type,
      }));
    onAttach(attachments);
    onClose(false);
  };

  const filteredFiles = files.filter((file) =>
    file.file_name.toLowerCase().includes(searchQuery.toLowerCase())
  );

  return (
    <Dialog open={isOpen} onOpenChange={onClose}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>Attach Documents</DialogTitle>
        </DialogHeader>
        <div className="space-y-4">
          {/* Upload Section */}
          <div className="flex items-center gap-3">
            <Button
              onClick={handleFileUpload}
              disabled={isUploading}
              variant="outline"
              className="flex items-center gap-2"
            >
              {isUploading ? (
                <Loader2 className="w-4 h-4 animate-spin" />
              ) : (
                <Upload className="w-4 h-4" />
              )}
              Upload New Document
            </Button>
            {isUploading && (
              <span className="text-sm text-gray-600 dark:text-gray-400">
                Processing...
              </span>
            )}
          </div>

          {/* Search */}
          <div className="relative">
            <Search className="absolute left-3 top-1/2 transform -translate-y-1/2 w-4 h-4 text-gray-400" />
            <Input
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder="Search documents..."
              className="pl-10"
            />
          </div>

          {/* File List */}
          <div className="border border-gray-200 dark:border-gray-700 rounded-lg max-h-96 overflow-y-auto">
            {isLoading ? (
              <div className="flex items-center justify-center p-8">
                <Loader2 className="w-6 h-6 animate-spin text-gray-400" />
              </div>
            ) : filteredFiles.length === 0 ? (
              <div className="p-8 text-center text-gray-500 dark:text-gray-400">
                {searchQuery
                  ? "No documents match your search"
                  : "No documents available. Upload some documents first."}
              </div>
            ) : (
              <div className="divide-y divide-gray-200 dark:divide-gray-700">
                {filteredFiles.map((file) => (
                  <div
                    key={file.file_path}
                    className={`p-3 hover:bg-gray-50 dark:hover:bg-gray-800 cursor-pointer transition-colors ${
                      selectedFiles.has(file.file_path)
                        ? "bg-primary/5 dark:bg-primary/10"
                        : ""
                    }`}
                    onClick={() => toggleFileSelection(file.file_path)}
                  >
                    <div className="flex items-center gap-3">
                      <input
                        type="checkbox"
                        checked={selectedFiles.has(file.file_path)}
                        onChange={() => toggleFileSelection(file.file_path)}
                        className="w-4 h-4 rounded border-gray-300 text-primary focus:ring-primary"
                        onClick={(e) => e.stopPropagation()}
                      />
                      <File className="w-5 h-5 text-gray-400 flex-shrink-0" />
                      <div className="flex-1 min-w-0">
                        <div className="font-medium text-sm truncate">
                          {file.file_name}
                        </div>
                        <div className="text-xs text-gray-500 dark:text-gray-400">
                          {file.file_type.toUpperCase()} • {file.chunk_count}{" "}
                          chunks
                        </div>
                      </div>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>

          {/* Selected Count */}
          {selectedFiles.size > 0 && (
            <div className="text-sm text-gray-600 dark:text-gray-400">
              {selectedFiles.size} document{selectedFiles.size !== 1 ? "s" : ""}{" "}
              selected
            </div>
          )}

          {/* Actions */}
          <DialogFooter className="flex justify-end gap-3 pt-4 border-t border-gray-200 dark:border-gray-700">
            <Button onClick={() => onClose(false)} variant="outline">
              Cancel
            </Button>
            <Button onClick={handleAttach} disabled={selectedFiles.size === 0}>
              Attach {selectedFiles.size > 0 && `(${selectedFiles.size})`}
            </Button>
          </DialogFooter>
        </div>
      </DialogContent>
    </Dialog>
  );
};
