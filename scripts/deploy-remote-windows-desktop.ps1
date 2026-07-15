[CmdletBinding()]
param(
    [string]$RemoteTarget = $env:CODEX_SWITCH_REMOTE_TARGET,
    [string]$RemoteDesktopLink = $env:CODEX_SWITCH_REMOTE_DESKTOP_LINK,
    [string]$RemoteStageRoot = $env:CODEX_REMOTE_STAGE_ROOT,
    [string]$ReleaseRoot,
    [string]$LoaderPath,
    [string]$EvidenceRoot = $env:CODEX_SWITCH_DEPLOY_EVIDENCE_ROOT,
    [string]$KnownHostsPath,
    [string]$AskPassPath,
    [string]$IdentityFile,
    [string]$RemotePowerShellPath = 'D:\Codex\tools\PowerShell\7.6.3\pwsh.exe',
    [switch]$InspectOnly
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Resolve-RepoRoot {
    return (Split-Path -Parent $PSScriptRoot)
}

function Quote-PSLiteral {
    param([AllowNull()][string]$Value)
    if ($null -eq $Value) {
        return "''"
    }

    return "'" + ($Value -replace "'", "''") + "'"
}

function ConvertTo-ScpRemotePath {
    param([string]$Path)
    return ($Path -replace '\\', '/')
}

function New-RunDirectory {
    param([string]$Root)

    New-Item -ItemType Directory -Force -Path $Root | Out-Null
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $path = Join-Path $Root "remote-desktop-deploy-$stamp"
    New-Item -ItemType Directory -Force -Path $path | Out-Null

    [pscustomobject]@{
        Stamp = $stamp
        Path = $path
    }
}

function Resolve-ReleaseArtifacts {
    param(
        [string]$Root,
        [string]$ExplicitLoaderPath
    )

    $exePath = Join-Path $Root 'codex_switch.exe'
    if (-not (Test-Path -LiteralPath $exePath)) {
        throw "Missing Windows release executable: $exePath"
    }

    $resolvedLoader = $ExplicitLoaderPath
    if (-not $resolvedLoader) {
        $buildRoot = Join-Path $Root 'build'
        if (-not (Test-Path -LiteralPath $buildRoot)) {
            throw "Missing Tauri build output directory: $buildRoot"
        }

        $resolvedLoader = Get-ChildItem -LiteralPath $buildRoot -Recurse -Filter 'WebView2Loader.dll' |
            Where-Object { $_.FullName -like '*\out\x64\WebView2Loader.dll' } |
            Sort-Object LastWriteTimeUtc -Descending |
            Select-Object -First 1 -ExpandProperty FullName
    }

    if (-not $resolvedLoader -or -not (Test-Path -LiteralPath $resolvedLoader)) {
        throw 'Missing x64 WebView2Loader.dll. Build with npm run tauri:build:windows:portable first.'
    }

    $exeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $exePath).Hash
    $loaderHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $resolvedLoader).Hash

    [pscustomobject]@{
        ExePath = $exePath
        LoaderPath = $resolvedLoader
        ExeHash = $exeHash
        LoaderHash = $loaderHash
    }
}

function Get-EffectiveAskPassPath {
    param([string]$RequestedPath)

    if ($RequestedPath) {
        if (-not (Test-Path -LiteralPath $RequestedPath)) {
            throw "Configured AskPassPath does not exist: $RequestedPath"
        }

        return $RequestedPath
    }

    if ($env:SSH_ASKPASS -and (Test-Path -LiteralPath $env:SSH_ASKPASS)) {
        return $env:SSH_ASKPASS
    }

    $defaultAskPass = Join-Path $EvidenceRoot 'ssh-askpass-home.cmd'
    if (Test-Path -LiteralPath $defaultAskPass) {
        return $defaultAskPass
    }

    return $null
}

