//! Оркестрация установки и запуска (способ Б).
//!
//! Тянет манифест, обеспечивает ваниль (Mojang) + JRE + файлы NeoForge/моды и
//! при необходимости запускает игру. Весь прогресс считается единым трекером
//! [`Progress`] (общий объём, скачано, скорость) и уходит во фронтенд событием
//! [`crate::progress::PROGRESS_EVENT`].

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::error::{LauncherError, Result};
use crate::progress::Progress;
use crate::{auth, config, download, java, launch, manifest, settings, vanilla};

/// Итог синхронизации файлов манифеста.
#[derive(Debug, Clone, Serialize)]
pub struct SyncSummary {
    pub total: usize,
    pub downloaded: usize,
    pub skipped: usize,
}

/// Понятная игроку подпись этапа по типу файла (для UI-прогресса). `verb` —
/// «Устанавливаем» при первой установке или «Проверяем» при запуске уже
/// установленной игры (тогда это сверка/докачка, а не установка).
fn friendly_label(kind: manifest::FileKind, verb: &str) -> String {
    let noun = match kind {
        manifest::FileKind::Mod => "моды Kingdom RP",
        manifest::FileKind::Library | manifest::FileKind::Client => "NeoForge",
        manifest::FileKind::Config => "конфигурацию",
        manifest::FileKind::Asset => "ресурсы",
    };
    format!("{verb} {noun}")
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
/// манифесте нет записи под платформу.
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
    let progress = Progress::new(app.clone());
    progress.add_total(entry.size);
    let java_exe = java::ensure_java(client, &install_dir, entry, &progress, "Устанавливаем").await?;
    Ok(Some(java_exe.to_string_lossy().into_owned()))
}

/// Обеспечить ванильные файлы Minecraft (с Mojang CDN) для целевой версии.
pub async fn ensure_vanilla(
    app: &AppHandle,
    client: &reqwest::Client,
    install_dir: PathBuf,
) -> Result<()> {
    let progress = Progress::new(app.clone());
    vanilla::ensure_vanilla(client, &install_dir, config::MINECRAFT_VERSION, &progress, "Устанавливаем")
        .await
}

/// Синхронизировать все файлы манифеста в `install_dir` (сам тянет манифест).
pub async fn sync_files(
    app: &AppHandle,
    client: &reqwest::Client,
    install_dir: PathBuf,
) -> Result<SyncSummary> {
    let manifest = manifest::fetch_manifest(client, &config::manifest_url()).await?;
    let progress = Progress::new(app.clone());
    progress.add_total(manifest.files.iter().map(|f| f.size).sum());
    sync_manifest(client, &install_dir, &manifest, &progress, "Проверяем").await
}

