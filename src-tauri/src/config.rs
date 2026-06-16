//! Статическая конфигурация лаунчера.

/// Базовый URL, где лежат `manifest.json` и файлы игры (способ Б — раздача
/// заранее собранного набора).
///
/// Источник — GitHub Pages организации Kingdom-RP (раздаёт `dist/` сборщика).
/// Меняется только здесь — остальной код берёт URL через [`manifest_url`].
pub const MANIFEST_BASE_URL: &str = "https://kingdom-rp.github.io/krp-mod";

/// Полный URL манифеста.
pub fn manifest_url() -> String {
    format!("{MANIFEST_BASE_URL}/manifest.json")
}

/// Целевая версия Minecraft (ваниль тянется с Mojang по этой версии).
pub const MINECRAFT_VERSION: &str = "1.20.1";

/// Целевая версия NeoForge (артефакт `net.neoforged:forge:1.20.1-<NEOFORGE_VERSION>`).
pub const NEOFORGE_VERSION: &str = "47.1.106";
