//! Единый трекер прогресса установки: общий объём, скачано, скорость.
//!
//! Раньше прогресс шёл по каждому файлу отдельно (и «20 МБ / 20 МБ» означало
//! один пакет, а не весь Minecraft). Теперь все этапы (ваниль, JRE, моды)
//! считаются в один счётчик: показываем «сколько скачано из общего объёма» и
//! текущую скорость. Событие — `sync://progress` (его слушает фронтенд).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Имя Tauri-события прогресса.
pub const PROGRESS_EVENT: &str = "sync://progress";

/// Полезная нагрузка события прогресса.
#[derive(Debug, Clone, Serialize)]
pub struct ProgressPayload {
    /// Что сейчас делаем (например «Устанавливаем Minecraft 1.20.1»).
    pub label: String,
    /// Скачано байт всего (с учётом уже имевшихся файлов).
    pub downloaded: u64,
    /// Общий объём к скачиванию, байт.
    pub total: u64,
    /// Текущая скорость, байт/с (0 при простое/пропуске).
    pub speed: f64,
}

/// Потокобезопасный трекер на всю установку.
pub struct Progress {
    app: AppHandle,
    total: AtomicU64,
    done: AtomicU64,
    label: Mutex<String>,
    speed: Mutex<SpeedState>,
}

struct SpeedState {
    window_start: Instant,
    window_bytes: u64,
    current: f64,
    last_emit: Instant,
}

impl Progress {
    pub fn new(app: AppHandle) -> Self {
        let now = Instant::now();
        Self {
            app,
            total: AtomicU64::new(0),
            done: AtomicU64::new(0),
            label: Mutex::new(String::new()),
            speed: Mutex::new(SpeedState {
                window_start: now,
                window_bytes: 0,
                current: 0.0,
                last_emit: now,
            }),
        }
    }

    /// Добавить байты к общему ожидаемому объёму.
    pub fn add_total(&self, n: u64) {
        self.total.fetch_add(n, Ordering::Relaxed);
    }

    /// Сменить подпись текущего этапа (и тут же отправить событие).
    pub fn set_label(&self, label: impl Into<String>) {
        *self.label.lock().unwrap() = label.into();
        self.emit(true);
    }

    /// Учесть реально скачанные из сети байты (двигают счётчик и скорость).
    pub fn add_net(&self, delta: u64) {
        if delta == 0 {
            return;
        }
        self.done.fetch_add(delta, Ordering::Relaxed);

        let mut sp = self.speed.lock().unwrap();
        sp.window_bytes += delta;
        let now = Instant::now();
        let win = now.duration_since(sp.window_start).as_secs_f64();
        if win >= 0.5 {
            sp.current = sp.window_bytes as f64 / win;
            sp.window_start = now;
            sp.window_bytes = 0;
        }
        if now.duration_since(sp.last_emit).as_millis() >= 100 {
            sp.last_emit = now;
            let payload = self.payload(sp.current);
            drop(sp);
            let _ = self.app.emit(PROGRESS_EVENT, payload);
        }
    }

    /// Учесть уже имевшийся (пропущенный) файл: двигает счётчик, но не скорость.
    pub fn add_skipped(&self, n: u64) {
        self.done.fetch_add(n, Ordering::Relaxed);
        self.emit(false);
    }

    fn payload(&self, speed: f64) -> ProgressPayload {
        ProgressPayload {
            label: self.label.lock().unwrap().clone(),
            downloaded: self.done.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
            speed,
        }
    }

    fn emit(&self, force: bool) {
        let mut sp = self.speed.lock().unwrap();
        let now = Instant::now();
        if !force && now.duration_since(sp.last_emit).as_millis() < 100 {
            return;
        }
        sp.last_emit = now;
        // Если давно ничего не качали — скорость гаснет.
        if now.duration_since(sp.window_start).as_secs_f64() > 1.0 {
            sp.current = 0.0;
        }
        let speed = sp.current;
        let payload = self.payload(speed);
        drop(sp);
        let _ = self.app.emit(PROGRESS_EVENT, payload);
    }

    /// Колбэк прогресса для ОДНОЙ загрузки: переводит «накоплено в файле» в
    /// дельты и скармливает трекеру. Каждой загрузке — свой колбэк.
    /// Внутри атомик (а не `Cell`), чтобы колбэк/future оставались `Send`.
    pub fn file_cb(&self) -> impl Fn(u64, Option<u64>) + '_ {
        let prev = AtomicU64::new(0);
        move |downloaded, _total| {
            let p = prev.swap(downloaded, Ordering::Relaxed);
            self.add_net(downloaded.saturating_sub(p));
        }
    }
}
