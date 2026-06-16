# Kingdom RP Launcher — инструкции для Claude

Лаунчер для проекта **Kingdom RP** (мод в соседней папке `krp-mod`,
Minecraft 1.20.1 + NeoForge). Заменяет старый Electron-лаунчер, который был
медленным и тяжёлым.

## Назначение (функционал)

1. Автоскачивание и установка Java 17 (если нет), Minecraft NeoForge 1.20.1
   и всех модов/файлов Kingdom RP.
2. Автообновление лаунчера и игры/модов.
3. Авторизация логин/пароль для входа в игру.
4. (Опционально) установка скина.

## Технологический стек

- **Tauri 2** — оболочка (лёгкая, кроссплатформенная, замена Electron).
- **Rust** — бэкенд: скачивание, проверка SHA-хешей, распаковка, запуск
  Java-процесса Minecraft.
- **React 19 + TypeScript + Vite** — фронтенд (UI).
- Менеджер пакетов — **npm**.

**Почему так:** Tauri использует системный WebView2 (вес ~единицы МБ против
100+ МБ у Electron), Rust закрывает всю «тяжёлую» работу лаунчера. React выбран
вместо Svelte ради максимума проверенных паттернов и читаемости для владельца.

## Ключевые проектные решения

- **Источник обновлений:** GitHub Releases. На релиз кладётся `manifest.json`
  (список файлов + SHA-256 + версии) и архивы. Лаунчер сверяет хеши и качает
  только изменённое. Сам лаунчер обновляется через Tauri Updater.
  Источник спрятан за абстракцией в коде — позже можно заменить на свой CDN.
- **Авторизация и скины:** своего auth-сервера ПОКА НЕТ. Схема — свой
  Yggdrasil-совместимый сервер + `authlib-injector` в команде запуска MC.
  Реализуется отдельной фазой; команда запуска проектируется так, чтобы
  injector добавлялся легко.
- **Загрузчик:** NeoForge **1.20.1 / 47.1.106** (артефакт
  `net.neoforged:forge:1.20.1-47.1.106`). Мод в `krp-mod` мигрирован с
  MinecraftForge на NeoForge. Ваниль — гибридом с Mojang (официальный CDN по
  хешам), у нас хостим NeoForge + моды + конфиги + JRE (способ Б).

## План по фазам

1. ✅ Каркас Tauri 2 (Rust + React-TS) — создан, GUI и мост JS↔Rust проверены.
2. 🚧 Установка окружения и модпак (способ Б):
   - ✅ Формат `manifest.json`, модуль скачивания (SHA-256 + прогресс),
     валидация пути, оркестрация sync, Tauri-команды — готовы.
   - ✅ UI-оболочка лаунчера (выбор/валидация пути, прогресс-бар, «Играть»).
   - ✅ Скачивание+распаковка Temurin JRE 17 (`java.rs`), проверено живым
     тестом против Adoptium (`cargo test jre_pipeline -- --ignored`).
   - ✅ Резолвер ванили с Mojang CDN (`vanilla.rs`), проверен живым тестом
     (`cargo test vanilla_resolver -- --ignored`): 1.20.1 = 88 либ, 3597 ассетов.
   - ✅ Сборщик дистрибутива (`builder/` — отдельный CLI-крейт): качает и
     прогоняет NeoForge installer headless, харвестит `libraries/` (вкл.
     processor-выводы) + version JSON + мод, считает SHA-256, пишет
     `manifest.json`. Проверен: 83 файла, dist = libraries/+mods/+versions/.
   - ✅ Команда запуска (`launch.rs`): слияние ванильного + NeoForge version
     JSON, classpath (дедуп), подстановка плейсхолдеров, спавн java (офлайн).
     Проверена тестом `build_launch_args` — совпадает с эталоном ForgeGradle.
   - ✅ Объединённый flow «Играть» (`install::play` + команда `play`): ваниль с
     Mojang → JRE → sync манифеста → запуск. UI: поле имени игрока, кнопка.
   - ✅ JRE-entry в манифесте (сборщик качает Temurin 17 → `dist/java/`,
     пишет `java["windows-x64"]`).
   - ⬜ Реальный `MANIFEST_BASE_URL` + заливка `dist/` на GitHub Releases;
     живой end-to-end запуск игры (проверяет владелец).