/// Докачать файлы уже полученного манифеста, отчитываясь в общий трекер.
/// Размеры файлов должны быть уже добавлены в `progress.add_total` вызывающим.
/// `verb` — «Устанавливаем» (первая установка) или «Проверяем» (запуск).
pub async fn sync_manifest(
    client: &reqwest::Client,
    install_dir: &Path,
    manifest: &manifest::Manifest,
    progress: &Progress,
    verb: &str,
) -> Result<SyncSummary> {
    let total = manifest.files.len();
    let mut downloaded = 0usize;
    let mut skipped = 0usize;

    for entry in &manifest.files {
        // Путь из манифеста — через safe_join (защита от path traversal).
        let dest = crate::paths::safe_join(install_dir, &entry.path)?;
        progress.set_label(friendly_label(entry.kind, verb));

        let did = download::ensure_file(client, &entry.url, &dest, &entry.sha256, progress.file_cb())
            .await?;

        if did {
            downloaded += 1;
        } else {
            progress.add_skipped(entry.size);
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

/// Удалить из папки `mods` всё, чего нет в манифесте — посторонние/читерские
/// моды, которые игрок мог подложить вручную перед запуском. Файлы из манифеста
/// остаются (их целостность уже гарантирована sync по SHA-256). Возвращает
/// число удалённых файлов.
fn prune_mods(install_dir: &Path, manifest: &manifest::Manifest) -> Result<usize> {
    let allowed: HashSet<String> = manifest
        .files
        .iter()
        .map(|f| f.path.replace('\\', "/").to_lowercase())
        .collect();

    let mods_dir = install_dir.join("mods");
    if !mods_dir.exists() {
        return Ok(0);
    }

    let mut removed = 0usize;
    let mut stack = vec![mods_dir];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path
                .strip_prefix(install_dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/")
                .to_lowercase();
            if !allowed.contains(&rel) && std::fs::remove_file(&path).is_ok() {
                log::warn!("prune: удалён посторонний файл в mods: {}", path.display());
                removed += 1;
            }
        }
    }
    Ok(removed)
}

/// Общие шаги установки (ваниль → JRE → файлы NeoForge/моды). Возвращает путь к
/// `java`. Прогресс — в общий трекер `progress`.
async fn sync_all(
    client: &reqwest::Client,
    install_dir: &Path,
    manifest: &manifest::Manifest,
    progress: &Progress,
    verb: &str,
) -> Result<PathBuf> {
    // Объём файлов манифеста (NeoForge + моды) знаем заранее.
    progress.add_total(manifest.files.iter().map(|f| f.size).sum());
    // Объём JRE.
    let java_entry = manifest.java.get(java::platform_key()).ok_or_else(|| {
        LauncherError::Other(format!(
            "в манифесте нет JRE для платформы {}",
            java::platform_key()
        ))
    })?;
    progress.add_total(java_entry.size);

    // 1. Ваниль с Mojang (client.jar + библиотеки + ассеты). Сама добавит свой
    //    объём в общий трекер.
    log::info!("install: [1/3] ваниль с Mojang ({})", config::MINECRAFT_VERSION);
    vanilla::ensure_vanilla(client, install_dir, config::MINECRAFT_VERSION, progress, verb).await?;

    // 2. JRE из манифеста.
    log::info!("install: [2/3] JRE ({})", java::platform_key());
    let java_exe = java::ensure_java(client, install_dir, java_entry, progress, verb).await?;

    // 3. Файлы NeoForge + моды.
    log::info!("install: [3/3] синхронизация файлов NeoForge + моды");
    sync_manifest(client, install_dir, manifest, progress, verb).await?;

    // Анти-чит: убираем из mods всё, чего нет в манифесте.
    let pruned = prune_mods(install_dir, manifest)?;
    if pruned > 0 {
        log::info!("install: удалено посторонних файлов из mods: {pruned}");
    }

    // Дефолтные настройки игры при первом запуске (язык RU, без онбординга
    // Narrator'а) — только если игрок ещё не создавал свой options.txt.
    ensure_default_options(install_dir);

    Ok(java_exe)
}

/// Событие «игра завершилась» — фронтенд по нему сбрасывает состояние, а окно
/// лаунчера показывается обратно (см. [`play`]).
pub const GAME_EXITED_EVENT: &str = "game://exited";

/// Записать дефолтный `options.txt` (язык — русский, онбординг Narrator'а
/// выключен, масштаб интерфейса = 2), но ТОЛЬКО если файла ещё нет — чтобы не
/// затирать настройки игрока. Minecraft при старте дочитает остальные ключи
/// значениями по умолчанию.
fn ensure_default_options(install_dir: &Path) {
    let path = install_dir.join("options.txt");
    if path.exists() {
        return;
    }
    let content = "lang:ru_ru\nonboardAccessibility:false\nguiScale:2\n";
    if let Err(e) = std::fs::write(&path, content) {
        log::warn!("не удалось записать options.txt: {e}");
    } else {
        log::info!("создан дефолтный options.txt (lang:ru_ru, guiScale:2)");
    }
}

/// Установить игру без запуска: ваниль → JRE → файлы NeoForge/моды.
pub async fn install_only(
    app: &AppHandle,
    client: &reqwest::Client,
    install_dir: PathBuf,
) -> Result<()> {
    let manifest = manifest::fetch_manifest(client, &config::manifest_url()).await?;
    log::info!(
        "install: манифест v{} (MC {}, NeoForge {}), файлов: {}",
        manifest.version,
        manifest.minecraft,
        manifest.neoforge,
        manifest.files.len()
    );
    let progress = Arc::new(Progress::new(app.clone()));
    sync_all(client, &install_dir, &manifest, &progress, "Устанавливаем").await?;
    Ok(())
}

/// Полный цикл «Играть»: установка (ваниль → JRE → файлы) → запуск.
/// Возвращает PID процесса игры.
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

    let progress = Arc::new(Progress::new(app.clone()));
    // Игра уже установлена (кнопка «Играть») — это сверка/докачка, не установка.
    let java_exe = sync_all(client, &install_dir, &manifest, &progress, "Проверяем").await?;

    // Запуск игры.
    log::info!("play: запуск игры");
    let profile = manifest
        .neoforge_profile
        .clone()
        .ok_or_else(|| LauncherError::Other("в манифесте нет neoforge_profile".into()))?;

    // Авторизация: если игрок вошёл (drasl) и в манифесте есть authlib-injector —
    // запускаем онлайн (реальные токены + javaagent); иначе офлайн.
    let base = settings::auth_base_url(app);
    let online_data: Option<(String, String, String, PathBuf, String)> =
        match settings::load(app).account {
            Some(mut account) => match manifest.authlib_injector.as_deref() {
                Some(rel) => {
                    auth::ensure_session(client, &base, &mut account).await?;
                    let _ = settings::set_account(app, Some(account.clone()));
                    let injector = crate::paths::safe_join(&install_dir, rel)?;
                    let api_url = format!("{}/authlib-injector", base.trim_end_matches('/'));
                    log::info!("play: онлайн-запуск как '{}'", account.player_name);
                    Some((
                        account.player_name,
                        account.mc_uuid,
                        account.access_token,
                        injector,
                        api_url,
                    ))
                }
                None => {
                    log::warn!("play: в манифесте нет authlib_injector — запуск офлайн");
                    None
                }
            },
            None => None,
        };

    // launch блокирует поток (спавн + короткое ожидание раннего краха) —
    // уносим в blocking-пул, чтобы не вешать async-исполнитель.
    let child = tokio::task::spawn_blocking(move || {
        let online = online_data.as_ref().map(|(u, uuid, token, inj, api)| {
            launch::OnlineAuth {
                username: u,
                uuid,
                access_token: token,
                injector_jar: inj.as_path(),
                api_url: api,
            }
        });
        launch::launch(
            &install_dir,
            config::MINECRAFT_VERSION,
            &profile,
            &java_exe,
            &player_name,
            online.as_ref(),
        )
    })
    .await
    .map_err(|e| LauncherError::Other(format!("задача запуска прервана: {e}")))??;
    let pid = child.id();

    // Прячем окно лаунчера на время игры и показываем обратно, когда игра
    // закроется (плюс шлём событие, чтобы фронтенд сбросил состояние).
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }
    let app_waiter = app.clone();
    let mut child = child;
    tokio::task::spawn_blocking(move || {
        let _ = child.wait();
        log::info!("play: игра закрыта (pid={pid}) — показываю лаунчер");
        let _ = app_waiter.emit(GAME_EXITED_EVENT, ());
        if let Some(win) = app_waiter.get_webview_window("main") {
            let _ = win.unminimize();
            let _ = win.show();
            let _ = win.set_focus();
        }
    });
    Ok(pid)
}
