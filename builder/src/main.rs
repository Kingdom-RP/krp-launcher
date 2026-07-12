//! Сборщик дистрибутива (способ Б) для лаунчера Kingdom RP.
//!
//! Что делает:
//! 1. Скачивает NeoForge installer (`net.neoforged:neoforge:<neo>`, 1.21+).
//! 2. Прогоняет его headless (`--installClient`) во временную папку — это
//!    выполняет processors (патчинг клиента) и раскладывает `libraries/`
//!    + version JSON.
//! 3. Собирает `dist/`: вся `libraries/` (включая processor-выводы) +
//!    version JSON NeoForge + jar мода. Ваниль (ассеты/клиент) НЕ хостим —
//!    лаунчер берёт её с Mojang.
//! 4. Считает SHA-256 каждого файла и пишет `manifest.json`.
//! 5. (Опц.) Сторонние моды из `--modlist <toml>`: качает каждый в кэш, считает/
//!    сверяет SHA-256 и пишет в манифест ВНЕШНИЙ url (НЕ base_url). Сами jar'ы в
//!    dist не кладутся — лаунчер качает их прямо из их источника (отд. репо/бакет).
//!
//! Запуск (из папки builder):
//!   cargo run --release -- --base-url https://.../download --mod-jar <path> [--modlist modlist.toml]
//!
//! Требуется `java` в PATH.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

// ---- выходной формат manifest.json (зеркалит src-tauri/src/manifest.rs) ----

#[derive(Serialize)]
struct Manifest {
    version: String,
    minecraft: String,
    neoforge: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    neoforge_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    authlib_injector: Option<String>,
    java: BTreeMap<String, JavaEntry>,
    files: Vec<FileEntry>,
    /// modId клиентских (side=client) модов, включая вложенные JiJ. Серверный
    /// синк пишет их в extraAllowedMods (анти-чит whitelist krp-mod).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    client_mod_ids: Vec<String>,
}

#[derive(Serialize)]
struct JavaEntry {
    url: String,
    sha256: String,
    size: u64,
    dir: String,
}

#[derive(Serialize)]
struct FileEntry {
    path: String,
    url: String,
    sha256: String,
    size: u64,
    kind: String,
    /// Сторона: "client" | "server" | "both". Лаунчер берёт client+both,
    /// будущий серверный синк — server+both.
    side: String,
}

struct Config {
    mc: String,
    neoforge: String,
    base_url: String,
    mod_jar: PathBuf,
    modlist: Option<PathBuf>,
    /// Шейдерпаки (.zip) — копируются как есть в `dist/shaderpacks/` (НЕ
    /// распаковываются). Лаунчер кладёт их в `shaderpacks/` папки игры.
    shaderpacks: Vec<PathBuf>,
    /// Путь к sides.toml (client/server-списки). Нет — все моды "both".
    sides: Option<PathBuf>,
    /// Папка конфигов модпака (копируется в dist/config, хостится на Pages).
    config_dir: Option<PathBuf>,
    /// config-sides.toml (сторона конфигов по подстроке пути). Нет — all both.
    config_sides: Option<PathBuf>,
    /// Репо `owner/name`, из Release которого автоматически берутся сторонние моды.
    mods_release: Option<String>,
    /// Тег Release для `mods_release`.
    mods_tag: String,
    modpack_version: String,
    out: PathBuf,
    work: PathBuf,
    skip_install: bool,
    skip_jre: bool,
    skip_authlib: bool,
    /// Только сторонние моды: пропустить установку NeoForge/JRE/dist, написать
    /// manifest лишь с модами (быстрая проверка modlist/release).
    modlist_only: bool,
}