function Invoke-CheckedProcess {
    param(
        [Parameter(Mandatory = $true)]
        [string]$FileName,

        [Parameter(Mandatory = $true)]
        [string[]]$Arguments,

        [string]$OutFile,

        [string]$ErrFile,

        [string]$AskPass
    )

    $psi = [Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $FileName
    $psi.UseShellExecute = $false
    $psi.RedirectStandardInput = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true

    if ($AskPass) {
        $psi.Environment['SSH_ASKPASS'] = $AskPass
        $psi.Environment['SSH_ASKPASS_REQUIRE'] = 'force'
        $psi.Environment['DISPLAY'] = 'codex'
    }

    foreach ($argument in $Arguments) {
        [void]$psi.ArgumentList.Add($argument)
    }

    $process = [Diagnostics.Process]::Start($psi)
    $process.StandardInput.Close()
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $process.WaitForExit()

    $stdout = $stdoutTask.Result
    $stderr = $stderrTask.Result

    if ($OutFile) {
        [IO.File]::WriteAllText($OutFile, $stdout, [Text.UTF8Encoding]::new($false))
    }

    if ($ErrFile) {
        [IO.File]::WriteAllText($ErrFile, $stderr, [Text.UTF8Encoding]::new($false))
    }

    if ($process.ExitCode -ne 0) {
        $detail = if ($stderr) { $stderr } else { $stdout }
        throw "$FileName failed with exit code $($process.ExitCode). $detail"
    }

    return $stdout
}

function Get-SshBaseArgs {
    param(
        [string]$KnownHosts,
        [bool]$UseAskPass,
        [string]$Identity
    )

    $arguments = @(
        '-o', 'StrictHostKeyChecking=accept-new',
        '-o', "UserKnownHostsFile=$KnownHosts"
    )

    if ($Identity) {
        $arguments += @('-i', $Identity, '-o', 'IdentitiesOnly=yes')
    }

    if ($UseAskPass) {
        $arguments += @(
            '-o', 'BatchMode=no',
            '-o', 'NumberOfPasswordPrompts=1',
            '-o', 'PreferredAuthentications=keyboard-interactive,password',
            '-o', 'PubkeyAuthentication=no',
            '-o', 'KbdInteractiveAuthentication=yes'
        )
    } else {
        $arguments += @(
            '-o', 'BatchMode=yes'
        )
    }

    return $arguments
}

function Invoke-RemotePowerShell {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Command,

        [Parameter(Mandatory = $true)]
        [string]$OutFile,

        [Parameter(Mandatory = $true)]
        [string]$ErrFile,

        [string]$AskPass,

        [Parameter(Mandatory = $true)]
        [string]$KnownHosts
    )

    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($Command))
    $arguments = @()
    $arguments += Get-SshBaseArgs -KnownHosts $KnownHosts -UseAskPass ([bool]$AskPass) -Identity $IdentityFile
    $arguments += @(
        $RemoteTarget,
        "$RemotePowerShellPath -NoLogo -NoProfile -ExecutionPolicy Bypass -EncodedCommand $encoded"
    )

    Invoke-CheckedProcess -FileName (Get-Command ssh).Source -Arguments $arguments -OutFile $OutFile -ErrFile $ErrFile -AskPass $AskPass
}

function Copy-ToRemote {
    param(
        [Parameter(Mandatory = $true)]
        [string]$LocalPath,

        [Parameter(Mandatory = $true)]
        [string]$RemotePath,

        [Parameter(Mandatory = $true)]
        [string]$OutFile,

        [Parameter(Mandatory = $true)]
        [string]$ErrFile,

        [string]$AskPass,

        [Parameter(Mandatory = $true)]
        [string]$KnownHosts
    )

    $arguments = @()
    $arguments += Get-SshBaseArgs -KnownHosts $KnownHosts -UseAskPass ([bool]$AskPass) -Identity $IdentityFile
    $arguments += @(
        $LocalPath,
        "${RemoteTarget}:$(ConvertTo-ScpRemotePath -Path $RemotePath)"
    )

    Invoke-CheckedProcess -FileName (Get-Command scp).Source -Arguments $arguments -OutFile $OutFile -ErrFile $ErrFile -AskPass $AskPass | Out-Null
}

