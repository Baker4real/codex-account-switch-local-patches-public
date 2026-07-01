import { existsSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { delimiter, dirname, isAbsolute, join } from "node:path";

function cleanEnv(name) {
  const value = process.env[name]?.trim();
  return value ? value : null;
}

function existingPath(path) {
  return path && existsSync(path) ? path : null;
}

function whereFirst(command) {
  const result = spawnSync("where", [command], {
    encoding: "utf8",
    stdio: "pipe",
  });
  if (result.error || result.status !== 0) {
    return null;
  }
  return result.stdout
    .split(/\r?\n/)
    .map((line) => line.trim())
    .find((line) => line.length > 0) ?? null;
}

function resolveTool(toolName, overrideEnv) {
  const configuredBin = cleanEnv("CODEX_SWITCH_RUSTUP_BIN");
  const cargoHome = cleanEnv("CODEX_SWITCH_CARGO_HOME") ?? cleanEnv("CARGO_HOME");
  const candidates = [
    cleanEnv(overrideEnv),
    configuredBin ? join(configuredBin, `${toolName}.exe`) : null,
    cargoHome ? join(cargoHome, "bin", `${toolName}.exe`) : null,
    whereFirst(`${toolName}.exe`),
    whereFirst(toolName),
  ];

  return candidates.find(existingPath) ?? toolName;
}

function toolBin(...tools) {
  for (const tool of tools) {
    if (isAbsolute(tool) && existsSync(tool)) {
      return dirname(tool);
    }
  }
  return cleanEnv("CODEX_SWITCH_RUSTUP_BIN");
}

function prependPath(pathValue, bin) {
  if (!bin) {
    return pathValue ?? "";
  }
  const entries = (pathValue ?? "")
    .split(delimiter)
    .filter((entry) => entry && entry.toLowerCase() !== bin.toLowerCase());
  return [bin, ...entries].join(delimiter);
}

export function resolveWindowsRustEnv() {
  const cargoExe = resolveTool("cargo", "CARGO");
  const rustcExe = resolveTool("rustc", "RUSTC");
  const bin = toolBin(cargoExe, rustcExe);
  const env = {
    ...process.env,
    RUSTC: rustcExe,
    PATH: prependPath(process.env.PATH, bin),
  };
  const cargoHome = cleanEnv("CODEX_SWITCH_CARGO_HOME") ?? cleanEnv("CARGO_HOME");
  const rustupHome = cleanEnv("CODEX_SWITCH_RUSTUP_HOME") ?? cleanEnv("RUSTUP_HOME");
  if (cargoHome) {
    env.CARGO_HOME = cargoHome;
  }
  if (rustupHome) {
    env.RUSTUP_HOME = rustupHome;
  }

  return { cargoExe, rustcExe, env };
}

export function assertMsvcRustc({ rustcExe, env, cwd, context }) {
  const rustcVersion = spawnSync(rustcExe, ["-vV"], {
    cwd,
    env,
    encoding: "utf8",
    stdio: "pipe",
  });

  if (rustcVersion.error) {
    throw new Error(
      `${context} could not start rustc. Put rustup on PATH or set CODEX_SWITCH_RUSTUP_BIN / RUSTC.\n${rustcVersion.error.message}`,
    );
  }

  if (rustcVersion.status !== 0) {
    throw new Error(
      `rustc -vV failed with exit code ${rustcVersion.status}\n${rustcVersion.stderr || rustcVersion.stdout}`,
    );
  }

  if (!rustcVersion.stdout.includes("host: x86_64-pc-windows-msvc")) {
    throw new Error(
      `${context} requires an x86_64-pc-windows-msvc Rust toolchain. Actual rustc:\n${rustcVersion.stdout}`,
    );
  }

  return rustcVersion.stdout.trim();
}