fn main() -> Result<()> {
    let cfg = parse_args()?;
    println!(
        "Kingdom RP builder: MC {} / NeoForge {}\n  base-url: {}\n  mod-jar:  {}\n  out:      {}\n  work:     {}",
        cfg.mc,
        cfg.neoforge,
        cfg.base_url,
        cfg.mod_jar.display(),
        cfg.out.display(),
        cfg.work.display()
    );

    // Режим «только моды»: пропускаем установку NeoForge/JRE/dist и пишем
    // manifest лишь со сторонними модами (быстрая проверка modlist/release).
    if cfg.modlist_only {
        return run_modlist_only(&cfg);
    }

    // NeoForge 1.21+ ставит профиль как `neoforge-<ver>` (без mc в имени);
    // на 1.20.1 это было `<mc>-forge-<ver>`.
    let version_id = format!("neoforge-{}", cfg.neoforge);
    let profile_rel = format!("versions/{0}/{0}.json", version_id);

    if !cfg.skip_install {
        run_installer(&cfg)?;
    } else {
        println!("--skip-install: переиспользую {}", cfg.work.display());
    }

    // Проверка, что установщик произвёл нужное.
    let work_libs = cfg.work.join("libraries");
    let work_profile = cfg
        .work
        .join("versions")
        .join(&version_id)
        .join(format!("{version_id}.json"));
    if !work_libs.is_dir() {
        bail!("нет {} — установщик не отработал?", work_libs.display());
    }
    if !work_profile.is_file() {
        bail!("нет version JSON {}", work_profile.display());
    }
    if !cfg.mod_jar.is_file() {
        bail!(
            "не найден jar мода: {} (собери krp-mod: ./gradlew build)",
            cfg.mod_jar.display()
        );
    }

    // Сборка dist/.
    if cfg.out.exists() {
        fs::remove_dir_all(&cfg.out).ok();
    }
    let dist = &cfg.out;
    fs::create_dir_all(dist)?;

    // 1) libraries/** (кроме .cache-файлов)
    let copied_libs = copy_tree(
        &work_libs,
        &dist.join("libraries"),
        |p| p.extension().map_or(true, |e| e != "cache"),
    )?;
    println!("скопировано библиотек: {copied_libs}");

    // 2) version JSON NeoForge
    let dist_profile = dist.join(profile_rel.replace('/', std::path::MAIN_SEPARATOR_STR));
    fs::create_dir_all(dist_profile.parent().unwrap())?;
    fs::copy(&work_profile, &dist_profile)?;

    // 3) jar мода
    let mod_name = cfg
        .mod_jar
        .file_name()
        .ok_or_else(|| anyhow!("плохое имя jar мода"))?
        .to_string_lossy()
        .into_owned();
    let dist_mods = dist.join("mods");
    fs::create_dir_all(&dist_mods)?;
    fs::copy(&cfg.mod_jar, dist_mods.join(&mod_name))?;
    println!("мод: mods/{mod_name}");

    // 3c) конфиги модпака (из krp-modpack/config) → dist/config (хостятся на Pages).
    if let Some(cd) = &cfg.config_dir {
        if cd.is_dir() {
            let n = copy_tree(cd, &dist.join("config"), |_| true)?;
            println!("скопировано конфигов: {n}");
        } else {
            eprintln!("--config-dir {} не папка — пропуск", cd.display());
        }
    }

    // 3a) шейдерпаки (.zip) — копируем как есть в dist/shaderpacks/ (НЕ распаковываем).
    if !cfg.shaderpacks.is_empty() {
        let dist_shaders = dist.join("shaderpacks");
        fs::create_dir_all(&dist_shaders)?;
        for sp in &cfg.shaderpacks {
            if !sp.is_file() {
                bail!("шейдерпак не найден: {}", sp.display());
            }
            let name = sp
                .file_name()
                .ok_or_else(|| anyhow!("плохое имя шейдерпака"))?;
            fs::copy(sp, dist_shaders.join(name))?;
            println!("шейдерпак: shaderpacks/{}", name.to_string_lossy());
        }
    }

    // 3b) authlib-injector.jar — Java-агент авторизации (фаза 6). Хостим у себя
    // (его релизы на GitHub в РФ режутся); качаем с официального maven yushi.moe.
    let authlib_injector = if cfg.skip_authlib {
        println!("--skip-authlib: authlib-injector не добавлен");
        None
    } else {
        let http = reqwest::blocking::Client::new();
        let meta_text = http
            .get("https://authlib-injector.yushi.moe/artifact/latest.json")
            .send()
            .context("запрос метаданных authlib-injector")?
            .error_for_status()?
            .text()?;
        let meta: serde_json::Value = serde_json::from_str(&meta_text)
            .context("разбор latest.json authlib-injector")?;
        let url = meta["download_url"]
            .as_str()
            .ok_or_else(|| anyhow!("нет download_url в latest.json authlib-injector"))?;
        let bytes = http
            .get(url)
            .send()
            .context("скачивание authlib-injector.jar")?
            .error_for_status()?
            .bytes()?;
        fs::write(dist.join("authlib-injector.jar"), &bytes)?;
        println!(
            "authlib-injector.jar: {} ({} КБ)",
            meta["version"].as_str().unwrap_or("?"),
            bytes.len() / 1024
        );
        Some("authlib-injector.jar".to_string())
    };

    // 4) JRE (Temurin 21) — снимки с Adoptium под каждую платформу, хостим у себя
    // с фикс. SHA-256. Windows = .zip, Linux = .tar.gz. Архивы кэшируем в
    // `<work>/jrecache` — на CI этот каталог переживает прогоны через actions/cache,
    // так что JRE не перекачивается, пока кэш жив.
    let mut java = BTreeMap::new();
    if !cfg.skip_jre {
        // (ключ платформы, os, arch, расширение архива Adoptium)
        let platforms = [
            ("windows-x64", "windows", "x64", "zip"),
            ("linux-x64", "linux", "x64", "tar.gz"),
        ];
        let http = reqwest::blocking::Client::builder().build()?;
        let jre_cache = cfg.work.join("jrecache");
        fs::create_dir_all(&jre_cache)?;
        for (key, os, arch, ext) in platforms {
            let rel = format!("java/jre21-{key}.{ext}");
            let dest = dist.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
            fs::create_dir_all(dest.parent().unwrap())?;
            let cached = jre_cache.join(format!("jre21-{key}.{ext}"));
            if cached.is_file() && fs::metadata(&cached)?.len() > 0 {
                println!("JRE из кэша: {key}");
            } else {
                let url = format!(
                    "https://api.adoptium.net/v3/binary/latest/21/ga/{os}/{arch}/jre/hotspot/normal/eclipse"
                );
                println!("скачиваю Temurin 21 JRE ({key})…");
                let bytes = http
                    .get(&url)
                    .send()
                    .context("запрос JRE")?
                    .error_for_status()
                    .context("HTTP JRE")?
                    .bytes()?;
                fs::write(&cached, &bytes)?;
            }
            fs::copy(&cached, &dest)?;
            let size = fs::metadata(&cached)?.len();
            java.insert(
                key.to_string(),
                JavaEntry {
                    url: format!("{}/{}", cfg.base_url.trim_end_matches('/'), rel),
                    sha256: sha256_file(&cached)?,
                    size,
                    dir: "runtime".to_string(),
                },
            );
            println!("JRE: {rel} ({} МБ)", size / 1024 / 1024);
        }
    } else {
        println!("--skip-jre: java-entry в манифест не добавлен");
    }

    // Обход dist/ → manifest (java/ исключаем — он идёт отдельной секцией `java`).
    // Сторона конфигов — из config-sides.toml (по подстроке пути); остальное both.
    let cfg_sides = Sides::load(cfg.config_sides.as_deref())?;
    let mut files = Vec::new();
    for entry in WalkDir::new(dist).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let abs = entry.path();
        let rel = abs
            .strip_prefix(dist)?
            .to_string_lossy()
            .replace('\\', "/");
        if rel == "manifest.json" || rel.starts_with("java/") {
            continue;
        }
        let size = fs::metadata(abs)?.len();
        let sha256 = sha256_file(abs)?;
        let kind = if rel.starts_with("mods/") {
            "mod"
        } else if rel.starts_with("config/") {
            "config"
        } else if rel.starts_with("shaderpacks/") {
            "shaderpack"
        } else {
            "library"
        };
        // Конфиги — сторона по config-sides; ядро NeoForge/мод — both.
        let side = if let Some(rest) = rel.strip_prefix("config/") {
            cfg_sides.side_for(rest)?
        } else {
            "both"
        };
        files.push(FileEntry {
            url: format!("{}/{}", cfg.base_url.trim_end_matches('/'), rel),
            path: rel,
            sha256,
            size,
            kind: kind.to_string(),
            side: side.to_string(),
        });
    }
    // Сторонние моды (modlist.toml и/или Release GitHub) — хостятся ВНЕ dist.
    // url в манифесте указывает на их источник напрямую, не на base_url.
    files.extend(collect_mods(&cfg)?);

    files.sort_by(|a, b| a.path.cmp(&b.path));
    println!("всего файлов в манифесте: {}", files.len());

    let cids = client_mod_ids(&cfg, &files)?;
    println!("client modId (для whitelist): {}", cids.len());
    let manifest = Manifest {
        version: cfg.modpack_version.clone(),
        minecraft: cfg.mc.clone(),
        neoforge: cfg.neoforge.clone(),
        neoforge_profile: Some(profile_rel),
        authlib_injector,
        java,
        files,
        client_mod_ids: cids,
    };
    let manifest_path = dist.join("manifest.json");
    write_and_sign_manifest(&manifest_path, &manifest)?;
    println!("\nГОТОВО → {}", manifest_path.display());
    println!("Залей содержимое {} на GitHub Releases (база = --base-url).", dist.display());
    Ok(())
}

