//! Скачивание файлов с потоковым прогрессом и проверкой SHA-256.
//!
//! Базовый кирпич для всего остального: загрузки Java, NeoForge и модов
//! проходят через эти функции.

use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::error::{LauncherError, Result};

/// Посчитать SHA-256 содержимого файла (нижний регистр hex).
pub async fn sha256_file(path: &Path) -> Result<String> {
    let bytes = tokio::fs::read(path).await?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()))
}

/// Посчитать SHA-1 содержимого файла (Mojang верифицирует ваниль по SHA-1).
pub async fn sha1_file(path: &Path) -> Result<String> {
    let bytes = tokio::fs::read(path).await?;
    let mut hasher = Sha1::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()))
}

/// Скачать `url` в `dest`, сообщая прогресс через `on_progress(downloaded, total)`.
/// `total` = `None`, если сервер не прислал `Content-Length`.
///
/// Качаем во временный файл `<dest>.part` и только при успешном завершении
/// атомарно переименовываем в `dest`. Так обрыв связи/ошибка не оставляют
/// «полуфайл» на месте готового: прежний валидный `dest` (если был) уцелеет, а
/// при повторном запуске проверка SHA-256 не пропустит битый файл, и докачается
/// только он (а не весь модпак заново).
pub async fn download_to_file<F>(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    on_progress: F,
) -> Result<()>
where
    F: Fn(u64, Option<u64>),
{
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let part = part_path(dest);

    let result = stream_to_file(client, url, &part, on_progress).await;
    if result.is_err() {
        // Чистим недокачанный временный файл, чтобы не копился мусор.
        let _ = tokio::fs::remove_file(&part).await;
        return result;
    }

    // Атомарная замена (на Windows перезапишет существующий dest).
    tokio::fs::rename(&part, dest).await?;
    Ok(())
}

/// Путь временного файла загрузки (`<dest>.part`).
fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// Потоковая запись тела ответа в файл с прогрессом.
async fn stream_to_file<F>(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    on_progress: F,
) -> Result<()>
where
    F: Fn(u64, Option<u64>),
{
    let resp = client.get(url).send().await?.error_for_status()?;
    let total = resp.content_length();

    let mut file = tokio::fs::File::create(dest).await?;
    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        on_progress(downloaded, total);
    }
    file.flush().await?;

    Ok(())
}

/// Гарантировать, что файл `dest` существует и совпадает по SHA-256.
///
/// Если файл уже на месте и хеш сходится — ничего не качает (`Ok(false)`).
/// Иначе качает и проверяет; при несовпадении хеша после загрузки —
/// [`LauncherError::Checksum`]. Возвращает `Ok(true)`, если файл был скачан.
pub async fn ensure_file<F>(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    expected_sha256: &str,
    on_progress: F,
) -> Result<bool>
where
    F: Fn(u64, Option<u64>),
{
    if dest.exists() {
        if let Ok(actual) = sha256_file(dest).await {
            if actual.eq_ignore_ascii_case(expected_sha256) {
                return Ok(false);
            }
        }
    }

    download_to_file(client, url, dest, on_progress).await?;

    let actual = sha256_file(dest).await?;
    if !actual.eq_ignore_ascii_case(expected_sha256) {
        return Err(LauncherError::Checksum {
            path: dest.display().to_string(),
            expected: expected_sha256.to_string(),
            actual,
        });
    }

    Ok(true)
}

/// Как [`ensure_file`], но проверка по SHA-1 (для файлов с Mojang CDN).
pub async fn ensure_file_sha1<F>(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    expected_sha1: &str,
    on_progress: F,
) -> Result<bool>
where
    F: Fn(u64, Option<u64>),
{
    if dest.exists() {
        if let Ok(actual) = sha1_file(dest).await {
            if actual.eq_ignore_ascii_case(expected_sha1) {
                return Ok(false);
            }
        }
    }

    download_to_file(client, url, dest, on_progress).await?;

    let actual = sha1_file(dest).await?;
    if !actual.eq_ignore_ascii_case(expected_sha1) {
        return Err(LauncherError::Checksum {
            path: dest.display().to_string(),
            expected: expected_sha1.to_string(),
            actual,
        });
    }

    Ok(true)
}
