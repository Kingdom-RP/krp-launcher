//! Постоянные настройки лаунчера (в его собственной config-папке).
//!
//! Главное, что здесь хранится — путь установки игры. Без этого лаунчер при
//! каждом старте «забывал» выбранную папку и предлагал установить игру заново
//! (особенно после переустановки/обновления самого лаунчера). Файл лежит в
//! `app_config_dir`/`settings.json` — отдельно от файлов игры.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::error::{LauncherError, Result};

/// Сохраняемые настройки лаунчера.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    /// Папка, куда установлена игра (если игрок её выбирал/устанавливал).
    #[serde(default)]
    pub install_dir: Option<String>,
    /// Последний введённый никнейм игрока (офлайн-режим, до фазы 6).
    #[serde(default)]
    pub player_name: Option<String>,
    /// Авторизованный аккаунт (drasl), если игрок вошёл (фаза 6).
    #[serde(default)]
    pub account: Option<crate::auth::Account>,
    /// Необязательный оверрайд адреса auth-сервера (для теста без пересборки).
    #[serde(default)]
    pub auth_base_url: Option<String>,
    /// Выделяемая игре память (МБ). Нет — берётся [`crate::config::DEFAULT_MAX_MEMORY_MB`].
    #[serde(default)]
    pub max_memory_mb: Option<u32>,
}

fn settings_path(app: &AppHandle) -> Result<PathBuf> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| LauncherError::Other(format!("не определить config-папку: {e}")))?;
    Ok(dir.join("settings.json"))
}

/// Загрузить настройки (или значения по умолчанию, если файла нет/битый).
pub fn load(app: &AppHandle) -> Settings {
    let Ok(path) = settings_path(app) else {
        return Settings::default();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return Settings::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Сохранить настройки на диск.
pub fn save(app: &AppHandle, settings: &Settings) -> Result<()> {
    let path = settings_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(settings)?;
    std::fs::write(&path, json)?;
    Ok(())
}

/// Запомнить путь установки игры.
pub fn set_install_dir(app: &AppHandle, dir: Option<String>) -> Result<()> {
    let mut s = load(app);
    s.install_dir = dir;
    save(app, &s)
}

/// Запомнить никнейм игрока.
pub fn set_player_name(app: &AppHandle, name: Option<String>) -> Result<()> {
    let mut s = load(app);
    s.player_name = name;
    save(app, &s)
}

/// Сохранить/очистить авторизованный аккаунт.
pub fn set_account(app: &AppHandle, account: Option<crate::auth::Account>) -> Result<()> {
    let mut s = load(app);
    s.account = account;
    save(app, &s)
}

/// Память игры (МБ): из настроек или дефолт, с зажимом в допустимые границы.
pub fn max_memory_mb(app: &AppHandle) -> u32 {
    let mb = load(app)
        .max_memory_mb
        .unwrap_or(crate::config::DEFAULT_MAX_MEMORY_MB);
    mb.clamp(crate::config::MIN_MEMORY_MB, crate::config::MAX_MEMORY_MB)
}

/// Запомнить память игры (МБ).
pub fn set_max_memory_mb(app: &AppHandle, mb: u32) -> Result<()> {
    let mut s = load(app);
    s.max_memory_mb = Some(mb.clamp(crate::config::MIN_MEMORY_MB, crate::config::MAX_MEMORY_MB));
    save(app, &s)
}

/// Адрес auth-сервера: оверрайд из настроек или константа из конфига.
pub fn auth_base_url(app: &AppHandle) -> String {
    load(app)
        .auth_base_url
        .unwrap_or_else(|| crate::config::AUTH_BASE_URL.to_string())
}
