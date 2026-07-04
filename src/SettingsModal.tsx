import { useEffect, useState } from "react";
import {
  getLaunchSettings,
  setLaunchSettings,
  validateJvmArgs,
} from "./lib/api";

interface Props {
  onClose: () => void;
  onToast: (kind: "ok" | "error" | "info", text: string) => void;
  installDir: string;
  onChangePath: () => void;
  busy: boolean;
}

/** Модальное окно настроек: память игры + режим JVM-аргументов
 *  (рекомендуемые / кастомные). */
export function SettingsModal({
  onClose,
  onToast,
  installDir,
  onChangePath,
  busy,
}: Props) {
  const [loaded, setLoaded] = useState(false);
  const [memory, setMemory] = useState(4096);
  const [range, setRange] = useState({ min: 2048, max: 16384 });
  const [useCustom, setUseCustom] = useState(false);
  const [custom, setCustom] = useState("");
  const [recommended, setRecommended] = useState("");
  const [saving, setSaving] = useState(false);
  const [checking, setChecking] = useState(false);
  // Кастомная строка прошла проверку (гейт кнопки «Сохранить»). При загрузке
  // сохранённые кастомные аргументы уже валидны (их проверяли при сохранении).
  const [validated, setValidated] = useState(false);

  useEffect(() => {
    getLaunchSettings()
      .then((s) => {
        setMemory(s.memory_mb);
        setRange({ min: s.min_memory, max: s.max_memory });
        setUseCustom(s.use_custom_jvm);
        setCustom(s.custom_jvm_args);
        setRecommended(s.recommended_jvm);
        setValidated(s.use_custom_jvm); // сохранённые кастомные уже валидны
        setLoaded(true);
      })
      .catch((e) => onToast("error", `Не удалось загрузить настройки: ${e}`));
    // Грузим ОДИН раз на открытии окна. onToast намеренно не в зависимостях:
    // pushToast пересоздаётся на каждый рендер App (poll сервера/тосты) — иначе
    // эффект перезагружал бы настройки и затирал несохранённый выбор.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Превью рекомендуемых обновляем при смене памяти (Xms/Xmx).
  const recommendedPreview = recommended.replace(
    /-Xms\d+M -Xmx\d+M/,
    `-Xms${memory}M -Xmx${memory}M`,
  );

  async function onCheck() {
    setChecking(true);
    try {
      await validateJvmArgs(installDir, custom);
      setValidated(true);
      onToast("ok", "Аргументы корректны");
    } catch (e) {
      setValidated(false);
      onToast("error", String(e));
    } finally {
      setChecking(false);
    }
  }

  async function onSave() {
    if (useCustom && custom.trim().length === 0) {
      onToast("error", "Строка JVM-аргументов пустая");
      return;
    }
    setSaving(true);
    try {
      // Кастомную строку проверяем перед сохранением (java -version).
      if (useCustom) {
        await validateJvmArgs(installDir, custom);
      }
      await setLaunchSettings(memory, useCustom, custom);
      onToast("ok", "Настройки сохранены");
      onClose();
    } catch (e) {
      onToast("error", String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h2>Настройки</h2>
          <button className="folder-btn" title="Закрыть" onClick={onClose}>
            ✕
          </button>
        </div>

        {!loaded ? (
          <p className="msg">Загрузка…</p>
        ) : (
          <div className="modal-body">
            {/* Папка установки */}
            <div className="set-block">
              <div className="set-row">
                <span className="label">Папка установки</span>
                <button className="ghost small" onClick={onChangePath} disabled={busy}>
                  Изменить
                </button>
              </div>
              <code className="jvm-preview" title={installDir}>
                {installDir || "…"}
              </code>
            </div>

            {/* Память */}
            <div className="set-block">
              <div className="set-row">
                <span className="label">Память игры</span>
                <span className="path-value">{(memory / 1024).toFixed(1)} ГБ</span>
              </div>
              <input
                type="range"
                className="mem-slider"
                min={range.min}
                max={range.max}
                step={512}
                value={memory}
                onChange={(e) => setMemory(Number(e.target.value))}
              />
            </div>

            {/* JVM-режим */}
            <div className="set-block">
              <span className="label">JVM-аргументы</span>

              <label className="radio">
                <input
                  type="radio"
                  name="jvm"
                  checked={!useCustom}
                  onChange={() => setUseCustom(false)}
                />
                <span>Рекомендуемые (по умолчанию)</span>
              </label>
              {!useCustom && (
                <code className="jvm-preview">{recommendedPreview}</code>
              )}

              <label className="radio">
                <input
                  type="radio"
                  name="jvm"
                  checked={useCustom}
                  onChange={() => setUseCustom(true)}
                />
                <span>Свои аргументы</span>
              </label>
              {useCustom && (
                <>
                  <p className="msg warn">
                    ⚠️ Только для опытных пользователей. Неверные аргументы могут
                    помешать запуску игры. Память (-Xmx) задавайте здесь сами.
                  </p>
                  <textarea
                    className="jvm-input"
                    rows={3}
                    spellCheck={false}
                    placeholder="-Xmx6G -XX:+UseG1GC …"
                    value={custom}
                    onChange={(e) => {
                      setCustom(e.target.value);
                      setValidated(false); // изменили → нужна повторная проверка
                    }}
                  />
                  <button
                    className="ghost small"
                    onClick={onCheck}
                    disabled={checking || custom.trim().length === 0}
                  >
                    {checking ? "Проверка…" : "Проверить"}
                  </button>
                </>
              )}
            </div>
          </div>
        )}

        <div className="modal-foot">
          {useCustom && !validated && (
            <span className="foot-hint">Проверьте аргументы перед сохранением</span>
          )}
          <button className="ghost" onClick={onClose} disabled={saving}>
            Отмена
          </button>
          <button
            className="play small"
            onClick={onSave}
            disabled={saving || !loaded || (useCustom && !validated)}
          >
            {saving ? "Сохранение…" : "Сохранить"}
          </button>
        </div>
      </div>
    </div>
  );
}
