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

/// Ключ платформы для секции `java` манифеста.
/// TODO: расширить при поддержке Linux/macOS.
pub fn platform_key() -> &'static str {
    "windows-x64"
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

/// Гарантировать наличие JRE. Если `java.exe` уже на месте — ничего не делает.
/// Иначе качает архив (с прогрессом), проверяет SHA-256 и распаковывает.
/// Возвращает путь к `java`-исполняемому.
pub async fn ensure_java<F>(
    client: &reqwest::Client,
    install_dir: &Path,
    entry: &JavaEntry,
    on_progress: F,
) -> Result<PathBuf>
where
    F: Fn(u64, Option<u64>),
{
    let java_exe = java_exe_path(install_dir, &entry.dir);
    if java_exe.exists() {
        return Ok(java_exe);
    }

    let runtime = install_dir.join(&entry.dir);
    let archive = install_dir.join(format!("{}.download.zip", entry.dir));

    download::download_to_file(client, &entry.url, &archive, on_progress).await?;

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
    extract_zip(&archive, &runtime, true).await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Живой тест пайплайна JRE против реального Adoptium (скачивает ~45 МБ).
    /// Запуск: `cargo test --release jre_pipeline -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn jre_pipeline() {
        let url = "https://api.adoptium.net/v3/binary/latest/17/ga/windows/x64/jre/hotspot/normal/eclipse";
        let tmp = std::env::temp_dir().join("krp_jre_test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let client = reqwest::Client::new();
        let archive = tmp.join("jre.zip");

        download::download_to_file(&client, url, &archive, |d, t| {
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
