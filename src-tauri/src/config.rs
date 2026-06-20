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

/// База auth-сервера (drasl, Yggdrasil-совместимый). Сейчас — локальный dev-инстанс;
/// при выезде на публичный сервер меняем здесь (или оверрайдим через
/// `settings.json` → `auth_base_url` для теста без пересборки). Отсюда же строится
/// URL для `authlib-injector` (`<base>/authlib-injector`).
pub const AUTH_BASE_URL: &str = "http://localhost:25585";

/// Целевая версия Minecraft (ваниль тянется с Mojang по этой версии).
pub const MINECRAFT_VERSION: &str = "1.21.1";

/// Целевая версия NeoForge (артефакт `net.neoforged:neoforge:<NEOFORGE_VERSION>`).
/// Справочная константа (фактическую версию запуска лаунчер берёт из манифеста).
#[allow(dead_code)]
pub const NEOFORGE_VERSION: &str = "21.1.233";
