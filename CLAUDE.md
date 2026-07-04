# Kingdom RP Launcher — инструкции для Claude

Лаунчер для проекта **Kingdom RP** (мод в соседней папке `krp-mod`,
Minecraft 1.21.1 + NeoForge). Заменяет старый Electron-лаунчер, который был
медленным и тяжёлым.

## Назначение (функционал)

1. Автоскачивание и установка Java 21 (если нет), Minecraft NeoForge 1.21.1
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

- **Источник модпака:** GitHub **Pages** `https://kingdom-rp.github.io/krp-mod`
  (= `MANIFEST_BASE_URL`). Туда CI репозитория **Kingdom-RP/krp-mod** автопубликует
  `dist/` сборщика (`manifest.json` + файлы). Лаунчер сверяет SHA-256 и качает
  только изменённое. Источник за абстракцией (`config.rs`) — заменяемо на CDN.
- **Автообновление лаунчера:** GitHub **Releases** репо **Kingdom-RP/krp-launcher**
  (Tauri Updater + `latest.json`), сборка по тегу `v*` (см. фаза 5).
- **Авторизация и скины (фаза 6):** свой Yggdrasil-совместимый сервер
  **drasl** + `authlib-injector` в команде запуска MC (и клиент, и игровой
  сервер). Регистрация/вход/скины — только в лаунчере. Хост конфигурируемый
  (`AUTH_BASE_URL`). Подробный план и решения — `docs/phase6-auth.md`. Пока не
  реализовано (запуск офлайн); готова только валидация PNG-скина (`skin.rs`).
- **Загрузчик:** NeoForge **1.21.1 / 21.1.233** (`net.neoforged:neoforge:21.1.233`).
  Ваниль — гибридом с Mojang (официальный CDN по хешам), у нас хостим NeoForge +
  моды + JRE (+ конфиги) — «способ Б».

## План по фазам

1. ✅ Каркас Tauri 2 (Rust + React-TS) — создан, GUI и мост JS↔Rust проверены.
2. ✅ Установка окружения и модпак (способ Б):
   - ✅ Формат `manifest.json`, модуль скачивания (SHA-256 + прогресс),
     валидация пути, оркестрация sync, Tauri-команды — готовы.
   - ✅ UI-оболочка лаунчера (выбор/валидация пути, прогресс-бар, «Играть»).
   - ✅ Скачивание+распаковка Temurin JRE 17 (`java.rs`), проверено живым
     тестом против Adoptium (`cargo test jre_pipeline -- --ignored`).
   - ✅ Резолвер ванили с Mojang CDN (`vanilla.rs`), проверен живым тестом
     (`cargo test vanilla_resolver -- --ignored`): резолв ванили целевой версии.
   - ✅ Сборщик дистрибутива (`builder/` — отдельный CLI-крейт): качает и
     прогоняет NeoForge installer headless, харвестит `libraries/` (вкл.
     processor-выводы) + version JSON + мод, считает SHA-256, пишет
     `manifest.json`. Проверен: 83 файла, dist = libraries/+mods/+versions/.
   - ✅ Команда запуска (`launch.rs`): слияние ванильного + NeoForge version
     JSON, classpath (дедуп), подстановка плейсхолдеров, спавн java (офлайн).
     Проверена тестом `build_launch_args` — совпадает с эталоном ForgeGradle.
   - ✅ Объединённый flow «Играть» (`install::play` + команда `play`): ваниль с
     Mojang → JRE → sync манифеста → запуск. UI: поле имени игрока, кнопка.
   - ✅ JRE-entry в манифесте (сборщик качает Temurin 21 под windows-x64 и
     linux-x64 → `dist/java/`).
   - ✅ Реальный источник: `MANIFEST_BASE_URL` → GitHub Pages, автопубликация CI.
   - ✅ Живой end-to-end запуск игры: 1.21.1 + NeoForge 21.1.233 запускается до
     главного меню (проверено владельцем после миграции).
3. Модпак: загрузка модов по `manifest.json` (база готова — `sync_files`).
4. Запуск игры: построение Java-команды, classpath, аргументы NeoForge.
5. ✅ Автообновление: дифф-обновление модпака (`sync_files` по SHA-256) +
   автообновление лаунчера (Tauri Updater, `release.yml` по тегу `v*`, матрица
   Windows+Linux). Ключ подписи сгенерён, pubkey в `tauri.conf.json`, секрет
   `TAURI_SIGNING_PRIVATE_KEY` задан. Релиз: просто тег `vX.Y.Z` (`git tag vX.Y.Z
   && git push origin vX.Y.Z`) — версию в `tauri.conf.json`/`package.json` CI
   проставляет сам из тега (`scripts/set-version.mjs`), ручной бамп не нужен.
