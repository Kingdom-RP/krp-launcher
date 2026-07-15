//! Локальный слепок установленной игры (`<install>/.krp_state.json`).
//!
//! Пишется после успешной синхронизации: версия манифеста, путь профиля NeoForge
//! и authlib-injector, плюс `size`+`mtime` каждого файла манифеста. На «Играть»
//! это позволяет:
//! - **не хешировать всё заново**, если версия та же и `size`+`mtime` файлов
//!   совпадают (проверка — только `stat`, без чтения байтов);
//! - **запуститься оффлайн**, если источник (GitHub) недоступен, взяв профиль/
//!   injector из слепка (fail-open — не блокировать игрока из-за 404).
//!
//! `size`+`mtime` ловят удаление, обрезку, правку, подмену. Не ловит только
//! тихий bit-rot (байты побились, size+mtime те же) — редкость; на этот случай
//! есть принудительная полная проверка (кнопка «Проверить файлы»).

use std::collections::HashMap;
use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

/// Отпечаток файла для быстрой проверки без чтения содержимого.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMark {
    pub size: u64,
    /// mtime в секундах Unix.
    pub mtime: i64,
}

/// Слепок установленной игры.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstallState {
    pub version: String,
    #[serde(default)]
    pub neoforge_profile: Option<String>,
    #[serde(default)]
    pub authlib_injector: Option<String>,
    /// rel-путь файла (как в манифесте, через `/`) → отпечаток (size+mtime).
    #[serde(default)]
    pub files: HashMap<String, FileMark>,
    /// rel-путь → ожидаемый sha256 (из манифеста, под который качали). Ловит смену
    /// версии на сервере при совпавших size+mtime локального файла (напр. бамп
    /// NeoForge 233→235 внутри jar не меняет его размер).
    #[serde(default)]
    pub sha256: HashMap<String, String>,
}

const STATE_FILE: &str = ".krp_state.json";

/// Отпечаток файла на диске (`None`, если файла нет/недоступен).
pub fn mark(path: &Path) -> Option<FileMark> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Some(FileMark {
        size: meta.len(),
        mtime,
    })
}

/// Прочитать слепок (или `None`, если файла нет/битый).
pub fn load(install_dir: &Path) -> Option<InstallState> {
    let bytes = std::fs::read(install_dir.join(STATE_FILE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Записать слепок на диск (ошибки не критичны — просто лог).
pub fn save(install_dir: &Path, state: &InstallState) {
    let path = install_dir.join(STATE_FILE);
    match serde_json::to_vec_pretty(state) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(&path, bytes) {
                log::warn!("state: не записать {}: {e}", path.display());
            }
        }
        Err(e) => log::warn!("state: не сериализовать слепок: {e}"),
    }
}
