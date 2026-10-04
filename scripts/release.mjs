#!/usr/bin/env node
// usage: node scripts/release.mjs <patch|minor|major|x.y.z> [flags]
// flags: --dry-run --yes --no-push --skip-tests --skip-build --bundle --android --allow-branch
import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { createInterface } from "node:readline/promises";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const A = process.argv.slice(2);
const has = (f) => A.includes(f);
const bump = A.find((a) => !a.startsWith("--"));
const win = process.platform === "win32";
const termux = process.platform === "android"; // no Tauri/frontend on Termux
const ICON_SRC = "../../assets/clipx-brand-mark-squircle.svg"; // relative to desktop/clipx-app

const at = (p) => join(root, p);
const die = (m) => { console.error(`\n✖ ${m}`); process.exit(1); };
const step = (m) => console.log(`\n▶ ${m}`);
const sh = (c) => win && /^(pnpm|npx|.*\.bat)$/.test(c);
const out = (cmd, args, cwd = root) =>
  spawnSync(cmd, args, { cwd, encoding: "utf8", shell: sh(cmd) });
const run = (cmd, args, cwd = root) => {
  console.log(`  $ ${cmd} ${args.join(" ")}`);
  const r = spawnSync(cmd, args, { cwd, stdio: "inherit", shell: sh(cmd) });
  if (r.status !== 0) throw new Error(`${cmd} ${args.join(" ")} (exit ${r.status})`);
};
const git = (...a) => (out("git", a).stdout || "").trim();
const revert = () => git("checkout", "--", ".");

const parse = (v) => v.split(".").map(Number);
const gt = (a, b) => { for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] > b[i]; return false; };

function nextVersion(cur) {
  const [M, m, p] = parse(cur);
  if (bump === "major") return `${M + 1}.0.0`;
  if (bump === "minor") return `${M}.${m + 1}.0`;
  if (bump === "patch") return `${M}.${m}.${p + 1}`;
  if (/^\d+\.\d+\.\d+$/.test(bump ?? "")) return bump;
  die("usage: node scripts/release.mjs <patch|minor|major|x.y.z> [flags]");
}

function checkTools() {
  const need = [
    ["git", "git", ["--version"]],
    ["node", "node", ["--version"]],
    ["rustc", "rustc", ["--version"]],
    ["cargo", "cargo", ["--version"]],
    ["protoc", "protoc", ["--version"]],
  ];
  if (!termux) need.push(["pnpm", "pnpm", ["--version"]]);
  if (has("--android")) {
    need.push(["java", "java", ["-version"]], ["cargo-ndk", "cargo", ["ndk", "--version"]]);
  }
  let bad = 0;
  for (const [label, cmd, args] of need) {
    const r = out(cmd, args);
    const v = `${r.stdout || ""}${r.stderr || ""}`.split("\n")[0];
    if (r.error || r.status !== 0) { console.log(`  ✖ ${label}`); bad++; }
    else console.log(`  ✔ ${label.padEnd(10)} ${v}`);
  }
  if (has("--android") && !out("rustup", ["target", "list", "--installed"]).stdout?.includes("aarch64-linux-android")) {
    console.log("  ✖ rust target aarch64-linux-android (rustup target add aarch64-linux-android)");
    bad++;
  }
  if (bad) die("missing tools above");
  if (Number(process.versions.node.split(".")[0]) < 20) die("node >= 20 required");
}

function gitChecks(tag) {
  if (git("status", "--porcelain")) die("working tree not clean — commit or stash first");
  const br = git("rev-parse", "--abbrev-ref", "HEAD");
  if (br !== "main" && !has("--allow-branch")) die(`on '${br}', expected main (--allow-branch to override)`);
  if (out("git", ["fetch", "origin", "--tags", "--quiet"]).status !== 0) console.log("  ! fetch failed, skipping remote checks");
  const behind = out("git", ["rev-list", "--count", "HEAD..@{u}"]);
  if (behind.status === 0 && behind.stdout.trim() !== "0") die("branch is behind remote — pull first");
  if (out("git", ["rev-parse", "-q", "--verify", `refs/tags/${tag}`]).status === 0) die(`tag ${tag} already exists`);
}

let prepped = false;
function prep() {
  if (prepped || termux) return;
  prepped = true;
  const app = at("desktop/clipx-app");
  run("pnpm", ["install", "--frozen-lockfile"], app);
  // icons are gitignored but tauri's build script needs them
  if (!existsSync(join(app, "src-tauri/icons/32x32.png"))) run("pnpm", ["tauri", "icon", ICON_SRC], app);
}

const exclude = () => (termux ? ["--exclude", "clipx-app"] : []);

function verify() {
  if (has("--skip-tests")) return;
  step("Tests");
  prep();
  run("cargo", ["test", "--workspace", ...exclude()], at("desktop"));
  run("cargo", ["check", "-p", "rust-mobile-ffi-bridge"], at("mobile/native"));
}

function build() {
  if (has("--skip-build")) return;
  step("Build");
  prep();
  const app = at("desktop/clipx-app");
  if (!termux) run("pnpm", ["build"], app);
  run("cargo", ["build", "--release", "--workspace", ...exclude()], at("desktop"));
  if (has("--bundle")) {
    if (termux) throw new Error("--bundle is not supported on Termux");
    run("pnpm", ["release"], app);
  }
  if (has("--android")) {
    run(win ? "gradlew.bat" : "./gradlew", ["assembleRelease"], at("mobile/android"));
  }
}

async function main() {
  const cur = readFileSync(at("VERSION"), "utf8").trim();
  const next = nextVersion(cur);
  const tag = `v${next}`;
  if (!gt(parse(next), parse(cur))) die(`${next} is not greater than ${cur}`);

  step("Toolchain");
  checkTools();
  step("Git state");
  gitChecks(tag);

  step(`Bump ${cur} → ${next}`);
  try {
    writeFileSync(at("VERSION"), next);
    run("node", ["scripts/sync-version.mjs"]);
    run("node", ["scripts/sync-version.mjs", "--check"]);
    run("cargo", ["update", "-w"], at("desktop"));
    run("cargo", ["update", "-w"], at("mobile/native"));
    verify();
    build();
  } catch (e) {
    revert();
    die(`${e.message}\n  version bump reverted`);
  }

  console.log(`\n${git("diff", "--stat")}`);
  if (has("--dry-run")) { revert(); console.log("\ndry run — reverted"); return; }

  if (!has("--yes")) {
    const rl = createInterface({ input: process.stdin, output: process.stdout });
    const ans = await rl.question(`\nCommit, tag ${tag}${has("--no-push") ? "" : " and push"}? [y/N] `);
    rl.close();
    if (!/^y/i.test(ans)) { revert(); console.log("aborted — reverted"); return; }
  }

  step("Commit + tag");
  run("git", ["add", "-u"]);
  run("git", ["commit", "-m", `release: ${tag}`]);
  run("git", ["tag", "-a", tag, "-m", tag]);
  if (has("--no-push")) return console.log(`\ntagged ${tag} locally. push: git push --atomic origin HEAD ${tag}`);
  run("git", ["push", "--atomic", "origin", "HEAD", tag]);
  console.log(`\n✔ pushed ${tag} — GitHub Actions will build and publish the release`);
}

main().catch((e) => die(e.message));