# build-gui-release.ps1 — Windows 版 DEX AI GUI release 构建脚本
# 等价于 macOS 的 scripts/build-gui-release.sh：
#   1) 按 deecodex-gui/build.rs 的同一算法计算第一方插件包 SHA256 并注入环境变量；
#   2) 先构建 core 二进制（deecodex），再构建 GUI（deecodex-gui）。
# 用法: .\scripts\build-gui-release.ps1 [cargo build 额外参数...]
$ErrorActionPreference = "Stop"

$RootDir = Resolve-Path (Join-Path $PSScriptRoot "..")

function Get-PackageHash {
    param([string]$PackageDir)
    # 与 deecodex-gui/build.rs 的 hash_dir 逐字一致：
    #   - 每层目录 read_dir 后按完整路径做 ordinal(字节序)排序（Windows 为反斜杠分隔符）
    #   - 跳过 .git / .DS_Store；目录追加 "/" 后递归，文件追加 "\0" 后写入内容(16KB 分块)
    $sha = [System.Security.Cryptography.SHA256]::Create()
    $buf = [byte[]]::new(16384)

    function Hash-Dir {
        param([string]$Current)
        $entries = @(Get-ChildItem -LiteralPath $Current -Force -ErrorAction Stop)
        $names = [string[]]@($entries | ForEach-Object { $_.FullName })
        [Array]::Sort($names, [System.StringComparer]::Ordinal)
        $byName = @{}
        foreach ($e in $entries) { $byName[$e.FullName] = $e }
        foreach ($n in $names) {
            $e = $byName[$n]
            if ($e.Name -eq ".git" -or $e.Name -eq ".DS_Store") { continue }
            $rel = $e.FullName.Substring($PackageDir.Length).TrimStart('\')
            $relBytes = [System.Text.Encoding]::UTF8.GetBytes($rel)
            [void]$sha.TransformBlock($relBytes, 0, $relBytes.Length, $null, 0)
            if ($e.PSIsContainer) {
                [void]$sha.TransformBlock([byte[]](0x2F), 0, 1, $null, 0)
                Hash-Dir $e.FullName
            } else {
                [void]$sha.TransformBlock([byte[]](0), 0, 1, $null, 0)
                $fs = [System.IO.File]::OpenRead($e.FullName)
                try {
                    while (($read = $fs.Read($buf, 0, $buf.Length)) -gt 0) {
                        [void]$sha.TransformBlock($buf, 0, $read, $null, 0)
                    }
                } finally { $fs.Close() }
            }
        }
    }

    Hash-Dir $PackageDir
    [void]$sha.TransformFinalBlock([byte[]]::new(0), 0, 0)
    $hex = ""
    foreach ($b in $sha.Hash) { $hex += $b.ToString("x2") }
    $sha.Dispose()
    return $hex
}

$Mobile = Join-Path $RootDir "deecodex-plugins\plugins\deecodex-mobile-codex"
$Sync   = Join-Path $RootDir "deecodex-plugins\plugins\deecodex-desktop-sync"
if (-not (Test-Path $Mobile)) { throw "插件目录不存在: $Mobile" }
if (-not (Test-Path $Sync))   { throw "插件目录不存在: $Sync" }

$env:DEECODEX_MOBILE_CODEX_PLUGIN_SHA256 = Get-PackageHash -PackageDir $Mobile
$env:DEECODEX_DESKTOP_SYNC_PLUGIN_SHA256 = Get-PackageHash -PackageDir $Sync

Write-Host "DEECODEX_MOBILE_CODEX_PLUGIN_SHA256=$($env:DEECODEX_MOBILE_CODEX_PLUGIN_SHA256)"
Write-Host "DEECODEX_DESKTOP_SYNC_PLUGIN_SHA256=$($env:DEECODEX_DESKTOP_SYNC_PLUGIN_SHA256)"

Push-Location $RootDir
try {
    Write-Host "==> cargo build -p deecodex --release"
    cargo build -p deecodex --release
    if ($LASTEXITCODE -ne 0) { throw "core release 构建失败 (exit=$LASTEXITCODE)" }
    Write-Host "==> cargo build -p deecodex-gui --release"
    cargo build -p deecodex-gui --release @args
    if ($LASTEXITCODE -ne 0) { throw "GUI release 构建失败 (exit=$LASTEXITCODE)" }
} finally {
    Pop-Location
}
Write-Host "release 构建完成: target-local\release\deecodex-gui.exe"
