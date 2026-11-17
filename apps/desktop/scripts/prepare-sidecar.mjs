// Compile le moteur studiod et le copie là où Tauri attend le sidecar :
// src-tauri/binaries/studiod-<triplet cible>[.exe] (voir bundle.externalBin).
//
// Usage : node scripts/prepare-sidecar.mjs [--release]
// Variables : TAURI_ENV_TARGET_TRIPLE (compilation croisée), CARGO_TARGET_DIR.

import { execFileSync } from "node:child_process";
import { copyFileSync, chmodSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, "../../..");
const release = process.argv.includes("--release");

function hostTriple() {
  const out = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  const line = out.split("\n").find((l) => l.startsWith("host:"));
  if (!line) throw new Error("triplet hôte introuvable dans « rustc -vV »");
  return line.slice("host:".length).trim();
}

const triple = process.env.TAURI_ENV_TARGET_TRIPLE || hostTriple();
const isWindows = triple.includes("windows");
const ext = isWindows ? ".exe" : "";
const profile = release ? "release" : "debug";

const args = ["build", "-p", "studiod"];
if (release) args.push("--release");
if (process.env.TAURI_ENV_TARGET_TRIPLE) args.push("--target", triple);
console.log(`[sidecar] cargo ${args.join(" ")}`);
execFileSync("cargo", args, { cwd: repoRoot, stdio: "inherit" });

const targetDir = process.env.CARGO_TARGET_DIR ? resolve(process.env.CARGO_TARGET_DIR) : join(repoRoot, "target");
const built = process.env.TAURI_ENV_TARGET_TRIPLE
  ? join(targetDir, triple, profile, `studiod${ext}`)
  : join(targetDir, profile, `studiod${ext}`);
const dest = join(here, "..", "src-tauri", "binaries", `studiod-${triple}${ext}`);
mkdirSync(dirname(dest), { recursive: true });
copyFileSync(built, dest);
if (!isWindows) chmodSync(dest, 0o755);
console.log(`[sidecar] ${built} → ${dest}`);