/// Записать `manifest.json` и, если задан секретный ключ в окружении, рядом
/// положить detached minisign-подпись `manifest.json.minisig`.
///
/// Ключ берётся из `KRP_MANIFEST_SECRET_KEY` (содержимое `.key`-файла minisign;
/// в CI — секрет репозитория), пароль — из `KRP_MANIFEST_SECRET_KEY_PASSWORD`
/// (пусто/нет → ключ без пароля). Если переменной нет — подпись пропускается
/// (локальные прогоны/совместимость), лаунчер тогда тоже не требует подпись.
fn write_and_sign_manifest(manifest_path: &Path, manifest: &Manifest) -> Result<()> {
    let json = serde_json::to_string_pretty(manifest)?;
    fs::write(manifest_path, &json)?;

    let sk_str = match std::env::var("KRP_MANIFEST_SECRET_KEY") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            println!("⚠ подпись манифеста пропущена (нет KRP_MANIFEST_SECRET_KEY)");
            return Ok(());
        }
    };
    let password = std::env::var("KRP_MANIFEST_SECRET_KEY_PASSWORD")
        .ok()
        .filter(|s| !s.is_empty());

    // ВАЖНО: НИКОГДА не вызываем into_secret_key(None) — при зашифрованном ключе
    // minisign уходит в интерактивный запрос пароля через stdin, а в CI нет tty →
    // зависание навсегда. Поэтому пароль всегда Some(...).
    let sk = match password {
        Some(pw) => minisign::SecretKeyBox::from_string(&sk_str)
            .context("разбор KRP_MANIFEST_SECRET_KEY")?
            .into_secret_key(Some(pw))
            .context("расшифровка секретного ключа манифеста (неверный пароль?)")?,
        // Без пароля: rsign `generate -W` создаёт ключ, зашифрованный ПУСТЫМ
        // паролем (kdf присутствует, не KDF_NONE), поэтому into_unencrypted падает.
        // Пробуем оба варианта: настоящий unencrypted-ключ и «пустой пароль».
        None => {
            let unenc = minisign::SecretKeyBox::from_string(&sk_str)
                .context("разбор KRP_MANIFEST_SECRET_KEY")?
                .into_unencrypted_secret_key();
            match unenc {
                Ok(sk) => sk,
                Err(_) => minisign::SecretKeyBox::from_string(&sk_str)
                    .context("разбор KRP_MANIFEST_SECRET_KEY")?
                    .into_secret_key(Some(String::new()))
                    .context(
                        "ключ не расшифровывается ни как passwordless, ни пустым паролем — \
                         задайте KRP_MANIFEST_SECRET_KEY_PASSWORD",
                    )?,
            }
        }
    };
    let sig_box = minisign::sign(None, &sk, std::io::Cursor::new(json.as_bytes()), None, None)
        .context("подпись манифеста")?;

    let sig_path = manifest_path.with_file_name("manifest.json.minisig");
    fs::write(&sig_path, sig_box.into_string())?;
    println!("✓ подпись манифеста → {}", sig_path.display());
    Ok(())
}

