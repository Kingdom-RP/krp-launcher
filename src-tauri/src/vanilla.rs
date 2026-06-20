//! Резолвер и загрузчик ванильного Minecraft с официального CDN Mojang
//! (гибридная часть способа Б — ваниль мы НЕ хостим у себя).
//!
//! Поток: манифест версий → version JSON нужной версии → скачивание
//! `client.jar` + библиотек (с учётом OS-правил) + индекса ассетов и всех
//! объектов ассетов. Всё проверяется по SHA-1 (формат Mojang).
//!
//! Раскладка на диске (стандартная для Minecraft, относительно install_dir):
//! - `versions/<id>/<id>.jar`
//! - `libraries/<maven-path>`
//! - `assets/indexes/<assets>.json`
//! - `assets/objects/<xx>/<hash>`

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use crate::config;
use crate::download;
use crate::error::{LauncherError, Result};
use crate::progress::Progress;

const VERSION_MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const RESOURCES_BASE: &str = "https://resources.download.minecraft.net";

// ---- структуры манифестов Mojang (берём только нужные поля) ----

#[derive(Debug, Deserialize)]
pub struct VersionManifest {
    pub versions: Vec<ManifestVersion>,
}

#[derive(Debug, Deserialize)]
pub struct ManifestVersion {
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub sha1: String,
}

#[derive(Debug, Deserialize)]
pub struct VersionJson {
    pub id: String,
    pub downloads: VersionDownloads,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(rename = "assetIndex")]
    pub asset_index: AssetIndexRef,
    pub assets: String,
}

#[derive(Debug, Deserialize)]
pub struct VersionDownloads {
    pub client: Download,
}

#[derive(Debug, Deserialize)]
pub struct Download {
    pub url: String,
    pub sha1: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Deserialize)]
pub struct Library {
    #[serde(default)]
    pub downloads: Option<LibraryDownloads>,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[derive(Debug, Deserialize)]
pub struct LibraryDownloads {
    #[serde(default)]
    pub artifact: Option<Artifact>,
}

#[derive(Debug, Deserialize)]
pub struct Artifact {
    pub path: String,
    pub url: String,
    pub sha1: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Deserialize)]
pub struct Rule {
    pub action: String,
    #[serde(default)]
    pub os: Option<OsRule>,
}

#[derive(Debug, Deserialize)]
pub struct OsRule {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AssetIndexRef {
    pub url: String,
    pub sha1: String,
}

#[derive(Debug, Deserialize)]
pub struct AssetIndex {
    pub objects: HashMap<String, AssetObject>,
}

#[derive(Debug, Deserialize)]
pub struct AssetObject {
    pub hash: String,
    #[serde(default)]
    pub size: u64,
}

// ---- логика ----

/// Имя текущей ОС в терминах правил Mojang.
pub fn os_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    }
}

/// Применима ли библиотека на текущей ОС (по списку `rules`).
fn library_allowed(rules: &[Rule]) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for rule in rules {
        let matches = match &rule.os {
            Some(os) => os.name.as_deref().map_or(true, |n| n == os_name()),
            None => true,
        };
        if matches {
            allowed = rule.action == "allow";
        }
    }
    allowed
}

/// Скачать манифест версий Mojang.
pub async fn fetch_version_manifest(client: &reqwest::Client) -> Result<VersionManifest> {
    Ok(client
        .get(VERSION_MANIFEST_URL)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

/// Найти URL version JSON конкретной версии. (Используется в тестах.)
#[allow(dead_code)]
pub fn find_version_url<'a>(manifest: &'a VersionManifest, version_id: &str) -> Result<&'a str> {
    manifest
        .versions
        .iter()
        .find(|v| v.id == version_id)
        .map(|v| v.url.as_str())
        .ok_or_else(|| {
            LauncherError::Other(format!("версия {version_id} не найдена в манифесте Mojang"))
        })
}

/// Скачать version JSON по URL.
pub async fn fetch_version_json(client: &reqwest::Client, url: &str) -> Result<VersionJson> {
    Ok(client.get(url).send().await?.error_for_status()?.json().await?)
}

