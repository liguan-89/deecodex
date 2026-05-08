# deecodex Windows 一键安装脚本
# 用法: irm https://raw.githubusercontent.com/liguan-89/deecodex/main/install.ps1 | iex

$ErrorActionPreference = "Stop"
$Version = "v1.0.0"
$BaseUrl = "https://github.com/liguan-89/deecodex/releases/download/$Version"
$InstallDir = "$env:LOCALAPPDATA\Programs\deecodex"

Write-Host "=== deecodex Windows 安装 ===" -ForegroundColor Cyan
Write-Host "  安装目录: $InstallDir"
Write-Host ""

# 1. 创建安装目录
if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}

# 2. 下载文件
Write-Host "[1/4] 下载 deecodex.exe..." -ForegroundColor Yellow
$ProgressPreference = 'SilentlyContinue'
Invoke-WebRequest -Uri "$BaseUrl/deecodex.exe" -OutFile "$InstallDir\deecodex.exe"
Write-Host "       deecodex.exe ✓"

Write-Host "[2/4] 下载管理脚本..." -ForegroundColor Yellow
Invoke-WebRequest -Uri "$BaseUrl/deecodex.bat" -OutFile "$InstallDir\deecodex.bat"
Write-Host "       deecodex.bat ✓"

Write-Host "[3/4] 下载配置模板..." -ForegroundColor Yellow
Invoke-WebRequest -Uri "$BaseUrl/env.example" -OutFile "$InstallDir\.env.example"
if (-not (Test-Path "$InstallDir\.env")) {
    Copy-Item "$InstallDir\.env.example" "$InstallDir\.env"
    Write-Host "       .env.example → .env ✓"
} else {
    Write-Host "       .env.example ✓ (.env 已存在，跳过覆盖)"
}

# 3. 添加到用户 PATH
Write-Host "[4/4] 配置 PATH..." -ForegroundColor Yellow
$UserPath = [Environment]::GetEnvironmentVariable("Path", "User") ?? ""
$Paths = $UserPath -split ";" | Where-Object { $_ }
if ($Paths -notcontains $InstallDir) {
    [Environment]::SetEnvironmentVariable("Path", "$UserPath;$InstallDir", "User")
    # 同步刷新当前会话 PATH
    $env:Path = "$env:Path;$InstallDir"
    Write-Host "       已添加到用户 PATH ✓"
} else {
    Write-Host "       PATH 已包含安装目录，跳过 ✓"
}

Write-Host ""
Write-Host "安装完成!" -ForegroundColor Green
Write-Host ""
Write-Host "下一步：" -ForegroundColor Cyan
Write-Host "  1. 编辑配置:  notepad $InstallDir\.env"
Write-Host "     填入你的 DeepSeek API Key (DEECODEX_API_KEY)"
Write-Host ""
Write-Host "  2. 启动服务:  deecodex.bat start"
Write-Host "  3. 检查状态:  deecodex.bat health"
Write-Host ""
Write-Host "  ⚠ 如果 deecodex.bat 找不到，请重新打开终端让 PATH 生效"
