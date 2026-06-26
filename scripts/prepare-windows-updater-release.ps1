$ErrorActionPreference = "Stop"

$RootDir = Resolve-Path (Join-Path $PSScriptRoot "..")
$VersionFile = Join-Path $RootDir "VERSION"
$Version = if ($args.Count -gt 0 -and $args[0]) {
    $args[0]
} else {
    (Get-Content $VersionFile -Raw).Trim()
}
$Version = $Version.TrimStart("v")

$BaseUrl = if ($env:DEX_AI_UPDATE_BASE_URL) {
    $env:DEX_AI_UPDATE_BASE_URL.TrimEnd("/")
} else {
    "https://api.liguan.me/releases/dex-ai/windows"
}
$OutDir = if ($env:DEX_AI_UPDATE_OUT_DIR) {
    $env:DEX_AI_UPDATE_OUT_DIR
} else {
    Join-Path $RootDir "dist\updater-release\windows\$Version"
}
$BundleDir = if ($env:DEX_AI_UPDATE_WINDOWS_BUNDLE_DIR) {
    $env:DEX_AI_UPDATE_WINDOWS_BUNDLE_DIR
} else {
    Join-Path $RootDir "target-local\release\bundle\nsis"
}
$DefaultNotesFile = Join-Path $RootDir "docs\releases\$Version.md"
$Notes = if ($env:DEX_AI_UPDATE_NOTES) {
    $env:DEX_AI_UPDATE_NOTES
} else {
    ""
}
if ($env:DEX_AI_UPDATE_NOTES_FILE) {
    $Notes = Get-Content $env:DEX_AI_UPDATE_NOTES_FILE -Raw -Encoding UTF8
} elseif (Test-Path $DefaultNotesFile) {
    $Notes = Get-Content $DefaultNotesFile -Raw -Encoding UTF8
}

New-Item -ItemType Directory -Force $OutDir | Out-Null

$Installer = Get-ChildItem $BundleDir -Recurse -Filter "*setup.exe" |
    Sort-Object LastWriteTime |
    Select-Object -Last 1
if (-not $Installer) {
    throw "missing Windows installer under $BundleDir"
}

$Signature = "$($Installer.FullName).sig"
if (-not (Test-Path $Signature)) {
    throw "missing updater signature: $Signature"
}

Copy-Item $Installer.FullName -Destination (Join-Path $OutDir $Installer.Name) -Force
Copy-Item $Signature -Destination (Join-Path $OutDir "$($Installer.Name).sig") -Force

$SignatureContent = (Get-Content $Signature -Raw).Trim()
$EncodedInstaller = [System.Uri]::EscapeDataString($Installer.Name)
$Manifest = [ordered]@{
    version = $Version
    notes = $Notes.Trim()
    pub_date = $null
    platforms = [ordered]@{
        "windows-x86_64" = [ordered]@{
            signature = $SignatureContent
            url = "$BaseUrl/$Version/$EncodedInstaller"
        }
    }
}

$ManifestPath = Join-Path $OutDir "latest.json"
$ManifestJson = $Manifest | ConvertTo-Json -Depth 8
$Utf8NoBom = [System.Text.UTF8Encoding]::new($false)
[System.IO.File]::WriteAllText($ManifestPath, $ManifestJson + [Environment]::NewLine, $Utf8NoBom)

Write-Host "Prepared Windows updater release:"
Write-Host "  $OutDir"
Write-Host "Upload target:"
Write-Host "  $BaseUrl/$Version/$EncodedInstaller"
Write-Host "  $BaseUrl/latest.json"
