import { existsSync } from "node:fs";
import { delimiter, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));
const cargoHome = process.env.CARGO_HOME || "D:\\Codex\\.cargo";
const rustupBin = join(cargoHome, "bin");
const tauriCli = join(repoRoot, "node_modules", "@tauri-apps", "cli", "tauri.js");

if (!existsSync(join(rustupBin, "rustc.exe"))) {
  throw new Error(`Missing rustup rustc shim: ${join(rustupBin, "rustc.exe")}`);
}
if (!existsSync(tauriCli)) {
  throw new Error(`Missing local Tauri CLI: ${tauriCli}`);
}

const pathEntries = (process.env.PATH || "")
  .split(delimiter)
  .filter((entry) => entry && entry.toLowerCase() !== rustupBin.toLowerCase());
const env = {
  ...process.env,
  CARGO_HOME: cargoHome,
  RUSTUP_HOME: process.env.RUSTUP_HOME || "D:\\Codex\\.rustup",
  PATH: [rustupBin, ...pathEntries].join(delimiter),
};

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

const rustcVersion = run("rustc", ["-vV"], { capture: true }).stdout;
if (!rustcVersion.includes("host: x86_64-pc-windows-msvc")) {
  throw new Error(`MSVC build requires rustup's MSVC rustc. Actual rustc:\n${rustcVersion}`);
}

console.log(rustcVersion.trim());
run(process.execPath, [tauriCli, "build", "--target", "x86_64-pc-windows-msvc", "--no-bundle"]);