// ---- сторонние моды (modlist.toml) ----

/// Корень modlist.toml: массив таблиц `[[mod]]`.
#[derive(serde::Deserialize)]
struct ModList {
    #[serde(default, rename = "mod")]
    mods: Vec<ModEntry>,
}

/// Одна запись стороннего мода. Хостится вне dist — url ведёт на его источник
/// (Release-ассет отдельного репо / объектный бакет).
#[derive(serde::Deserialize)]
struct ModEntry {
    /// Имя jar в папке mods игрока (без слэшей).
    file: String,
    /// Прямой URL для скачивания (уже URL-энкоден, если в имени есть `+`).
    url: String,
    /// Ожидаемый SHA-256 (hex). Если задан — сборщик сверяет загруженное;
    /// если опущен — вычисляет сам и подставляет в манифест.
    #[serde(default)]
    sha256: Option<String>,
}

/// modId клиентских модов (side=client) из кэша, включая вложенные JiJ.
/// Идут в manifest.client_mod_ids → серверный синк пишет их в extraAllowedMods.
fn client_mod_ids(cfg: &Config, files: &[FileEntry]) -> Result<Vec<String>> {
    let cache = cfg.work.join("modcache");
    let mut ids = std::collections::BTreeSet::new();
    for f in files {
        if f.kind != "mod" || f.side != "client" {
            continue;
        }
        let name = f.path.rsplit('/').next().unwrap_or(&f.path);
        let jar = cache.join(name);
        if !jar.is_file() {
            eprintln!("client_mod_ids: нет в кэше {} — пропуск", jar.display());
            continue;
        }
        for id in scan_mod_ids(&jar)? {
            ids.insert(id);
        }
    }
    Ok(ids.into_iter().collect())
}

