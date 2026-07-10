import { useState } from "react";
import { authLogin, authRegister, type AccountInfo } from "./lib/api";
import { error as logError, info as logInfo } from "@tauri-apps/plugin-log";

// Ник = логин: латиница/цифры/_ , 3–16 символов (ограничение имени Minecraft).
const NAME_RE = /^[A-Za-z0-9_]{3,16}$/;
const PASS_MIN = 8;

type Mode = "login" | "register";

/** Экран входа/регистрации. Показывается, пока игрок не вошёл.
 *  Один переключаемый режим: регистрация ↔ вход. */
export function LoginScreen({ onAuthed }: { onAuthed: (a: AccountInfo) => void }) {
  const [mode, setMode] = useState<Mode>("register");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [errorMsg, setErrorMsg] = useState("");

  const isRegister = mode === "register";
  const nameOk = NAME_RE.test(username.trim());
  const passOk = password.length >= PASS_MIN;
  const confirmOk = password === confirm;
  const canSubmit =
    !busy && nameOk && passOk && (!isRegister || (confirmOk && confirm.length > 0));

  function switchMode() {
    setMode((m) => (m === "register" ? "login" : "register"));
    setErrorMsg("");
    setConfirm("");
  }

  async function submit() {
    if (!canSubmit) return;
    setBusy(true);
    setErrorMsg("");
    logInfo(`UI: ${mode} '${username.trim()}'`);
    try {
      const account = isRegister
        ? await authRegister(username.trim(), password)
        : await authLogin(username.trim(), password);
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
        <p className="subtitle">{isRegister ? "Создание аккаунта" : "Вход в аккаунт"}</p>
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
              onKeyDown={(e) => e.key === "Enter" && !isRegister && submit()}
            />
          </div>
        </section>

        {isRegister && (
          <section className="row">
            <div className="path-info">
              <span className="label">Повторите пароль</span>
              <input
                className="path-input"
                type="password"
                value={confirm}
                spellCheck={false}
                placeholder="Ещё раз тот же пароль"
                disabled={busy}
                onChange={(e) => setConfirm(e.currentTarget.value)}
                onKeyDown={(e) => e.key === "Enter" && submit()}
              />
            </div>
          </section>
        )}

        {username.length > 0 && !nameOk && (
          <p className="msg error">⛔ Логин: латиница, цифры и _, 3–16 символов.</p>
        )}
        {isRegister && confirm.length > 0 && !confirmOk && (
          <p className="msg error">⛔ Пароли не совпадают.</p>
        )}
        {errorMsg && <p className="msg error">⛔ {errorMsg}</p>}

        <button className="play" disabled={!canSubmit} onClick={submit}>
          {busy ? "Подождите…" : isRegister ? "Регистрация" : "Войти"}
        </button>

        <p className="auth-switch">
          {isRegister ? "Уже есть аккаунт? " : "Нет аккаунта? "}
          <button type="button" className="link" disabled={busy} onClick={switchMode}>
            {isRegister ? "Войти" : "Регистрация"}
          </button>
        </p>
      </main>

      <footer className="footer">
        <span>Kingdom RP Launcher</span>
      </footer>
    </div>
  );
}
