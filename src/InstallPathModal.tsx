import { useEffect, useState } from "react";
import {
  defaultInstallDir,
  pickInstallDir,
  resolveInstallDir,
  validateInstallPath,
  type PathValidation,
} from "./lib/api";

interface Props {
  onClose: () => void;
  onConfirm: (dir: string) => void;
  onToast: (kind: "ok" | "error" | "info", text: string) => void;
}

type Mode = "recommended" | "custom";

/** Окно выбора места установки при первой установке игры:
 *  рекомендуемое место по умолчанию или папка, указанная игроком. */
export function InstallPathModal({ onClose, onConfirm, onToast }: Props) {
  const [mode, setMode] = useState<Mode>("recommended");
  const [defaultDir, setDefaultDir] = useState("");
  const [customDir, setCustomDir] = useState("");
  const [validation, setValidation] = useState<PathValidation | null>(null);
  const [checking, setChecking] = useState(false);

  const chosen = mode === "recommended" ? defaultDir : customDir;

  useEffect(() => {
    defaultInstallDir()
      .then(setDefaultDir)
      .catch((e) => onToast("error", `Не удалось определить папку: ${e}`));
  }, [onToast]);

  // Валидируем выбранный путь (в основном важно для своей папки).
  useEffect(() => {
    if (!chosen) {
      setValidation(null);
      return;
    }
    let active = true;
    setChecking(true);
    validateInstallPath(chosen)
      .then((v) => active && setValidation(v))
      .catch(() => active && setValidation(null))
      .finally(() => active && setChecking(false));
    return () => {
      active = false;
    };
  }, [chosen]);

  async function pickFolder() {
    try {
      const picked = await pickInstallDir(customDir || defaultDir);
      if (!picked) return;
      const dir = await resolveInstallDir(picked); // добавит подпапку «Kingdom RP»
      setCustomDir(dir);
      setMode("custom");
    } catch (e) {
      onToast("error", `Ошибка выбора папки: ${e}`);
    }
  }

  const canConfirm =
    chosen.length > 0 && !checking && (validation ? validation.valid : true);

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h2>Куда установить игру?</h2>
          <button className="folder-btn" title="Закрыть" onClick={onClose}>
            ✕
          </button>
        </div>

        <div className="modal-body">
          <div className="set-block">
            <label className="radio">
              <input
                type="radio"
                name="installdir"
                checked={mode === "recommended"}
                onChange={() => setMode("recommended")}
              />
              <span>Рекомендуемое место</span>
            </label>
            {mode === "recommended" && (
              <code className="jvm-preview" title={defaultDir}>
                {defaultDir || "…"}
              </code>
            )}

            <label className="radio">
              <input
                type="radio"
                name="installdir"
                checked={mode === "custom"}
                onChange={() => setMode("custom")}
              />
              <span>Своя папка</span>
            </label>
            {mode === "custom" && (
              <>
                <code className="jvm-preview" title={customDir}>
                  {customDir || "Папка не выбрана"}
                </code>
                <button className="ghost small" onClick={pickFolder}>
                  Выбрать папку…
                </button>
              </>
            )}
          </div>

          {validation?.errors.map((m) => (
            <p key={m} className="msg error">
              ⛔ {m}
            </p>
          ))}
          {validation?.warnings.map((m) => (
            <p key={m} className="msg warn">
              ⚠️ {m}
            </p>
          ))}
        </div>

        <div className="modal-foot">
          <button className="ghost" onClick={onClose}>
            Отмена
          </button>
          <button
            className="play small"
            disabled={!canConfirm}
            onClick={() => onConfirm(chosen)}
          >
            Установить
          </button>
        </div>
      </div>
    </div>
  );
}
