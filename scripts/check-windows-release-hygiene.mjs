import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));

const todoFiles = [
  "src-tauri/win/front/index.html",
  "src-tauri/shared/front/i18n.ts",
];

const hygieneEntries = [
  "src-tauri/win/runtime",
  "src-tauri/shared/runtime",
  "docs/IMPLEMENTATION.md",
  "docs/SECURITY.md",
  "AGENTS.md",
  "CHANGELOG.md",
  "LOCAL_PATCHES.md",
  "README.md",
  "README.zh-CN.md",
  "scripts",
];

const todoPattern = /\bTODO\b/i;
const localCodexPathPattern = /[A-Za-z]:[\\/]+Codex(?:[\\/]|$)/i;
const hardcodedRemoteEndpointPattern =
  /\b[A-Za-z0-9._-]+@(?:10(?:\.\d{1,3}){3}|100\.(?:6[4-9]|[78]\d|9\d|1[01]\d|12[0-7])(?:\.\d{1,3}){2}|172\.(?:1[6-9]|2\d|3[01])(?:\.\d{1,3}){2}|192\.168(?:\.\d{1,3}){2})\b/i;
const deviceCodexRootPattern =
  /\bGet-DefaultCodexRoot\b|['"]D:['"][\s\S]{0,160}['"]Codex['"]/i;
const emailPattern = /[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}/g;
const allowedEmailDomains = new Set([
  "example.com",
  "example.net",
  "example.org",
  "example.test",
  "example.invalid",
]);

function toRepoPath(path) {
  return relative(repoRoot, path).replaceAll("\\", "/");
}

function read(path) {
  return readFileSync(path, "utf8");
}

function lineForOffset(text, offset) {
  return text.slice(0, offset).split(/\r?\n/).length;
}

function listFiles(entry) {
  const absolute = join(repoRoot, entry);
  if (!existsSync(absolute)) {
    return [];
  }
  const stats = statSync(absolute);
  if (stats.isFile()) {
    return [absolute];
  }
  return readdirSync(absolute, { withFileTypes: true }).flatMap((dirent) => {
    const child = join(absolute, dirent.name);
    if (dirent.isDirectory()) {
      return listFiles(toRepoPath(child));
    }
    return dirent.isFile() ? [child] : [];
  });
}

function isAllowedEmail(email) {
  const domain = email.split("@").pop()?.toLowerCase();
  return Boolean(
    domain &&
      (allowedEmailDomains.has(domain) ||
        domain.endsWith(".example") ||
        domain.endsWith(".example.com") ||
        domain.endsWith(".example.net") ||
        domain.endsWith(".example.org") ||
        domain.endsWith(".example.test") ||
        domain.endsWith(".example.invalid")),
  );
}

const failures = [];

for (const file of todoFiles.map((entry) => join(repoRoot, entry))) {
  const text = read(file);
  const match = todoPattern.exec(text);
  if (match) {
    failures.push(`${toRepoPath(file)}:${lineForOffset(text, match.index)} contains user-visible TODO`);
  }
}

for (const file of hygieneEntries.flatMap(listFiles)) {
  const text = read(file);
  const localPathMatch = localCodexPathPattern.exec(text);
  if (localPathMatch) {
    failures.push(
      `${toRepoPath(file)}:${lineForOffset(text, localPathMatch.index)} contains a local Codex absolute path`,
    );
  }

  if (toRepoPath(file).startsWith("scripts/")) {
    const remoteEndpointMatch = hardcodedRemoteEndpointPattern.exec(text);
    if (remoteEndpointMatch) {
      failures.push(
        `${toRepoPath(file)}:${lineForOffset(text, remoteEndpointMatch.index)} contains a hardcoded private remote endpoint`,
      );
    }

    const deviceRootMatch = deviceCodexRootPattern.exec(text);
    if (deviceRootMatch) {
      failures.push(
        `${toRepoPath(file)}:${lineForOffset(text, deviceRootMatch.index)} constructs a device-specific Codex root`,
      );
    }
  }

  for (const match of text.matchAll(emailPattern)) {
    const email = match[0];
    if (!isAllowedEmail(email)) {
      failures.push(
        `${toRepoPath(file)}:${lineForOffset(text, match.index ?? 0)} contains non-example email ${email}`,
      );
    }
  }
}

if (failures.length > 0) {
  console.error("Windows release hygiene check failed:");
  for (const failure of failures) {
    console.error(`- ${failure}`);
  }
  process.exit(1);
}

console.log("Windows release hygiene check passed.");
