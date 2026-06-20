import { useEffect, useState } from "react";
import {
  authAccount,
  authLogout,
  confirmAction,
  getInstallDir,
  installGame,
  isGameInstalled,
  onGameExited,
  onSyncProgress,
  openInstallDir,
  pickInstallDir,
  play,
  resolveInstallDir,
  setInstallDir as persistInstallDir,
  uninstallGame,
  validateInstallPath,
  type AccountInfo,
  type PathValidation,
  type SyncProgress,
} from "./lib/api";
import { LoginScreen } from "./LoginScreen";
import { SkinPanel } from "./SkinPanel";
import { checkUpdate, installUpdate, type Update } from "./lib/updater";
import { getVersion } from "@tauri-apps/api/app";
import { error as logError, info as logInfo } from "@tauri-apps/plugin-log";
import "./App.css";

type Phase = "idle" | "syncing" | "done" | "error";

type ToastKind = "ok" | "error" | "info";
interface Toast {
  id: number;
  kind: ToastKind;
  text: string;
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} Б`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} КБ`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} МБ`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} ГБ`;
}

/** Скорость: ≥1 МБ/с — в МБ/с, иначе в КБ/с. */
function formatSpeed(bytesPerSec: number): string {
  if (bytesPerSec >= 1024 * 1024) {
    return `${(bytesPerSec / 1024 / 1024).toFixed(1)} МБ/с`;
  }
  return `${(bytesPerSec / 1024).toFixed(0)} КБ/с`;
}

