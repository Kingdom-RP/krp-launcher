// Типизированная обёртка над Tauri-командами и событиями бэкенда.
// Типы зеркалят Rust-структуры из src-tauri/src.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** Результат проверки пути установки (paths.rs::PathValidation). */
export interface PathValidation {
  valid: boolean;
  errors: string[];
  warnings: string[];
}

/** Прогресс по текущему файлу (install.rs::SyncProgress). */
export interface SyncProgress {
  index: number;
  total: number;
  file: string;
  downloaded: number;
  total_bytes: number | null;
}

/** Итог синхронизации (install.rs::SyncSummary). */
export interface SyncSummary {
  total: number;
  downloaded: number;
  skipped: number;
}

/** Имя события прогресса (install.rs::PROGRESS_EVENT). */
export const PROGRESS_EVENT = "sync://progress";

/** Папка установки по умолчанию (%APPDATA%\KingdomRP). */
export function defaultInstallDir(): Promise<string> {
  return invoke<string>("default_install_dir");
}

/** Проверить выбранный путь установки (включая права на запись). */
export function validateInstallPath(path: string): Promise<PathValidation> {
  return invoke<PathValidation>("validate_install_path", { path });
}

/** Синхронизировать файлы игры в указанную папку. */
export function syncFiles(installDir: string): Promise<SyncSummary> {
  return invoke<SyncSummary>("sync_files", { installDir });
}

/** Полный цикл «Играть»: ваниль + JRE + файлы + запуск. Возвращает PID. */
export function play(installDir: string, playerName: string): Promise<number> {
  return invoke<number>("play", { installDir, playerName });
}

/** Подписаться на события прогресса. Возвращает функцию отписки. */
export function onSyncProgress(
  cb: (p: SyncProgress) => void,
): Promise<UnlistenFn> {
  return listen<SyncProgress>(PROGRESS_EVENT, (e) => cb(e.payload));
}
