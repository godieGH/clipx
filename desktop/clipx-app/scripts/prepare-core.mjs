import { execSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";

const triple = /host: (\S+)/.exec(execSync("rustc -vV").toString())[1];
const ext = process.platform === "win32" ? ".exe" : "";

// the workspace root is one level up from clipx-app
execSync("cargo build -p clipx-core --release", { stdio: "inherit", cwd: ".." });
execSync("cargo build -p clipx --bin clipx-send --release", { stdio: "inherit", cwd: ".." });

mkdirSync("src-tauri/binaries", { recursive: true });
copyFileSync(
  `../target/release/clipx-core${ext}`,
  `src-tauri/binaries/clipx-core-${triple}${ext}`
);
copyFileSync(
  `../target/release/clipx-send${ext}`,
  `src-tauri/binaries/clipx-send-${triple}${ext}`
);