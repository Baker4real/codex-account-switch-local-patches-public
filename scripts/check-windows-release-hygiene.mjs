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

const deployScriptPath = join(repoRoot, "scripts/deploy-remote-windows-desktop.ps1");
const buildWorkflowPath = join(repoRoot, ".github/workflows/build.yml");
const packageJsonPath = join(repoRoot, "package.json");

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

const deployScript = read(deployScriptPath);
const packageJson = JSON.parse(read(packageJsonPath));
const deployRequirements = [
  {
    pattern:
      /if\s*\(\s*-not\s+\$InspectOnly\s+-and\s+\[string\]::IsNullOrWhiteSpace\(\$RemoteStageRoot\)\s*\)/i,
    message: "must require RemoteStageRoot only for deploy mode",
  },
  {
    pattern: /EvidenceRoot is required[\s\S]*CODEX_SWITCH_DEPLOY_EVIDENCE_ROOT/i,
    message: "must fail clearly when no explicit evidence root is available",
  },
  {
    pattern:
      /\[string\]::Equals\(\$processPath,\s*\$targetExe,\s*\[StringComparison\]::OrdinalIgnoreCase\)/i,
    message: "must stop only the process launched from the shortcut target",
  },
  {
    pattern:
      /Stop-Process\s+-Id\s+\$processId\s+-Force\s+-ErrorAction\s+SilentlyContinue[\s\S]{0,240}Wait-Process\s+-Id\s+\$processId/i,
    message: "must tolerate already-exited PIDs and wait for every target process",
  },
  {
    pattern: /\$hadOriginalLoader[\s\S]*catch\s*\{[\s\S]*rollback[\s\S]*backupDir/i,
    message: "must roll back failed replacement from the backup directory",
  },
  {
    pattern: /-not\s+\$hadOriginalLoader[\s\S]*Remove-Item\s+-LiteralPath\s+\$targetLoader/i,
    message: "must remove a newly introduced loader when rollback restores an absent loader",
  },
  {
    pattern: /StrictHostKeyChecking=yes[\s\S]*UserKnownHostsFile=\$KnownHosts/i,
    message: "must require strict host-key verification with the supplied known-hosts file",
  },
  {
    pattern: /-i',\s*\$Identity[\s\S]*BatchMode=yes[\s\S]*IdentitiesOnly=yes/i,
    message: "must use the supplied identity non-interactively",
  },
  {
    pattern: /KnownHostsPath is required[\s\S]*IdentityFile is required/i,
    message: "must require explicit known-hosts and identity inputs",
  },
  {
    pattern: /PSVersionTable\.PSVersion\.Major\s+-lt\s+7/i,
    message: "must require PowerShell 7 instead of silently using the legacy host",
  },
  {
    pattern: /function Get-RemotePowerShellCommand[\s\S]*'"\{0\}" -NoLogo -NoProfile -ExecutionPolicy Bypass -EncodedCommand \{1\}'/i,
    message: "must quote an explicit remote PowerShell path before execution",
  },
];

if (/\.scratch[\s\S]{0,100}run-evidence/i.test(deployScript)) {
  failures.push("scripts/deploy-remote-windows-desktop.ps1 must not fall back to .scratch evidence");
}

if (/StrictHostKeyChecking=accept-new|AskPassPath|SSH_ASKPASS|BatchMode=no/i.test(deployScript)) {
  failures.push(
    "scripts/deploy-remote-windows-desktop.ps1 must not weaken strict SSH with host enrollment or password fallback",
  );
}

if (!/^pwsh\s/i.test(packageJson.scripts?.["deploy:windows:remote-desktop"] ?? "")) {
  failures.push(
    "package.json deploy:windows:remote-desktop must default to PowerShell 7 (pwsh)",
  );
}

for (const requirement of deployRequirements) {
  if (!requirement.pattern.test(deployScript)) {
    failures.push(`scripts/deploy-remote-windows-desktop.ps1 ${requirement.message}`);
  }
}

const residualProcessCheckIndex = deployScript.indexOf(
  "Target process still running before deployment copy",
);
const safeStopIndex = deployScript.indexOf(
  "Stop-Process -Id $processId -Force -ErrorAction SilentlyContinue",
);
const waitProcessIndex = deployScript.indexOf("Wait-Process -Id $processId");
const targetCopyIndex = deployScript.indexOf(
  "Copy-Item -LiteralPath $stageExe -Destination $targetExe -Force",
);
if (
  safeStopIndex < 0 ||
  waitProcessIndex < safeStopIndex ||
  residualProcessCheckIndex < 0 ||
  residualProcessCheckIndex < waitProcessIndex ||
  targetCopyIndex < 0 ||
  residualProcessCheckIndex > targetCopyIndex
) {
  failures.push(
    "scripts/deploy-remote-windows-desktop.ps1 must reject residual target processes before replacement",
  );
}

const buildWorkflow = read(buildWorkflowPath);
const frontTestIndex = buildWorkflow.indexOf("npm run test:front");
const hygieneIndex = buildWorkflow.indexOf("npm run check:windows-release-hygiene");
if (hygieneIndex < 0 || frontTestIndex < 0 || hygieneIndex < frontTestIndex) {
  failures.push(".github/workflows/build.yml must run release hygiene after front-end tests");
}

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
