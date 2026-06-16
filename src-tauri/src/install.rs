//! Оркестрация синхронизации файлов по манифесту (способ Б).
//!
//! Тянет манифест, проходит по списку файлов и докачивает только изменённые,
//! эмитя прогресс во фронтенд через событие [`PROGRESS_EVENT`].

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::error::{LauncherError, Result};
use crate::{config, download, java, launch, manifest, vanilla};

/// Имя Tauri-события прогресса (фронтенд слушает его через `listen`).
pub const PROGRESS_EVENT: &str = "sync://progress";

/// Прогресс по текущему файлу.
#[derive(Debug, Clone, Serialize)]
pub struct SyncProgress {
    /// Индекс текущего файла (0-based).
    pub index: usize,
    /// Всего файлов в манифесте.
    pub total: usize,
    /// Путь текущего файла (как в манифесте).
    pub file: String,
    /// Скачано байт текущего файла.
    pub downloaded: u64,
    /// Полный размер файла, если известен.
    pub total_bytes: Option<u64>,
}

/// Итог синхронизации.
#[derive(Debug, Clone, Serialize)]
pub struct SyncSummary {
    pub total: usize,
    pub downloaded: usize,
    pub skipped: usize,
}

/// Убедиться, что JRE установлена (по секции `java` манифеста для текущей
/// платформы). Возвращает путь к `java`-исполняемому или `None`, если в
/// манифесте нет записи под платформу. Прогресс идёт тем же событием с
/// `file = "Java Runtime"`.
pub async fn ensure_java(
    app: &AppHandle,
    client: &reqwest::Client,
    install_dir: PathBuf,
) -> Result<Option<String>> {
    let manifest = manifest::fetch_manifest(client, &config::manifest_url()).await?;
    let key = java::platform_key();
    let Some(entry) = manifest.java.get(key) else {
        return Ok(None);
    };

    let java_exe = java::ensure_java(client, &install_dir, entry, |d, t| {
        let _ = app.emit(
            PROGRESS_EVENT,
            SyncProgress {
                index: 0,
                total: 1,
                file: "Java Runtime".into(),
                downloaded: d,
                total_bytes: t,
            },
        );
    })
    .await?;

    Ok(Some(java_exe.to_string_lossy().into_owned()))
}

/// Обеспечить ванильные файлы Minecraft (с Mojang CDN) для целевой версии.
/// Прогресс идёт тем же событием.
pub async fn ensure_vanilla(
    app: &AppHandle,
    client: &reqwest::Client,
    install_dir: PathBuf,
) -> Result<()> {
    vanilla::ensure_vanilla(
        client,
        &install_dir,
        config::MINECRAFT_VERSION,
        |index, total, name, downloaded, total_bytes| {
            let _ = app.emit(
                PROGRESS_EVENT,
                SyncProgress {
                    index,
                    total,
                    file: name.to_string(),
                    downloaded,
                    total_bytes,
                },
            );
        },
    )
    .await
}

/// Синхронизировать все файлы манифеста в `install_dir` (сам тянет манифест).
pub async fn sync_files(
    app: &AppHandle,
    client: &reqwest::Client,
    install_dir: PathBuf,
) -> Result<SyncSummary> {
    let manifest = manifest::fetch_manifest(client, &config::manifest_url()).await?;
    sync_manifest(app, client, &install_dir, &manifest).await
}

/// Докачать файлы уже полученного манифеста.
pub async fn sync_manifest(
    app: &AppHandle,
    client: &reqwest::Client,
    install_dir: &Path,
    manifest: &manifest::Manifest,
) -> Result<SyncSummary> {
    let total = manifest.files.len();
    let mut downloaded = 0usize;
    let mut skipped = 0usize;

    for (index, entry) in manifest.files.iter().enumerate() {
        // В манифесте пути через `/` — переводим в разделитель ОС.
        let rel = entry.path.replace('/', std::path::MAIN_SEPARATOR_STR);
        let dest = install_dir.join(rel);
        let file = entry.path.clone();

        let did = download::ensure_file(client, &entry.url, &dest, &entry.sha256, |d, t| {
            let _ = app.emit(
                PROGRESS_EVENT,
                SyncProgress {
                    index,
                    total,
                    file: file.clone(),
                    downloaded: d,
                    total_bytes: t,
                },
            );
        })
        .await?;

        if did {
            downloaded += 1;
        } else {
            skipped += 1;
        }
    }

    Ok(SyncSummary {
        total,
        downloaded,
        skipped,
    })
}

/// Полный цикл «Играть»: ваниль (Mojang) → JRE → файлы манифеста → запуск.
/// Возвращает PID процесса игры. Прогресс — событием [`PROGRESS_EVENT`].
pub async fn play(
    app: &AppHandle,
    client: &reqwest::Client,
    install_dir: PathBuf,
    player_name: String,
) -> Result<u32> {
    let manifest = manifest::fetch_manifest(client, &config::manifest_url()).await?;

    // 1. Ваниль с Mojang (client.jar + библиотеки + ассеты).
    vanilla::ensure_vanilla(
        client,
        &install_dir,
        config::MINECRAFT_VERSION,
        |index, total, name, downloaded, total_bytes| {
            let _ = app.emit(
                PROGRESS_EVENT,
                SyncProgress {
                    index,
                    total,
                    file: format!("Ваниль: {name}"),
                    downloaded,
                    total_bytes,
                },
            );
        },
    )
    .await?;

    // 2. JRE из манифеста.
    let entry = manifest.java.get(java::platform_key()).ok_or_else(|| {
        LauncherError::Other(format!(
            "в манифесте нет JRE для платформы {}",
            java::platform_key()
        ))
    })?;
    let java_exe = java::ensure_java(client, &install_dir, entry, |d, t| {
        let _ = app.emit(
            PROGRESS_EVENT,
            SyncProgress {
                index: 0,
                total: 1,
                file: "Java Runtime".into(),
                downloaded: d,
                total_bytes: t,
            },
        );
    })
    .await?;

    // 3. Файлы NeoForge + моды.
    sync_manifest(app, client, &install_dir, &manifest).await?;

    // 4. Запуск игры.
    let profile = manifest
        .neoforge_profile
        .clone()
        .ok_or_else(|| LauncherError::Other("в манифесте нет neoforge_profile".into()))?;
    let pid = launch::launch(
        &install_dir,
        config::MINECRAFT_VERSION,
        &profile,
        &java_exe,
        &player_name,
    )?;
    Ok(pid)
}
