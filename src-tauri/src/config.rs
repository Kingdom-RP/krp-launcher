//! Статическая конфигурация лаунчера.

/// Базовый URL, где лежат `manifest.json` и файлы игры (способ Б — раздача
/// заранее собранного набора).
///
/// Источник — GitHub Pages организации Kingdom-RP (раздаёт `dist/` сборщика).
/// Меняется только здесь — остальной код берёт URL через [`manifest_url`].
pub const MANIFEST_BASE_URL: &str = "https://kingdom-rp.github.io/krp-mod";

/// Зеркала [`MANIFEST_BASE_URL`] в порядке приоритета (fallback, если основной
/// источник недоступен — например, GitHub блокируется с российских IP). Каждое
/// зеркало — база с той же структурой каталогов, что и основной источник
/// (заливается тем же `dist/` сборщика). Пустой список = зеркал нет.
///
/// Yandex Object Storage (path-style): `https://storage.yandexcloud.net/<bucket>`.
pub const MIRRORS: &[&str] = &["https://storage.yandexcloud.net/kingdomrp"];

/// «Липкое» предпочтение зеркала на время сессии: как только зеркало впервые
/// помогло (основной источник не ответил), дальше пробуем зеркало ПЕРВЫМ — чтобы
/// заблокированный игрок не ждал connect-timeout основного источника на каждом
/// файле модпака.
static PREFER_MIRROR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Запомнить, что зеркало сработало (основной источник, вероятно, недоступен).
pub fn set_prefer_mirror() {
    PREFER_MIRROR.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn prefer_mirror() -> bool {
    PREFER_MIRROR.load(std::sync::atomic::Ordering::Relaxed)
}

/// Список URL-кандидатов для загрузки: основной источник (primary) + зеркала.
///
/// Зеркало строится как `{MIRROR}/{ключ}`, где ключ — путь файла в раскладке
/// `dist/` (совпадает с `FileEntry.path`). S3-бакет повторяет структуру `dist/`,
/// поэтому один ключ работает и для наших файлов, и для сторонних модов (их
/// сборщик тоже кладёт в `dist/mods/`).
///
/// Ключ зеркала определяется так:
/// - `mirror_key = Some(path)` — явный путь (сторонние моды: primary-url = внешний
///   CDN, а mirror-ключ = `mods/<jar>`);
/// - `mirror_key = None` — вывести из `url`, сняв префикс [`MANIFEST_BASE_URL`]
///   (наши файлы/JRE/манифест). Если url не с нашего base (ваниль с Mojang) —
///   зеркал нет, возвращается только исходный url.
///
/// Порядок: если в этой сессии зеркало уже выручало ([`prefer_mirror`]) — зеркала
/// идут первыми (основной источник, вероятно, заблокирован), иначе — основной.
pub fn url_candidates(url: &str, mirror_key: Option<&str>) -> Vec<String> {
    let key: Option<String> = match mirror_key {
        Some(k) => Some(k.trim_start_matches('/').to_string()),
        None => url
            .strip_prefix(MANIFEST_BASE_URL)
            .map(|r| r.trim_start_matches('/').to_string()),
    };
    let Some(key) = key else {
        return vec![url.to_string()];
    };
    if MIRRORS.is_empty() {
        return vec![url.to_string()];
    }
    let mirrored: Vec<String> = MIRRORS
        .iter()
        .map(|m| format!("{}/{}", m.trim_end_matches('/'), key))
        .collect();
    if prefer_mirror() {
        let mut out = mirrored;
        out.push(url.to_string());
        out
    } else {
        let mut out = vec![url.to_string()];
        out.extend(mirrored);
        out
    }
}

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
pub const SERVER_ADDR: &str = "45.11.16.75:25061";

/// Имя сервера в списке мультиплеера (`servers.dat`), которое лаунчер прописывает.
pub const SERVER_NAME: &str = "Kingdom RP";

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
pub const AUTH_BASE_URL: &str = "https://45.87.121.82.nip.io";

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
