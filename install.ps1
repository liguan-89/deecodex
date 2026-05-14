# deecodex Windows 一键安装向导
# 用法: iex (irm https://raw.githubusercontent.com/liguan-89/deecodex/main/install.ps1)

$ErrorActionPreference = "Stop"
$Host.UI.RawUI.WindowTitle = "deecodex 安装向导"

# ===== 全局配置 =====
$Repo = "liguan-89/deecodex"
$InstallDir = "$env:LOCALAPPDATA\Programs\deecodex"
$FallbackVersion = "v1.3.19"
$Port = "4446"

# ===== 辅助函数 =====
function Write-Step {
    param([int]$Num, [string]$Text)
    Write-Host "  [$Num/$TotalSteps] " -NoNewline -ForegroundColor Cyan
    Write-Host $Text -ForegroundColor White
}

function Write-Ok {
    param([string]$Text)
    Write-Host "       " -NoNewline
    Write-Host "✓" -NoNewline -ForegroundColor Green
    Write-Host " $Text"
}

function Write-Warn {
    param([string]$Text)
    Write-Host "       " -NoNewline
    Write-Host "⚠" -NoNewline -ForegroundColor Yellow
    Write-Host " $Text"
}

function Write-Err {
    param([string]$Text)
    Write-Host "       " -NoNewline
    Write-Host "✗" -NoNewline -ForegroundColor Red
    Write-Host " $Text"
}

function Write-Url {
    param([string]$Text)
    Write-Host "         $Text" -ForegroundColor Cyan
}

function Write-Banner {
    Write-Host ""
    Write-Host "  deecodex Windows 一键安装向导  " -ForegroundColor Cyan -BackgroundColor Black
    Write-Host ""
}

function Test-Command {
    param([string]$Cmd)
    $found = Get-Command $Cmd -ErrorAction SilentlyContinue
    if ($found) {
        Write-Ok "$Cmd 已安装"
        return $true
    } else {
        Write-Err "$Cmd 未安装"
        return $false
    }
}

function Get-ReleaseTag {
    try {
        $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -TimeoutSec 15
        return $release.tag_name
    } catch {
        Write-Warn "获取最新版本失败，使用默认版本 $FallbackVersion"
        return $FallbackVersion
    }
}

function Confirm-Prompt {
    param([string]$Prompt)
    $response = Read-Host "       $Prompt [Y/n]"
    if ([string]::IsNullOrEmpty($response)) { return $true }
    return $response -match '^[Yy]'
}

# ===== 开始 =====
$TotalSteps = 6
Write-Banner

# ===== Phase 1: 环境检测 =====
Write-Step 1 "检测安装环境"

$GitOk = Test-Command "git"
if (-not $GitOk) {
    Write-Host "         安装 Git: winget install Git.Git" -ForegroundColor Yellow
    Write-Url "https://git-scm.com/downloads/win"
}

$RustOk = Test-Command "cargo"
if (-not $RustOk) {
    Write-Host "         安装 Rust: winget install Rustlang.Rustup" -ForegroundColor Yellow
    Write-Url "https://rustup.rs"
}

Write-Host ""

# ===== Phase 2: Codex 检测 =====
Write-Step 2 "检测 Codex 安装状态"

# 检测 Codex CLI
$CodexCli = Get-Command codex -ErrorAction SilentlyContinue
if ($CodexCli) {
    Write-Ok "Codex CLI 已安装"
} else {
    Write-Warn "Codex CLI 未安装"
    Write-Host "         安装: npm install -g @anthropic-ai/codex" -ForegroundColor Yellow
    Write-Url "https://github.com/anthropics/codex"
}

# 检测 Codex 桌面版
$DesktopPaths = @(
    "$env:LOCALAPPDATA\Programs\codex\Codex.exe",
    "$env:APPDATA\codex\Codex.exe",
    "${env:ProgramFiles}\Codex\Codex.exe"
)
$DesktopFound = $false
foreach ($p in $DesktopPaths) {
    if (Test-Path $p) {
        Write-Ok "Codex 桌面版: $p"
        $DesktopFound = $true
        break
    }
}
# 检测 Microsoft Store 版本
if (-not $DesktopFound) {
    $storeBase = "C:\Program Files\WindowsApps"
    if (Test-Path $storeBase) {
        $storeDirs = Get-ChildItem $storeBase -Directory -Filter "OpenAI.Codex*" -ErrorAction SilentlyContinue
        if ($storeDirs) {
            Write-Ok "Codex Microsoft Store 版: $($storeDirs[0].FullName)"
            $DesktopFound = $true
        }
    }
}
if (-not $DesktopFound) {
    Write-Warn "Codex 桌面版未安装"
    Write-Url "https://github.com/anthropics/codex/releases"
}

Write-Host ""

# ===== Phase 3: 配置 .env =====
Write-Step 3 "配置环境变量"

if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}

$EnvFile = Join-Path $InstallDir ".env"
$SkipEnv = $false

if (Test-Path $EnvFile) {
    Write-Host "       " -NoNewline
    Write-Warn ".env 已存在于 $EnvFile"
    Write-Host "       [K] 保留现有配置    [O] 覆盖为新模板    [U] 仅更新 API Key"
    $choice = Read-Host "       请选择 [K/o/u]"
    if ([string]::IsNullOrEmpty($choice)) { $choice = "K" }
    switch -Regex ($choice) {
        "[Oo]" { Remove-Item $EnvFile -Force }
        "[Uu]" { $SkipEnv = $true }
        default { $SkipEnv = $true }
    }
}

if ($SkipEnv) {
    Write-Ok "保留现有 .env"
} else {
    $EnvTemplate = @'
# deecodex 环境变量配置
# DeepSeek API 地址
DEECODEX_UPSTREAM=https://api.deepseek.com/v1

# DeepSeek API Key（必填）
# 登录 https://platform.deepseek.com → API Keys 获取
DEECODEX_API_KEY=sk-your-deepseek-api-key-here

# 本地客户端访问 deecodex 的 Bearer Token
# 留空可关闭本地鉴权
DEECODEX_CLIENT_API_KEY=

# 监听端口
DEECODEX_PORT=4446

# 模型名映射（JSON 格式）
DEECODEX_MODEL_MAP={"GPT-5.5":"deepseek-v4-pro","gpt-5.5":"deepseek-v4-pro","gpt-5.4":"deepseek-v4-flash","gpt-5.4-mini":"deepseek-v4-flash","gpt-5.3-codex":"deepseek-v4-pro","codex-auto-review":"deepseek-v4-flash"}

# 日志级别
RUST_LOG=deecodex=info
'@
    $utf8NoBom = New-Object System.Text.UTF8Encoding $false
    [System.IO.File]::WriteAllText($EnvFile, $EnvTemplate, $utf8NoBom)
    Write-Ok "配置模板已写入 $EnvFile"
}

# 交互式引导填写 API Key
Write-Host ""
Write-Host "  请输入你的 DeepSeek API Key" -ForegroundColor White
Write-Host "  （从 https://platform.deepseek.com → API Keys 获取）" -ForegroundColor Yellow
Write-Host "  不填写将导致服务启动后无法正常工作！" -ForegroundColor Yellow
Write-Host ""
$ApiKey = Read-Host "  API Key"

if ([string]::IsNullOrEmpty($ApiKey) -or $ApiKey -eq "sk-your-deepseek-api-key-here") {
    Write-Host ""
    Write-Host "  ╔══════════════════════════════════════════╗" -ForegroundColor Red
    Write-Host "  ║  ⚠ 警告：未填写 API Key                  ║" -ForegroundColor Red
    Write-Host "  ║  服务启动后将无法正常调用 LLM 接口        ║" -ForegroundColor Red
    Write-Host "  ║  你可以在安装完成后编辑 .env 手动填入     ║" -ForegroundColor Red
    Write-Host "  ╚══════════════════════════════════════════╝" -ForegroundColor Red
    Write-Host ""
    $confirm = Read-Host "       确认跳过 API Key 配置？[y/N]"
    if ($confirm -notmatch '^[Yy]') {
        $ApiKey = Read-Host "  请重新输入 API Key"
        if (-not [string]::IsNullOrEmpty($ApiKey) -and $ApiKey -ne "sk-your-deepseek-api-key-here") {
            Write-Ok "API Key 已记录"
        } else {
            Write-Warn "仍然为空，稍后可编辑 $EnvFile 手动填入"
        }
    }
} else {
    Write-Ok "API Key 已记录"
}

