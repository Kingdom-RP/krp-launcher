mod config;
mod download;
mod error;
mod install;
mod java;
mod launch;
mod manifest;
mod paths;
mod vanilla;

use std::path::{Path, PathBuf};

use error::Result;
use install::SyncSummary;
use manifest::Manifest;
use paths::PathValidation;

/// Демо-команда из шаблона — пока оставлена, чтобы стартовый UI работал.
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

/// Папка установки по умолчанию (`%APPDATA%\KingdomRP`).
#[tauri::command]
fn default_install_dir() -> Result<String> {
    Ok(paths::default_install_dir()?
        .to_string_lossy()
        .into_owned())
}

/// Проверить выбранный игроком путь установки (включая права на запись).
#[tauri::command]
async fn validate_install_path(path: String) -> PathValidation {
    paths::validate_install_dir_full(Path::new(&path)).await
}

/// Скачать и вернуть манифест профиля.
#[tauri::command]
async fn get_manifest(client: tauri::State<'_, reqwest::Client>) -> Result<Manifest> {
    manifest::fetch_manifest(client.inner(), &config::manifest_url()).await
}

/// Убедиться, что JRE установлена (скачать/распаковать при необходимости).
/// Возвращает путь к `java`-исполняемому или `null`, если в манифесте нет
/// записи под текущую платформу.
#[tauri::command]
async fn ensure_java(
    app: tauri::AppHandle,
    client: tauri::State<'_, reqwest::Client>,
    install_dir: String,
) -> Result<Option<String>> {
    install::ensure_java(&app, client.inner(), PathBuf::from(install_dir))
        .await
        .inspect_err(|e| log::error!("ensure_java: ошибка: {e}"))
}

/// Обеспечить ванильные файлы Minecraft (client.jar, библиотеки, ассеты) с
/// Mojang CDN. Прогресс — событием [`install::PROGRESS_EVENT`].
#[tauri::command]
async fn ensure_vanilla(
    app: tauri::AppHandle,
    client: tauri::State<'_, reqwest::Client>,
    install_dir: String,
) -> Result<()> {
    install::ensure_vanilla(&app, client.inner(), PathBuf::from(install_dir))
        .await
        .inspect_err(|e| log::error!("ensure_vanilla: ошибка: {e}"))
}

/// Синхронизировать все файлы игры в указанную папку. Прогресс приходит во
/// фронтенд событием [`install::PROGRESS_EVENT`].
#[tauri::command]
async fn sync_files(
    app: tauri::AppHandle,
    client: tauri::State<'_, reqwest::Client>,
    install_dir: String,
) -> Result<SyncSummary> {
    install::sync_files(&app, client.inner(), PathBuf::from(install_dir))
        .await
        .inspect_err(|e| log::error!("sync_files: ошибка: {e}"))
}

/// Полный цикл «Играть»: ваниль (Mojang) → JRE → файлы манифеста → запуск.
/// Возвращает PID. Прогресс — событием `sync://progress`.
#[tauri::command]
async fn play(
    app: tauri::AppHandle,
    client: tauri::State<'_, reqwest::Client>,
    install_dir: String,
    player_name: String,
) -> Result<u32> {
    log::info!("play: установка и запуск для '{player_name}' в {install_dir}");
    install::play(&app, client.inner(), PathBuf::from(install_dir), player_name)
        .await
        .inspect(|pid| log::info!("play: игра запущена, pid={pid}"))
        .inspect_err(|e| log::error!("play: ошибка: {e}"))
}

/// Запустить игру (офлайн-режим). Возвращает PID процесса.
/// `neoforge_profile` — из манифеста (`Manifest::neoforge_profile`),
/// `java_exe` — путь к java (из `ensure_java`).
#[tauri::command]
async fn launch_game(
    install_dir: String,
    neoforge_profile: String,
    java_exe: String,
    player_name: String,
) -> Result<u32> {
    launch::launch(
        std::path::Path::new(&install_dir),
        config::MINECRAFT_VERSION,
        &neoforge_profile,
        std::path::Path::new(&java_exe),
        &player_name,
    )
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: Some("krp-launcher".into()),
                    }),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Webview),
                ])
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(reqwest::Client::new())
        .invoke_handler(tauri::generate_handler![
            greet,
            default_install_dir,
            validate_install_path,
            get_manifest,
            ensure_java,
            ensure_vanilla,
            sync_files,
            launch_game,
            play,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