6. ⬜ Авторизация + скины (после поднятия auth-сервера) — текущий запуск офлайн.

## Автообновление лаунчера (фаза 5)

- Плагины: `tauri-plugin-updater` + `tauri-plugin-process` (Rust, в `lib.rs`);
  `@tauri-apps/plugin-updater` + `-process` (JS).
- `tauri.conf.json`: `bundle.createUpdaterArtifacts: true`, `plugins.updater`
  (`pubkey` задан; `endpoints` → `.../releases/latest/download/latest.json`).
  `app.security.csp` — задана (объектный вид по директивам, чтобы новый адрес
  добавлялся одной строкой). Почти весь сетевой трафик идёт через Rust (reqwest,
  не под CSP webview); `connect-src` нужен в основном для IPC (+ dev-HMR
  `ws://localhost:1421`). Новый внешний хост (CDN модов/auth) → добавить в
  `connect-src` (и `img-src`, если оттуда грузятся картинки в webview).
- Capabilities: `updater:default`, `process:allow-restart`.
- UI: `src/lib/updater.ts` + баннер в `App.tsx` (проверка при старте,
  скачать+установить+перезапуск).
- Релиз: `.github/workflows/release.yml` (по тегу `v*`, `tauri-action`
  собирает+подписывает+публикует Release с `latest.json`). Шаг «Set version from
  tag» перед сборкой ставит версию из имени тега (`scripts/set-version.mjs`:
  `vX.Y.Z` → правит поле `version` в `tauri.conf.json` и `package.json`, точечно,
  без переформатирования). Локально: `npm run set-version X.Y.Z` (необязательно —
  для синхронизации версии в репозитории).

## Карта Rust-модулей (`src-tauri/src/`)

- `config.rs` — константы: `MANIFEST_BASE_URL` (GitHub Pages kingdom-rp.github.io/krp-mod),
  `MINECRAFT_VERSION` 1.21.1, `NEOFORGE_VERSION` 21.1.233, `MANIFEST_PUBKEY`
  (minisign-ключ проверки подписи манифеста; пуст = проверка выключена — см.
  `docs/manifest-signing.md`), `manifest_sig_url()`.
- `error.rs` — `LauncherError` + `Result`, сериализуется во фронтенд.
- `paths.rs` — `default_install_dir` (`%APPDATA%\KingdomRP`) + валидация пути
  (кириллица/Program Files/OneDrive/длина/права записи). `safe_join(base, rel)` —
  безопасный джойн путей из манифеста/Mojang (только Normal-компоненты; `..`,
  абсолютные, префикс диска → ошибка). Защита от path traversal: применяется в
  `install::sync_manifest`/authlib-injector, `vanilla` (lib path), `launch`
  (neoforge_profile) — SHA проверяет содержимое, но не путь назначения.
- `manifest.rs` — структуры манифеста (`Manifest`, `FileEntry`, `JavaEntry`,
  `FileKind`) + `fetch_manifest`. Пример: `docs/manifest.example.json`. Если задан
  `config::MANIFEST_PUBKEY` — `fetch_manifest` качает `manifest.json.minisig` и
  проверяет minisign-подпись тела ДО разбора (fail-closed); пустой ключ → пропуск
  (`verify_signature`). Подробности и порядок раскатки — `docs/manifest-signing.md`.
- `download.rs` — `download_to_file` (потоковый прогресс; качает в `<dest>.part`
  и атомарно переименовывает — обрыв связи не оставляет битый файл на месте
  готового), `sha256_file`, `ensure_file` (скип по совпадению хеша + проверка
  после загрузки → устойчивость к прерванной установке, докачка только битых файлов).
- `java.rs` — `ensure_java`: скачивание/проверка/распаковка JRE (zip для Windows,
  tar.gz для Linux/macOS — `extract_zip`/`extract_targz`), срез верхнего каталога,
  поиск `java`; `platform_key()` определяет ОС/арх (`windows-x64`/`linux-x64`/
  `macos-x64`/`macos-arm64`). Лаунчер управляет СВОЕЙ Temurin (системную Java не
  использует); перед пропуском прогоняет `java -version` и сверяет мажор
  (`JAVA_MAJOR=21`, `java_is_valid`/`parse_java_major`) — битую/чужую версию
  сносит и перекачивает. Живой тест `jre_pipeline` (`#[ignore]`).
