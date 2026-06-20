// Обёртка над Tauri-плагином автообновления лаунчера.

import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export type { Update };

/** Таймаут проверки обновления (мс). С российских IP сервер обновлений (GitHub
 *  CDN) часто недоступен — без таймаута запрос «висит» бесконечно. */
const CHECK_TIMEOUT_MS = 5000;

function withTimeout<T>(p: Promise<T>, ms: number, message: string): Promise<T> {
  return Promise.race([
    p,
    new Promise<T>((_, reject) =>
      setTimeout(() => reject(new Error(message)), ms),
    ),
  ]);
}

/** Проверить обновление лаунчера. `null` — обновлений нет. Бросает при ошибке
 *  сети/конфига или по таймауту (сервер обновлений недоступен). */
export function checkUpdate(): Promise<Update | null> {
  // timeout у плагина + жёсткий JS-таймаут на случай зависшего DNS/коннекта.
  return withTimeout(
    check({ timeout: CHECK_TIMEOUT_MS }),
    CHECK_TIMEOUT_MS + 1000,
    "Превышено время ожидания (сервер обновлений недоступен)",
  );
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
