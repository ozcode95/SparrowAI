import { StateCreator } from "zustand";
import type { AppState } from "../types";
import { logStateChange } from "../../lib/logger";

export interface GeneratedImage {
  id: string;
  prompt: string;
  imagePath: string;
  timestamp: number;
  modelId: string;
  size: string;
  numInferenceSteps?: number;
}

export interface GallerySlice {
  generatedImages: GeneratedImage[];
  isGenerating: boolean;
  currentGeneratingImage: GeneratedImage | null;
  setGeneratedImages: (images: GeneratedImage[]) => void;
  addGeneratedImage: (image: GeneratedImage) => void;
  setIsGenerating: (isGenerating: boolean) => void;
  setCurrentGeneratingImage: (image: GeneratedImage | null) => void;
  clearGallery: () => void;
}

export const createGallerySlice: StateCreator<
  AppState,
  [],
  [],
  GallerySlice
> = (set) => ({
  generatedImages: [],
  isGenerating: false,
  currentGeneratingImage: null,

  setGeneratedImages: (images) => {
    logStateChange("gallery", "setGeneratedImages", { count: images.length });
    set({ generatedImages: images });
  },

  addGeneratedImage: (image) => {
    logStateChange("gallery", "addGeneratedImage", { imageId: image.id });
    set((state) => ({
      generatedImages: [image, ...state.generatedImages],
    }));
  },

  setIsGenerating: (isGenerating) => set({ isGenerating }),

  setCurrentGeneratingImage: (image) => set({ currentGeneratingImage: image }),

  clearGallery: () => {
    logStateChange("gallery", "clearGallery", {});
    set({ generatedImages: [], currentGeneratingImage: null });
  },
});
