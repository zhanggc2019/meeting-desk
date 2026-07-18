import { relaunch } from "@tauri-apps/plugin-process";
import { check } from "@tauri-apps/plugin-updater";

export interface UpdateDownloadProgress {
  downloadedBytes: number;
  totalBytes: number | null;
}
export interface AvailableUpdate {
  currentVersion: string;
  version: string;
  date: string | null;
  downloadAndInstall: (onProgress: (progress: UpdateDownloadProgress) => void) => Promise<void>;
  dispose: () => Promise<void>;
}

export interface UpdateService {
  checkForUpdate: () => Promise<AvailableUpdate | null>;
  relaunch: () => Promise<void>;
}

/** 创建只在 Tauri 桌面运行时调用的签名更新服务。 */
export function createTauriUpdateService(): UpdateService {
  return {
    async checkForUpdate() {
      const update = await check({ timeout: 15_000 });
      if (!update) return null;
      return {
        currentVersion: update.currentVersion,
        version: update.version,
        date: update.date ?? null,
        async downloadAndInstall(onProgress) {
          let downloadedBytes = 0;
          let totalBytes: number | null = null;
          await update.downloadAndInstall((event) => {
            if (event.event === "Started") {
              totalBytes = event.data.contentLength ?? null;
              onProgress({ downloadedBytes, totalBytes });
            } else if (event.event === "Progress") {
              downloadedBytes += event.data.chunkLength;
              onProgress({ downloadedBytes, totalBytes });
            } else {
              onProgress({ downloadedBytes: totalBytes ?? downloadedBytes, totalBytes });
            }
          }, { timeout: 10 * 60_000 });
        },
        async dispose() {
          await update.close();
        },
      };
    },
    relaunch,
  };
}
