param(
  [string]$ProjectDir = "",
  [string]$KeyPath = "$env:USERPROFILE\.tauri\dex-ai-updater.key",
  [string]$PasswordPath = "$env:USERPROFILE\.tauri\dex-ai-updater.key.password"
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($ProjectDir)) {
  $ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
  $ProjectDir = Join-Path $ScriptDir "..\deecodex-gui"
}

if (!(Test-Path $KeyPath)) {
  throw "Missing updater signing key: $KeyPath"
}

if (!(Test-Path $PasswordPath)) {
  throw "Missing updater signing key password: $PasswordPath"
}

$env:TAURI_SIGNING_PRIVATE_KEY_PATH = $KeyPath
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = (Get-Content $PasswordPath -Raw).Trim()

Push-Location $ProjectDir
try {
  cargo tauri build
} finally {
  Pop-Location
}
