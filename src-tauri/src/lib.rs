mod auth;
mod config;
mod download;
mod error;
mod install;
mod java;
mod launch;
mod manifest;
mod paths;
mod progress;
mod settings;
mod skin;
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

/// Папка установки для показа при старте: запомненная в настройках, иначе —
/// по умолчанию. Так лаунчер «помнит», куда игрок поставил игру, и не предлагает
/// переустановку (в т.ч. после обновления самого лаунчера).
#[tauri::command]
fn get_install_dir(app: tauri::AppHandle) -> Result<String> {
    if let Some(dir) = settings::load(&app).install_dir {
        return Ok(dir);
    }
    Ok(paths::default_install_dir()?
        .to_string_lossy()
        .into_owned())
}

/// Запомнить выбранную игроком папку установки.
#[tauri::command]
fn set_install_dir(app: tauri::AppHandle, install_dir: String) -> Result<()> {
    settings::set_install_dir(&app, Some(install_dir))
}

/// Привести выбранный каталог к папке установки (добавить `Kingdom RP`).
#[tauri::command]
fn resolve_install_dir(picked: String) -> String {
    paths::resolve_install_dir(Path::new(&picked))
        .to_string_lossy()
        .into_owned()
}

/// Запомненный никнейм игрока (пустая строка, если ещё не вводили).
#[tauri::command]
fn get_player_name(app: tauri::AppHandle) -> String {
    settings::load(&app).player_name.unwrap_or_default()
}

/// Запомнить никнейм игрока.
#[tauri::command]
fn set_player_name(app: tauri::AppHandle, player_name: String) -> Result<()> {
    let value = if player_name.is_empty() {
        None
    } else {
        Some(player_name)
    };
    settings::set_player_name(&app, value)
}

/// Текущий вошедший аккаунт (без секретов) + URL скина, или `null`.
#[tauri::command]
async fn auth_account(
    app: tauri::AppHandle,
    client: tauri::State<'_, reqwest::Client>,
) -> Result<Option<auth::AccountInfo>> {
    let Some(account) = settings::load(&app).account else {
        return Ok(None);
    };
    let base = settings::auth_base_url(&app);
    let skin = auth::skin_url(client.inner(), &base, &account).await.unwrap_or(None);
    Ok(Some(account.info(skin)))
}

/// Регистрация нового аккаунта (логин/пароль) → сохранение сессии.
#[tauri::command]
async fn auth_register(
    app: tauri::AppHandle,
    client: tauri::State<'_, reqwest::Client>,
    username: String,
    password: String,
) -> Result<auth::AccountInfo> {
    let base = settings::auth_base_url(&app);
    let account = auth::register(client.inner(), &base, &username, &password)
        .await
        .inspect_err(|e| log::error!("auth_register: {e}"))?;
    settings::set_account(&app, Some(account.clone()))?;
    Ok(account.info(None))
}

/// Вход существующего аккаунта → сохранение сессии.
#[tauri::command]
async fn auth_login(
    app: tauri::AppHandle,
    client: tauri::State<'_, reqwest::Client>,
    username: String,
    password: String,
) -> Result<auth::AccountInfo> {
    let base = settings::auth_base_url(&app);
    let account = auth::login(client.inner(), &base, &username, &password)
        .await
        .inspect_err(|e| log::error!("auth_login: {e}"))?;
    settings::set_account(&app, Some(account.clone()))?;
    Ok(account.info(None))
}

/// Выйти из аккаунта (забыть сессию).
#[tauri::command]
fn auth_logout(app: tauri::AppHandle) -> Result<()> {
    settings::set_account(&app, None)
}

/// Загрузить скин: проверяем PNG (64×64/64×32) и отправляем в drasl.
#[tauri::command]
async fn upload_skin(
    app: tauri::AppHandle,
    client: tauri::State<'_, reqwest::Client>,
    path: String,
    slim: bool,
) -> Result<()> {
    let bytes = std::fs::read(&path)
        .map_err(|e| error::LauncherError::Other(format!("не прочитать файл: {e}")))?;
    skin::validate_skin(&bytes)?; // отклоняем не-скины ещё до отправки
    let account = settings::load(&app)
        .account
        .ok_or_else(|| error::LauncherError::Other("сначала войдите в аккаунт".into()))?;
    let base = settings::auth_base_url(&app);
    auth::upload_skin(client.inner(), &base, &account, &bytes, slim)
        .await
        .inspect_err(|e| log::error!("upload_skin: {e}"))
}

/// Проверить, что выбранный PNG — корректная развёртка скина Minecraft
/// (64×64 или 64×32). Возвращает формат (`modern`/`legacy`) или ошибку с
/// понятным игроку текстом. Используется перед загрузкой скина на auth-сервер.
#[tauri::command]
fn validate_skin(path: String) -> Result<skin::SkinFormat> {
    skin::validate_skin_file(Path::new(&path))
}

