//! Определение и валидация папки установки игры.
//!
//! По умолчанию — `%APPDATA%\KingdomRP` (Roaming). Игрок может сменить путь,
//! но он проходит проверки: не-ASCII (кириллица ломает нативные библиотеки
//! Minecraft), системные папки, OneDrive, длина пути, права на запись.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{LauncherError, Result};

/// Имя папки игры, которое лаунчер добавляет к выбранному каталогу.
pub const APP_DIR_NAME: &str = "Kingdom RP";

/// Безопасно присоединить относительный путь (из манифеста/Mojang) к базовой
/// папке установки. Путь приходит из сети, поэтому защищаемся от path traversal:
/// разрешены только «нормальные» компоненты (`mods/x.jar`), а абсолютные пути,
/// префиксы диска (`C:\`), корень и `..` отклоняются. Без этого
/// скомпрометированный/подменённый манифест мог бы записать файл вне `base`
/// (напр. в автозагрузку) — SHA-256 проверяет содержимое, но не путь назначения.
pub fn safe_join(base: &Path, rel: &str) -> Result<PathBuf> {
    use std::path::Component;

    // Нормализуем разделители к разделителю ОС.
    let rel = rel.replace('/', std::path::MAIN_SEPARATOR_STR);
    let mut out = base.to_path_buf();
    let mut pushed = false;
    for comp in Path::new(&rel).components() {
        match comp {
            Component::Normal(seg) => {
                out.push(seg);
                pushed = true;
            }
            // `.` игнорируем; всё остальное (RootDir, Prefix `C:\`, `..`) — отказ.
            Component::CurDir => {}
            _ => {
                return Err(LauncherError::Other(format!(
                    "недопустимый путь в манифесте: {rel:?}"
                )));
            }
        }
    }
    if !pushed {
        return Err(LauncherError::Other(format!(
            "пустой путь в манифесте: {rel:?}"
        )));
    }
    Ok(out)
}

/// Папка установки по умолчанию: `%APPDATA%\Kingdom RP`.
pub fn default_install_dir() -> Result<PathBuf> {
    let base = dirs::data_dir()
        .ok_or_else(|| LauncherError::Other("не удалось определить папку AppData".into()))?;
    Ok(base.join(APP_DIR_NAME))
}

/// Привести выбранный игроком каталог к папке установки: если он ещё не
/// заканчивается на `Kingdom RP`, добавляем эту подпапку. Так выбор `E:\Games`
/// превращается в `E:\Games\Kingdom RP`, и лаунчер сам создаёт нужную папку.
pub fn resolve_install_dir(picked: &Path) -> PathBuf {
    if picked.file_name().and_then(|n| n.to_str()) == Some(APP_DIR_NAME) {
        picked.to_path_buf()
    } else {
        picked.join(APP_DIR_NAME)
    }
}

/// Результат проверки выбранного пути.
#[derive(Debug, Clone, Serialize)]
pub struct PathValidation {
    /// Путь пригоден (нет блокирующих ошибок).
    pub valid: bool,
    /// Блокирующие проблемы — установку нельзя продолжать.
    pub errors: Vec<String>,
    /// Предупреждения — можно продолжать, но есть риск.
    pub warnings: Vec<String>,
}

/// Статические проверки пути (без обращения к диску).
pub fn validate_install_dir(path: &Path) -> PathValidation {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    let s = path.to_string_lossy();
    let lower = s.to_lowercase();

    // Не-ASCII (кириллица и т.п.) ломает загрузку нативных библиотек LWJGL.
    if !s.is_ascii() {
        errors.push(
            "Путь содержит не-ASCII символы (например кириллицу) — это вызывает краши \
             Minecraft. Выберите путь только из латинских букв и цифр."
                .into(),
        );
    }

    // Системные папки — нет прав на запись.
    if lower.contains("program files") || lower.contains("\\windows\\") {
        errors.push(
            "Путь в системной папке (Program Files / Windows) — нет прав на запись. \
             Выберите другую папку."
                .into(),
        );
    }

    // OneDrive лочит файлы и тормозит игру.
    if lower.contains("onedrive") {
        warnings.push(
            "Путь внутри OneDrive — синхронизация может блокировать файлы игры и замедлять её."
                .into(),
        );
    }

    // Запас по лимиту длины пути Windows (MAX_PATH ≈ 260).
    if s.chars().count() > 100 {
        warnings.push(
            "Длинный путь установки — при вложенных файлах модов возможно превышение \
             лимита Windows в 260 символов."
                .into(),
        );
    }

    PathValidation {
        valid: errors.is_empty(),
        errors,
        warnings,
    }
}

/// Полная проверка: статические правила + реальная проверка прав на запись
/// (пытается создать папку и записать временный файл).
pub async fn validate_install_dir_full(path: &Path) -> PathValidation {
    let mut result = validate_install_dir(path);
    if !result.valid {
        return result;
    }

    if let Err(e) = check_writable(path).await {
        result.valid = false;
        result
            .errors
            .push(format!("Нет прав на запись в эту папку: {e}"));
    }

    result
}

async fn check_writable(path: &Path) -> Result<()> {
    tokio::fs::create_dir_all(path).await?;
    let probe = path.join(".krp_write_test");
    tokio::fs::write(&probe, b"ok").await?;
    tokio::fs::remove_file(&probe).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_join_allows_normal_paths() {
        let base = Path::new("/install");
        assert_eq!(
            safe_join(base, "mods/x.jar").unwrap(),
            base.join("mods").join("x.jar")
        );
        // `.` компоненты игнорируются.
        assert_eq!(
            safe_join(base, "./libraries/a/b.jar").unwrap(),
            base.join("libraries").join("a").join("b.jar")
        );
    }

    #[test]
    fn safe_join_rejects_traversal() {
        let base = Path::new("/install");
        assert!(safe_join(base, "../evil").is_err());
        assert!(safe_join(base, "mods/../../evil").is_err());
        assert!(safe_join(base, "/etc/passwd").is_err());
        assert!(safe_join(base, "").is_err());
        #[cfg(windows)]
        {
            assert!(safe_join(base, "C:\\Windows\\x").is_err());
            assert!(safe_join(base, "..\\evil").is_err());
        }
    }
}
