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
    /// Целевая версия Minecraft, например `"1.21.1"`.
    pub minecraft: String,
    /// Целевая версия NeoForge.
    pub neoforge: String,
    /// Путь (относительно install_dir) к version JSON NeoForge — его читает
    /// построитель команды запуска. Напр. `versions/neoforge-21.1.233/neoforge-21.1.233.json`.
    #[serde(default)]
    pub neoforge_profile: Option<String>,
    /// Путь (относительно install_dir) к `authlib-injector.jar` — Java-агент
    /// авторизации (фаза 6). Напр. `authlib-injector.jar`.
    #[serde(default)]
    pub authlib_injector: Option<String>,
    /// JRE по платформам. Ключ — платформа, например `"windows-x64"`.
    #[serde(default)]
    pub java: HashMap<String, JavaEntry>,
    /// Все файлы игры относительно папки установки.
    #[serde(default)]
    pub files: Vec<FileEntry>,
}

impl Manifest {
    /// Файлы, нужные клиенту (лаунчеру): client + both. Server-only исключаются.
    pub fn client_files(&self) -> impl Iterator<Item = &FileEntry> {
        self.files.iter().filter(|f| f.for_client())
    }
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
    /// Сторона файла. Лаунчер (клиент) берёт client+both; server-only пропускает.
    /// Старые манифесты без поля → both (обратная совместимость).
    #[serde(default)]
    pub side: Side,
}

impl FileEntry {
    /// Нужен ли файл клиенту (лаунчеру): всё, кроме чисто серверного.
    pub fn for_client(&self) -> bool {
        !matches!(self.side, Side::Server)
    }
}

/// Сторона файла в манифесте.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Client,
    Server,
    #[default]
    Both,
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
    Shaderpack,
    /// Неизвестный kind из более нового манифеста — чтобы старый лаунчер не падал
    /// на разборе (forward-compat). Файл всё равно качается по path/sha.
    #[serde(other)]
    Unknown,
}

/// Скачать и разобрать манифест по URL. Манифест маленький — ограничиваем
/// время запроса (источник на GitHub Pages с российских IP бывает недоступен).
///
/// Если задан [`crate::config::MANIFEST_PUBKEY`], манифест ДОЛЖЕН иметь валидную
/// minisign-подпись (`manifest.json.minisig` рядом на источнике) — иначе ошибка
/// (fail-closed, защита от подмены манифеста при компрометации источника/MITM).
/// Пустой ключ → проверка пропускается (доверие TLS+GitHub Pages).
pub async fn fetch_manifest(client: &reqwest::Client, url: &str) -> Result<Manifest> {
    let resp = client
        .get(url)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| {
            crate::error::LauncherError::Other(format!(
                "Не удалось получить manifest.json ({url}). Возможно, источник \
                 (GitHub) недоступен с вашего интернета. Подробности: {e}"
            ))
        })?
        .error_for_status()?;
    let body = resp.text().await?;

    verify_signature(client, body.as_bytes()).await?;

    let manifest: Manifest = serde_json::from_str(&body)?;
    Ok(manifest)
}

/// Проверить minisign-подпись тела манифеста. Без ключа — no-op.
async fn verify_signature(client: &reqwest::Client, body: &[u8]) -> Result<()> {
    use crate::error::LauncherError;

    let pubkey = crate::config::MANIFEST_PUBKEY.trim();
    if pubkey.is_empty() {
        log::warn!("manifest: проверка подписи отключена (MANIFEST_PUBKEY пуст)");
        return Ok(());
    }

    let sig_url = crate::config::manifest_sig_url();
    let sig_text = client
        .get(&sig_url)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| {
            LauncherError::Other(format!("не удалось получить подпись манифеста ({sig_url}): {e}"))
        })?
        .error_for_status()
        .map_err(|e| LauncherError::Other(format!("подпись манифеста недоступна: {e}")))?
        .text()
        .await?;

    let public_key = minisign_verify::PublicKey::from_base64(pubkey)
        .map_err(|e| LauncherError::Other(format!("некорректный MANIFEST_PUBKEY: {e}")))?;
    let signature = minisign_verify::Signature::decode(&sig_text)
        .map_err(|e| LauncherError::Other(format!("некорректная подпись манифеста: {e}")))?;

    // allow_legacy=true: принимаем и prehashed, и legacy (минисайн-крейт сборщика
    // по умолчанию подписывает legacy; для маленького manifest.json это безопасно).
    public_key
        .verify(body, &signature, true)
        .map_err(|_| {
            LauncherError::Other(
                "подпись manifest.json не прошла проверку — источник модпака мог быть \
                 подменён. Обновление отменено."
                    .into(),
            )
        })?;
    log::info!("manifest: подпись проверена ✓");
    Ok(())
}
