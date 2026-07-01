import { resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { assertMsvcRustc, resolveWindowsRustEnv } from "./windows-rust-env.mjs";

const repoRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));
const args = process.argv.slice(2);

if (args.length === 0) {
  throw new Error("Usage: node scripts/cargo-windows-msvc.mjs <cargo args...>");
}

const { cargoExe, rustcExe, env } = resolveWindowsRustEnv();
const rustcVersion = assertMsvcRustc({
  rustcExe,
  env,
  cwd: repoRoot,
  context: "Windows MSVC checks",
});

console.log(rustcVersion);

const result = spawnSync(cargoExe, args, {
  cwd: repoRoot,
  env,
  stdio: "inherit",
});

if (result.error) {
  throw result.error;
}

process.exit(result.status ?? 1);
