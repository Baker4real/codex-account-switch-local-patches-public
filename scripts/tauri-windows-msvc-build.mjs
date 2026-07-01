import { existsSync } from "node:fs";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { assertMsvcRustc, resolveWindowsRustEnv } from "./windows-rust-env.mjs";

const repoRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));
const tauriCli = join(repoRoot, "node_modules", "@tauri-apps", "cli", "tauri.js");
const { rustcExe, env } = resolveWindowsRustEnv();

if (!existsSync(tauriCli)) {
  throw new Error(`Missing local Tauri CLI: ${tauriCli}`);
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    env,
    encoding: "utf8",
    stdio: options.capture ? "pipe" : "inherit",
  });

  if (result.error) {
    throw result.error;
  }

  if (result.status !== 0) {
    const detail = result.stderr || result.stdout || "";
    throw new Error(`${command} ${args.join(" ")} failed with exit code ${result.status}${detail ? `\n${detail}` : ""}`);
  }

  return result;
}

const rustcVersion = assertMsvcRustc({
  rustcExe,
  env,
  cwd: repoRoot,
  context: "Windows MSVC build",
});

console.log(rustcVersion);
run(process.execPath, [tauriCli, "build", "--target", "x86_64-pc-windows-msvc", "--no-bundle"]);
