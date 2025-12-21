// TypeScript interfaces for model-related types

export type ModelTaskType =
  | "text_generation"
  | "embeddings_ov"
  | "rerank_ov"
  | "text2speech"
  | "speech2text"
  | "image_generation";

export interface GraphGenerationParams {
  // Task type selection
  task_type?: ModelTaskType;

  // Common parameters
  target_device?: string; // CPU, GPU, NPU, AUTO

  // Text generation specific
  enable_prefix_caching?: boolean;
  cache_size?: number;
  max_num_seqs?: number;
  kv_cache_precision?: string;
  max_num_batched_tokens?: number;
  dynamic_split_fuse?: boolean;
  pipeline_type?: string;

  // Embeddings specific
  normalize?: boolean;
  pooling?: string;
  truncate?: boolean;

  // Common stream parameter
  num_streams?: number;

  // Image generation specific
  resolution?: string;
  guidance_scale?: string;
  num_images_per_prompt?: string;
  max_resolution?: string;
  default_resolution?: string;
  max_num_images_per_prompt?: number;
  default_num_inference_steps?: number;
  max_num_inference_steps?: number;
}

export interface ModelSibling {
  rfilename: string;
}

export interface ModelInfo {
  id: string;
  author: string | null;
  sha: string | null;
  pipeline_tag: string | null;
  tags: string[];
  downloads: number | null;
  likes: number | null;
  created_at: string | null;
  last_modified: string | null;
  collections: string[] | null;
  siblings: ModelSibling[];
}

export interface SearchResult {
  model_ids: string[];
  total_count: number | null;
}

export interface DownloadProgress {
  modelId: string;
  progress: number;
  currentFile: string;
  fileIndex: number;
  totalFiles: number;
  fileProgress?: number;
  downloadedBytes?: number;
  totalBytes?: number;
  currentFileDownloaded?: number;
  currentFileTotal?: number;
}