3. Модпак: загрузка модов по `manifest.json` (база готова — `sync_files`).
4. Запуск игры: построение Java-команды, classpath, аргументы NeoForge.
5. Автообновление: Tauri Updater + дифф-обновление модов по манифесту.
6. Авторизация + скины (после поднятия auth-сервера).

## Карта Rust-модулей (`src-tauri/src/`)

- `config.rs` — константы; `MANIFEST_BASE_URL` (TODO: реальный источник).
- `error.rs` — `LauncherError` + `Result`, сериализуется во фронтенд.
- `paths.rs` — `default_install_dir` (`%APPDATA%\KingdomRP`) + валидация пути
  (кириллица/Program Files/OneDrive/длина/права записи).
- `manifest.rs` — структуры манифеста (`Manifest`, `FileEntry`, `JavaEntry`,
  `FileKind`) + `fetch_manifest`. Пример: `docs/manifest.example.json`.
- `download.rs` — `download_to_file` (потоковый прогресс), `sha256_file`,
  `ensure_file` (скип по совпадению хеша + проверка после загрузки).
- `java.rs` — `ensure_java`: скачивание/проверка/распаковка JRE, срез верхнего
  каталога архива, поиск `java.exe`; `platform_key()` (пока `windows-x64`).
  Живой тест `jre_pipeline` (`#[ignore]`).
- `vanilla.rs` — `ensure_vanilla`: резолв и загрузка ванильного MC с Mojang CDN
  (манифест версий → version JSON → `client.jar` + библиотеки с OS-правилами +
  индекс ассетов + объекты), проверка по SHA-1; гибридная часть способа Б.
  Живой тест `vanilla_resolver` (`#[ignore]`).
- `install.rs` — оркестрация: `sync_files`/`sync_manifest` (докачка файлов),
  `ensure_java`, `ensure_vanilla` и `play` (полный цикл: ваниль→JRE→sync→запуск);
  эмитят прогресс `sync://progress`.
- `launch.rs` — `build_args`/`launch`: сливает ванильный + NeoForge version
  JSON, строит classpath и аргументы (плейсхолдеры), спавнит java. Офлайн-режим.
  Тест `build_launch_args` (требует `KRP_TEST_INSTALL`).
- `lib.rs` — Tauri-команды: `default_install_dir`, `validate_install_path`,
  `get_manifest`, `ensure_java`, `ensure_vanilla`, `sync_files`, `launch_game`,
  `play` (+ демо `greet`).
  Общий `reqwest::Client` в managed-state. Фронтенд-обёртки — `src/lib/api.ts`.

## Сборщик дистрибутива (`builder/`)

Отдельный standalone CLI-крейт (без Tauri). Команда:
`cargo run --manifest-path builder/Cargo.toml --release -- --base-url <URL> [--skip-install --work <dir>]`.
Качает+прогоняет NeoForge installer (`--installClient`), харвестит `libraries/`
+ version JSON NeoForge + jar мода (`krp-mod/build/libs/...`), качает Temurin 17
JRE → `dist/java/` (+ `java`-entry в манифест), считает SHA-256, пишет
`dist/manifest.json`. `dist/` заливается на источник (`--base-url`).
Флаги: `--skip-install` (переиспользовать `--work`), `--skip-jre`.
Ваниль (client.jar+ассеты) НЕ хостится — лаунчер берёт её с Mojang (`vanilla.rs`).

## Окружение разработки (Windows)

- Java 17 (Temurin) — установлено.
- Node.js LTS (v24), npm — установлено через winget.
- Rust/cargo (stable, `x86_64-pc-windows-msvc`) — установлено через winget.
- Visual Studio Community 2022 с C++ tools (MSVC-линкер) — есть.
- WebView2 Runtime — есть.

### Команды

- `npm install` — зависимости фронтенда.
- `npm run tauri dev` — запуск лаунчера в dev-режиме (GUI-окно).
- `npm run tauri build` — сборка релизного бинарника.
- `cargo build` (в `src-tauri/`) — сборка только Rust-части.

## Заметки

- PowerShell-сессии не наследуют свежий PATH после установки тулчейнов —
  при необходимости обновлять:
  `$env:Path = [Environment]::GetEnvironmentVariable("Path","Machine") + ";" + [Environment]::GetEnvironmentVariable("Path","User")`
