//! Формат манифеста (способ Б) и его загрузка.
//!
//! Манифест — единый список всего, что нужно скачать игроку: ваниль + NeoForge
//! + моды + конфиги. Лаунчер сверяет SHA-256 и качает только изменённое.
//! Пример формата — в `docs/manifest.example.json`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Корень `manifest.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Версия профиля/модпака целиком (semver). Используется для проверки
    /// обновлений.
    pub version: String,
    /// Целевая версия Minecraft, например `"1.20.1"`.
    pub minecraft: String,
    /// Целевая версия NeoForge.
    pub neoforge: String,
    /// Путь (относительно install_dir) к version JSON NeoForge — его читает
    /// построитель команды запуска. Напр. `versions/1.20.1-forge-47.1.106/1.20.1-forge-47.1.106.json`.
    #[serde(default)]
    pub neoforge_profile: Option<String>,
    /// JRE по платформам. Ключ — платформа, например `"windows-x64"`.
    #[serde(default)]
    pub java: HashMap<String, JavaEntry>,
    /// Все файлы игры относительно папки установки.
    #[serde(default)]
    pub files: Vec<FileEntry>,
}

/// Описание JRE для конкретной платформы.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JavaEntry {
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
    /// Папка (относительно установки) для распаковки JRE, например `"runtime"`.
    pub dir: String,
}

/// Один файл игры.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// Путь назначения относительно папки установки, через прямой слэш
    /// (`mods/kingdomrp-core-1.0.0.jar`).
    pub path: String,
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub kind: FileKind,
}

/// Категория файла — пригодится для UI и логики запуска.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    #[default]
    Mod,
    Library,
    Config,
    Asset,
    Client,
}

/// Скачать и разобрать манифест по URL.
pub async fn fetch_manifest(client: &reqwest::Client, url: &str) -> Result<Manifest> {
    let manifest = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json::<Manifest>()
        .await?;
    Ok(manifest)
}
