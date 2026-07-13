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
use crate::{auth, config, download, java, launch, manifest, settings, state, vanilla};

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
        manifest::FileKind::Shaderpack => "шейдеры",
        manifest::FileKind::Unknown => "файлы Kingdom RP",
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
    progress.add_total(manifest.client_files().map(|f| f.size).sum());
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
    // Клиентский потребитель: качаем только client+both (server-only пропускаем).
    let client_files: Vec<&manifest::FileEntry> = manifest.client_files().collect();
    let total = client_files.len();
    let mut downloaded = 0usize;
    let mut skipped = 0usize;

    for entry in &client_files {
        // Путь из манифеста — через safe_join (защита от path traversal).
        let dest = crate::paths::safe_join(install_dir, &entry.path)?;
        progress.set_label(friendly_label(entry.kind, verb));

        let did = download::ensure_file(
            client,
            &entry.url,
            Some(&entry.path),
            &dest,
            &entry.sha256,
            progress.file_cb(),
        )
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
    // Разрешённые в mods/ — только клиентские файлы манифеста (server-only сюда
    // и не качаются, поэтому в allowed их не включаем).
    let allowed: HashSet<String> = manifest
        .client_files()
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
    // Объём файлов манифеста (NeoForge + моды) знаем заранее — только клиентские.
    progress.add_total(manifest.client_files().map(|f| f.size).sum());
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

    // Прописать наш сервер в список мультиплеера (servers.dat).
    ensure_server_entry(install_dir);

    Ok(java_exe)
}

/// Добавить наш сервер (`config::SERVER_ADDR`) в `servers.dat` (несжатый NBT —
/// список сохранённых серверов мультиплеера), если его там ещё нет. Чужие
/// сервера игрока сохраняются как есть (парсим через `fastnbt::Value`). Наш
/// entry ставим первым. Форс-добавление при каждом sync: удалил в игре — вернём.
fn ensure_server_entry(install_dir: &Path) {
    use fastnbt::Value;
    use std::collections::HashMap;

    let path = install_dir.join("servers.dat");
    let mut map: HashMap<String, Value> = std::fs::read(&path)
        .ok()
        .and_then(|b| fastnbt::from_bytes::<Value>(&b).ok())
        .and_then(|v| match v {
            Value::Compound(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default();

    let mut servers: Vec<Value> = match map.remove("servers") {
        Some(Value::List(l)) => l,
        _ => Vec::new(),
    };

    let present = servers.iter().any(|e| {
        matches!(e, Value::Compound(c)
            if matches!(c.get("ip"), Some(Value::String(ip)) if ip == config::SERVER_ADDR))
    });

    if present {
        return; // уже есть — ничего не пишем
    }

    let mut entry = HashMap::new();
    entry.insert("name".to_string(), Value::String(config::SERVER_NAME.to_string()));
    entry.insert("ip".to_string(), Value::String(config::SERVER_ADDR.to_string()));
    servers.insert(0, Value::Compound(entry));
    map.insert("servers".to_string(), Value::List(servers));

    match fastnbt::to_bytes(&Value::Compound(map)) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(&path, bytes) {
                log::warn!("servers.dat: не записать: {e}");
            } else {
                log::info!("servers.dat: прописан сервер {}", config::SERVER_ADDR);
            }
        }
        Err(e) => log::warn!("servers.dat: не сериализовать NBT: {e}"),
    }
}

/// Событие «игра завершилась» — фронтенд по нему сбрасывает состояние, а окно
/// лаунчера показывается обратно (см. [`play`]).
pub const GAME_EXITED_EVENT: &str = "game://exited";

/// Записать дефолтный `options.txt` (язык — русский, онбординг Narrator'а
/// выключен, масштаб интерфейса = 2, полноэкранный режим), но ТОЛЬКО если файла
/// ещё нет — чтобы не
/// затирать настройки игрока. Minecraft при старте дочитает остальные ключи
/// значениями по умолчанию.
fn ensure_default_options(install_dir: &Path) {
    let path = install_dir.join("options.txt");
    if path.exists() {
        return;
    }
    let content = "lang:ru_ru\nonboardAccessibility:false\nguiScale:2\nfullscreen:true\n";
    if let Err(e) = std::fs::write(&path, content) {
        log::warn!("не удалось записать options.txt: {e}");
    } else {
        log::info!("создан дефолтный options.txt (lang:ru_ru, guiScale:2)");
    }
}

/// Скачать манифест с несколькими попытками (GitHub Pages бывает недоступен /
/// отдаёт 404 транзиентно). `None`, если все попытки провалились.
async fn fetch_manifest_retry(client: &reqwest::Client) -> Option<manifest::Manifest> {
    const ATTEMPTS: u32 = 3;
    for i in 0..ATTEMPTS {
        match manifest::fetch_manifest(client, &config::manifest_url()).await {
            Ok(m) => return Some(m),
            Err(e) => {
                log::warn!("fetch_manifest: попытка {}/{ATTEMPTS} не удалась: {e}", i + 1);
                if i + 1 < ATTEMPTS {
                    tokio::time::sleep(std::time::Duration::from_millis(600 * (i as u64 + 1))).await;
                }
            }
        }
    }
    None
}

/// Слепок актуален: версия совпадает и все клиентские файлы на месте с тем же
/// `size`+`mtime` (проверка только `stat`, без чтения/хеширования). Так удаление/
/// правка мода ловятся, а неизменённая установка запускается без ресинка.
fn state_up_to_date(install_dir: &Path, manifest: &manifest::Manifest) -> bool {
    let Some(st) = state::load(install_dir) else {
        return false;
    };
    if st.version != manifest.version {
        return false;
    }
    for f in manifest.client_files() {
        let Ok(dest) = crate::paths::safe_join(install_dir, &f.path) else {
            return false;
        };
        match (state::mark(&dest), st.files.get(&f.path)) {
            (Some(m), Some(c)) if &m == c => {}
            _ => return false, // отсутствует или изменён
        }
    }
    true
}

/// Записать слепок установки (версия + профиль/injector + size/mtime файлов).
fn write_state(install_dir: &Path, manifest: &manifest::Manifest) {
    let mut files = std::collections::HashMap::new();
    for f in manifest.client_files() {
        if let Ok(dest) = crate::paths::safe_join(install_dir, &f.path) {
            if let Some(mk) = state::mark(&dest) {
                files.insert(f.path.clone(), mk);
            }
        }
    }
    state::save(
        install_dir,
        &state::InstallState {
            version: manifest.version.clone(),
            neoforge_profile: manifest.neoforge_profile.clone(),
            authlib_injector: manifest.authlib_injector.clone(),
            files,
        },
    );
}

/// Установить игру без запуска: ваниль → JRE → файлы NeoForge/моды.
pub async fn install_only(
    app: &AppHandle,
    client: &reqwest::Client,
    install_dir: PathBuf,
) -> Result<()> {
    let manifest = fetch_manifest_retry(client).await.ok_or_else(|| {
        LauncherError::Other(
            "Не удалось получить manifest.json — источник обновлений (GitHub) недоступен. \
             Попробуйте позже."
                .into(),
        )
    })?;
    log::info!(
        "install: манифест v{} (MC {}, NeoForge {}), файлов: {}",
        manifest.version,
        manifest.minecraft,
        manifest.neoforge,
        manifest.files.len()
    );
    let progress = Arc::new(Progress::new(app.clone()));
    sync_all(client, &install_dir, &manifest, &progress, "Устанавливаем").await?;
    write_state(&install_dir, &manifest);
    Ok(())
}

/// Принудительная полная проверка/восстановление файлов (хеш всех, докачка
/// битых) — для кнопки «Проверить файлы» (страховка от bit-rot).
pub async fn verify_files(
    app: &AppHandle,
    client: &reqwest::Client,
    install_dir: PathBuf,
) -> Result<()> {
    let manifest = fetch_manifest_retry(client).await.ok_or_else(|| {
        LauncherError::Other("Источник обновлений недоступен. Попробуйте позже.".into())
    })?;
    let progress = Arc::new(Progress::new(app.clone()));
    sync_all(client, &install_dir, &manifest, &progress, "Проверяем").await?;
    write_state(&install_dir, &manifest);
    Ok(())
}

/// Общая финальная часть «Играть»: авторизация (drasl) + спавн игры + скрытие/
/// показ окна лаунчера. `injector_rel` — путь к authlib-injector (из манифеста
/// или из слепка при офлайн-запуске); `None` → офлайн-режим авторизации.
async fn launch_flow(
    app: &AppHandle,
    client: &reqwest::Client,
    install_dir: PathBuf,
    profile: String,
    injector_rel: Option<String>,
    java_exe: PathBuf,
    player_name: String,
) -> Result<u32> {
    let base = settings::auth_base_url(app);
    let online_data: Option<(String, String, String, PathBuf, String)> =
        match settings::load(app).account {
            Some(mut account) => match injector_rel.as_deref() {
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
                    log::warn!("play: нет authlib_injector — запуск офлайн");
                    None
                }
            },
            None => None,
        };

    let jvm_prefix = settings::jvm_args(app);

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
            &jvm_prefix,
        )
    })
    .await
    .map_err(|e| LauncherError::Other(format!("задача запуска прервана: {e}")))??;
    let pid = child.id();

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

/// Полный цикл «Играть». Логика проверки:
/// - манифест доступен + версия/файлы совпали со слепком → **быстрый запуск**
///   без ресинка (только stat, анти-чит prune, servers.dat);
/// - манифест доступен, но версия/файлы отличаются → полный sync + слепок;
/// - манифест НЕдоступен (GitHub лёг), но игра установлена → **офлайн-запуск**
///   на текущих файлах (профиль/injector из слепка) — не блокируем игрока.
pub async fn play(
    app: &AppHandle,
    client: &reqwest::Client,
    install_dir: PathBuf,
    player_name: String,
) -> Result<u32> {
    match fetch_manifest_retry(client).await {
        Some(manifest) => {
            log::info!(
                "play: манифест v{} (MC {}, NeoForge {}), файлов: {}",
                manifest.version,
                manifest.minecraft,
                manifest.neoforge,
                manifest.files.len()
            );

            let java_exe = if is_installed(&install_dir) && state_up_to_date(&install_dir, &manifest)
            {
                log::info!(
                    "play: версия {} без изменений — быстрый запуск (без ресинка)",
                    manifest.version
                );
                // Манифест есть — держим анти-чит и мелочи актуальными (дёшево).
                let _ = prune_mods(&install_dir, &manifest);
                ensure_default_options(&install_dir);
                ensure_server_entry(&install_dir);
                java::java_exe_path(&install_dir, "runtime")
            } else {
                let progress = Arc::new(Progress::new(app.clone()));
                let je = sync_all(client, &install_dir, &manifest, &progress, "Проверяем").await?;
                write_state(&install_dir, &manifest);
                je
            };

            let profile = manifest
                .neoforge_profile
                .clone()
                .ok_or_else(|| LauncherError::Other("в манифесте нет neoforge_profile".into()))?;
            launch_flow(
                app,
                client,
                install_dir,
                profile,
                manifest.authlib_injector.clone(),
                java_exe,
                player_name,
            )
            .await
        }
        None => {
            // GitHub недоступен — не блокируем, если игра стоит (fail-open).
            if !is_installed(&install_dir) {
                return Err(LauncherError::Other(
                    "Источник обновлений недоступен, а игра ещё не установлена. Попробуйте позже.".into(),
                ));
            }
            let st = state::load(&install_dir).ok_or_else(|| {
                LauncherError::Other(
                    "Источник недоступен и нет локального слепка игры — запустите с интернетом хотя бы раз.".into(),
                )
            })?;
            let profile = st
                .neoforge_profile
                .clone()
                .ok_or_else(|| LauncherError::Other("нет neoforge_profile в слепке".into()))?;
            log::warn!(
                "play: источник недоступен — офлайн-запуск на текущих файлах (v{})",
                st.version
            );
            let java_exe = java::java_exe_path(&install_dir, "runtime");
            launch_flow(app, client, install_dir, profile, st.authlib_injector.clone(), java_exe, player_name).await
        }
    }
}