fn png_data_url(bytes: &[u8]) -> String {
    use base64::Engine;
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// Локальный PNG-скин → data-URL для превью (с валидацией формата). Так
/// webview рисует картинку без проблем с доступом к файлу/CORS.
#[tauri::command]
fn skin_preview_file(path: String) -> Result<String> {
    let bytes = std::fs::read(&path)
        .map_err(|e| error::LauncherError::Other(format!("не прочитать файл: {e}")))?;
    skin::validate_skin(&bytes)?;
    Ok(png_data_url(&bytes))
}

/// Скачать скин по URL (с drasl) и вернуть data-URL — для превью текущего скина
/// без CORS-ограничений webview.
#[tauri::command]
async fn skin_preview_url(
    client: tauri::State<'_, reqwest::Client>,
    url: String,
) -> Result<String> {
    let bytes = client
        .inner()
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    Ok(png_data_url(&bytes))
}

/// Открыть папку в системном файловом менеджере (Explorer / xdg-open).
/// Свой обработчик надёжнее scope-ограничений plugin-opener для произвольных
/// путей вроде `E:\Games\Kingdom RP`.
#[tauri::command]
fn open_dir(path: String) -> Result<()> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(error::LauncherError::Other(format!(
            "папка не найдена: {path}"
        )));
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        // explorer возвращает ненулевой код даже при успехе — не проверяем статус.
        let _ = std::process::Command::new("explorer")
            .creation_flags(CREATE_NO_WINDOW)
            .arg(p)
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(p)
            .spawn()
            .map_err(|e| error::LauncherError::Other(format!("не открыть папку: {e}")))?;
    }
    Ok(())
}

/// Установлена ли игра в указанной папке (JRE + ванильный client.jar на месте).
/// Нужна фронтенду, чтобы подписать кнопку «Играть» или «Установить».
#[tauri::command]
fn is_game_installed(install_dir: String) -> bool {
    install::is_installed(Path::new(&install_dir))
}

/// Удалить установленную игру из папки и забыть путь в настройках.
/// Миры/настройки игрока сохраняются (см. [`install::uninstall`]).
#[tauri::command]
async fn uninstall_game(app: tauri::AppHandle, install_dir: String) -> Result<()> {
    let dir = install_dir.clone();
    tokio::task::spawn_blocking(move || install::uninstall(Path::new(&dir)))
        .await
        .map_err(|e| error::LauncherError::Other(format!("задача удаления прервана: {e}")))??;
    let _ = settings::set_install_dir(&app, None);
    Ok(())
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
/// Mojang CDN. Прогресс — событием `sync://progress`.
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
/// фронтенд событием `sync://progress`.
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

/// Установить игру без запуска: ваниль (Mojang) → JRE → файлы манифеста.
/// Прогресс — событием `sync://progress`. Запоминает путь установки.
#[tauri::command]
async fn install_game(
    app: tauri::AppHandle,
    client: tauri::State<'_, reqwest::Client>,
    install_dir: String,
) -> Result<()> {
    log::info!("install_game: установка в {install_dir}");
    let _ = settings::set_install_dir(&app, Some(install_dir.clone()));
    install::install_only(&app, client.inner(), PathBuf::from(install_dir))
        .await
        .inspect_err(|e| log::error!("install_game: ошибка: {e}"))
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
    // Запоминаем путь и ник сразу: даже если игра упадёт на старте, файлы уже
    // там, и при следующем запуске лаунчер не предложит ставить заново.
    let _ = settings::set_install_dir(&app, Some(install_dir.clone()));
    if !player_name.is_empty() {
        let _ = settings::set_player_name(&app, Some(player_name.clone()));
    }
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
    // launch блокирует поток (ожидание раннего краха) — в blocking-пул.
    // Возвращаем только PID; дочерний процесс отпускаем (игра живёт сама).
    tokio::task::spawn_blocking(move || {
        launch::launch(
            Path::new(&install_dir),
            config::MINECRAFT_VERSION,
            &neoforge_profile,
            Path::new(&java_exe),
            &player_name,
            None,
        )
        .map(|child| child.id())
    })
    .await
    .map_err(|e| error::LauncherError::Other(format!("задача запуска прервана: {e}")))?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Логируем паники самого лаунчера в общий лог (плагин log пишет в файл),
    // чтобы причина падения лаунчера не терялась.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "<неизвестно>".into());
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<без сообщения>".into());
        log::error!("PANIC лаунчера в {location}: {msg}");
        default_hook(info);
    }));

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
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(reqwest::Client::new())
        .invoke_handler(tauri::generate_handler![
            greet,
            default_install_dir,
            get_install_dir,
            set_install_dir,
            resolve_install_dir,
            get_player_name,
            set_player_name,
            open_dir,
            validate_skin,
            skin_preview_file,
            skin_preview_url,
            auth_account,
            auth_register,
            auth_login,
            auth_logout,
            upload_skin,
            is_game_installed,
            uninstall_game,
            validate_install_path,
            get_manifest,
            ensure_java,
            ensure_vanilla,
            sync_files,
            install_game,
            launch_game,
            play,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