function New-InspectCommand {
    param([string]$DesktopLink)

    $linkLiteral = Quote-PSLiteral -Value $DesktopLink
    @"
`$ErrorActionPreference = 'Stop'
`$ProgressPreference = 'SilentlyContinue'
`$desktopLink = $linkLiteral
`$shortcut = `$null
if (Test-Path -LiteralPath `$desktopLink) {
    `$shell = New-Object -ComObject WScript.Shell
    `$shortcut = `$shell.CreateShortcut(`$desktopLink)
}
`$targetPath = if (`$shortcut) { `$shortcut.TargetPath } else { `$null }
`$workingDirectory = if (`$shortcut) { `$shortcut.WorkingDirectory } else { `$null }
`$targetDir = if (`$targetPath) { Split-Path -Parent `$targetPath } else { `$null }
`$loaderPath = if (`$targetDir) { Join-Path `$targetDir 'WebView2Loader.dll' } else { `$null }
[pscustomobject]@{
    computerName = `$env:COMPUTERNAME
    desktopLink = `$desktopLink
    desktopLinkExists = Test-Path -LiteralPath `$desktopLink
    targetPath = `$targetPath
    targetExists = if (`$targetPath) { Test-Path -LiteralPath `$targetPath } else { `$false }
    workingDirectory = `$workingDirectory
    loaderPath = `$loaderPath
    loaderExists = if (`$loaderPath) { Test-Path -LiteralPath `$loaderPath } else { `$false }
    targetHash = if (`$targetPath -and (Test-Path -LiteralPath `$targetPath)) { (Get-FileHash -Algorithm SHA256 -LiteralPath `$targetPath).Hash } else { `$null }
    loaderHash = if (`$loaderPath -and (Test-Path -LiteralPath `$loaderPath)) { (Get-FileHash -Algorithm SHA256 -LiteralPath `$loaderPath).Hash } else { `$null }
} | ConvertTo-Json -Depth 5
"@
}

function Write-RemoteDeployFiles {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RunDir,

        [Parameter(Mandatory = $true)]
        [string]$StageDir,

        [Parameter(Mandatory = $true)]
        [object]$Artifacts,

        [Parameter(Mandatory = $true)]
        [string]$DesktopLink,

        [Parameter(Mandatory = $true)]
        [string]$Stamp
    )

    $configPath = Join-Path $RunDir 'remote-deploy-config.json'
    $scriptPath = Join-Path $RunDir 'remote-deploy-codex-switch.ps1'

    [pscustomobject]@{
        desktopLink = $DesktopLink
        stageDir = $StageDir
        expectedExeHash = $Artifacts.ExeHash
        expectedLoaderHash = $Artifacts.LoaderHash
        stamp = $Stamp
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $configPath -Encoding UTF8

    @'
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$configPath = Join-Path $PSScriptRoot 'remote-deploy-config.json'
$config = Get-Content -LiteralPath $configPath -Raw | ConvertFrom-Json

$stageExe = Join-Path $config.stageDir 'codex_switch.exe'
$stageLoader = Join-Path $config.stageDir 'WebView2Loader.dll'
if (-not (Test-Path -LiteralPath $stageExe)) { throw "Missing staged exe: $stageExe" }
if (-not (Test-Path -LiteralPath $stageLoader)) { throw "Missing staged loader: $stageLoader" }

$stageExeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $stageExe).Hash
$stageLoaderHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $stageLoader).Hash
if ($stageExeHash -ne $config.expectedExeHash) { throw "Staged exe hash mismatch: $stageExeHash" }
if ($stageLoaderHash -ne $config.expectedLoaderHash) { throw "Staged loader hash mismatch: $stageLoaderHash" }

if (-not (Test-Path -LiteralPath $config.desktopLink)) {
    throw "Desktop shortcut not found: $($config.desktopLink)"
}

