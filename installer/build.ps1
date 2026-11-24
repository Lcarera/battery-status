# Builds the release exe and the installer.
# Output: installer\Output\battery-status-setup-<version>.exe
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

Push-Location $root
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

    $meta = cargo metadata --format-version 1 --no-deps | ConvertFrom-Json
    $version = ($meta.packages | Where-Object { $_.name -eq 'battery-status' }).version
    $exeDir = Join-Path $meta.target_directory 'release'
} finally {
    Pop-Location
}

$iscc = (Get-Command iscc -ErrorAction SilentlyContinue).Source
if (-not $iscc) {
    $candidates = @(
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
    )
    $iscc = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $iscc) { throw "ISCC.exe not found. Install it: winget install JRSoftware.InnoSetup" }

& $iscc "/DAppVersion=$version" "/DExeDir=$exeDir" (Join-Path $PSScriptRoot 'battery-status.iss')
if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }
