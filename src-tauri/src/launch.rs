//! Построение команды запуска Minecraft + NeoForge (1.20.1) и старт процесса.
//!
//! Алгоритм — стандартный для сторонних лаунчеров: NeoForge-профиль
//! `inheritsFrom` ваниль, поэтому сливаем оба version JSON:
//! - classpath = библиотеки NeoForge + ванильные (дедуп по group:artifact) +
//!   ванильный `client.jar`;
//! - mainClass — из NeoForge-профиля (`cpw.mods.bootstraplauncher.BootstrapLauncher`);
//! - JVM- и game-аргументы — ванильные (только строковые, без rule-объектов —
//!   они для опциональных фич) + NeoForge, с подстановкой плейсхолдеров.
//!
//! Патченый клиент (client-srg/extra, forge-client) НЕ кладём в classpath —
//! modlauncher NeoForge находит их по `-DlibraryDirectory`.
//!
//! Авторизация пока офлайн (фиктивные uuid/token); онлайн-вход — отдельная фаза.

use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::process::{Command, Stdio};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::{LauncherError, Result};

#[derive(Debug, Deserialize)]
struct RawVersion {
    #[serde(rename = "mainClass", default)]
    main_class: Option<String>,
    #[serde(default)]
    arguments: Option<Arguments>,
    #[serde(default)]
    libraries: Vec<RawLibrary>,
    #[serde(default)]
    assets: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Arguments {
    #[serde(default)]
    jvm: Vec<serde_json::Value>,
    #[serde(default)]
    game: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct RawLibrary {
    name: String,
    #[serde(default)]
    downloads: Option<LibDownloads>,
    #[serde(default)]
    rules: Vec<Rule>,
}

#[derive(Debug, Deserialize)]
struct LibDownloads {
    #[serde(default)]
    artifact: Option<LibArtifact>,
}

#[derive(Debug, Deserialize)]
struct LibArtifact {
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Rule {
    action: String,
    #[serde(default)]
    os: Option<OsRule>,
}

#[derive(Debug, Deserialize)]
struct OsRule {
    #[serde(default)]
    name: Option<String>,
}

fn os_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    }
}

fn lib_allowed(rules: &[Rule]) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for r in rules {
        let matches = match &r.os {
            Some(os) => os.name.as_deref().map_or(true, |n| n == os_name()),
            None => true,
        };
        if matches {
            allowed = r.action == "allow";
        }
    }
    allowed
}

/// `group:artifact:version[:classifier]` → относительный путь maven.
fn maven_to_path(name: &str) -> String {
    let parts: Vec<&str> = name.splitn(4, ':').collect();
    let group = parts.first().copied().unwrap_or("").replace('.', "/");
    let artifact = parts.get(1).copied().unwrap_or("");
    let version = parts.get(2).copied().unwrap_or("");
    let file = match parts.get(3) {
        Some(classifier) => format!("{artifact}-{version}-{classifier}.jar"),
        None => format!("{artifact}-{version}.jar"),
    };
    format!("{group}/{artifact}/{version}/{file}")
}

/// Ключ дедупа classpath: `group:artifact`.
fn lib_key(name: &str) -> String {
    let parts: Vec<&str> = name.splitn(3, ':').collect();
    format!(
        "{}:{}",
        parts.first().copied().unwrap_or(""),
        parts.get(1).copied().unwrap_or("")
    )
}

fn read_version(path: &Path) -> Result<RawVersion> {
    let bytes = std::fs::read(path)
        .map_err(|e| LauncherError::Other(format!("не прочитать {}: {e}", path.display())))?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Офлайн-UUID из имени (детерминированный; для одиночной игры/нашего сервера).
fn offline_uuid(name: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("OfflinePlayer:{name}").as_bytes());
    let h = hasher.finalize();
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        u32::from_be_bytes([h[0], h[1], h[2], h[3]]),
        u16::from_be_bytes([h[4], h[5]]),
        (u16::from_be_bytes([h[6], h[7]]) & 0x0fff) | 0x3000, // версия 3
        (u16::from_be_bytes([h[8], h[9]]) & 0x3fff) | 0x8000, // вариант
        u64::from_be_bytes([0, 0, h[10], h[11], h[12], h[13], h[14], h[15]])
    )
}

fn substitute(arg: &str, vars: &[(&str, String)]) -> String {
    let mut out = arg.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("${{{k}}}"), v);
    }
    out
}

