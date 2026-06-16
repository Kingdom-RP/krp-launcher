import { useEffect, useRef, useState } from "react";
import {
  defaultInstallDir,
  onSyncProgress,
  play,
  validateInstallPath,
  type PathValidation,
  type SyncProgress,
} from "./lib/api";
import "./App.css";

type Phase = "idle" | "syncing" | "done" | "error";

function formatBytes(n: number): string {
  if (n < 1024) return `${n} Б`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} КБ`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} МБ`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} ГБ`;
}

function App() {
  const [installDir, setInstallDir] = useState("");
  const [editingPath, setEditingPath] = useState(false);
  const [validation, setValidation] = useState<PathValidation | null>(null);

  const [playerName, setPlayerName] = useState("");
  const [phase, setPhase] = useState<Phase>("idle");
  const [progress, setProgress] = useState<SyncProgress | null>(null);
  const [pid, setPid] = useState<number | null>(null);
  const [errorMsg, setErrorMsg] = useState("");

  // Папка по умолчанию при старте.
  useEffect(() => {
    defaultInstallDir()
      .then((dir) => {
        setInstallDir(dir);
        return validateInstallPath(dir);
      })
      .then((v) => v && setValidation(v))
      .catch((e) => setErrorMsg(String(e)));
  }, []);

  // Подписка на прогресс синхронизации.
  useEffect(() => {
    const unlisten = onSyncProgress(setProgress);
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Валидация при ручном вводе пути (с дебаунсом).
  const debounceRef = useRef<number | undefined>(undefined);
  function onPathChange(value: string) {
    setInstallDir(value);
    window.clearTimeout(debounceRef.current);
    debounceRef.current = window.setTimeout(() => {
      validateInstallPath(value).then(setValidation).catch(() => {});
    }, 300);
  }

  async function onPlay() {
    setPhase("syncing");
    setErrorMsg("");
    setPid(null);
    setProgress(null);
    try {
      const launchedPid = await play(installDir, playerName.trim());
      setPid(launchedPid);
      setPhase("done");
    } catch (e) {
      setErrorMsg(String(e));
      setPhase("error");
    }
  }

  const canPlay =
    phase !== "syncing" &&
    (validation?.valid ?? false) &&
    installDir.length > 0 &&
    playerName.trim().length > 0;

  const fileProgress =
    progress && progress.total_bytes
      ? Math.round((progress.downloaded / progress.total_bytes) * 100)
      : null;
  const overallProgress = progress
    ? Math.round(((progress.index + (fileProgress ?? 0) / 100) / progress.total) * 100)
    : 0;

  return (
    <div className="app">
      <header className="hero">
        <h1 className="title">KINGDOM&nbsp;RP</h1>
        <p className="subtitle">Minecraft 1.20.1 · NeoForge</p>
      </header>

      <main className="panel">
        {/* Путь установки */}
        <section className="row path-row">
          <div className="path-info">
            <span className="label">Папка установки</span>
            {editingPath ? (
              <input
                className="path-input"
                value={installDir}
                spellCheck={false}
                onChange={(e) => onPathChange(e.currentTarget.value)}
              />
            ) : (
              <span className="path-value" title={installDir}>
                {installDir || "…"}
              </span>
            )}
          </div>
          <button
            className="ghost"
            disabled={phase === "syncing"}
            onClick={() => setEditingPath((v) => !v)}
          >
            {editingPath ? "Готово" : "Изменить"}
          </button>
        </section>

        {/* Имя игрока */}
        <section className="row">
          <div className="path-info">
            <span className="label">Имя игрока</span>
            <input
              className="path-input"
              value={playerName}
              spellCheck={false}
              maxLength={16}
              placeholder="Ник в игре"
              disabled={phase === "syncing"}
              onChange={(e) => setPlayerName(e.currentTarget.value)}
            />
          </div>
        </section>

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

        {/* Прогресс синхронизации */}
        {phase === "syncing" && (
          <section className="progress-block">
            <div className="progress-line">
              <span className="file-name">{progress?.file ?? "Получение манифеста…"}</span>
              {progress?.total_bytes != null && (
                <span className="file-size">
                  {formatBytes(progress.downloaded)} / {formatBytes(progress.total_bytes)}
                </span>
              )}
            </div>
            <div className="bar">
              <div className="bar-fill" style={{ width: `${overallProgress}%` }} />
            </div>
            <span className="progress-meta">
              {progress ? `Файл ${progress.index + 1} из ${progress.total}` : "Старт…"}
              {" · "}
              {overallProgress}%
            </span>
          </section>
        )}

        {/* Итог */}
        {phase === "done" && pid != null && (
          <p className="msg ok">
            ✅ Игра запущена (PID {pid}). Лаунчер можно закрыть после загрузки игры.
          </p>
        )}

        {/* Ошибка синхронизации */}
        {phase === "error" && (
          <p className="msg error">⛔ {errorMsg}</p>
        )}

        {/* Кнопка запуска */}
        <button className="play" disabled={!canPlay} onClick={onPlay}>
          {phase === "syncing" ? "Установка и запуск…" : "ИГРАТЬ"}
        </button>
      </main>

      <footer className="footer">
        <span>Kingdom RP Launcher</span>
        <span className="version">v0.1.0</span>
      </footer>
    </div>
  );
}

export default App;