$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($config.desktopLink)
$targetExe = $shortcut.TargetPath
if ([string]::IsNullOrWhiteSpace($targetExe)) {
    throw "Desktop shortcut has no target path: $($config.desktopLink)"
}
if (-not (Test-Path -LiteralPath $targetExe)) {
    throw "Desktop shortcut target does not exist: $targetExe"
}

$targetDir = Split-Path -Parent $targetExe
$targetLoader = Join-Path $targetDir 'WebView2Loader.dll'
$backupDir = Join-Path $targetDir ("backup\deploy-" + $config.stamp)
New-Item -ItemType Directory -Force -Path $backupDir | Out-Null

$beforeExeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $targetExe).Hash
$hadOriginalLoader = Test-Path -LiteralPath $targetLoader
$beforeLoaderHash = if ($hadOriginalLoader) { (Get-FileHash -Algorithm SHA256 -LiteralPath $targetLoader).Hash } else { $null }
$backupExe = Join-Path $backupDir 'codex_switch.exe'
$backupLoader = Join-Path $backupDir 'WebView2Loader.dll'

Copy-Item -LiteralPath $targetExe -Destination $backupExe -Force
if ($hadOriginalLoader) {
    Copy-Item -LiteralPath $targetLoader -Destination $backupLoader -Force
}

function Get-TargetProcesses {
    param([string]$TargetExe)

    @(
        Get-Process -Name 'codex_switch' -ErrorAction SilentlyContinue | Where-Object {
            $processPath = $null
            try { $processPath = $_.Path } catch { $processPath = $null }
            $processPath -and [string]::Equals($processPath, $TargetExe, [StringComparison]::OrdinalIgnoreCase)
        }
    )
}

$stopped = @()
Get-TargetProcesses -TargetExe $targetExe | ForEach-Object {
    $processId = $_.Id
    $processPath = $_.Path
    $stopped += [pscustomobject]@{ id = $processId; path = $processPath }
    Stop-Process -Id $processId -Force -ErrorAction SilentlyContinue
    Wait-Process -Id $processId -Timeout 15 -ErrorAction SilentlyContinue
    if (Get-Process -Id $processId -ErrorAction SilentlyContinue) {
        throw "Target process did not exit after Stop-Process: PID $processId ($processPath)"
    }
}

$remainingTargetProcesses = @(Get-TargetProcesses -TargetExe $targetExe)
if ($remainingTargetProcesses.Count -gt 0) {
    $remainingIds = ($remainingTargetProcesses | ForEach-Object { $_.Id }) -join ', '
    throw "Target process still running before deployment copy: PID(s) $remainingIds ($targetExe)"
}

try {
    Copy-Item -LiteralPath $stageExe -Destination $targetExe -Force
    Copy-Item -LiteralPath $stageLoader -Destination $targetLoader -Force

    $afterExeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $targetExe).Hash
    $afterLoaderHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $targetLoader).Hash
    if ($afterExeHash -ne $config.expectedExeHash) { throw "Target exe hash mismatch after copy: $afterExeHash" }
    if ($afterLoaderHash -ne $config.expectedLoaderHash) { throw "Target loader hash mismatch after copy: $afterLoaderHash" }
} catch {
    $deployError = $_.Exception.Message
    $rollbackStatus = 'started'
    try {
        Copy-Item -LiteralPath $backupExe -Destination $targetExe -Force
        $restoredExeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $targetExe).Hash
        if ($restoredExeHash -ne $beforeExeHash) {
            throw "Restored executable hash mismatch: $restoredExeHash"
        }

        if (-not $hadOriginalLoader) {
            if (Test-Path -LiteralPath $targetLoader) {
                Remove-Item -LiteralPath $targetLoader -Force
            }
            if (Test-Path -LiteralPath $targetLoader) {
                throw "Restored loader state mismatch: expected no file at $targetLoader"
            }
        } else {
            Copy-Item -LiteralPath $backupLoader -Destination $targetLoader -Force
            $restoredLoaderHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $targetLoader).Hash
            if ($restoredLoaderHash -ne $beforeLoaderHash) {
                throw "Restored loader hash mismatch: $restoredLoaderHash"
            }
        }
        $rollbackStatus = 'succeeded'
    } catch {
        $rollbackStatus = "failed: $($_.Exception.Message)"
    }
    throw "Deployment failed: $deployError; backupDir=$backupDir; rollback=$rollbackStatus"
}

