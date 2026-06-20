import { useEffect, useRef, useState } from "react";
import { SkinViewer } from "skinview3d";
import {
  pickSkinFile,
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
  const [slim, setSlim] = useState(false);
  const [busy, setBusy] = useState(false);

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
    if (!pendingPath) return;
    setBusy(true);
    try {
      await uploadSkin(pendingPath, slim);
      onToast("ok", "Скин обновлён");
      setPendingPath(null);
    } catch (e) {
      onToast("error", `Не удалось загрузить скин: ${e}`);
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
        <span className="label">Скин</span>
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
          Выбрать PNG…
        </button>
        <button
          className="ghost"
          disabled={disabled || busy || !pendingPath}
          onClick={onApply}
        >
          {busy ? "Загрузка…" : "Применить скин"}
        </button>
      </div>
    </section>
  );
}
