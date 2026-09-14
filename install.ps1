#!/usr/bin/env pwsh
# Installs the latest tt (timetracker-rs) release for Windows.
#
#   irm https://raw.githubusercontent.com/linus-skold/timetracker-rs/main/install.ps1 | iex
#
# Override the install directory with $env:TT_INSTALL_DIR (defaults to
# %LOCALAPPDATA%\Programs\tt\bin, created if missing).

$ErrorActionPreference = "Stop"

$Repo = "linus-skold/timetracker-rs"
$InstallDir = if ($env:TT_INSTALL_DIR) { $env:TT_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "Programs\tt\bin" }

$arch = $env:PROCESSOR_ARCHITECTURE
if ($arch -ne "AMD64") {
    Write-Error "Unsupported architecture: $arch (only x86_64/AMD64 has a prebuilt binary)"
    exit 1
}

$asset = "tt-x86_64-pc-windows-msvc.exe"
$url = "https://github.com/$Repo/releases/latest/download/$asset"

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$dest = Join-Path $InstallDir "tt.exe"

Write-Host "Downloading tt for x86_64-pc-windows-msvc..."
Invoke-WebRequest -Uri $url -OutFile $dest -UseBasicParsing

Write-Host "Installed tt to $dest"

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
$pathEntries = $userPath -split ";"
if ($pathEntries -notcontains $InstallDir) {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$InstallDir", "User")
    $env:Path = "$env:Path;$InstallDir"
    Write-Host "Added $InstallDir to your user PATH. Restart your terminal for it to take effect in new shells."
} else {
    Write-Host "$InstallDir is already on your PATH."
}

# Mirrors the per-shell hint table in src/commands.rs (`completions`).
$previousErrorAction = $ErrorActionPreference
$ErrorActionPreference = "Continue"
& $dest completions --help *> $null
$ErrorActionPreference = $previousErrorAction
if ($LASTEXITCODE -eq 0) {
    Write-Host ""
    Write-Host "Shell completion is available. To enable it, run:"
    Write-Host "  Add-Content -Path `$PROFILE -Value 'tt completions powershell | Out-String | Invoke-Expression'"
    Write-Host "(`$PROFILE may not exist yet; Add-Content creates it.)"
}

& $dest --version

# The agent skill and Claude Code's hooks for it. `tt` carries the skill files
# itself, so this needs no network and no `npx skills`.
#
# Set $env:TT_INSTALL_SKILL to 1 to install without being asked, or 0 to skip.
function Test-SkillWanted {
    switch ($env:TT_INSTALL_SKILL) {
        { $_ -in @("0", "n", "no") } { return $false }
        { $_ -in @("1", "y", "yes") } { return $true }
    }
    if (-not [Environment]::UserInteractive) { return $false }
    $reply = Read-Host "`nInstall the tt-time-logging skill for your coding agents? [Y/n]"
    return $reply -notmatch '^\s*[Nn]'
}

# A release older than the subcommand would fail with clap's usage error, which
# reads like a broken install rather than an old one.
$ErrorActionPreference = "Continue"
& $dest skill install --help *> $null
$understandsSkill = $LASTEXITCODE -eq 0
# Left on Continue on purpose: a non-zero exit from the install below is
# reported, not turned into a terminating error on the way out.

if (-not $understandsSkill) {
    Write-Host ""
    Write-Host "This tt is older than ``tt skill install``. Upgrade with ``tt update``,"
    Write-Host "then run ``tt skill install`` to set up the agent skill."
} elseif (Test-SkillWanted) {
    Write-Host ""
    & $dest skill install
} else {
    Write-Host ""
    Write-Host "To teach your coding agent the tt workflow later, run:"
    Write-Host "  tt skill install"
}