/// Полное обеспечение ванильных файлов для `version_id` в `install_dir`.
///
/// Считает общий объём ванили, добавляет его в общий трекер `progress` и качает
/// недостающее, сообщая скачанные/пропущенные байты в трекер.
pub async fn ensure_vanilla(
    client: &reqwest::Client,
    install_dir: &Path,
    version_id: &str,
    progress: &Progress,
    verb: &str,
) -> Result<()> {
    let manifest = fetch_version_manifest(client).await?;
    let mv = manifest
        .versions
        .iter()
        .find(|v| v.id == version_id)
        .ok_or_else(|| {
            LauncherError::Other(format!("версия {version_id} не найдена в манифесте Mojang"))
        })?;
    let version = fetch_version_json(client, &mv.url).await?;

    // Сохраняем ванильный version JSON на диск — его читает построитель
    // команды запуска (launch.rs) офлайн. (Маленький, в трекере не учитываем.)
    let version_json_dest = install_dir
        .join("versions")
        .join(version_id)
        .join(format!("{version_id}.json"));
    if !mv.sha1.is_empty() {
        download::ensure_file_sha1(client, &mv.url, &version_json_dest, &mv.sha1, |_, _| {})
            .await?;
    } else {
        download::download_to_file(client, &mv.url, &version_json_dest, |_, _| {}).await?;
    }

    // Индекс ассетов нужен заранее, чтобы знать размеры объектов.
    let asset_index: AssetIndex = client
        .get(&version.asset_index.url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let libs: Vec<&Library> = version
        .libraries
        .iter()
        .filter(|l| library_allowed(&l.rules))
        .collect();

    // Общий объём ванили = client.jar + библиотеки + объекты ассетов.
    let mut vanilla_total = version.downloads.client.size;
    for lib in &libs {
        if let Some(a) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()) {
            vanilla_total += a.size;
        }
    }
    for object in asset_index.objects.values() {
        vanilla_total += object.size;
    }
    progress.add_total(vanilla_total);
    progress.set_label(format!("{verb} Minecraft {}", config::MINECRAFT_VERSION));

    // 1. client.jar
    let client_jar = install_dir
        .join("versions")
        .join(&version.id)
        .join(format!("{}.jar", version.id));
    let did = download::ensure_file_sha1(
        client,
        &version.downloads.client.url,
        &client_jar,
        &version.downloads.client.sha1,
        progress.file_cb(),
    )
    .await?;
    if !did {
        progress.add_skipped(version.downloads.client.size);
    }

    // 2. библиотеки
    for lib in &libs {
        if let Some(artifact) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()) {
            let dest = install_dir
                .join("libraries")
                .join(artifact.path.replace('/', std::path::MAIN_SEPARATOR_STR));
            let did =
                download::ensure_file_sha1(client, &artifact.url, &dest, &artifact.sha1, progress.file_cb())
                    .await?;
            if !did {
                progress.add_skipped(artifact.size);
            }
        }
    }

    // 3. индекс ассетов (сохраняем на диск; маленький, в трекере не учитываем)
    let index_dest = install_dir
        .join("assets")
        .join("indexes")
        .join(format!("{}.json", version.assets));
    download::ensure_file_sha1(
        client,
        &version.asset_index.url,
        &index_dest,
        &version.asset_index.sha1,
        |_, _| {},
    )
    .await?;

    // 4. объекты ассетов
    for object in asset_index.objects.values() {
        let prefix = &object.hash[..2];
        let dest = install_dir
            .join("assets")
            .join("objects")
            .join(prefix)
            .join(&object.hash);
        let url = format!("{RESOURCES_BASE}/{prefix}/{}", object.hash);
        let did =
            download::ensure_file_sha1(client, &url, &dest, &object.hash, progress.file_cb()).await?;
        if !did {
            progress.add_skipped(object.size);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Живой тест против Mojang: манифест → version JSON 1.21.1 → скачать
    /// client.jar (проверка SHA-1) + распарсить индекс ассетов. Объекты
    /// ассетов (~300 МБ) НЕ качаем — проверяем механику ядра.
    /// `cargo test vanilla_resolver -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn vanilla_resolver() {
        let client = reqwest::Client::new();
        let manifest = fetch_version_manifest(&client).await.expect("manifest");
        let url = find_version_url(&manifest, "1.21.1").expect("1.21.1 url");
        let version = fetch_version_json(&client, url).await.expect("version json");
        assert_eq!(version.id, "1.21.1");
        assert!(!version.libraries.is_empty(), "ожидались библиотеки");

        let tmp = std::env::temp_dir().join("krp_vanilla_test");
        let _ = std::fs::remove_dir_all(&tmp);
        let client_jar = tmp.join("client.jar");
        download::ensure_file_sha1(
            &client,
            &version.downloads.client.url,
            &client_jar,
            &version.downloads.client.sha1,
            |_, _| {},
        )
        .await
        .expect("client.jar download/verify");
        assert!(client_jar.exists());

        let index: AssetIndex = client
            .get(&version.asset_index.url)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(!index.objects.is_empty(), "ожидались объекты ассетов");
        eprintln!(
            "OK: 1.21.1, libs={}, assets={}",
            version.libraries.len(),
            index.objects.len()
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