/// Собрать аргументы и запустить процесс игры. Возвращает PID.
///
/// - `install_dir` — папка игры (содержит versions/libraries/assets/mods…)
/// - `mc_version` — ванильная версия (напр. "1.20.1")
/// - `neoforge_profile_rel` — путь профиля NeoForge относительно install_dir
/// - `java_exe` — путь к java
/// - `player_name` — имя игрока (офлайн)
pub fn build_args(
    install_dir: &Path,
    mc_version: &str,
    neoforge_profile_rel: &str,
    player_name: &str,
) -> Result<Vec<String>> {
    let vanilla_path = install_dir
        .join("versions")
        .join(mc_version)
        .join(format!("{mc_version}.json"));
    let forge_path = install_dir.join(neoforge_profile_rel.replace('/', std::path::MAIN_SEPARATOR_STR));

    let vanilla = read_version(&vanilla_path)?;
    let forge = read_version(&forge_path)?;

    // ---- classpath: NeoForge-либы, затем ванильные (дедуп), затем client.jar ----
    let sep = if cfg!(windows) { ';' } else { ':' };
    let libraries_dir = install_dir.join("libraries");
    let mut seen: HashSet<String> = HashSet::new();
    let mut cp: Vec<String> = Vec::new();

    for lib in forge.libraries.iter().chain(vanilla.libraries.iter()) {
        if !lib_allowed(&lib.rules) {
            continue;
        }
        if !seen.insert(lib_key(&lib.name)) {
            continue;
        }
        let rel = lib
            .downloads
            .as_ref()
            .and_then(|d| d.artifact.as_ref())
            .and_then(|a| a.path.clone())
            .unwrap_or_else(|| maven_to_path(&lib.name));
        cp.push(
            libraries_dir
                .join(rel.replace('/', std::path::MAIN_SEPARATOR_STR))
                .to_string_lossy()
                .into_owned(),
        );
    }
    // Ванильный client.jar.
    cp.push(
        install_dir
            .join("versions")
            .join(mc_version)
            .join(format!("{mc_version}.jar"))
            .to_string_lossy()
            .into_owned(),
    );
    let classpath = cp.join(&sep.to_string());

    let assets_index = vanilla.assets.clone().unwrap_or_else(|| mc_version.to_string());
    let version_name = forge_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| mc_version.to_string());
    let natives_dir = install_dir.join("natives");
    std::fs::create_dir_all(&natives_dir).ok();

    let vars: Vec<(&str, String)> = vec![
        ("classpath", classpath),
        ("classpath_separator", sep.to_string()),
        ("library_directory", libraries_dir.to_string_lossy().into_owned()),
        ("natives_directory", natives_dir.to_string_lossy().into_owned()),
        ("version_name", version_name),
        ("launcher_name", "krp-launcher".into()),
        ("launcher_version", "0.1.0".into()),
        ("game_directory", install_dir.to_string_lossy().into_owned()),
        ("assets_root", install_dir.join("assets").to_string_lossy().into_owned()),
        ("assets_index_name", assets_index),
        ("auth_player_name", player_name.into()),
        ("auth_uuid", offline_uuid(player_name)),
        ("auth_access_token", "0".into()),
        ("clientid", String::new()),
        ("auth_xuid", String::new()),
        ("user_type", "msa".into()),
        ("version_type", "release".into()),
        ("user_properties", "{}".into()),
    ];

    // Только строковые аргументы (rule-объекты — опциональные фичи, пропускаем).
    let collect_str = |args: &Option<Arguments>, pick: fn(&Arguments) -> &Vec<serde_json::Value>| {
        args.as_ref()
            .map(|a| {
                pick(a)
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| substitute(s, &vars)))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };

    let mut cmd_args: Vec<String> = Vec::new();
    // JVM: ваниль + NeoForge.
    cmd_args.extend(collect_str(&vanilla.arguments, |a| &a.jvm));
    cmd_args.extend(collect_str(&forge.arguments, |a| &a.jvm));
    // main class — из NeoForge.
    let main_class = forge
        .main_class
        .or(vanilla.main_class)
        .ok_or_else(|| LauncherError::Other("нет mainClass в version JSON".into()))?;
    cmd_args.push(main_class);
    // game: ваниль + NeoForge.
    cmd_args.extend(collect_str(&vanilla.arguments, |a| &a.game));
    cmd_args.extend(collect_str(&forge.arguments, |a| &a.game));

    Ok(cmd_args)
}

