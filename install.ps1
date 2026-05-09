# deecodex Windows 一键安装向导
# 用法: irm https://raw.githubusercontent.com/liguan-89/deecodex/main/install.ps1 | iex

$ErrorActionPreference = "Stop"
$Host.UI.RawUI.WindowTitle = "deecodex 安装向导"

# ===== 全局配置 =====
$Repo = "liguan-89/deecodex"
$InstallDir = "$env:LOCALAPPDATA\Programs\deecodex"
$FallbackVersion = "v1.0.0"
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
        $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -TimeoutSec 5
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
DEECODEX_MODEL_MAP={"GPT-5.5":"deepseek-v4-pro","gpt-5.5":"deepseek-v4-pro","gpt-5.4":"deepseek-v4-flash","gpt-5.4-mini":"deepseek-v4-flash","codex-auto-review":"deepseek-v4-flash"}

# 日志级别
RUST_LOG=deecodex=info
'@
    Set-Content -Path $EnvFile -Value $EnvTemplate -Encoding UTF8
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
    Set-Content -Path $EnvFile -Value $content -Encoding UTF8 -NoNewline
}

Write-Host ""

# ===== Phase 4: 安装 deecodex =====
Write-Step 4 "安装 deecodex"

Write-Host "       获取最新版本..."
$Tag = Get-ReleaseTag
Write-Host "       版本: $Tag"

$ProgressPreference = 'SilentlyContinue'
$ReleaseUrl = "https://github.com/$Repo/releases/download/$Tag"

# 下载二进制
Write-Host "       下载 deecodex.exe..."
try {
    Invoke-WebRequest -Uri "$ReleaseUrl/deecodex.exe" -OutFile "$InstallDir\deecodex.exe"
    Write-Ok "deecodex.exe → $InstallDir\deecodex.exe"
} catch {
    Write-Err "二进制下载失败: $_"
    if (-not $RustOk) {
        Write-Host "       请确认 Release 中包含 Windows 二进制，或安装 Rust 后从源码编译"
        exit 1
    }
}

# 下载管理脚本
Write-Host "       下载管理脚本..."
try {
    Invoke-WebRequest -Uri "$ReleaseUrl/deecodex.bat" -OutFile "$InstallDir\deecodex.bat"
    Write-Ok "deecodex.bat → $InstallDir\deecodex.bat"
} catch {
    Write-Warn "管理脚本下载失败"
}

# 添加到用户 PATH
Write-Host "       配置 PATH..."
$userPath = [Environment]::GetEnvironmentVariable("Path", "User"); if (-not $userPath) { $userPath = "" }
$paths = $userPath -split ";" | Where-Object { $_ }
if ($paths -notcontains $InstallDir) {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$InstallDir", "User")
    $env:Path = "$env:Path;$InstallDir"
    Write-Ok "已添加 $InstallDir 到用户 PATH"
} else {
    Write-Ok "PATH 已包含安装目录"
}

Write-Host ""

# ===== Phase 5: 启动服务 =====
Write-Step 5 "启动服务"

$Started = $false

if (Confirm-Prompt "是否现在启动 deecodex？") {
    # 检测端口是否被占用
    $portInUse = Get-NetTCPConnection -LocalPort $Port -ErrorAction SilentlyContinue
    if ($portInUse) {
        Write-Warn "端口 $Port 已被占用"
        if (Confirm-Prompt "是否终止占用进程并继续？") {
            $proc = Get-Process -Id $portInUse.OwningProcess -ErrorAction SilentlyContinue
            if ($proc) { Stop-Process -Id $portInUse.OwningProcess -Force }
            Start-Sleep -Seconds 1
        } else {
            Write-Host "       请修改 $EnvFile 中的 DEECODEX_PORT 后手动启动"
        }
    }

    Write-Host "       启动中..."
    try {
        $proc = Start-Process -FilePath "$InstallDir\deecodex.bat" -ArgumentList "start" -WorkingDirectory $InstallDir -WindowStyle Hidden -PassThru
        Write-Ok "deecodex 已启动"

        # 等待服务就绪
        Write-Host "       等待服务就绪..."
        for ($i = 1; $i -le 15; $i++) {
            try {
                $null = Invoke-WebRequest -Uri "http://127.0.0.1:$Port/api/status" -TimeoutSec 2 -UseBasicParsing
                Write-Ok "服务就绪"
                $Started = $true
                break
            } catch {
                Start-Sleep -Seconds 1
            }
        }

        if (-not $Started) {
            Write-Warn "服务可能启动较慢，请稍后检查"
        }
    } catch {
        Write-Err "启动失败: $_"
    }
} else {
    Write-Ok "跳过启动（可稍后手动启动）"
}

Write-Host ""

# ===== Phase 6: 完成安装 =====
Write-Step 6 "完成安装"

$PanelUrl = "http://127.0.0.1:$Port"

if ($Started) {
    if (Confirm-Prompt "是否打开 Web 配置面板？") {
        Start-Process $PanelUrl
    }
}

Write-Host ""
Write-Host "╔══════════════════════════════════════════╗" -ForegroundColor Green
Write-Host "║         🎉 deecodex 安装完成！            ║" -ForegroundColor Green
Write-Host "╚══════════════════════════════════════════╝" -ForegroundColor Green
Write-Host ""
Write-Host "  管理命令（在 $InstallDir 目录下执行）:" -ForegroundColor White
Write-Host "    deecodex.bat start     启动服务" -ForegroundColor Cyan
Write-Host "    deecodex.bat stop      停止服务" -ForegroundColor Cyan
Write-Host "    deecodex.bat restart   重启服务" -ForegroundColor Cyan
Write-Host "    deecodex.bat status    查看状态" -ForegroundColor Cyan
Write-Host "    deecodex.bat logs      查看日志" -ForegroundColor Cyan
Write-Host "    deecodex.bat health    健康检查" -ForegroundColor Cyan
Write-Host "    deecodex.bat update    一键升级" -ForegroundColor Cyan
Write-Host ""
Write-Host "  配置面板: " -NoNewline -ForegroundColor White
Write-Host $PanelUrl -ForegroundColor Cyan
Write-Host "  配置文件: $EnvFile"
Write-Host "  安装目录: $InstallDir"
Write-Host ""
Write-Host "  提醒：如修改了 .env，需重启服务生效" -ForegroundColor Yellow
Write-Host "        deecodex.bat restart" -ForegroundColor Cyan
Write-Host ""
