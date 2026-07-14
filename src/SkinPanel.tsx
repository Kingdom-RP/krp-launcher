import { useEffect, useRef, useState } from "react";
import { SkinViewer } from "skinview3d";
import {
  pickSkinFile,
  setSkinModel,
  skinPreviewFile,
  skinPreviewUrl,
  uploadSkin,
  type AccountInfo,
} from "./lib/api";
import { error as logError } from "@tauri-apps/plugin-log";

type Toast = (kind: "ok" | "error" | "info", text: string) => void;

/** Панель скина: 3D-превью (skinview3d) + выбор PNG + загрузка на drasl. */
export function SkinPanel({
  account,
  onToast,
  disabled,
}: {
  account: AccountInfo;
  onToast: Toast;
  disabled: boolean;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const viewerRef = useRef<SkinViewer | null>(null);

  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [pendingPath, setPendingPath] = useState<string | null>(null);
  // Тип модели: `slim` — текущий выбор (может быть несохранён), `savedSlim` —
  // последнее ПРИМЕНЁННОЕ значение (помним между запусками через localStorage,
  // иначе галочка сбрасывалась в classic при перезаходе). Их расхождение = грязь.
  const [savedSlim, setSavedSlim] = useState(
    () => localStorage.getItem("krp.skin.slim") === "1",
  );
  const [slim, setSlim] = useState(savedSlim);
  const [busy, setBusy] = useState(false);

  // Есть несохранённые изменения: выбран новый файл ИЛИ модель отличается от
  // применённой. Пока грязно — подсвечиваем «Применить» и просим сохранить.
  const dirty = pendingPath !== null || slim !== savedSlim;

  // Инициализация 3D-вьюера (один раз).
  useEffect(() => {
    if (!canvasRef.current) return;
    const viewer = new SkinViewer({
      canvas: canvasRef.current,
      width: 180,
      height: 260,
    });
    viewer.autoRotate = true;
    viewer.zoom = 0.85;
    viewerRef.current = viewer;
    return () => {
      viewer.dispose();
      viewerRef.current = null;
    };
  }, []);

  // Подгрузить текущий скин с сервера при первом показе.
  useEffect(() => {
    if (!account.skin_url) return;
    skinPreviewUrl(account.skin_url)
      .then(setPreviewUrl)
      .catch(() => {});
  }, [account.skin_url]);

  // Перерисовать модель при смене скина или типа модели.
  useEffect(() => {
    const viewer = viewerRef.current;
    if (!viewer || !previewUrl) return;
    viewer
      .loadSkin(previewUrl, { model: slim ? "slim" : "default" })
      .catch((e) => logError(`UI: не отрисовать скин: ${e}`));
  }, [previewUrl, slim]);

  async function onPick() {
    try {
      const path = await pickSkinFile();
      if (!path) return;
      // Валидация формата (64×64/64×32) + data-URL для превью.
      const dataUrl = await skinPreviewFile(path);
      setPendingPath(path);
      setPreviewUrl(dataUrl);
    } catch (e) {
      onToast("error", `Скин: ${e}`);
    }
  }

  async function onApply() {
    if (!dirty) return;
    setBusy(true);
    try {
      if (pendingPath) {
        // Новый файл: загружаем PNG + модель.
        await uploadSkin(pendingPath, slim);
        setPendingPath(null);
      } else {
        // Файл не менялся — правим только тип модели у текущего скина.
        await setSkinModel(slim);
      }
      setSavedSlim(slim);
      localStorage.setItem("krp.skin.slim", slim ? "1" : "0");
      onToast("ok", "Скин обновлён");
    } catch (e) {
      onToast("error", `Не удалось сохранить: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="skin-panel">
      <div className="skin-preview">
        <canvas ref={canvasRef} />
      </div>
      <div className="skin-controls">
        <span className="label">
          Скин
          <span
            className="help-tip"
            tabIndex={0}
            aria-label="Как установить скин"
            data-tip={
              "Как установить скин:\n" +
              "1. «Выбрать файл скина» — PNG 64×64 (или 64×32).\n" +
              "2. При желании включите «Тонкая модель» (руки Alex, 3px).\n" +
              "3. Нажмите «Применить скин», чтобы сохранить.\n\n" +
              "Модель можно менять и без нового файла — просто\n" +
              "переключите галочку и нажмите «Применить»."
            }
          >
            ?
          </span>
        </span>
        <label className="skin-model">
          <input
            type="checkbox"
            checked={slim}
            disabled={disabled || busy}
            onChange={(e) => setSlim(e.currentTarget.checked)}
          />
          Тонкая модель
        </label>
        <button className="ghost" disabled={disabled || busy} onClick={onPick}>
          Выбрать файл скина
        </button>
        <button
          className={dirty ? "primary dirty" : "ghost"}
          disabled={disabled || busy || !dirty}
          onClick={onApply}
        >
          {busy ? "Сохранение…" : "Применить скин"}
        </button>
        {dirty && !busy && (
          <span className="save-hint">Сохраните изменения</span>
        )}
      </div>
    </section>
  );
}