[pscustomobject]@{
    computerName = $env:COMPUTERNAME
    desktopLink = $config.desktopLink
    targetExe = $targetExe
    targetLoader = $targetLoader
    workingDirectory = $shortcut.WorkingDirectory
    stageDir = $config.stageDir
    backupDir = $backupDir
    stoppedProcesses = $stopped
    beforeExeHash = $beforeExeHash
    beforeLoaderHash = $beforeLoaderHash
    afterExeHash = $afterExeHash
    afterLoaderHash = $afterLoaderHash
} | ConvertTo-Json -Depth 8
'@ | Set-Content -LiteralPath $scriptPath -Encoding UTF8

    [pscustomobject]@{
        ConfigPath = $configPath
        ScriptPath = $scriptPath
    }
}

$repoRoot = Resolve-RepoRoot
if ([string]::IsNullOrWhiteSpace($RemoteTarget)) {
    throw 'RemoteTarget is required. Pass -RemoteTarget or set CODEX_SWITCH_REMOTE_TARGET.'
}
if ([string]::IsNullOrWhiteSpace($RemoteDesktopLink)) {
    throw 'RemoteDesktopLink is required. Pass -RemoteDesktopLink or set CODEX_SWITCH_REMOTE_DESKTOP_LINK.'
}
if (-not $InspectOnly -and [string]::IsNullOrWhiteSpace($RemoteStageRoot)) {
    throw 'RemoteStageRoot is required. Pass -RemoteStageRoot or set CODEX_REMOTE_STAGE_ROOT.'
}
if ([string]::IsNullOrWhiteSpace($EvidenceRoot)) {
    if (-not [string]::IsNullOrWhiteSpace($env:CODEX_HOME)) {
        $EvidenceRoot = Join-Path (Join-Path $env:CODEX_HOME 'run-evidence') 'codex-account-switch'
    } else {
        throw 'EvidenceRoot is required. Pass -EvidenceRoot, set CODEX_SWITCH_DEPLOY_EVIDENCE_ROOT, or set CODEX_HOME.'
    }
}
if (-not $ReleaseRoot) {
    $ReleaseRoot = Join-Path $repoRoot 'src-tauri\target\x86_64-pc-windows-msvc\release'
}

if (-not $KnownHostsPath) {
    $KnownHostsPath = Join-Path $EvidenceRoot 'home_known_hosts'
}
if ($IdentityFile -and -not (Test-Path -LiteralPath $IdentityFile -PathType Leaf)) {
    throw "IdentityFile does not exist: $IdentityFile"
}

$run = New-RunDirectory -Root $EvidenceRoot
$askPass = Get-EffectiveAskPassPath -RequestedPath $AskPassPath
$knownHostsParent = Split-Path -Parent $KnownHostsPath
New-Item -ItemType Directory -Force -Path $knownHostsParent | Out-Null

$inspectCommand = New-InspectCommand -DesktopLink $RemoteDesktopLink
$inspectBeforePath = Join-Path $run.Path 'inspect-before.json'
$inspectBeforeErrPath = Join-Path $run.Path 'inspect-before.stderr.txt'
$inspectBefore = Invoke-RemotePowerShell -Command $inspectCommand -OutFile $inspectBeforePath -ErrFile $inspectBeforeErrPath -AskPass $askPass -KnownHosts $KnownHostsPath

if ($InspectOnly) {
    [pscustomobject]@{
        mode = 'inspect'
        evidenceDir = $run.Path
        remoteTarget = $RemoteTarget
        inspect = ($inspectBefore | ConvertFrom-Json)
    } | ConvertTo-Json -Depth 8
    return
}

