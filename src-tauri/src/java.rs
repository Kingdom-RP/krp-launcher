//! Скачивание, проверка и распаковка Temurin JRE 17 (способ Б).
//!
//! JRE качается как zip-архив, проверяется по SHA-256 и распаковывается в
//! папку `<install>/<entry.dir>` (по умолчанию `runtime`). Верхний каталог
//! архива (`jdk-17.x.y-jre/`) срезается, чтобы путь к `java.exe` был
//! предсказуемым: `<install>/runtime/bin/java.exe`.

use std::path::{Path, PathBuf};

use crate::download;
use crate::error::{LauncherError, Result};
use crate::manifest::JavaEntry;
use crate::progress::Progress;

/// Ключ платформы для секции `java` манифеста (под текущую ОС/арх сборки).
/// Сборки делаем x64 для Windows/Linux; macOS — задел (arm64/x64).
pub fn platform_key() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows-x64"
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "macos-arm64"
        } else {
            "macos-x64"
        }
    } else if cfg!(target_os = "linux") {
        "linux-x64"
    } else {
        "unknown"
    }
}

/// Требуемая мажорная версия Java (проект на Java 21 — см. CLAUDE.md).
pub const JAVA_MAJOR: u32 = 21;

/// Достать мажорную версию Java из вывода `java -version` (Java печатает его в
/// stderr), например `openjdk version "17.0.19"` → `17`. Понимает и старый
/// формат `1.8.0` → `8`.
fn parse_java_major(output: &str) -> Option<u32> {
    let idx = output.find("version \"")?;
    let rest = &output[idx + "version \"".len()..];
    let end = rest.find('"')?;
    let mut parts = rest[..end].split(['.', '_']);
    let first: u32 = parts.next()?.parse().ok()?;
    if first == 1 {
        parts.next()?.parse().ok() // 1.8 → 8
    } else {
        Some(first)
    }
}

/// Запустить `java -version` и вернуть мажорную версию (или `None`, если
/// исполняемый файл битый/не запускается).
fn java_major_version(java_exe: &Path) -> Option<u32> {
    let output = std::process::Command::new(java_exe)
        .arg("-version")
        .output()
        .ok()?;
    // Версия обычно в stderr, но на всякий случай смотрим и stdout.
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    parse_java_major(&text)
}

/// Подходит ли установленная по пути JRE (существует и нужной мажорной версии).
async fn java_is_valid(java_exe: &Path) -> bool {
    if !java_exe.exists() {
        return false;
    }
    let exe = java_exe.to_path_buf();
    tokio::task::spawn_blocking(move || java_major_version(&exe) == Some(JAVA_MAJOR))
        .await
        .unwrap_or(false)
}

/// Путь к исполняемому файлу Java внутри распакованного runtime.
pub fn java_exe_path(install_dir: &Path, runtime_dir: &str) -> PathBuf {
    let bin = install_dir.join(runtime_dir).join("bin");
    if cfg!(windows) {
        bin.join("java.exe")
    } else {
        bin.join("java")
    }
}

/// Гарантировать наличие JRE нужной версии. Если в `runtime` уже лежит рабочая
/// Java [`JAVA_MAJOR`] — ничего не качает (учитывает размер как пропущенный).
/// Если её нет или версия не та (битая/чужая) — перекачивает: качает архив (с
/// прогрессом), проверяет SHA-256 и распаковывает. Возвращает путь к `java`.
pub async fn ensure_java(
    client: &reqwest::Client,
    install_dir: &Path,
    entry: &JavaEntry,
    progress: &Progress,
    verb: &str,
) -> Result<PathBuf> {
    let java_exe = java_exe_path(install_dir, &entry.dir);
    let runtime = install_dir.join(&entry.dir);

    if java_is_valid(&java_exe).await {
        progress.add_skipped(entry.size);
        return Ok(java_exe);
    }
    // Папка есть, но Java битая/не та версия — сносим перед перекачкой.
    if runtime.exists() {
        log::warn!(
            "ensure_java: JRE в {} отсутствует или не Java {JAVA_MAJOR} — перекачиваем",
            runtime.display()
        );
        let _ = tokio::fs::remove_dir_all(&runtime).await;
    }

    progress.set_label(format!("{verb} Java"));
    // Adoptium: Windows = .zip, Linux/macOS = .tar.gz. Тип берём из имени файла.
    let is_tgz = entry.url.ends_with(".tar.gz") || entry.url.ends_with(".tgz");
    let archive = install_dir.join(format!(
        "{}.download.{}",
        entry.dir,
        if is_tgz { "tar.gz" } else { "zip" }
    ));

    // mirror_key=None: JRE хостится на нашем base — ключ выведется снятием префикса.
    download::download_to_file(client, &entry.url, None, &archive, progress.file_cb()).await?;

    let actual = download::sha256_file(&archive).await?;
    if !actual.eq_ignore_ascii_case(&entry.sha256) {
        let _ = tokio::fs::remove_file(&archive).await;
        return Err(LauncherError::Checksum {
            path: archive.display().to_string(),
            expected: entry.sha256.clone(),
            actual,
        });
    }

    tokio::fs::create_dir_all(&runtime).await?;
    if is_tgz {
        extract_targz(&archive, &runtime, true).await?;
    } else {
        extract_zip(&archive, &runtime, true).await?;
    }
    let _ = tokio::fs::remove_file(&archive).await;

    if !java_exe.exists() {
        return Err(LauncherError::Other(format!(
            "после распаковки JRE не найден {}",
            java_exe.display()
        )));
    }

    Ok(java_exe)
}

