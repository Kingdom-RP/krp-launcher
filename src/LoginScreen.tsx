import { useState } from "react";
import { authLogin, authRegister, type AccountInfo } from "./lib/api";
import { error as logError, info as logInfo } from "@tauri-apps/plugin-log";

// Ник = логин: латиница/цифры/_ , 3–16 символов (ограничение имени Minecraft).
const NAME_RE = /^[A-Za-z0-9_]{3,16}$/;
const PASS_MIN = 8;

/** Экран входа/регистрации. Показывается, пока игрок не вошёл. */
export function LoginScreen({ onAuthed }: { onAuthed: (a: AccountInfo) => void }) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [errorMsg, setErrorMsg] = useState("");

  const nameOk = NAME_RE.test(username.trim());
  const passOk = password.length >= PASS_MIN;
  const canSubmit = !busy && nameOk && passOk;

  async function submit(mode: "login" | "register") {
    if (!canSubmit) return;
    setBusy(true);
    setErrorMsg("");
    logInfo(`UI: ${mode} '${username.trim()}'`);
    try {
      const account =
        mode === "login"
          ? await authLogin(username.trim(), password)
          : await authRegister(username.trim(), password);
      onAuthed(account);
    } catch (e) {
      setErrorMsg(String(e));
      logError(`UI: ошибка ${mode}: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="app">
      <header className="hero">
        <h1 className="title">KINGDOM&nbsp;RP</h1>
        <p className="subtitle">Вход в аккаунт</p>
      </header>

      <main className="panel">
        <section className="row">
          <div className="path-info">
            <span className="label">Логин (он же ник в игре)</span>
            <input
              className="path-input"
              value={username}
              spellCheck={false}
              maxLength={16}
              placeholder="Латиница, цифры и _"
              disabled={busy}
              onChange={(e) => setUsername(e.currentTarget.value)}
            />
          </div>
        </section>

        <section className="row">
          <div className="path-info">
            <span className="label">Пароль</span>
            <input
              className="path-input"
              type="password"
              value={password}
              spellCheck={false}
              placeholder={`Минимум ${PASS_MIN} символов`}
              disabled={busy}
              onChange={(e) => setPassword(e.currentTarget.value)}
              onKeyDown={(e) => e.key === "Enter" && submit("login")}
            />
          </div>
        </section>

        {username.length > 0 && !nameOk && (
          <p className="msg error">⛔ Логин: латиница, цифры и _, 3–16 символов.</p>
        )}
        {errorMsg && <p className="msg error">⛔ {errorMsg}</p>}

        <button className="play" disabled={!canSubmit} onClick={() => submit("login")}>
          {busy ? "Подождите…" : "ВОЙТИ"}
        </button>
        <button
          className="ghost"
          disabled={!canSubmit}
          onClick={() => submit("register")}
        >
          Создать аккаунт
        </button>
      </main>

      <footer className="footer">
        <span>Kingdom RP Launcher</span>
      </footer>
    </div>
  );
}