/// Прочитать последние `max_bytes` байт текстового файла (хвост лога игры).
fn read_log_tail(path: &Path, max_bytes: u64) -> String {
    let Ok(mut file) = std::fs::File::open(path) else {
        return String::new();
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(max_bytes);
    if start > 0 {
        let _ = file.seek(SeekFrom::Start(start));
    }
    let mut buf = String::new();
    let _ = file.read_to_string(&mut buf);
    buf
}

/// Собрать аргументы и запустить java. Возвращает PID процесса игры.
///
/// stdout/stderr игры перенаправляются в `<install>/logs/latest-launch.log`
/// (раньше java открывала отдельное окно консоли, которое мелькало и
/// закрывалось — причину падения было не видно). На Windows окно консоли не
/// создаётся. После старта короткое время ждём: если процесс упал сразу
/// (битый classpath, ошибка JVM и т.п.) — возвращаем ошибку с хвостом лога,
/// а не ложное «игра запущена».
pub fn launch(
    install_dir: &Path,
    mc_version: &str,
    neoforge_profile_rel: &str,
    java_exe: &Path,
    player_name: &str,
) -> Result<u32> {
    let cmd_args = build_args(install_dir, mc_version, neoforge_profile_rel, player_name)?;

    let logs_dir = install_dir.join("logs");
    std::fs::create_dir_all(&logs_dir).ok();
    let log_path = logs_dir.join("latest-launch.log");
    let stdout_file = std::fs::File::create(&log_path).map_err(|e| {
        LauncherError::Other(format!("не создать лог запуска {}: {e}", log_path.display()))
    })?;
    let stderr_file = stdout_file
        .try_clone()
        .map_err(|e| LauncherError::Other(format!("клонирование лог-файла игры: {e}")))?;

    log::info!(
        "launch: {} (аргументов: {}); лог игры → {}",
        java_exe.display(),
        cmd_args.len(),
        log_path.display()
    );

    let mut command = Command::new(java_exe);
    command
        .args(&cmd_args)
        .current_dir(install_dir)
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file));

    // Windows: не плодить мелькающее окно консоли для java.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = command
        .spawn()
        .map_err(|e| LauncherError::Other(format!("не запустить java: {e}")))?;
    let pid = child.id();

    // Детект раннего краха: даём процессу немного времени и проверяем статус.
    std::thread::sleep(std::time::Duration::from_millis(2500));
    match child.try_wait() {
        Ok(Some(status)) if !status.success() => {
            let tail = read_log_tail(&log_path, 4000);
            log::error!("launch: игра завершилась сразу ({status}). Хвост лога:\n{tail}");
            return Err(LauncherError::Other(format!(
                "Игра завершилась сразу после запуска ({status}). \
                 Полный лог: {}\n\n…{tail}",
                log_path.display()
            )));
        }
        Ok(Some(status)) => {
            log::warn!("launch: процесс игры завершился сразу с кодом успеха ({status})");
        }
        Ok(None) => log::info!("launch: игра работает, pid={pid}"),
        Err(e) => log::warn!("launch: не удалось опросить статус процесса игры: {e}"),
    }

    Ok(pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Сборка аргументов на реальных version JSON. Укажи папку установки в
    /// `KRP_TEST_INSTALL` (иначе тест пропускается).
    /// `KRP_TEST_INSTALL=D:\Temp\krp_nf_install cargo test build_launch_args -- --nocapture`
    #[test]
    fn build_launch_args() {
        let Ok(dir) = std::env::var("KRP_TEST_INSTALL") else {
            eprintln!("skip: задай KRP_TEST_INSTALL");
            return;
        };
        let install = Path::new(&dir);
        let args = build_args(
            install,
            "1.20.1",
            "versions/1.20.1-forge-47.1.106/1.20.1-forge-47.1.106.json",
            "Tester",
        )
        .expect("build_args");
        let joined = args.join(" ");
        assert!(
            args.iter().any(|a| a == "cpw.mods.bootstraplauncher.BootstrapLauncher"),
            "нет mainClass BootstrapLauncher"
        );
        assert!(args.iter().any(|a| a == "forgeclient"), "нет --launchTarget forgeclient");
        assert!(!joined.contains("${"), "остались нераскрытые плейсхолдеры:\n{joined}");
        eprintln!("ARGS ({}):\n{}", args.len(), joined);
    }
}
