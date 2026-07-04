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

/// Адрес игрового MC-сервера (host:port) для проверки статуса (Server List Ping)
/// и подписи «Онлайн/Оффлайн» + текущего онлайна в лаунчере. Конфигурируемо, как
/// и остальные адреса; при выезде на публичный сервер — сменить здесь.
pub const SERVER_ADDR: &str = "localhost:25565";

/// Разбить [`SERVER_ADDR`] на (host, port). Порт по умолчанию — 25565.
pub fn server_host_port() -> (String, u16) {
    match SERVER_ADDR.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().unwrap_or(25565)),
        None => (SERVER_ADDR.to_string(), 25565),
    }
}

/// База auth-сервера (drasl, Yggdrasil-совместимый). Сейчас — локальный dev-инстанс;
/// при выезде на публичный сервер меняем здесь (или оверрайдим через
/// `settings.json` → `auth_base_url` для теста без пересборки). Отсюда же строится
/// URL для `authlib-injector` (`<base>/authlib-injector`).
pub const AUTH_BASE_URL: &str = "http://localhost:25585";

/// Целевая версия Minecraft (ваниль тянется с Mojang по этой версии).
pub const MINECRAFT_VERSION: &str = "1.21.1";

/// Память клиента по умолчанию (МБ), если игрок не задал своё.
pub const DEFAULT_MAX_MEMORY_MB: u32 = 4096;
/// Границы ползунка памяти в UI (МБ).
pub const MIN_MEMORY_MB: u32 = 2048;
pub const MAX_MEMORY_MB: u32 = 16384;

/// Рекомендуемые JVM-аргументы производительности для КЛИЕНТА (G1GC-твики,
/// Java 21, моддед). Память (`-Xms/-Xmx`) добавляется отдельно из настроек.
pub const JVM_PERF_ARGS: &[&str] = &[
    "-XX:+UnlockExperimentalVMOptions",
    "-XX:+UseG1GC",
    "-XX:G1NewSizePercent=20",
    "-XX:G1ReservePercent=20",
    "-XX:MaxGCPauseMillis=50",
    "-XX:G1HeapRegionSize=32M",
    "-XX:+ParallelRefProcEnabled",
    "-XX:+DisableExplicitGC",
];

/// Целевая версия NeoForge (артефакт `net.neoforged:neoforge:<NEOFORGE_VERSION>`).
/// Справочная константа (фактическую версию запуска лаунчер берёт из манифеста).
#[allow(dead_code)]
pub const NEOFORGE_VERSION: &str = "21.1.233";
