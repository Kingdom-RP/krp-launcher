//! Единый тип ошибок лаунчера.
//!
//! Реализует `Serialize`, поэтому ошибки можно напрямую возвращать из
//! Tauri-команд во фронтенд (там придёт строка с описанием).

use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum LauncherError {
    #[error("сетевая ошибка: {0}")]
    Http(#[from] reqwest::Error),

    #[error("ошибка файловой системы: {0}")]
    Io(#[from] std::io::Error),

    #[error("ошибка разбора JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("ошибка распаковки архива: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("не совпала контрольная сумма для {path}: ожидалось {expected}, получено {actual}")]
    Checksum {
        path: String,
        expected: String,
        actual: String,
    },

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, LauncherError>;

impl Serialize for LauncherError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
