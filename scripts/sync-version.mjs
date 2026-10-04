import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const version = readFileSync(join(root, "VERSION"), "utf8").trim();
const check = process.argv.includes("--check");

const cargo = [/(\[workspace\.package\][^\[]*?version\s*=\s*")[^"]*(")/];
const json = [/("version"\s*:\s*")[^"]*(")/];

const targets = [
  ["desktop/Cargo.toml", cargo],
  ["mobile/native/Cargo.toml", cargo],
  ["desktop/clipx-app/src-tauri/tauri.conf.json", json],
  ["desktop/clipx-app/package.json", json],
];

let drift = false;
for (const [rel, [re]] of targets) {
  const path = join(root, rel);
  const src = readFileSync(path, "utf8");
  if (!re.test(src)) {
    console.error(`no version field found: ${rel}`);
    process.exit(1);
  }
  const out = src.replace(re, `$1${version}$2`);
  if (out === src) continue;
  drift = true;
  if (check) console.error(`out of sync: ${rel}`);
  else writeFileSync(path, out);
}

if (check && drift) process.exit(1);
console.log(drift && !check ? `synced to ${version}` : `ok (${version})`);