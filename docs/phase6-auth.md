# Фаза 6 — авторизация (логин/пароль) + скины

План и решения по авторизации Kingdom RP. Текущий запуск — офлайн (фиктивные
uuid/token в `launch.rs`); цель фазы — реальные аккаунты логин/пароль и скины.

## Архитектура

**Свой Yggdrasil-совместимый auth-сервер ([drasl](https://github.com/unmojang/drasl)) + `authlib-injector`.**

- `authlib-injector` — Java-агент (`-javaagent:...jar=<AUTH_BASE_URL>`), подменяет
  обращения Minecraft к auth/session/skin-эндпоинтам Mojang на наш сервер.
  Ставится и в **клиент** (лаунчер добавляет в команду запуска), и в **игровой
  сервер** (`online-mode=true`).
- Игроки логинятся своим логином/паролем (не Mojang/Microsoft). Сервер в online-mode
  валидирует сессии через drasl → защита ников + рабочие скины.
- Скины раздаёт drasl (в Yggdrasil-профиле — ссылки на текстуры).

## Принятые решения

1. **Auth-сервер:** drasl (один бинарь/Docker, SQLite, Yggdrasil + скины + аккаунты).
2. **Хост:** конфигурируемый. Источник правды — `AUTH_BASE_URL` в `config.rs`
   (как `MANIFEST_BASE_URL`) + необязательный оверрайд в `settings.json` для
   локального теста без пересборки. Сейчас — локально у владельца (режим
   разработки: доступен только на его машине); при появлении публичного сервера —
   меняем константу и выпускаем релиз.
3. **Регистрация — только в лаунчере.** Первый запуск: поля логин/пароль →
   «Создать аккаунт/Войти». Дальше — токен в `settings.json`, сразу кнопка
   «Играть» без повторного ввода (validate/refresh; пароль НЕ храним).
4. **Скины — только через лаунчер** (сайт не делаем). Диалог выбора PNG →
   **валидация формата** → загрузка в drasl → превью на 3D-модели.

## Валидация скина (реализовано — `skin.rs`)

Перед загрузкой лаунчер проверяет PNG: скин Minecraft — это **64×64** (современный)
или **64×32** (устаревший). Любая другая картинка отклоняется с понятной ошибкой,
на сервер не отправляется. Команда `validate_skin(path)` → `modern`/`legacy`/ошибка.
Финальную проверку дублирует и сам drasl.

## Co-hosting (MC-сервер + drasl на одной машине)

Да, уживаются. drasl лёгкий (Go, ~100–300 МБ RAM, SQLite). Основной едок — MC-сервер.
- VPS 6–8 ГБ RAM (под MC) + drasl сверху почти бесплатно.
- Порты: MC `25565`; drasl за reverse-proxy (**Caddy** — авто-TLS) на `443`
  (`authlib-injector` хочет HTTPS).
- MC-сервер обращается к drasl по `localhost`, игроки — по публичному домену.
- Минус: одна машина = единая точка отказа (для небольшого проекта ок).

### Регистрация только через лаунчер (закрыть веб-форму)

Регистрация идёт двумя разными путями: лаунчер → API `POST /drasl/api/v2/users`;
браузерная форма → `GET /web/registration` + `POST /web/register`. Чтобы аккаунты
создавались только через лаунчер, режем веб-маршруты на reverse-proxy, API не
трогаем (`Allow=false` в конфиге drasl НЕ подходит — вырубит и API лаунчера):

```caddy
auth.kingdom-rp.example {
    @webreg path /web/registration /web/register
    respond @webreg 404
    reverse_proxy localhost:25585
}
```

Средний игрок через браузер не зарегается; лаунчер (`/drasl/api/*`,
`/authlib-injector/*`) работает как есть. Хочешь совсем убрать веб-морду →
режь весь `/web/*` (скины/профиль всё равно через лаунчер). Это не защита от
технически продвинутых (API открыт), но для рядовой аудитории достаточно — плюс
реальный гейт входа = `white-list=true` на сервере.

## Точки интеграции в лаунчере

- `config.rs` — `AUTH_BASE_URL` (+ оверрайд из settings).
- `auth.rs` (новый) — Yggdrasil-клиент: `authenticate`/`refresh`/`validate`/
  `invalidate` + drasl-эндпоинты регистрации и загрузки скина. Токены/профиль в
  `settings.json`. Tauri-команды: `register`, `login`, `logout`, `current_account`,
  `ensure_session`, `upload_skin`.
- `launch.rs` — `-javaagent:<authlib-injector.jar>=<AUTH_BASE_URL>` + реальные
  `--username/--uuid/--accessToken` из сессии (заменяют офлайн-заглушки).
- Манифест/сборщик — хостим `authlib-injector.jar` у себя (его релизы на GitHub
  в РФ режутся), кладём в `dist/`.
- `skin.rs` — валидация PNG (готово).
- Фронтенд — экран логина/регистрации; «вошёл как…»; кнопка «Сменить скин»
  (диалог PNG → `validate_skin` → upload) + превью через `skinview3d` (WebGL).
  Поле ручного ввода ника убирается — ник = имя аккаунта.

## Серверная часть (вне лаунчера, для e2e)

MC-сервер: `authlib-injector` с тем же `AUTH_BASE_URL` + `online-mode=true`.

## Проверенный контракт drasl (живой инстанс)

Локальный drasl: `docker run unmojang/drasl` на `:25585`, конфиг — `drasl-local/config/config.toml`.

- **Регистрация** (без токена): `POST /drasl/api/v2/users`
  `{username, password, playerName, requestApiToken:true}` → `{apiToken, user{players[]{uuid,name}}}`.
- **Вход**: `POST /drasl/api/v2/login` `{username, password}` → `{apiToken, user}`.
- **Игровая сессия (Yggdrasil)**: `POST /authlib-injector/authserver/authenticate`
  `{agent:{name:"Minecraft",version:1}, username, password}` →
  `{accessToken, clientToken, selectedProfile:{id (undashed uuid), name}}`.
  Плюс `validate`/`refresh` там же.
- **Скин**: `PATCH /drasl/api/v2/players/{uuid}` (Bearer apiToken)
  `{skinBase64, skinModel:"classic"|"slim"}`; текущий скин — `GET …/players/{uuid}` → `skinUrl`.
- **javaagent URL**: `<base>/authlib-injector`.
- **Аргументы запуска MC**: `--username <name> --uuid <id> --accessToken <accessToken>`
  + `-javaagent:authlib-injector.jar=<base>/authlib-injector`.

## Порядок реализации и статус

1. ✅ Развернуть drasl локально (Docker) + изучить/проверить API (register/login/
   yggdrasil/skin — все потоки подтверждены вживую).
2. ✅ Бэкенд `auth.rs` + хранение аккаунта (`settings.json`) + Tauri-команды
   (`auth_register/login/logout/account`, `upload_skin`), `config::AUTH_BASE_URL`
   (+ оверрайд `settings.auth_base_url`). Компилируется, 0 предупреждений.
   *Сессионные `ensure_session/validate/refresh` готовы, ждут launch-интеграции.*
3. ✅ (код) `launch.rs` — `OnlineAuth` (javaagent + реальные `--username/--uuid/
   --accessToken`); `install::play` грузит аккаунт, `ensure_session`, строит
   путь к injector из манифеста; офлайн-фолбэк, если не вошёл/нет injector.
   Сборщик качает `authlib-injector.jar` (yushi.moe) → `dist/` + поле
   `authlib_injector` в манифесте (`--skip-authlib` для пропуска). Компилируется,
   тесты зелёные. *Живой e2e — после UI (шаг 4) + пересборки dist + сервера (шаг 6).*
4. ✅ UI: `LoginScreen` (вход/регистрация, гейт перед лаунчером), «вошёл как…» +
   выход, ручной ввод ника убран (ник = аккаунт).
5. ✅ Скины: `SkinPanel` — выбор PNG → `validate_skin`/`skin_preview_file` →
   `upload_skin` + 3D-превью (`skinview3d`), переключатель classic/slim.
6. ⬜ MC-сервер: injector + online-mode → реальный e2e (инфра владельца).
7. 🟡 Polish: авто-refresh (✅ `ensure_session`), выход (✅), тосты ошибок (✅);
   остаётся обкатать вживую и причесать сообщения.

> ⚠️ Не выпускать релиз (тег) с обязательным логином, пока `AUTH_BASE_URL`
> локальный/нет публичного auth-сервера — иначе игроки упрутся в экран входа.

## Что нужно от владельца

- Поставить Docker Desktop (для локального drasl) — или разрешить установку.
- Точные эндпоинты регистрации/скина drasl зафиксировать по его докам на шаге 1.
