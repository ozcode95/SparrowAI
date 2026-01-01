import { useState, useEffect, useRef } from "react";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { PageContainer } from "../layout";
import { Card, Button, Input } from "../ui";
import { useAppStore } from "@/store";
import type { GeneratedImage } from "@/store/slices/gallerySlice";
import {
  Send,
  Loader2,
  Paperclip,
  X,
  ExternalLink,
  Trash2,
} from "lucide-react";
import { logUserAction, logInfo } from "@/lib/logger";
import { categorizeModel } from "@/lib/modelUtils";

interface AttachmentInfo {
  file_path: string;
  file_name: string;
  file_type: string;
}

export const GalleryPage = () => {
  const {
    downloadedModels,
    loadedModelsByType,
    setLoadedModelByType,
    showNotification,
  } = useAppStore();

  const {
    generatedImages,
    isGenerating,
    currentGeneratingImage,
    addGeneratedImage,
    setIsGenerating,
    setCurrentGeneratingImage,
    setGeneratedImages,
  } = useAppStore();

  const [prompt, setPrompt] = useState("");
  const [selectedModel, setSelectedModel] = useState<string | null>(null);
  const [imageSize, setImageSize] = useState("512x512");
  const [numInferenceSteps, setNumInferenceSteps] = useState(10);
  const [attachments, setAttachments] = useState<AttachmentInfo[]>([]);
  const [showConfigPanel, setShowConfigPanel] = useState(false);

  const galleryEndRef = useRef<HTMLDivElement>(null);

  // Filter image generation models
  const imageGenModels = Array.from(downloadedModels).filter(
    (modelId) => categorizeModel(modelId) === "image-gen"
  );

  // Auto-select loaded image-gen model or first available
  useEffect(() => {
    const loadedImageGenModel = loadedModelsByType["image-gen"];
    if (loadedImageGenModel && imageGenModels.includes(loadedImageGenModel)) {
      setSelectedModel(loadedImageGenModel);
    } else if (imageGenModels.length > 0 && !selectedModel) {
      setSelectedModel(imageGenModels[0]);
    }
  }, [downloadedModels, loadedModelsByType]);

  // Load generated images on mount
  useEffect(() => {
    loadGeneratedImages();
  }, []);

  const loadGeneratedImages = async () => {
    try {
      const images = await invoke<GeneratedImage[]>("get_generated_images");
      setGeneratedImages(images);
      logInfo("Loaded generated images", { count: images.length });
      console.log("Generated images:", images);
    } catch (error) {
      console.error("Failed to load generated images:", error);
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      showNotification(`Failed to load images: ${errorMessage}`, "error", 3000);
    }
  };

  const handleFileUpload = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        multiple: true,
        filters: [
          {
            name: "Images",
            extensions: ["png", "jpg", "jpeg", "webp", "gif"],
          },
        ],
      });

      if (!selected || (Array.isArray(selected) && selected.length === 0)) {
        return;
      }

      const filePaths = Array.isArray(selected) ? selected : [selected];

      for (const filePath of filePaths) {
        const extension = filePath.split(".").pop()?.toLowerCase() || "";
        const fileName = filePath.split(/[\\/]/).pop() || filePath;

        setAttachments((prev) => [
          ...prev,
          {
            file_path: filePath,
            file_name: fileName,
            file_type: extension,
          },
        ]);
      }
    } catch (error) {
      console.error("Failed to upload images:", error);
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      showNotification(
        `Failed to upload images: ${errorMessage}`,
        "error",
        3000
      );
    }
  };

  const removeAttachment = (filePath: string) => {
    setAttachments((prev) => prev.filter((a) => a.file_path !== filePath));
  };

  const handleGenerate = async () => {
    if (!prompt.trim()) {
      showNotification("Please enter a prompt", "warning", 3000);
      return;
    }

    if (!selectedModel) {
      showNotification(
        "Please select an image generation model",
        "warning",
        3000
      );
      return;
    }

    try {
      logUserAction("generate_image", {
        model: selectedModel,
        prompt: prompt.substring(0, 50),
        size: imageSize,
        steps: numInferenceSteps,
      });

      setIsGenerating(true);

      // Load the model if not already loaded
      const currentLoadedModel = loadedModelsByType["image-gen"];
      if (currentLoadedModel !== selectedModel) {
        showNotification("Loading model...", "info", 2000);
        await invoke("load_model", { modelId: selectedModel });
        setLoadedModelByType("image-gen", selectedModel);
      }

      // Generate image
      const result = await invoke<{
        image_path: string;
        id: string;
      }>("generate_image", {
        modelId: selectedModel,
        prompt,
        size: imageSize,
        numInferenceSteps,
        referenceImages: attachments.map((a) => a.file_path),
      });

      const newImage: GeneratedImage = {
        id: result.id,
        prompt,
        imagePath: result.image_path,
        timestamp: Date.now(),
        modelId: selectedModel,
        size: imageSize,
        numInferenceSteps,
      };

      addGeneratedImage(newImage);
      setCurrentGeneratingImage(newImage);
      showNotification("Image generated successfully!", "success", 3000);

      // Clear prompt and attachments
      setPrompt("");
      setAttachments([]);
    } catch (error) {
      console.error("Failed to generate image:", error);
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      showNotification(
        `Failed to generate image: ${errorMessage}`,
        "error",
        5000
      );
    } finally {
      setIsGenerating(false);
    }
  };

  const handleOpenImage = async (imagePath: string) => {
    try {
      console.log("Opening image:", imagePath);
      if (!imagePath) {
        showNotification("Invalid image path", "error", 3000);
        return;
      }
      const { openPath } = await import("@tauri-apps/plugin-opener");
      await openPath(imagePath);
    } catch (error) {
      console.error("Failed to open image:", error);
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      showNotification(`Failed to open image: ${errorMessage}`, "error", 3000);
    }
  };

  const handleDeleteImage = async (imageId: string, imagePath: string) => {
    try {
      await invoke("delete_generated_image", { imageId, imagePath });
      setGeneratedImages(generatedImages.filter((img) => img.id !== imageId));
      if (currentGeneratingImage?.id === imageId) {
        setCurrentGeneratingImage(null);
      }
      showNotification("Image deleted", "success", 2000);
    } catch (error) {
      console.error("Failed to delete image:", error);
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      showNotification(
        `Failed to delete image: ${errorMessage}`,
        "error",
        3000
      );
    }
  };

  return (
    <PageContainer>
      <div className="flex flex-col h-full">
        {/* Header */}
        <div className="flex items-center justify-between mb-6">
          <h1 className="text-2xl font-bold text-gray-900 dark:text-white">
            Image Gallery
          </h1>
        </div>

        {/* Main Content */}
        <div className="flex-1 flex flex-col overflow-hidden">
          {/* Image Grid */}
          <div className="flex-1 overflow-y-auto mb-4">
            <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-4 p-4">
              {/* Current/Last Generated Image */}
              {currentGeneratingImage && (
                <Card className="relative group overflow-hidden aspect-square">
                  <img
                    src={convertFileSrc(currentGeneratingImage.imagePath)}
                    alt={currentGeneratingImage.prompt}
                    className="w-full h-full object-cover"
                    onError={() => {
                      console.error(
                        "Failed to load current image:",
                        currentGeneratingImage.imagePath
                      );
                      console.error(
                        "Converted src:",
                        convertFileSrc(currentGeneratingImage.imagePath)
                      );
                    }}
                    onLoad={() =>
                      console.log("Current image loaded successfully")
                    }
                  />
                  <div className="absolute inset-0 group-hover:bg-black/50 transition-all duration-200 flex items-end">
                    <div className="p-3 w-full opacity-0 group-hover:opacity-100 transition-opacity duration-200">
                      <p className="text-white text-sm font-medium mb-2 line-clamp-2">
                        {currentGeneratingImage.prompt}
                      </p>
                      <div className="flex gap-2">
                        <Button
                          onClick={() =>
                            handleOpenImage(currentGeneratingImage.imagePath)
                          }
                          variant="secondary"
                          size="sm"
                        >
                          <ExternalLink className="h-4 w-4" />
                        </Button>
                        <Button
                          onClick={() =>
                            handleDeleteImage(
                              currentGeneratingImage.id,
                              currentGeneratingImage.imagePath
                            )
                          }
                          variant="secondary"
                          size="sm"
                        >
                          <Trash2 className="h-4 w-4" />
                        </Button>
                      </div>
                    </div>
                  </div>
                  <div className="absolute top-2 right-2 bg-green-500 text-white px-2 py-1 rounded text-xs font-medium">
                    Latest
                  </div>
                </Card>
              )}

              {/* Generating Placeholder */}
              {isGenerating && (
                <Card className="aspect-square flex items-center justify-center bg-gray-100 dark:bg-gray-800">
                  <div className="flex flex-col items-center gap-3">
                    <Loader2 className="h-12 w-12 animate-spin text-orange-500" />
                    <p className="text-sm text-gray-600 dark:text-gray-400">
                      Generating...
                    </p>
                  </div>
                </Card>
              )}

              {/* Previously Generated Images */}
              {generatedImages
                .filter((image) => image.id !== currentGeneratingImage?.id)
                .map((image) => (
                  <Card
                    key={image.id}
                    className="relative group overflow-hidden aspect-square"
                  >
                    <img
                      src={convertFileSrc(image.imagePath)}
                      alt={image.prompt}
                      className="w-full h-full object-cover"
                      title="test"
                    />
                    <div className="absolute inset-0 group-hover:bg-black/50 transition-all duration-200 flex items-end">
                      <div className="p-3 w-full opacity-0 group-hover:opacity-100 transition-opacity duration-200">
                        <p className="text-white text-sm font-medium mb-2 line-clamp-2">
                          {image.prompt}
                        </p>
                        <div className="flex gap-2">
                          <Button
                            onClick={() => handleOpenImage(image.imagePath)}
                            variant="secondary"
                            size="sm"
                          >
                            <ExternalLink className="h-4 w-4" />
                          </Button>
                          <Button
                            onClick={() =>
                              handleDeleteImage(image.id, image.imagePath)
                            }
                            variant="secondary"
                            size="sm"
                          >
                            <Trash2 className="h-4 w-4" />
                          </Button>
                        </div>
                      </div>
                    </div>
                  </Card>
                ))}
            </div>
            <div ref={galleryEndRef} />
          </div>

          {/* Input Section */}
          <Card className="p-4 space-y-4">
            {/* Model Selection */}
            <div className="flex items-center gap-3">
              <label className="text-sm font-medium text-gray-700 dark:text-gray-300 min-w-20">
                Model:
              </label>
              <select
                value={selectedModel || ""}
                onChange={(e) => setSelectedModel(e.target.value)}
                className="flex-1 px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-md bg-white dark:bg-gray-800 text-gray-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-orange-500"
                disabled={isGenerating}
              >
                {imageGenModels.length === 0 ? (
                  <option value="">No image generation models available</option>
                ) : (
                  imageGenModels.map((modelId) => (
                    <option key={modelId} value={modelId}>
                      {modelId}
                    </option>
                  ))
                )}
              </select>

              <Button
                onClick={() => setShowConfigPanel(!showConfigPanel)}
                variant="outline"
                size="sm"
              >
                {showConfigPanel ? "Hide" : "Config"}
              </Button>
            </div>

            {/* Configuration Panel */}
            {showConfigPanel && (
              <div className="space-y-3 p-3 bg-gray-50 dark:bg-gray-800 rounded-md">
                <div className="flex items-center gap-3">
                  <label className="text-sm font-medium text-gray-700 dark:text-gray-300 min-w-30">
                    Size:
                  </label>
                  <select
                    value={imageSize}
                    onChange={(e) => setImageSize(e.target.value)}
                    className="flex-1 px-3 py-1.5 border border-gray-300 dark:border-gray-600 rounded bg-white dark:bg-gray-700 text-sm"
                  >
                    <option value="256x256">256x256</option>
                    <option value="512x512">512x512</option>
                    <option value="768x768">768x768</option>
                    <option value="1024x1024">1024x1024</option>
                  </select>
                </div>

                <div className="flex items-center gap-3">
                  <label className="text-sm font-medium text-gray-700 dark:text-gray-300 min-w-30">
                    Inference Steps:
                  </label>
                  <input
                    type="number"
                    value={numInferenceSteps}
                    onChange={(e) =>
                      setNumInferenceSteps(parseInt(e.target.value) || 10)
                    }
                    min="1"
                    max="100"
                    className="flex-1 px-3 py-1.5 border border-gray-300 dark:border-gray-600 rounded bg-white dark:bg-gray-700 text-sm"
                  />
                </div>
              </div>
            )}

            {/* Attachments */}
            {attachments.length > 0 && (
              <div className="flex flex-wrap gap-2">
                {attachments.map((attachment) => (
                  <div key={attachment.file_path} className="relative group">
                    <img
                      src={convertFileSrc(attachment.file_path)}
                      alt={attachment.file_name}
                      className="w-20 h-20 object-cover rounded border-2 border-gray-300 dark:border-gray-600"
                    />
                    <button
                      onClick={() => removeAttachment(attachment.file_path)}
                      className="absolute -top-2 -right-2 bg-red-500 text-white rounded-full p-1 opacity-0 group-hover:opacity-100 transition-opacity"
                    >
                      <X className="h-3 w-3" />
                    </button>
                  </div>
                ))}
              </div>
            )}

            {/* Input Row */}
            <div className="flex gap-2">
              <Button
                onClick={handleFileUpload}
                variant="outline"
                disabled={isGenerating}
              >
                <Paperclip className="h-4 w-4" />
              </Button>

              <Input
                value={prompt}
                onChange={(e) => setPrompt(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    handleGenerate();
                  }
                }}
                placeholder="Describe the image you want to generate..."
                disabled={isGenerating}
                className="flex-1"
              />

              <Button
                onClick={handleGenerate}
                disabled={isGenerating || !prompt.trim() || !selectedModel}
              >
                {isGenerating ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Send className="h-4 w-4" />
                )}
              </Button>
            </div>
          </Card>
        </div>
      </div>
    </PageContainer>
  );
};
