//! Проверка, что выбранный игроком PNG — корректная развёртка скина Minecraft.
//!
//! Скин Minecraft — это PNG строго **64×64** (современный формат) или **64×32**
//! (устаревший, только классическая модель). Любую другую картинку (фото, мем,
//! арт не того размера) грузить нельзя — лаунчер отклоняет её ещё до отправки на
//! auth-сервер (drasl). Финальную проверку делает и сам сервер, но быстрый
//! пред-чек в лаунчере даёт понятную ошибку и не гоняет мусор по сети.

use serde::Serialize;

use crate::error::{LauncherError, Result};

/// Формат развёртки скина.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SkinFormat {
    /// 64×64 — современный формат (классическая или slim-модель).
    Modern,
    /// 64×32 — устаревший формат (только классическая модель).
    Legacy,
}

/// Сигнатура PNG (первые 8 байт файла).
const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// Максимальный размер файла скина. Скин Minecraft 64×64 PNG — единицы КБ;
/// 512 КБ — с большим запасом. Защита от «скин на 1 ГБ» (OOM лаунчера + мусор
/// на auth-сервере): проверяем ДО чтения файла в память.
pub const MAX_SKIN_BYTES: u64 = 512 * 1024;

/// Прочитать файл скина с лимитом размера (проверка по метаданным ДО чтения).
pub fn read_skin_file(path: &std::path::Path) -> Result<Vec<u8>> {
    let meta = std::fs::metadata(path)
        .map_err(|e| LauncherError::Other(format!("не прочитать файл скина: {e}")))?;
    if meta.len() > MAX_SKIN_BYTES {
        return Err(LauncherError::Other(format!(
            "Файл слишком большой ({} КБ). Скин Minecraft — это маленький PNG 64×64 \
             (до {} КБ).",
            meta.len() / 1024,
            MAX_SKIN_BYTES / 1024
        )));
    }
    std::fs::read(path).map_err(|e| LauncherError::Other(format!("не прочитать файл скина: {e}")))
}

/// Прочитать ширину/высоту из заголовка PNG (chunk `IHDR`). Возвращает ошибку,
/// если это не PNG или заголовок повреждён.
fn png_dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    // 8 (сигнатура) + 4 (длина) + 4 ("IHDR") + 4 (width) + 4 (height) = минимум 24,
    // дальше идут bit depth/color type — берём с запасом.
    if bytes.len() < 24 || bytes[..8] != PNG_SIGNATURE {
        return Err(LauncherError::Other(
            "Файл не является PNG-изображением.".into(),
        ));
    }
    if &bytes[12..16] != b"IHDR" {
        return Err(LauncherError::Other("Повреждённый PNG (нет IHDR).".into()));
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Ok((width, height))
}

/// Проверить, что байты — корректный PNG-скин. Возвращает формат или понятную
/// ошибку (которую можно показать игроку), если это не развёртка скина.
pub fn validate_skin(bytes: &[u8]) -> Result<SkinFormat> {
    // Защита в глубину: даже если пришли байты напрямую — не крупнее лимита.
    if bytes.len() as u64 > MAX_SKIN_BYTES {
        return Err(LauncherError::Other("Файл скина слишком большой.".into()));
    }
    let (w, h) = png_dimensions(bytes)?;
    match (w, h) {
        (64, 64) => Ok(SkinFormat::Modern),
        (64, 32) => Ok(SkinFormat::Legacy),
        _ => Err(LauncherError::Other(format!(
            "Это не похоже на скин: PNG {w}×{h}. Скин Minecraft должен быть \
             64×64 (или 64×32 для старого формата)."
        ))),
    }
}

/// Прочитать файл и проверить, что это PNG-скин.
pub fn validate_skin_file(path: &std::path::Path) -> Result<SkinFormat> {
    let bytes = read_skin_file(path)?;
    validate_skin(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Собрать первые байты PNG с нужными width/height (для validate достаточно
    /// заголовка — пиксели не нужны).
    fn png_header(w: u32, h: u32) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&PNG_SIGNATURE);
        b.extend_from_slice(&13u32.to_be_bytes()); // длина IHDR
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0]); // bit depth, color type (RGBA), …
        b
    }

    #[test]
    fn accepts_modern_and_legacy() {
        assert_eq!(validate_skin(&png_header(64, 64)).unwrap(), SkinFormat::Modern);
        assert_eq!(validate_skin(&png_header(64, 32)).unwrap(), SkinFormat::Legacy);
    }

    #[test]
    fn rejects_wrong_dimensions() {
        assert!(validate_skin(&png_header(128, 128)).is_err());
        assert!(validate_skin(&png_header(100, 50)).is_err());
        assert!(validate_skin(&png_header(64, 33)).is_err());
    }

    #[test]
    fn rejects_non_png() {
        assert!(validate_skin(b"not a png at all, just text......").is_err());
        assert!(validate_skin(&[0u8; 4]).is_err()); // слишком короткий
    }

    #[test]
    fn rejects_oversized() {
        // Валидный заголовок 64×64, но раздутый файл больше лимита → отказ.
        let mut big = png_header(64, 64);
        big.resize((MAX_SKIN_BYTES + 1) as usize, 0);
        assert!(validate_skin(&big).is_err());
    }
}