- `vanilla.rs` — `ensure_vanilla`: резолв и загрузка ванильного MC с Mojang CDN
  (манифест версий → version JSON → `client.jar` + библиотеки с OS-правилами +
  индекс ассетов + объекты), проверка по SHA-1; гибридная часть способа Б.
  Живой тест `vanilla_resolver` (`#[ignore]`).
- `install.rs` — оркестрация: `sync_all` (ваниль→JRE→файлы) лежит в основе
  `install_only` (установка без запуска) и `play` (установка + запуск);
  `sync_files`/`sync_manifest` — докачка файлов. Прогресс — единым трекером
  (`progress.rs`) событием `sync://progress`. Подписи этапов понятны игроку
  («Устанавливаем Minecraft 1.21.1», «Устанавливаем Java», «…моды» по `FileKind`).
  Анти-чит: `prune_mods` после sync удаляет из `mods` всё, чего нет в манифесте
  (посторонние/читерские jar'ы, подложенные игроком) — вызывается в `sync_all`,
  т.е. и при установке, и при «Играть». `ensure_default_options` пишет дефолтный
  `options.txt` (`lang:ru_ru`, `onboardAccessibility:false` — RU-язык и без
  онбординга Narrator'а) только если файла ещё нет (не затирая настройки игрока).
  `play` после старта прячет окно лаунчера (`win.hide()`), а фоновый
  `spawn_blocking` ждёт `child.wait()` и при выходе из игры шлёт
  `GAME_EXITED_EVENT` (`game://exited`) + показывает окно обратно.
  `is_installed` — проверка наличия игры (JRE + client.jar) для подписи кнопки;
  `uninstall` — удаление управляемых каталогов (`MANAGED_DIRS`: runtime/versions/
  libraries/assets/natives/mods/logs/crash-reports) + хвостов загрузок; миры и
  настройки игрока (saves/options/screenshots) сохраняются.
- `progress.rs` — `Progress`: единый трекер всей установки (общий объём `total`,
  скачано `downloaded`, скорость). `add_total`/`set_label`/`add_net` (сетевые
  байты двигают счётчик и скорость) / `add_skipped` (уже скачанные — только
  счётчик) / `file_cb` (delta-колбэк на одну загрузку, атомик ради `Send`).
  Шлёт `ProgressPayload{label,downloaded,total,speed}` событием `sync://progress`
  (троттлинг ~100мс).
- `settings.rs` — постоянные настройки лаунчера в `app_config_dir/settings.json`
  (главное — `install_dir`, `player_name`, `max_memory_mb`). Лаунчер «помнит»
  выбранную папку между запусками и после своего обновления → не предлагает
  переустановку заново. `max_memory_mb` (get/set с зажимом в
  `config::MIN/MAX_MEMORY_MB`) — память игры, слайдер в UI.
- `server.rs` — `status()`: Server List Ping (SLP) игрового сервера
  `config::SERVER_ADDR` (handshake+status, VarInt, таймаут 3с) → `ServerStatus
  {online, players_online, players_max}`; ошибка/таймаут → `online:false`.
  Команда `server_status`. UI: бейдж онлайн/оффлайн + счётчик, кнопка «Играть
  оффлайн», poll 30с.
- `skin.rs` — валидация PNG-скина (фаза 6): `validate_skin` принимает только
  PNG 64×64 (`modern`) или 64×32 (`legacy`), иначе понятная ошибка. Команда
  `validate_skin(path)`. С юнит-тестами. Загрузка скина на drasl — позже.
- `launch.rs` — `build_args`/`launch`: сливает ванильный + NeoForge version
  JSON, строит classpath и аргументы (плейсхолдеры), спавнит java. `launch`
  возвращает `Child` (на нём `wait()` ради скрытия/показа окна лаунчера).
  JVM-аргументы первыми: `-Xms/-Xmx<memory_mb>M` (из `settings.max_memory_mb`) +
  `config::JVM_PERF_ARGS` (клиентские G1GC-твики), затем ваниль+NeoForge.
  Дедуп classpath по `group:artifact:classifier` (классификатор обязателен —
  иначе нативный jar `org.lwjgl:lwjgl:…:natives-windows` принимается за дубликат
  обычного и выпадает из classpath → `UnsatisfiedLinkError: lwjgl.dll` на старте).
  Ванильный `client.jar` (`versions/1.21.1/1.21.1.jar`) в classpath НЕ кладём:
  патченый клиент приходит модулем `minecraft` из `libraries/.../client-…-srg.jar`
  (по `-DlibraryDirectory`); иначе ванильный jar становится автомодулем `_1._20._1`
  и конфликтует с `minecraft` (`ResolutionException` на старте).
  stdout/stderr игры → `<install>/logs/latest-launch.log` (без окна консоли на
  Windows, `CREATE_NO_WINDOW`); после старта ждёт ~2.5 с и при раннем крахе
  возвращает ошибку с хвостом лога (а не ложное «игра запущена»).
  Тест `build_launch_args` (требует `KRP_TEST_INSTALL`).
- `lib.rs` — Tauri-команды: `default_install_dir`, `get_install_dir`/
  `set_install_dir`/`resolve_install_dir` (запомненный путь + добавление подпапки
  «Kingdom RP»), `get_player_name`/`set_player_name`, `open_dir`,
  `is_game_installed`, `uninstall_game`, `validate_install_path`,
  `get_manifest`, `ensure_java`, `ensure_vanilla`, `sync_files`, `install_game`
  (установка без запуска), `play` (единый flow установки+запуска). `install_game`/
  `play` запоминают путь установки; `play`/`uninstall_game` уносят
  блокирующую работу в `spawn_blocking`. `open_dir` открывает только каталог
  (`is_dir`) — `explorer <файл>` на Windows мог бы запустить .exe. В `run()` —
  panic-hook, пишущий паники лаунчера в лог. Плагины: log/opener/dialog/updater/process.
  (Команды `launch_game` и демо-`greet` удалены: `launch_game` принимал `java_exe`
  от фронтенда — при XSS гаджет запуска произвольного .exe.)

## Логирование и UI-мелочи

- **Логи** — `tauri-plugin-log` (Rust) + `@tauri-apps/plugin-log` (JS): пишутся в
  файл (app log dir, `krp-launcher.log`), stdout и webview-консоль. Бэкенд логирует
  фазы `play` ([1/4]…[4/4]), итоги sync и ошибки команд (`inspect_err`); фронтенд —
  клики/ошибки. Уровень Info.
- **Кнопка-папка** (📁, правый нижний угол) — `open_dir` (своя Tauri-команда:
  Explorer на Windows / `xdg-open` на Linux), открывает папку установки игры.
  Через бэкенд, а не plugin-opener — у того scope ограничивает произвольные пути
  (`E:\Games\…` отдавал «Not allowed to open path»). Общий `reqwest::Client` в
  managed-state. Фронтенд-обёртки — `src/lib/api.ts`.
- **Никнейм** хранится в `settings.json` (`player_name`): грузится при старте
  (`get_player_name`), сохраняется при вводе и при `play` (`set_player_name`).
- **Проверка обновления вручную** — кнопка 🔄 в правом нижнем углу (`onCheckUpdate`):
  результат показывается тостом (есть обновление / последняя версия / ошибка).
  `checkUpdate` (`lib/updater.ts`) с таймаутом ~12с (`check({timeout})` + жёсткий
  JS-`Promise.race`): с российских IP сервер обновлений (GitHub CDN) часто
  недоступен, без таймаута запрос «висел» бесконечно. Ошибку проверки при старте
  тоже показываем тостом (раньше глоталась молча).
- **Тосты** — своя реализация (в Tauri нет экранных тостов, только OS-нотификации):
  `toast-stack` в правом нижнем углу, авто-скрытие через 4с (`pushToast`).
- **Выбор папки** — `pickInstallDir` через `tauri-plugin-dialog` (системный
  диалог выбора каталога); кнопка «Изменить» открывает его, а не правит строку.
  К выбранному каталогу добавляется подпапка «Kingdom RP» (`resolve_install_dir`:
  `E:\Games` → `E:\Games\Kingdom RP`). Путь сохраняется (`set_install_dir`), при
  старте читается `get_install_dir` → лаунчер помнит, где игра.
- **Удаление игры** — кнопка 🗑️ (видна, когда игра установлена) → подтверждение
  через `confirm` (plugin-dialog) → `uninstall_game`.
- **Прогресс** — общий объём (скачано/всего) + скорость (`formatSpeed`: ≥1 МБ/с —
  в МБ/с, иначе КБ/с); бар = downloaded/total. Поле никнейма показывается только
  после установки; кнопка «УСТАНОВИТЬ» (install_game) → после установки «ИГРАТЬ».
- **Никнейм** — валидация на фронте: латиница/цифры/`_`, 3–16 символов (лимит
  игрового сервера MC). Кнопка запуска: «УСТАНОВИТЬ», пока `is_game_installed`
  ложно, иначе «ИГРАТЬ».

## Сборщик дистрибутива (`builder/`)

Отдельный standalone CLI-крейт (без Tauri). Команда:
`cargo run --manifest-path builder/Cargo.toml --release -- --base-url <URL> [--skip-install --work <dir>]`.
Качает+прогоняет NeoForge installer (`--installClient`), харвестит `libraries/`
+ version JSON NeoForge + jar мода (`krp-mod/build/libs/...`), качает Temurin 21
JRE под **каждую платформу** (windows-x64 .zip + linux-x64 .tar.gz) → `dist/java/`
(+ `java`-entries в манифест), считает SHA-256, пишет `dist/manifest.json`.
`dist/` заливается на источник (`--base-url`). Если в окружении задан
`KRP_MANIFEST_SECRET_KEY` (+ опц. `KRP_MANIFEST_SECRET_KEY_PASSWORD`) — рядом
пишется minisign-подпись `dist/manifest.json.minisig` (`write_and_sign_manifest`);
нет ключа → подпись пропускается. Лаунчер проверяет её (см. `docs/manifest-signing.md`).
Флаги: `--skip-install` (переиспользовать `--work`), `--skip-jre`.

**Сторонние моды.** Их jar'ы НЕ перехостятся в `dist`: лежат в отдельном источнике
(Releases репо `krp-modpack`). Сборщик дописывает их в `manifest.json` как
`FileEntry` с **внешним** url (не base_url); лаунчер качает их напрямую в `mods/`
и сверяет хеш; `prune_mods` их не трогает (они в манифесте). Два источника
(можно вместе, при совпадении пути приоритет у modlist):
- **`--mods-release owner/repo [--mods-tag v1]`** — авто: читает ассеты Release
  через GitHub API, качает каждый `.jar`, считает SHA-256, url = ассет Release.
  Ничего вручную вписывать не надо — «залил jar в Release → пересборка → готово».
  `GH_TOKEN`/`GITHUB_TOKEN` — против лимита API (в GitHub Actions есть из коробки).
- **`--modlist <toml>`** — явный пин-список (`[[mod]]`: `file`, `url`, опц.
  `sha256`); для модов с прямых CDN (Modrinth/CF) или жёсткой фиксации версии.
  Формат — `docs/modlist.example.toml`.

**`--modlist-only`** — быстрая проверка модов без установки NeoForge/JRE/dist
(пишет manifest лишь с модами). Удобно валидировать Release/modlist за секунды.

**Стороны (`--sides sides.toml`).** Каждому `FileEntry` проставляется `side`:
`client` | `server` | `both`. Источник — `sides.toml` в krp-modpack (списки
`client`/`server` по ПОДСТРОКЕ имени jar; не указано → `both`; ядро NeoForge и наш
мод — всегда `both`). Один манифест обслуживает обе стороны: **лаунчер качает и
прунит только `client`+`both`** (`Manifest::client_files` / `FileEntry::for_client`
в `manifest.rs`, фильтр в `install.rs`), будущий серверный синк — `server`+`both`.
Поле `side` с `#[serde(default)] = Both` → старые манифесты совместимы.
Конфиги модов лягут на ту же механику (`kind:"config"` + `side`), когда понадобятся;
политика перезаписи (клиентские — если отсутствует, серверные — принудительно) —
открытое решение, не реализовано.

⚠️ Лицензии ARR ⇒ ре-хостинг чужих jar в наших Releases юридически спорен;
спорные оставлять ссылкой на исходный CDN через `--modlist`.

## Платформы

- **Windows** и **Linux** — поддерживаются (лаунчер ОС-aware: ваниль с Mojang по
  ОС/арх, JRE под платформу, classpath/нативы корректные; релиз — матрица CI
  windows + ubuntu-22.04, `tauri-action` сливает `latest.json` по платформам).
- **macOS** — отложен (нужны арх-нативы arm64 + подпись/нотаризация Apple).
- На Linux автообновление работает для **AppImage** (Tauri Updater); `.deb`
  обновляется вручную.
Ваниль (client.jar+ассеты) НЕ хостится — лаунчер берёт её с Mojang (`vanilla.rs`).

## Окружение разработки (Windows)

- Java 21 (Temurin) — нужна для сборщика дистрибутива (NeoForge installer/processors 1.21).
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
