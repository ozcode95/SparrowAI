import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useModels } from "@/store";

export const useDownloadedModels = () => {
  const { setDownloadedModels, setDownloadProgress } = useModels();

  useEffect(() => {
    const checkDownloadedModels = async () => {
      try {
        const downloadedModels: string[] = await invoke(
          "check_downloaded_models",
          {
            downloadPath: null,
          }
        );

        console.log("Found downloaded models in filesystem:", downloadedModels);
        setDownloadedModels(downloadedModels);
      } catch (error) {
        console.error("Failed to check downloaded models:", error);
      }
    };

    checkDownloadedModels();
  }, [setDownloadedModels]);

  useEffect(() => {
    const unlisten = listen<{
      modelId: string;
      progress: number;
      currentFile: string;
      fileIndex: number;
      totalFiles: number;
    }>("download-progress", (event) => {
      const { modelId, progress } = event.payload;
      setDownloadProgress(modelId, progress);
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, [setDownloadProgress]);
};
