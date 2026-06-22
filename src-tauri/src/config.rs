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

/// URL detached-подписи манифеста (minisign, `.minisig`).
pub fn manifest_sig_url() -> String {
    format!("{MANIFEST_BASE_URL}/manifest.json.minisig")
}

/// Публичный ключ minisign для проверки подписи `manifest.json` — строка вида
/// `RWR...` (вторая строка `.pub`-файла, без комментария).
///
/// ВАЖНО (порядок раскатки, fail-safe): пока ключ ПУСТОЙ, проверка подписи
/// пропускается (текущее поведение — доверие TLS+GitHub Pages). Как только сюда
/// вписан ключ — лаунчер ТРЕБУЕТ валидную подпись (fail-closed). Поэтому сначала
/// включаем подпись в CI `krp-mod` (чтобы `manifest.json.minisig` уже лежал на
/// Pages), убеждаемся, что он отдаётся, и ТОЛЬКО потом вписываем ключ сюда и
/// выпускаем релиз лаунчера. Иначе все установки упрутся в ошибку проверки.
///
/// Ключ отдельный от ключа автообновления (изоляция компрометации). Генерация —
/// см. `docs/manifest-signing.md`.
pub const MANIFEST_PUBKEY: &str = "RWTVxCmeK8DmSTMj246tJXSGU2zYprKzwE8f+mM7aTkHUVqnzuUEKQ/X";

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
