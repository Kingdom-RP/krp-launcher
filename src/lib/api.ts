// Типизированная обёртка над Tauri-командами и событиями бэкенда.
// Типы зеркалят Rust-структуры из src-tauri/src.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open as openDialog, confirm as confirmDialog } from "@tauri-apps/plugin-dialog";

/** Открыть папку установки игры в системном файловом менеджере. */
export function openInstallDir(path: string): Promise<void> {
  return invoke<void>("open_dir", { path });
}

/** Статус игрового MC-сервера (server.rs::ServerStatus). */
export interface ServerStatus {
  online: boolean;
  players_online: number;
  players_max: number;
}

/** Проверить статус MC-сервера (онлайн + число игроков). Не бросает: при
 *  недоступности возвращает `online:false`. */
export function serverStatus(): Promise<ServerStatus> {
  return invoke<ServerStatus>("server_status");
}

/** Показать системный диалог выбора каталога установки.
 *  Возвращает выбранный путь или `null`, если пользователь отменил выбор. */
export async function pickInstallDir(defaultPath?: string): Promise<string | null> {
  const selected = await openDialog({
    directory: true,
    multiple: false,
    title: "Выберите папку для установки Kingdom RP",
    defaultPath: defaultPath || undefined,
  });
  // directory:false multiple:false → string | null; здесь всегда string | null.
  return typeof selected === "string" ? selected : null;
}

/** Аккаунт игрока (auth.rs::AccountInfo). */
export interface AccountInfo {
  username: string;
  player_name: string;
  uuid: string;
  skin_url: string | null;
}

/** Текущий вошедший аккаунт или `null`. */
export function authAccount(): Promise<AccountInfo | null> {
  return invoke<AccountInfo | null>("auth_account");
}

/** Регистрация нового аккаунта (логин/пароль). */
export function authRegister(username: string, password: string): Promise<AccountInfo> {
  return invoke<AccountInfo>("auth_register", { username, password });
}

/** Вход существующего аккаунта. */
export function authLogin(username: string, password: string): Promise<AccountInfo> {
  return invoke<AccountInfo>("auth_login", { username, password });
}

/** Выйти из аккаунта. */
export function authLogout(): Promise<void> {
  return invoke<void>("auth_logout");
}

/** Загрузить скин (PNG уже валидируется на бэкенде). `slim` — тонкая модель. */
export function uploadSkin(path: string, slim: boolean): Promise<void> {
  return invoke<void>("upload_skin", { path, slim });
}

/** Диалог выбора PNG-скина. `null`, если игрок отменил. */
export async function pickSkinFile(): Promise<string | null> {
  const sel = await openDialog({
    multiple: false,
    directory: false,
    title: "Выберите PNG-скин (64×64)",
    filters: [{ name: "PNG-скин", extensions: ["png"] }],
  });
  return typeof sel === "string" ? sel : null;
}

/** Локальный PNG → data-URL (валидирует формат) для превью. */
export function skinPreviewFile(path: string): Promise<string> {
  return invoke<string>("skin_preview_file", { path });
}

/** Скин по URL (drasl) → data-URL для превью без CORS. */
export function skinPreviewUrl(url: string): Promise<string> {
  return invoke<string>("skin_preview_url", { url });
}

/** Результат проверки пути установки (paths.rs::PathValidation). */
export interface PathValidation {
  valid: boolean;
  errors: string[];
  warnings: string[];
}

/** Общий прогресс установки (progress.rs::ProgressPayload). */
export interface SyncProgress {
  /** Что сейчас делаем, напр. «Устанавливаем Minecraft 1.21.1». */
  label: string;
  /** Скачано байт всего. */
  downloaded: number;
  /** Общий объём к скачиванию, байт. */
  total: number;
  /** Текущая скорость, байт/с. */
  speed: number;
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

/** Папка установки для показа при старте: запомненная или по умолчанию. */
export function getInstallDir(): Promise<string> {
  return invoke<string>("get_install_dir");
}

/** Запомнить выбранную папку установки в настройках лаунчера. */
export function setInstallDir(installDir: string): Promise<void> {
  return invoke<void>("set_install_dir", { installDir });
}

/** Привести выбранный каталог к папке установки (добавить «Kingdom RP»). */
export function resolveInstallDir(picked: string): Promise<string> {
  return invoke<string>("resolve_install_dir", { picked });
}

/** Запомненный никнейм игрока (пустая строка, если ещё не вводили). */
export function getPlayerName(): Promise<string> {
  return invoke<string>("get_player_name");
}

/** Запомнить никнейм игрока. */
export function setPlayerName(playerName: string): Promise<void> {
  return invoke<void>("set_player_name", { playerName });
}

/** Удалить установленную игру (миры/настройки игрока сохраняются). */
export function uninstallGame(installDir: string): Promise<void> {
  return invoke<void>("uninstall_game", { installDir });
}

/** Системный диалог подтверждения (да/нет). */
export function confirmAction(message: string, title?: string): Promise<boolean> {
  return confirmDialog(message, { title, kind: "warning" });
}

/** Проверить выбранный путь установки (включая права на запись). */
export function validateInstallPath(path: string): Promise<PathValidation> {
  return invoke<PathValidation>("validate_install_path", { path });
}

/** Установлена ли уже игра в указанной папке (JRE + client.jar на месте). */
export function isGameInstalled(installDir: string): Promise<boolean> {
  return invoke<boolean>("is_game_installed", { installDir });
}

/** Синхронизировать файлы игры в указанную папку. */
export function syncFiles(installDir: string): Promise<SyncSummary> {
  return invoke<SyncSummary>("sync_files", { installDir });
}

/** Установить игру без запуска: ваниль + JRE + файлы. */
export function installGame(installDir: string): Promise<void> {
  return invoke<void>("install_game", { installDir });
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

/** Событие «игра закрыта» (install.rs::GAME_EXITED_EVENT). */
export const GAME_EXITED_EVENT = "game://exited";

/** Подписаться на закрытие игры (бэкенд снова показывает окно лаунчера). */
export function onGameExited(cb: () => void): Promise<UnlistenFn> {
  return listen(GAME_EXITED_EVENT, () => cb());
}