$artifacts = Resolve-ReleaseArtifacts -Root $ReleaseRoot -ExplicitLoaderPath $LoaderPath
$remoteStageDir = ($RemoteStageRoot.TrimEnd([char[]]@('\', '/')) + '\' + $run.Stamp)
$createStageCommand = @"
`$ErrorActionPreference = 'Stop'
`$ProgressPreference = 'SilentlyContinue'
New-Item -ItemType Directory -Force -Path $(Quote-PSLiteral -Value $remoteStageDir) | Out-Null
"@

Invoke-RemotePowerShell -Command $createStageCommand -OutFile (Join-Path $run.Path 'create-stage.stdout.txt') -ErrFile (Join-Path $run.Path 'create-stage.stderr.txt') -AskPass $askPass -KnownHosts $KnownHostsPath | Out-Null

$remoteFiles = Write-RemoteDeployFiles -RunDir $run.Path -StageDir $remoteStageDir -Artifacts $artifacts -DesktopLink $RemoteDesktopLink -Stamp $run.Stamp

Copy-ToRemote -LocalPath $artifacts.ExePath -RemotePath (Join-Path $remoteStageDir 'codex_switch.exe') -OutFile (Join-Path $run.Path 'copy-exe.stdout.txt') -ErrFile (Join-Path $run.Path 'copy-exe.stderr.txt') -AskPass $askPass -KnownHosts $KnownHostsPath
Copy-ToRemote -LocalPath $artifacts.LoaderPath -RemotePath (Join-Path $remoteStageDir 'WebView2Loader.dll') -OutFile (Join-Path $run.Path 'copy-loader.stdout.txt') -ErrFile (Join-Path $run.Path 'copy-loader.stderr.txt') -AskPass $askPass -KnownHosts $KnownHostsPath
Copy-ToRemote -LocalPath $remoteFiles.ConfigPath -RemotePath (Join-Path $remoteStageDir 'remote-deploy-config.json') -OutFile (Join-Path $run.Path 'copy-config.stdout.txt') -ErrFile (Join-Path $run.Path 'copy-config.stderr.txt') -AskPass $askPass -KnownHosts $KnownHostsPath
Copy-ToRemote -LocalPath $remoteFiles.ScriptPath -RemotePath (Join-Path $remoteStageDir 'remote-deploy-codex-switch.ps1') -OutFile (Join-Path $run.Path 'copy-script.stdout.txt') -ErrFile (Join-Path $run.Path 'copy-script.stderr.txt') -AskPass $askPass -KnownHosts $KnownHostsPath

$deployCommand = "& $(Quote-PSLiteral -Value (Join-Path $remoteStageDir 'remote-deploy-codex-switch.ps1'))"
$deployResultPath = Join-Path $run.Path 'deploy-result.json'
$deployErrPath = Join-Path $run.Path 'deploy-result.stderr.txt'
$deployResult = Invoke-RemotePowerShell -Command $deployCommand -OutFile $deployResultPath -ErrFile $deployErrPath -AskPass $askPass -KnownHosts $KnownHostsPath

$inspectAfterPath = Join-Path $run.Path 'inspect-after.json'
$inspectAfterErrPath = Join-Path $run.Path 'inspect-after.stderr.txt'
$inspectAfter = Invoke-RemotePowerShell -Command $inspectCommand -OutFile $inspectAfterPath -ErrFile $inspectAfterErrPath -AskPass $askPass -KnownHosts $KnownHostsPath

[pscustomobject]@{
    mode = 'deploy'
    evidenceDir = $run.Path
    remoteTarget = $RemoteTarget
    localExe = $artifacts.ExePath
    localLoader = $artifacts.LoaderPath
    localExeHash = $artifacts.ExeHash
    localLoaderHash = $artifacts.LoaderHash
    inspectBefore = ($inspectBefore | ConvertFrom-Json)
    deploy = ($deployResult | ConvertFrom-Json)
    inspectAfter = ($inspectAfter | ConvertFrom-Json)
} | ConvertTo-Json -Depth 10