# 写入 API Key
if (-not [string]::IsNullOrEmpty($ApiKey) -and $ApiKey -ne "sk-your-deepseek-api-key-here") {
    $content = Get-Content $EnvFile -Raw -Encoding UTF8
    $content = $content -replace 'DEECODEX_API_KEY=.*', "DEECODEX_API_KEY=$ApiKey"
    $utf8NoBom = New-Object System.Text.UTF8Encoding $false
    [System.IO.File]::WriteAllText($EnvFile, $content, $utf8NoBom)
}

Write-Host ""

# ===== Phase 4: 安装 deecodex =====
Write-Step 4 "安装 deecodex"

Write-Host "       获取最新版本..."
$Tag = Get-ReleaseTag
Write-Host "       版本: $Tag"

$ProgressPreference = 'SilentlyContinue'
$ReleaseUrl = "https://github.com/$Repo/releases/download/$Tag"
$TagNoV = $Tag -replace '^v', ''
$SetupExe = "deecodex-$TagNoV-setup.exe"
$SetupPath = "$env:TEMP\$SetupExe"

# 下载一键安装包
Write-Host "       下载一键安装包..."
try {
    Invoke-WebRequest -Uri "$ReleaseUrl/$SetupExe" -OutFile $SetupPath
    Write-Ok "$SetupExe 下载完成"
} catch {
    Write-Err "下载失败: $_"
    Write-Host "       请检查网络连接或访问 $ReleaseUrl 手动下载"
    exit 1
}

# 静默安装
Write-Host "       正在安装..."
$installArgs = "/S /D=$InstallDir"
try {
    $proc = Start-Process -FilePath $SetupPath -ArgumentList $installArgs -Wait -PassThru
    if ($proc.ExitCode -eq 0) {
        Write-Ok "安装完成 → $InstallDir"
    } else {
        Write-Warn "安装程序退出码: $($proc.ExitCode)，请尝试手动运行 $SetupPath"
    }
} catch {
    Write-Err "安装失败: $_"
    exit 1
}
Remove-Item $SetupPath -Force -ErrorAction SilentlyContinue

Write-Host ""

# ===== Phase 5: 启动服务 =====
Write-Step 5 "启动服务"

$Started = $false

if (Confirm-Prompt "是否现在启动 deecodex？") {
    Write-Host "       启动中..."
    try {
        Start-Process -FilePath "$InstallDir\deecodex-gui.exe" -WorkingDirectory $InstallDir
        Write-Ok "deecodex 已启动（系统托盘）"
        $Started = $true
    } catch {
        Write-Err "启动失败: $_"
    }
} else {
    Write-Ok "跳过启动（可稍后从开始菜单或桌面快捷方式启动）"
}

Write-Host ""

# ===== Phase 6: 完成安装 =====
Write-Step 6 "完成安装"

Write-Host ""
Write-Host "╔══════════════════════════════════════════╗" -ForegroundColor Green
Write-Host "║         🎉 deecodex 安装完成！            ║" -ForegroundColor Green
Write-Host "╚══════════════════════════════════════════╝" -ForegroundColor Green
Write-Host ""
Write-Host "  桌面应用：开始菜单 → deecodex 控制台" -ForegroundColor White
Write-Host "           （系统托盘图标 + Web 控制面板）" -ForegroundColor Cyan
Write-Host ""
Write-Host "  命令行管理：" -ForegroundColor White
Write-Host "    deecodex.bat start     启动服务" -ForegroundColor Cyan
Write-Host "    deecodex.bat stop      停止服务" -ForegroundColor Cyan
Write-Host "    deecodex.bat restart   重启服务" -ForegroundColor Cyan
Write-Host "    deecodex.bat status    查看状态" -ForegroundColor Cyan
Write-Host "    deecodex.bat logs      查看日志" -ForegroundColor Cyan
Write-Host "    deecodex.bat update    一键升级" -ForegroundColor Cyan
Write-Host ""
Write-Host "  配置文件: $EnvFile"
Write-Host "  安装目录: $InstallDir"
Write-Host ""
Write-Host "  修改 .env 后需重启服务生效" -ForegroundColor Yellow
Write-Host ""
