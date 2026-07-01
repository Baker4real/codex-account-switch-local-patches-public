import { existsSync } from "node:fs";
import { delimiter, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));
const cargoHome = "D:\\Codex\\.cargo";
const rustupBin = join(cargoHome, "bin");
const cargoExe = join(rustupBin, "cargo.exe");
const rustcExe = join(rustupBin, "rustc.exe");
const args = process.argv.slice(2);

if (args.length === 0) {
  throw new Error("Usage: node scripts/cargo-windows-msvc.mjs <cargo args...>");
}

for (const path of [cargoExe, rustcExe]) {
  if (!existsSync(path)) {
    throw new Error(`Missing D-drive Rust executable: ${path}`);
  }
}

const pathEntries = (process.env.PATH || "")
  .split(delimiter)
  .filter((entry) => entry && entry.toLowerCase() !== rustupBin.toLowerCase());
const env = {
  ...process.env,
  CARGO_HOME: cargoHome,
  RUSTUP_HOME: "D:\\Codex\\.rustup",
  RUSTC: rustcExe,
  PATH: [rustupBin, ...pathEntries].join(delimiter),
};

const rustcVersion = spawnSync(rustcExe, ["-vV"], {
  cwd: repoRoot,
  env,
  encoding: "utf8",
  stdio: "pipe",
});

if (rustcVersion.error) {
  throw rustcVersion.error;
}

if (rustcVersion.status !== 0) {
  throw new Error(`rustc -vV failed with exit code ${rustcVersion.status}\n${rustcVersion.stderr || rustcVersion.stdout}`);
}

if (!rustcVersion.stdout.includes("host: x86_64-pc-windows-msvc")) {
  throw new Error(`Windows MSVC checks require D-drive MSVC rustc. Actual rustc:\n${rustcVersion.stdout}`);
}

console.log(rustcVersion.stdout.trim());

const result = spawnSync(cargoExe, args, {
  cwd: repoRoot,
  env,
  stdio: "inherit",
});

if (result.error) {
  throw result.error;
}

process.exit(result.status ?? 1);
