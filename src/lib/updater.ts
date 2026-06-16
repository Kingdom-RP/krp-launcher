// Обёртка над Tauri-плагином автообновления лаунчера.

import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export type { Update };

/** Проверить обновление лаунчера. `null` — обновлений нет. Бросает при ошибке сети/конфига. */
export function checkUpdate(): Promise<Update | null> {
  return check();
}

/**
 * Скачать и установить обновление, сообщая прогресс, затем перезапустить лаунчер.
 * После `relaunch()` процесс завершается, поэтому код после вызова не выполняется.
 */
export async function installUpdate(
  update: Update,
  onProgress?: (downloaded: number, total: number | null) => void,
): Promise<void> {
  let downloaded = 0;
  let total: number | null = null;

  await update.downloadAndInstall((event) => {
    switch (event.event) {
      case "Started":
        total = event.data.contentLength ?? null;
        break;
      case "Progress":
        downloaded += event.data.chunkLength;
        onProgress?.(downloaded, total);
        break;
    }
  });

  await relaunch();
}