/// modId из jar: собственные (META-INF/neoforge.mods.toml) + вложенные JiJ,
/// РЕКУРСИВНО и в обоих layout вложенности:
///   - META-INF/jarjar/ (NeoForge JiJ),
///   - META-INF/jars/   (Fabric-toolchain JiJ, напр. LambDynLights: там сидят
///                        spruceui/yumi_mc_core/pride/transition/trender и т.п.).
/// Без рекурсии терялись субмоды на 2-3 уровне -> анти-чит кикал легит-клиентов.
fn scan_mod_ids(jar: &Path) -> Result<Vec<String>> {
    let bytes = fs::read(jar).with_context(|| format!("чтение {}", jar.display()))?;
    let mut out = Vec::new();
    scan_jar_bytes(&bytes, &mut out).with_context(|| format!("zip {}", jar.display()))?;
    out.sort();
    out.dedup();
    Ok(out)
}

/// Рекурсивный скан modId из байтов jar: собственный neoforge.mods.toml +
/// вложенные jar (META-INF/jarjar/ и META-INF/jars/), спускаясь на все уровни.
fn scan_jar_bytes(bytes: &[u8], out: &mut Vec<String>) -> Result<()> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    if let Ok(mut e) = zip.by_name("META-INF/neoforge.mods.toml") {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut e, &mut s)?;
        out.extend(mod_ids_from_toml(&s));
    }
    let nested: Vec<String> = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|e| e.name().to_string()))
        .filter(|n| {
            (n.starts_with("META-INF/jarjar/") || n.starts_with("META-INF/jars/"))
                && n.ends_with(".jar")
        })
        .collect();
    for name in nested {
        let mut buf = Vec::new();
        {
            let mut e = zip.by_name(&name)?;
            std::io::Read::read_to_end(&mut e, &mut buf)?;
        }
        // Вложенный jar сам может нести JiJ -> рекурсия. Ошибку внутри не роняем.
        let _ = scan_jar_bytes(&buf, out);
    }
    Ok(())
}

