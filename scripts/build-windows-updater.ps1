$ErrorActionPreference = "Stop"

$RootDir = Resolve-Path (Join-Path $PSScriptRoot "..")
$GuiDir = Join-Path $RootDir "deecodex-gui"
$KeyPath = if ($env:TAURI_SIGNING_PRIVATE_KEY_PATH) {
    $env:TAURI_SIGNING_PRIVATE_KEY_PATH
} else {
    Join-Path $env:USERPROFILE ".tauri\dex-ai-windows-updater.key"
}
$PasswordPath = Join-Path $env:USERPROFILE ".tauri\dex-ai-windows-updater.key.password"

if (-not $env:TAURI_SIGNING_PRIVATE_KEY -and -not (Test-Path $KeyPath)) {
    throw "missing updater private key: $KeyPath"
}

if (-not $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD) {
    if (-not (Test-Path $PasswordPath)) {
        throw "missing updater private key password: $PasswordPath"
    }
    $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = (Get-Content $PasswordPath -Raw).Trim()
}

if (-not $env:TAURI_SIGNING_PRIVATE_KEY) {
    $env:TAURI_SIGNING_PRIVATE_KEY_PATH = $KeyPath
    $env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content $KeyPath -Raw).Trim()
}

Push-Location $GuiDir
try {
    cargo tauri build --bundles nsis --config tauri.windows.conf.json
} finally {
    Pop-Location
}

$BundleDir = Join-Path $RootDir "target-local\release\bundle\nsis"
$Installer = Get-ChildItem $BundleDir -Filter "*setup.exe" | Sort-Object LastWriteTime | Select-Object -Last 1
if (-not $Installer) {
    throw "missing NSIS installer under $BundleDir"
}

$Signature = "$($Installer.FullName).sig"
if (-not (Test-Path $Signature)) {
    throw "missing updater signature: $Signature"
}

Write-Host "Built Windows updater artifacts:"
Write-Host "  $($Installer.FullName)"
Write-Host "  $Signature"
