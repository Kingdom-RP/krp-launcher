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
//!
//! Запуск (из папки builder):
//!   cargo run --release -- --base-url https://.../download --mod-jar <path>
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
}

struct Config {
    mc: String,
    neoforge: String,
    base_url: String,
    mod_jar: PathBuf,
    modpack_version: String,
    out: PathBuf,
    work: PathBuf,
    skip_install: bool,
    skip_jre: bool,
    skip_authlib: bool,
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
    // с фикс. SHA-256. Windows = .zip, Linux = .tar.gz.
    let mut java = BTreeMap::new();
    if !cfg.skip_jre {
        // (ключ платформы, os, arch, расширение архива Adoptium)
        let platforms = [
            ("windows-x64", "windows", "x64", "zip"),
            ("linux-x64", "linux", "x64", "tar.gz"),
        ];
        let http = reqwest::blocking::Client::builder().build()?;
        for (key, os, arch, ext) in platforms {
            let rel = format!("java/jre21-{key}.{ext}");
            let dest = dist.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
            fs::create_dir_all(dest.parent().unwrap())?;
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
            fs::write(&dest, &bytes)?;
            let sha256 = sha256_file(&dest)?;
            java.insert(
                key.to_string(),
                JavaEntry {
                    url: format!("{}/{}", cfg.base_url.trim_end_matches('/'), rel),
                    sha256,
                    size: bytes.len() as u64,
                    dir: "runtime".to_string(),
                },
            );
            println!("JRE: {rel} ({} МБ)", bytes.len() / 1024 / 1024);
        }
    } else {
        println!("--skip-jre: java-entry в манифест не добавлен");
    }

    // Обход dist/ → manifest (java/ исключаем — он идёт отдельной секцией `java`).
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
        } else {
            "library"
        };
        files.push(FileEntry {
            url: format!("{}/{}", cfg.base_url.trim_end_matches('/'), rel),
            path: rel,
            sha256,
            size,
            kind: kind.to_string(),
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    println!("всего файлов в манифесте: {}", files.len());

    let manifest = Manifest {
        version: cfg.modpack_version.clone(),
        minecraft: cfg.mc.clone(),
        neoforge: cfg.neoforge.clone(),
        neoforge_profile: Some(profile_rel),
        authlib_injector,
        java,
        files,
    };
    let manifest_path = dist.join("manifest.json");
    fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)?;
    println!("\nГОТОВО → {}", manifest_path.display());
    println!("Залей содержимое {} на GitHub Releases (база = --base-url).", dist.display());
    Ok(())
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
            "--version" => {
                modpack_version = args.next().ok_or_else(|| anyhow!("--version requires value"))?
            }
            "--out" => out = PathBuf::from(args.next().ok_or_else(|| anyhow!("--out requires value"))?),
            "--work" => work = PathBuf::from(args.next().ok_or_else(|| anyhow!("--work requires value"))?),
            "--skip-install" => skip_install = true,
            "--skip-jre" => skip_jre = true,
            "--skip-authlib" => skip_authlib = true,
            "-h" | "--help" => {
                println!("krp-builder --base-url URL [--mod-jar PATH] [--mc 1.21.1] [--neoforge 21.1.233] [--version 1.0.0] [--out dist] [--work build-work] [--skip-install]");
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
        modpack_version,
        out,
        work,
        skip_install,
        skip_jre,
        skip_authlib,
    })
}