/// modId из секций [[mods]] содержимого neoforge.mods.toml (зависимости игнор).
fn mod_ids_from_toml(text: &str) -> Vec<String> {
    let Ok(val) = text.parse::<toml::Value>() else {
        return Vec::new();
    };
    val.get("mods")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("modId").and_then(|v| v.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Режим `--modlist-only`: только сторонние моды, без NeoForge/JRE/dist.
/// Пишет минимальный manifest (моды + версии MC/NeoForge) для быстрой проверки.
fn run_modlist_only(cfg: &Config) -> Result<()> {
    println!("--modlist-only: пропускаю установку NeoForge/JRE/dist");
    let files = collect_mods(cfg)?;
    if files.is_empty() {
        bail!("нет модов: укажи --modlist <toml> и/или --mods-release <owner/repo>");
    }
    fs::create_dir_all(&cfg.out)?;
    let cids = client_mod_ids(cfg, &files)?;
    let manifest = Manifest {
        version: cfg.modpack_version.clone(),
        minecraft: cfg.mc.clone(),
        neoforge: cfg.neoforge.clone(),
        neoforge_profile: None,
        authlib_injector: None,
        java: BTreeMap::new(),
        files,
        client_mod_ids: cids,
    };
    let manifest_path = cfg.out.join("manifest.json");
    write_and_sign_manifest(&manifest_path, &manifest)?;
    println!("\nГОТОВО (только моды) → {}", manifest_path.display());
    Ok(())
}

/// Сторона мода: client/server-списки из sides.toml (подстрока имени файла).
/// Не перечислено → "both". Совпадение в обоих списках — ошибка конфигурации.
#[derive(Default)]
struct Sides {
    client: Vec<String>,
    server: Vec<String>,
}

impl Sides {
    fn load(path: Option<&Path>) -> Result<Self> {
        let Some(p) = path else {
            return Ok(Self::default());
        };
        #[derive(serde::Deserialize)]
        struct Raw {
            #[serde(default)]
            client: Vec<String>,
            #[serde(default)]
            server: Vec<String>,
        }
        let text =
            fs::read_to_string(p).with_context(|| format!("чтение sides {}", p.display()))?;
        let raw: Raw =
            toml::from_str(&text).with_context(|| format!("разбор TOML {}", p.display()))?;
        println!(
            "sides: {} client, {} server (остальное both)",
            raw.client.len(),
            raw.server.len()
        );
        Ok(Self {
            client: raw.client,
            server: raw.server,
        })
    }

    /// Определить сторону по имени jar. Возвращает "client" | "server" | "both".
    fn side_for(&self, filename: &str) -> Result<&'static str> {
        let lower = filename.to_lowercase();
        let is_client = self.client.iter().any(|s| lower.contains(&s.to_lowercase()));
        let is_server = self.server.iter().any(|s| lower.contains(&s.to_lowercase()));
        match (is_client, is_server) {
            (true, true) => bail!("{filename} перечислен и в client, и в server (sides.toml)"),
            (true, false) => Ok("client"),
            (false, true) => Ok("server"),
            (false, false) => Ok("both"),
        }
    }
}

/// Собирает сторонние моды из обоих источников: `--modlist` (явный пин-список)
/// и `--mods-release` (авто-список ассетов GitHub Release). При совпадении пути
/// приоритет у modlist (явный пин важнее авто-выборки). Сторона — из sides.toml.
fn collect_mods(cfg: &Config) -> Result<Vec<FileEntry>> {
    let sides = Sides::load(cfg.sides.as_deref())?;
    let mut out = collect_external_mods(cfg, &sides)?;
    let have: std::collections::HashSet<String> = out.iter().map(|e| e.path.clone()).collect();
    for e in collect_release_mods(cfg, &sides)? {
        if have.contains(&e.path) {
            println!("пропускаю {} из Release — уже задан в modlist", e.path);
            continue;
        }
        out.push(e);
    }
    Ok(out)
}

/// Авто-список модов из GitHub Release (`--mods-release owner/name --mods-tag`).
/// Читает ассеты через API, качает каждый `.jar` в кэш, считает SHA-256 и
/// возвращает `FileEntry` с url ассета. GH_TOKEN/GITHUB_TOKEN — против лимитов API.
fn collect_release_mods(cfg: &Config, sides: &Sides) -> Result<Vec<FileEntry>> {
    let Some(repo) = &cfg.mods_release else {
        return Ok(Vec::new());
    };
    #[derive(serde::Deserialize)]
    struct GhRelease {
        assets: Vec<GhAsset>,
    }
    #[derive(serde::Deserialize)]
    struct GhAsset {
        name: String,
        browser_download_url: String,
        #[serde(default)]
        size: u64,
    }

    let api = format!(
        "https://api.github.com/repos/{repo}/releases/tags/{}",
        cfg.mods_tag
    );
    println!("читаю Release: {api}");
    let http = reqwest::blocking::Client::builder()
        .user_agent("krp-builder")
        .build()?;
    let mut req = http.get(&api).header("Accept", "application/vnd.github+json");
    if let Ok(tok) = std::env::var("GH_TOKEN").or_else(|_| std::env::var("GITHUB_TOKEN")) {
        if !tok.is_empty() {
            req = req.bearer_auth(tok);
        }
    }
    let body = req
        .send()
        .context("запрос релиза GitHub")?
        .error_for_status()
        .with_context(|| format!("Release {repo}@{} не найден?", cfg.mods_tag))?
        .text()?;
    let rel: GhRelease = serde_json::from_str(&body).context("разбор JSON релиза")?;

    let cache = cfg.work.join("modcache");
    fs::create_dir_all(&cache)?;
    let mut out = Vec::new();
    for a in &rel.assets {
        if !a.name.ends_with(".jar") {
            continue;
        }
        let cached = cache.join(&a.name);
        // Кэш переиспользуем, если размер совпал (sha заранее неизвестен).
        let reuse = cached.is_file() && fs::metadata(&cached)?.len() == a.size && a.size != 0;
        if reuse {
            println!("мод из кэша: {}", a.name);
        } else {
            println!("качаю мод: {} ← {}", a.name, a.browser_download_url);
            let bytes = http
                .get(&a.browser_download_url)
                .send()
                .with_context(|| format!("запрос {}", a.browser_download_url))?
                .error_for_status()
                .with_context(|| format!("HTTP {}", a.browser_download_url))?
                .bytes()?;
            fs::write(&cached, &bytes)?;
        }
        out.push(FileEntry {
            path: format!("mods/{}", a.name),
            url: a.browser_download_url.clone(),
            sha256: sha256_file(&cached)?,
            size: fs::metadata(&cached)?.len(),
            kind: "mod".to_string(),
            side: sides.side_for(&a.name)?.to_string(),
        });
    }
    println!("модов из Release: {}", out.len());
    Ok(out)
}

/// Читает modlist.toml, скачивает каждый мод в кэш (`<work>/modcache`), считает
/// и при наличии сверяет SHA-256, и возвращает `FileEntry` с ВНЕШНИМ url.
/// Сами jar'ы в dist не копируются — лаунчер качает их напрямую по url.
fn collect_external_mods(cfg: &Config, sides: &Sides) -> Result<Vec<FileEntry>> {
    let Some(list_path) = &cfg.modlist else {
        return Ok(Vec::new());
    };
    let text = fs::read_to_string(list_path)
        .with_context(|| format!("чтение modlist {}", list_path.display()))?;
    let list: ModList = toml::from_str(&text)
        .with_context(|| format!("разбор TOML {}", list_path.display()))?;
    if list.mods.is_empty() {
        println!("modlist {} пуст — сторонних модов нет", list_path.display());
        return Ok(Vec::new());
    }

    let cache = cfg.work.join("modcache");
    fs::create_dir_all(&cache)?;
    let http = reqwest::blocking::Client::builder().build()?;

    let mut out = Vec::with_capacity(list.mods.len());
    for m in &list.mods {
        if m.file.contains('/') || m.file.contains('\\') || m.file.is_empty() {
            bail!("плохое имя файла мода в modlist: {:?}", m.file);
        }
        let cached = cache.join(&m.file);
        // Кэш переиспользуем, только если хеш совпадает с заявленным.
        let need_download = match (&m.sha256, cached.is_file()) {
            (Some(want), true) => !want.eq_ignore_ascii_case(&sha256_file(&cached)?),
            (None, true) => false, // без заявленного хеша доверяем уже скачанному
            (_, false) => true,
        };
        if need_download {
            println!("качаю мод: {} ← {}", m.file, m.url);
            let bytes = http
                .get(&m.url)
                .send()
                .with_context(|| format!("запрос {}", m.url))?
                .error_for_status()
                .with_context(|| format!("HTTP {}", m.url))?
                .bytes()?;
            fs::write(&cached, &bytes)?;
        } else {
            println!("мод из кэша: {}", m.file);
        }

        let sha256 = sha256_file(&cached)?;
        if let Some(want) = &m.sha256 {
            if !want.eq_ignore_ascii_case(&sha256) {
                bail!(
                    "SHA-256 не совпал для {}: ожидался {want}, получен {sha256}",
                    m.file
                );
            }
        }
        out.push(FileEntry {
            path: format!("mods/{}", m.file),
            url: m.url.clone(),
            sha256,
            size: fs::metadata(&cached)?.len(),
            kind: "mod".to_string(),
            side: sides.side_for(&m.file)?.to_string(),
        });
    }
    println!("сторонних модов из modlist: {}", out.len());
    Ok(out)
}

/// Скачать установщик и прогнать `--installClient`.
fn run_installer(cfg: &Config) -> Result<()> {
    fs::create_dir_all(&cfg.work)?;
    // Установщику нужен launcher_profiles.json в целевой папке.
    fs::write(
        cfg.work.join("launcher_profiles.json"),
        r#"{"profiles":{},"settings":{},"version":3}"#,
    )?;

    let installer = cfg.work.join("installer.jar");
    if !installer.is_file() {
        // NeoForge 1.21+: артефакт `net.neoforged:neoforge:<ver>` (без mc).
        let url = format!(
            "https://maven.neoforged.net/releases/net/neoforged/neoforge/{0}/neoforge-{0}-installer.jar",
            cfg.neoforge
        );
        println!("скачиваю установщик: {url}");
        let bytes = reqwest::blocking::Client::new()
            .get(&url)
            .send()
            .context("запрос установщика")?
            .error_for_status()
            .context("HTTP установщика")?
            .bytes()?;
        fs::write(&installer, &bytes)?;
        println!("установщик: {} байт", bytes.len());
    }

    println!("запускаю installer --installClient (headless)…");
    let status = Command::new("java")
        .arg("-jar")
        .arg(&installer)
        .arg("--installClient")
        .arg(&cfg.work)
        .status()
        .context("не удалось запустить java — он есть в PATH?")?;
    if !status.success() {
        bail!("установщик завершился с ошибкой: {status}");
    }
    Ok(())
}

/// Рекурсивно копирует `src` → `dst`, фильтруя файлы предикатом. Возвращает
/// число скопированных файлов.
fn copy_tree(src: &Path, dst: &Path, keep: impl Fn(&Path) -> bool) -> Result<usize> {
    let mut count = 0;
    for entry in WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() || !keep(entry.path()) {
            continue;
        }
        let rel = entry.path().strip_prefix(src)?;
        let target = dst.join(rel);
        fs::create_dir_all(target.parent().unwrap())?;
        fs::copy(entry.path(), &target)?;
        count += 1;
    }
    Ok(count)
}

