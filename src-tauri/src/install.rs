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

/// Понятная игроку подпись этапа по типу файла (для UI-прогресса).
fn friendly_label(kind: manifest::FileKind) -> &'static str {
    match kind {
        manifest::FileKind::Mod => "Устанавливаем моды Kingdom RP",
        manifest::FileKind::Library | manifest::FileKind::Client => {
            "Устанавливаем NeoForge"
        }
        manifest::FileKind::Config => "Устанавливаем конфигурацию",
        manifest::FileKind::Asset => "Устанавливаем ресурсы",
    }
}

/// Грубая проверка «игра установлена»: распакованная JRE + ванильный client.jar
/// уже на диске. Без обращения к сети — нужна для подписи кнопки
/// «Играть» / «Установить» (см. фронтенд).
pub fn is_installed(install_dir: &Path) -> bool {
    let java_ok = java::java_exe_path(install_dir, "runtime").exists();
    let client_ok = install_dir
        .join("versions")
        .join(config::MINECRAFT_VERSION)
        .join(format!("{}.jar", config::MINECRAFT_VERSION))
        .exists();
    java_ok && client_ok
}

/// Каталоги, которыми управляет лаунчер (удаляются при деинсталляции).
/// Пользовательские данные (`saves`, `screenshots`, `resourcepacks`,
/// `options.txt`) НЕ трогаем — это миры и настройки игрока.
const MANAGED_DIRS: &[&str] = &[
    "runtime",
    "versions",
    "libraries",
    "assets",
    "natives",
    "mods",
    "logs",
    "crash-reports",
];

/// Удалить установленную игру из `install_dir`: сносит каталоги, которыми
/// управляет лаунчер, и временные файлы загрузок. Миры/настройки игрока
/// сохраняются. Саму папку установки не удаляем (в неё могли вложить игру
/// вручную в общей директории).
pub fn uninstall(install_dir: &Path) -> Result<()> {
    for name in MANAGED_DIRS {
        let p = install_dir.join(name);
        if p.exists() {
            std::fs::remove_dir_all(&p)
                .map_err(|e| LauncherError::Other(format!("не удалить {}: {e}", p.display())))?;
        }
    }
    // Хвосты прерванных загрузок (.part и архивы JRE *.download.*).
    if let Ok(entries) = std::fs::read_dir(install_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(".part") || name.contains(".download.") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    log::info!("uninstall: игра удалена из {}", install_dir.display());
    Ok(())
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
        let label = friendly_label(entry.kind);

        let did = download::ensure_file(client, &entry.url, &dest, &entry.sha256, |d, t| {
            let _ = app.emit(
                PROGRESS_EVENT,
                SyncProgress {
                    index,
                    total,
                    file: label.to_string(),
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

    log::info!("sync: всего {total}, скачано {downloaded}, без изменений {skipped}");
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
    log::info!(
        "play: манифест v{} (MC {}, NeoForge {}), файлов: {}",
        manifest.version,
        manifest.minecraft,
        manifest.neoforge,
        manifest.files.len()
    );

    // 1. Ваниль с Mojang (client.jar + библиотеки + ассеты).
    log::info!("play: [1/4] ваниль с Mojang ({})", config::MINECRAFT_VERSION);
    vanilla::ensure_vanilla(
        client,
        &install_dir,
        config::MINECRAFT_VERSION,
        |index, total, _name, downloaded, total_bytes| {
            let _ = app.emit(
                PROGRESS_EVENT,
                SyncProgress {
                    index,
                    total,
                    file: format!("Устанавливаем Minecraft {}", config::MINECRAFT_VERSION),
                    downloaded,
                    total_bytes,
                },
            );
        },
    )
    .await?;

    // 2. JRE из манифеста.
    log::info!("play: [2/4] JRE ({})", java::platform_key());
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
                file: "Устанавливаем Java".into(),
                downloaded: d,
                total_bytes: t,
            },
        );
    })
    .await?;

    // 3. Файлы NeoForge + моды.
    log::info!("play: [3/4] синхронизация файлов NeoForge + моды");
    sync_manifest(app, client, &install_dir, &manifest).await?;

    // 4. Запуск игры.
    log::info!("play: [4/4] запуск игры");
    let profile = manifest
        .neoforge_profile
        .clone()
        .ok_or_else(|| LauncherError::Other("в манифесте нет neoforge_profile".into()))?;
    // launch блокирует поток (спавн + короткое ожидание раннего краха) —
    // уносим в blocking-пул, чтобы не вешать async-исполнитель.
    let pid = tokio::task::spawn_blocking(move || {
        launch::launch(
            &install_dir,
            config::MINECRAFT_VERSION,
            &profile,
            &java_exe,
            &player_name,
        )
    })
    .await
    .map_err(|e| LauncherError::Other(format!("задача запуска прервана: {e}")))??;
    Ok(pid)
}
