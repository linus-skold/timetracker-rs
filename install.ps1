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