/// Распаковать zip в `dest`. При `strip_top` срезает первый компонент пути
/// каждой записи (общий корневой каталог архива). Выполняется в blocking-пуле.
pub async fn extract_zip(archive: &Path, dest: &Path, strip_top: bool) -> Result<()> {
    let archive = archive.to_path_buf();
    let dest = dest.to_path_buf();
    tokio::task::spawn_blocking(move || extract_zip_blocking(&archive, &dest, strip_top))
        .await
        .map_err(|e| LauncherError::Other(format!("задача распаковки прервана: {e}")))?
}

fn extract_zip_blocking(archive: &Path, dest: &Path, strip_top: bool) -> Result<()> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)?;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;

        // enclosed_name отсекает попытки выхода за пределы dest (zip-slip).
        let Some(enclosed) = entry.enclosed_name() else {
            continue;
        };

        let rel: PathBuf = if strip_top {
            let mut comps = enclosed.components();
            comps.next(); // срезаем корневой каталог архива
            comps.as_path().to_path_buf()
        } else {
            enclosed
        };

        if rel.as_os_str().is_empty() {
            continue;
        }

        let out = dest.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out_file = std::fs::File::create(&out)?;
            std::io::copy(&mut entry, &mut out_file)?;
        }
    }

    Ok(())
}

/// Распаковать `.tar.gz` в `dest` (JRE под Linux/macOS). При `strip_top` срезает
/// корневой каталог архива. `unpack` сохраняет права (важно для бита +x у `java`).
pub async fn extract_targz(archive: &Path, dest: &Path, strip_top: bool) -> Result<()> {
    let archive = archive.to_path_buf();
    let dest = dest.to_path_buf();
    tokio::task::spawn_blocking(move || extract_targz_blocking(&archive, &dest, strip_top))
        .await
        .map_err(|e| LauncherError::Other(format!("задача распаковки прервана: {e}")))?
}

fn extract_targz_blocking(archive: &Path, dest: &Path, strip_top: bool) -> Result<()> {
    let file = std::fs::File::open(archive)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);
    tar.set_preserve_permissions(true);

    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();

        let rel: PathBuf = if strip_top {
            let mut comps = path.components();
            comps.next(); // срезаем корневой каталог архива
            comps.as_path().to_path_buf()
        } else {
            path
        };

        if rel.as_os_str().is_empty() {
            continue;
        }
        // защита от выхода за пределы dest (tar-slip)
        if rel
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            continue;
        }

        let out = dest.join(&rel);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        entry.unpack(&out)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Живой тест пайплайна JRE против реального Adoptium (скачивает ~45 МБ).
    /// Запуск: `cargo test --release jre_pipeline -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn jre_pipeline() {
        let url = "https://api.adoptium.net/v3/binary/latest/21/ga/windows/x64/jre/hotspot/normal/eclipse";
        let tmp = std::env::temp_dir().join("krp_jre_test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let client = reqwest::Client::new();
        let archive = tmp.join("jre.zip");

        download::download_to_file(&client, url, None, &archive, |d, t| {
            if let Some(t) = t {
                eprint!("\rскачано {d}/{t}");
            }
        })
        .await
        .expect("download failed");
        eprintln!();

        let runtime = tmp.join("runtime");
        extract_zip(&archive, &runtime, true)
            .await
            .expect("extract failed");

        let java_exe = java_exe_path(&tmp, "runtime");
        assert!(java_exe.exists(), "java.exe не найден: {}", java_exe.display());
        eprintln!("OK: {}", java_exe.display());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