function App() {
  const [installDir, setInstallDir] = useState("");
  const [installed, setInstalled] = useState(false);
  const [validation, setValidation] = useState<PathValidation | null>(null);

  const [phase, setPhase] = useState<Phase>("idle");
  const [progress, setProgress] = useState<SyncProgress | null>(null);
  const [errorMsg, setErrorMsg] = useState("");

  const [version, setVersion] = useState("");

  // Авторизация: пока не вошёл — показываем экран логина.
  const [account, setAccount] = useState<AccountInfo | null>(null);
  const [authChecked, setAuthChecked] = useState(false);

  // Тосты-уведомления.
  const [toasts, setToasts] = useState<Toast[]>([]);

  // Автообновление лаунчера.
  const [update, setUpdate] = useState<Update | null>(null);
  const [updatingLauncher, setUpdatingLauncher] = useState(false);
  const [updatePct, setUpdatePct] = useState(0);
  const [checkingUpdate, setCheckingUpdate] = useState(false);

  function pushToast(kind: ToastKind, text: string) {
    const id = Date.now() + Math.random();
    setToasts((prev) => [...prev, { id, kind, text }]);
    window.setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id));
    }, 4000);
  }

  // Запомненная (или дефолтная) папка при старте.
  useEffect(() => {
    getInstallDir()
      .then((dir) => applyPath(dir))
      .catch((e) => setErrorMsg(String(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Применить выбранный путь: запомнить, проверить валидность и факт установки.
  async function applyPath(dir: string) {
    setInstallDir(dir);
    try {
      const v = await validateInstallPath(dir);
      setValidation(v);
    } catch {
      /* ошибки валидации не критичны для UI */
    }
    try {
      setInstalled(await isGameInstalled(dir));
    } catch {
      setInstalled(false);
    }
  }

  // Подписка на прогресс синхронизации.
  useEffect(() => {
    const unlisten = onSyncProgress(setProgress);
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Игра закрыта → бэкенд показал окно лаунчера обратно; сбрасываем состояние.
  useEffect(() => {
    const unlisten = onGameExited(() => {
      setPhase("idle");
      setProgress(null);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Проверка обновления лаунчера при старте. Ошибку (часто — недоступность
  // сервера обновлений с российских IP) показываем тостом, а не глотаем молча.
  useEffect(() => {
    checkUpdate()
      .then((u) => setUpdate(u))
      .catch((e) => {
        logError(`UI: проверка обновления при старте не удалась: ${e}`);
        pushToast(
          "error",
          "Не удалось проверить обновление — сервер обновлений недоступен",
        );
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Версия лаунчера для футера (из tauri.conf.json).
  useEffect(() => {
    getVersion().then(setVersion).catch(() => {});
  }, []);

  // Проверка авторизации при старте (вошёл ли игрок ранее).
  useEffect(() => {
    authAccount()
      .then((a) => setAccount(a))
      .catch(() => {})
      .finally(() => setAuthChecked(true));
  }, []);

  async function onLogout() {
    try {
      await authLogout();
      setAccount(null);
      logInfo("UI: выход из аккаунта");
    } catch (e) {
      logError(`UI: ошибка выхода: ${e}`);
    }
  }

  async function onUpdateLauncher() {
    if (!update) return;
    setUpdatingLauncher(true);
    try {
      await installUpdate(update, (downloaded, total) => {
        if (total) setUpdatePct(Math.round((downloaded / total) * 100));
      });
      // После установки лаунчер перезапустится сам.
    } catch (e) {
      setErrorMsg(String(e));
      setUpdatingLauncher(false);
    }
  }

  // Ручная проверка обновления лаунчера (с уведомлением-тостом).
  async function onCheckUpdate() {
    setCheckingUpdate(true);
    try {
      const u = await checkUpdate();
      setUpdate(u);
      if (u) {
        pushToast("ok", `Доступно обновление лаунчера: v${u.version}`);
      } else {
        pushToast("info", `Установлена последняя версия${version ? ` (v${version})` : ""}`);
      }
    } catch (e) {
      pushToast("error", `Не удалось проверить обновление: ${e}`);
      logError(`UI: ошибка проверки обновления: ${e}`);
    } finally {
      setCheckingUpdate(false);
    }
  }

  // Выбор папки установки через системный диалог.
  // К выбранному каталогу добавляем подпапку «Kingdom RP» (E:\Games → E:\Games\Kingdom RP).
  async function onChangePath() {
    try {
      const picked = await pickInstallDir(installDir);
      if (picked) {
        const dir = await resolveInstallDir(picked);
        await applyPath(dir);
        persistInstallDir(dir).catch(() => {});
        logInfo(`UI: выбрана папка установки ${dir}`);
      }
    } catch (e) {
      logError(`UI: ошибка выбора папки: ${e}`);
    }
  }

  // Удаление установленной игры (с подтверждением).
  async function onUninstall() {
    const ok = await confirmAction(
      `Удалить игру из папки:\n${installDir}\n\nМиры и настройки игрока сохранятся. Файлы игры (Java, моды, Minecraft) будут удалены.`,
      "Удаление игры",
    );
    if (!ok) return;
    setPhase("idle");
    setErrorMsg("");
    try {
      await uninstallGame(installDir);
      setInstalled(false);
      logInfo(`UI: игра удалена из ${installDir}`);
    } catch (e) {
      setErrorMsg(String(e));
      setPhase("error");
      logError(`UI: ошибка удаления: ${e}`);
    }
  }

  // Установка (если игра ещё не установлена) — без запуска.
  async function onInstall() {
    setPhase("syncing");
    setErrorMsg("");
    setProgress(null);
    logInfo(`UI: нажата «Установить» (путь=${installDir})`);
    try {
      await installGame(installDir);
      setInstalled(true);
      setPhase("idle");
      logInfo("UI: установка завершена");
    } catch (e) {
      setErrorMsg(String(e));
      setPhase("error");
      logError(`UI: ошибка установки: ${e}`);
    }
  }

  // Запуск установленной игры (с докачкой обновлений). После успешного старта
  // бэкенд прячет окно лаунчера; оно вернётся по событию onGameExited.
  async function onPlay() {
    setPhase("syncing");
    setErrorMsg("");
    setProgress(null);
    const name = account?.player_name ?? "";
    logInfo(`UI: нажата «Играть» (игрок=${name}, путь=${installDir})`);
    try {
      await play(installDir, name);
      setPhase("idle");
      isGameInstalled(installDir).then(setInstalled).catch(() => {});
    } catch (e) {
      setErrorMsg(String(e));
      setPhase("error");
      logError(`UI: ошибка запуска: ${e}`);
    }
  }

  async function onOpenFolder() {
    try {
      await openInstallDir(installDir);
      logInfo(`UI: открыта папка установки ${installDir}`);
    } catch (e) {
      logError(`UI: не удалось открыть папку: ${e}`);
    }
  }

  const pathOk = (validation?.valid ?? false) && installDir.length > 0;
  const canAct = phase !== "syncing" && pathOk;

  const overallProgress =
    progress && progress.total > 0
      ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100))
      : 0;

  // Пока проверяем сессию — ничего не мигаем; затем либо логин, либо лаунчер.
  if (!authChecked) return <div className="app" />;
  if (!account) return <LoginScreen onAuthed={setAccount} />;

  return (
    <div className="app">
      {update && (
        <div className="update-banner">
          {updatingLauncher ? (
            <span>Обновление лаунчера… {updatePct}%</span>
          ) : (
            <>
              <span>🔄 Доступно обновление лаунчера: v{update.version}</span>
              <button className="ghost small" onClick={onUpdateLauncher}>
                Обновить
              </button>
            </>
          )}
        </div>
      )}

      <header className="hero">
        <h1 className="title">KINGDOM&nbsp;RP</h1>
      </header>

      <main className="panel">
        {/* Путь установки */}
        <section className="row path-row">
          <div className="path-info">
            <span className="label">Папка установки</span>
            <span className="path-value" title={installDir}>
              {installDir || "…"}
            </span>
          </div>
          <button
            className="ghost"
            disabled={phase === "syncing"}
            onClick={onChangePath}
          >
            Изменить
          </button>
        </section>

        {/* Аккаунт игрока */}
        <section className="row path-row">
          <div className="path-info">
            <span className="label">Аккаунт</span>
            <span className="path-value">{account.player_name}</span>
          </div>
          <button
            className="ghost"
            disabled={phase === "syncing"}
            onClick={onLogout}
          >
            Выйти
          </button>
        </section>

        {/* Скин: 3D-превью + загрузка */}
        <SkinPanel account={account} onToast={pushToast} disabled={phase === "syncing"} />

        {/* Ошибки/предупреждения валидации пути */}
        {validation?.errors.map((msg) => (
          <p key={msg} className="msg error">
            ⛔ {msg}
          </p>
        ))}
        {validation?.warnings.map((msg) => (
          <p key={msg} className="msg warn">
            ⚠️ {msg}
          </p>
        ))}

        {/* Прогресс установки: общий объём + скорость */}
        {phase === "syncing" && (
          <section className="progress-block">
            <div className="progress-line">
              <span className="file-name">
                {progress?.label || "Получение манифеста…"}
              </span>
              {progress && progress.total > 0 && (
                <span className="file-size">
                  {formatBytes(progress.downloaded)} / {formatBytes(progress.total)}
                </span>
              )}
            </div>
            <div className="bar">
              <div className="bar-fill" style={{ width: `${overallProgress}%` }} />
            </div>
            <span className="progress-meta">
              {overallProgress}%
              {progress && progress.speed > 0 && ` · ${formatSpeed(progress.speed)}`}
            </span>
          </section>
        )}

        {/* Ошибка синхронизации */}
        {phase === "error" && (
          <p className="msg error">⛔ {errorMsg}</p>
        )}

        {/* Кнопка: «Установить», пока игра не установлена, иначе «Играть» */}
        <button
          className="play"
          disabled={!canAct}
          onClick={installed ? onPlay : onInstall}
        >
          {phase === "syncing"
            ? installed
              ? "Запуск…"
              : "Установка…"
            : installed
              ? "ИГРАТЬ"
              : "УСТАНОВИТЬ"}
        </button>
      </main>

      <footer className="footer">
        <span className="footer-left">
          Kingdom RP Launcher
          {version && <span className="version">v{version}</span>}
        </span>
        <div className="footer-right">
          {installed && (
            <button
              className="folder-btn"
              title="Удалить игру"
              disabled={phase === "syncing"}
              onClick={onUninstall}
            >
              🗑️
            </button>
          )}
          <button
            className="folder-btn"
            title="Проверить обновление лаунчера"
            disabled={checkingUpdate}
            onClick={onCheckUpdate}
          >
            🔄
          </button>
          <button
            className="folder-btn"
            title="Открыть папку игры"
            disabled={!installDir}
            onClick={onOpenFolder}
          >
            📁
          </button>
        </div>
      </footer>

      {/* Тосты-уведомления */}
      <div className="toast-stack">
        {toasts.map((t) => (
          <div key={t.id} className={`toast ${t.kind}`}>
            {t.kind === "ok" ? "✅ " : t.kind === "error" ? "⛔ " : "ℹ️ "}
            {t.text}
          </div>
        ))}
      </div>
    </div>
  );
}

export default App;
