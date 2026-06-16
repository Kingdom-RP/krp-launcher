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

use crate::download;
use crate::error::{LauncherError, Result};

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
    pub id: String,
    pub url: String,
    pub sha1: String,
    #[serde(default)]
    pub size: u64,
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

/// Найти URL version JSON конкретной версии.
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
/// `on_file(index, total, name, downloaded, total_bytes)` вызывается по ходу
/// (для эмита прогресса наружу).
pub async fn ensure_vanilla<F>(
    client: &reqwest::Client,
    install_dir: &Path,
    version_id: &str,
    on_file: F,
) -> Result<()>
where
    F: Fn(usize, usize, &str, u64, Option<u64>),
{
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
    // команды запуска (launch.rs) офлайн.
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

    // Индекс ассетов нужен заранее, чтобы знать общее число шагов.
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

    // total = client.jar + библиотеки + индекс ассетов + объекты ассетов
    let total = 1 + libs.len() + 1 + asset_index.objects.len();
    let mut idx = 0usize;

    // 1. client.jar
    let client_jar = install_dir
        .join("versions")
        .join(&version.id)
        .join(format!("{}.jar", version.id));
    download::ensure_file_sha1(
        client,
        &version.downloads.client.url,
        &client_jar,
        &version.downloads.client.sha1,
        |d, t| on_file(idx, total, "client.jar", d, t),
    )
    .await?;
    idx += 1;

    // 2. библиотеки
    for lib in &libs {
        if let Some(artifact) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()) {
            let dest = install_dir
                .join("libraries")
                .join(artifact.path.replace('/', std::path::MAIN_SEPARATOR_STR));
            let name = artifact.path.clone();
            download::ensure_file_sha1(client, &artifact.url, &dest, &artifact.sha1, |d, t| {
                on_file(idx, total, &name, d, t)
            })
            .await?;
        }
        idx += 1;
    }

    // 3. индекс ассетов (сохраняем на диск)
    let index_dest = install_dir
        .join("assets")
        .join("indexes")
        .join(format!("{}.json", version.assets));
    download::ensure_file_sha1(
        client,
        &version.asset_index.url,
        &index_dest,
        &version.asset_index.sha1,
        |d, t| on_file(idx, total, "asset index", d, t),
    )
    .await?;
    idx += 1;

    // 4. объекты ассетов
    for object in asset_index.objects.values() {
        let prefix = &object.hash[..2];
        let dest = install_dir
            .join("assets")
            .join("objects")
            .join(prefix)
            .join(&object.hash);
        let url = format!("{RESOURCES_BASE}/{prefix}/{}", object.hash);
        download::ensure_file_sha1(client, &url, &dest, &object.hash, |d, t| {
            on_file(idx, total, &object.hash, d, t)
        })
        .await?;
        idx += 1;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Живой тест против Mojang: манифест → version JSON 1.20.1 → скачать
    /// client.jar (проверка SHA-1) + распарсить индекс ассетов. Объекты
    /// ассетов (~300 МБ) НЕ качаем — проверяем механику ядра.
    /// `cargo test vanilla_resolver -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn vanilla_resolver() {
        let client = reqwest::Client::new();
        let manifest = fetch_version_manifest(&client).await.expect("manifest");
        let url = find_version_url(&manifest, "1.20.1").expect("1.20.1 url");
        let version = fetch_version_json(&client, url).await.expect("version json");
        assert_eq!(version.id, "1.20.1");
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
            "OK: 1.20.1, libs={}, assets={}",
            version.libraries.len(),
            index.objects.len()
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
