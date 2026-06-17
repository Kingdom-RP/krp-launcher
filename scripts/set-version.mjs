// Проставляет версию релиза в tauri.conf.json и package.json.
//
// Использование:
//   node scripts/set-version.mjs 0.2.2     (локально)
//   node scripts/set-version.mjs v0.2.2    (префикс «v» убирается)
// В CI вызывается с именем тега, поэтому версия в файлах всегда совпадает с
// тегом — больше не нужно помнить про ручной бамп перед релизом.
//
// Правит только поле "version" (точечной заменой), не переформатируя файлы.

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

const raw = (process.argv[2] ?? process.env.RELEASE_VERSION ?? "").trim();
const version = raw.replace(/^v/, "");

if (!/^\d+\.\d+\.\d+$/.test(version)) {
  console.error(
    `Некорректная версия: '${raw}'. Ожидается X.Y.Z (можно с префиксом v).`,
  );
  process.exit(1);
}

const VERSION_RE = /("version":\s*")[^"]+(")/;
const files = ["src-tauri/tauri.conf.json", "package.json"];
for (const rel of files) {
  const path = join(root, rel);
  const content = readFileSync(path, "utf8");
  if (!VERSION_RE.test(content)) {
    console.error(`Не найдено поле "version" в ${rel}`);
    process.exit(1);
  }
  const updated = content.replace(VERSION_RE, `$1${version}$2`);
  if (updated !== content) {
    writeFileSync(path, updated);
    console.log(`${rel} → ${version}`);
  } else {
    console.log(`${rel} уже ${version} — пропускаем`);
  }
}