fn sha256_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()))
}

fn parse_args() -> Result<Config> {
    let mut mc = "1.21.1".to_string();
    let mut neoforge = "21.1.233".to_string();
    let mut base_url = "https://example.com/kingdomrp".to_string();
    let mut mod_jar = PathBuf::from("../../krp-mod/build/libs/kingdomrpcore-0.1.0.jar");
    let mut modlist: Option<PathBuf> = None;
    let mut shaderpacks: Vec<PathBuf> = Vec::new();
    let mut sides: Option<PathBuf> = None;
    let mut config_dir: Option<PathBuf> = None;
    let mut config_sides: Option<PathBuf> = None;
    let mut mods_release: Option<String> = None;
    let mut mods_tag = "v1".to_string();
    let mut modlist_only = false;
    let mut modpack_version = "1.0.0".to_string();
    let mut out = PathBuf::from("dist");
    let mut work = PathBuf::from("build-work");
    let mut skip_install = false;
    let mut skip_jre = false;
    let mut skip_authlib = false;

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--mc" => mc = args.next().ok_or_else(|| anyhow!("--mc requires value"))?,
            "--neoforge" => {
                neoforge = args.next().ok_or_else(|| anyhow!("--neoforge requires value"))?
            }
            "--base-url" => {
                base_url = args.next().ok_or_else(|| anyhow!("--base-url requires value"))?
            }
            "--mod-jar" => {
                mod_jar = PathBuf::from(args.next().ok_or_else(|| anyhow!("--mod-jar requires value"))?)
            }
            "--modlist" => {
                modlist = Some(PathBuf::from(
                    args.next().ok_or_else(|| anyhow!("--modlist requires value"))?,
                ))
            }
            "--shaderpack" => shaderpacks.push(PathBuf::from(
                args.next().ok_or_else(|| anyhow!("--shaderpack requires value"))?,
            )),
            "--sides" => {
                sides = Some(PathBuf::from(
                    args.next().ok_or_else(|| anyhow!("--sides requires value"))?,
                ))
            }
            "--config-dir" => {
                config_dir = Some(PathBuf::from(
                    args.next().ok_or_else(|| anyhow!("--config-dir requires value"))?,
                ))
            }
            "--config-sides" => {
                config_sides = Some(PathBuf::from(
                    args.next().ok_or_else(|| anyhow!("--config-sides requires value"))?,
                ))
            }
            "--mods-release" => {
                mods_release =
                    Some(args.next().ok_or_else(|| anyhow!("--mods-release requires value"))?)
            }
            "--mods-tag" => {
                mods_tag = args.next().ok_or_else(|| anyhow!("--mods-tag requires value"))?
            }
            "--modlist-only" => modlist_only = true,
            "--version" => {
                modpack_version = args.next().ok_or_else(|| anyhow!("--version requires value"))?
            }
            "--out" => out = PathBuf::from(args.next().ok_or_else(|| anyhow!("--out requires value"))?),
            "--work" => work = PathBuf::from(args.next().ok_or_else(|| anyhow!("--work requires value"))?),
            "--skip-install" => skip_install = true,
            "--skip-jre" => skip_jre = true,
            "--skip-authlib" => skip_authlib = true,
            "-h" | "--help" => {
                println!("krp-builder --base-url URL [--mod-jar PATH]\n  Сторонние моды: [--modlist modlist.toml] [--mods-release owner/repo] [--mods-tag v1] [--sides sides.toml] [--modlist-only]\n  Шейдеры: [--shaderpack pack.zip] (повторяемо, кладётся в shaderpacks/)\n  Прочее: [--mc 1.21.1] [--neoforge 21.1.233] [--version 1.0.0] [--out dist] [--work build-work] [--skip-install] [--skip-jre] [--skip-authlib]");
                std::process::exit(0);
            }
            other => bail!("неизвестный аргумент: {other}"),
        }
    }

    Ok(Config {
        mc,
        neoforge,
        base_url,
        mod_jar,
        modlist,
        shaderpacks,
        sides,
        config_dir,
        config_sides,
        mods_release,
        mods_tag,
        modlist_only,
        modpack_version,
        out,
        work,
        skip_install,
        skip_jre,
        skip_authlib,
    })
}
