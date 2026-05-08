#!/usr/bin/env bash
# deecodex macOS 一键安装向导
# 用法: curl -fsSL https://raw.githubusercontent.com/liguan-89/deecodex/main/install.sh | bash
set -euo pipefail

# ===== 颜色常量 =====
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

# ===== 全局配置 =====
GH_REPO="liguan-89/deecodex"
BIN_DIR="$HOME/.local/bin"
CONFIG_DIR="$HOME/.deecodex"
PORT="4446"
FALLBACK_VERSION="v1.0.0"

# ===== 工具函数 =====
print_header() {
    echo ""
    echo -e "${CYAN}╔══════════════════════════════════════════╗${NC}"
    echo -e "${CYAN}║${NC}     ${BOLD}deecodex macOS 一键安装向导${NC}       ${CYAN}║${NC}"
    echo -e "${CYAN}╚══════════════════════════════════════════╝${NC}"
    echo ""
}

print_step() {
    echo -e "  ${CYAN}[$1/$TOTAL_STEPS]${NC} ${BOLD}$2${NC}"
}

print_ok() {
    echo -e "       ${GREEN}✓${NC} $1"
}

print_warn() {
    echo -e "       ${YELLOW}⚠${NC} $1"
}

print_err() {
    echo -e "       ${RED}✗${NC} $1"
}

print_url() {
    echo -e "         ${CYAN}$1${NC}"
}

check_cmd() {
    if command -v "$1" &>/dev/null; then
        print_ok "$1 已安装"
        return 0
    else
        print_err "$1 未安装"
        return 1
    fi
}

confirm() {
    local prompt="$1"
    local default="${2:-Y}"
    local yn
    read -r -p "       ${prompt} [Y/n]: " yn
    yn="${yn:-$default}"
    [[ "$yn" =~ ^[Yy]$ ]]
}

get_latest_tag() {
    local tag
    tag=$(curl -s "https://api.github.com/repos/$GH_REPO/releases/latest" 2>/dev/null | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')
    if [ -z "$tag" ]; then
        echo "$FALLBACK_VERSION"
    else
        echo "$tag"
    fi
}

# ===== Phase 0: 欢迎 =====
print_header

TOTAL_STEPS=6

# ===== Phase 1: 环境检测 =====
print_step 1 "检测安装环境"

MISSING_DEPS=()
BINARY_ONLY=false

if check_cmd "git"; then
    GIT_OK=true
else
    GIT_OK=false
    MISSING_DEPS+=("git")
    echo -e "         ${YELLOW}安装 Git: brew install git${NC}"
    echo -e "         ${YELLOW}或访问: https://git-scm.com/downloads/mac${NC}"
fi

if check_cmd "cargo"; then
    RUST_OK=true
    if rustc --version 2>/dev/null | grep -q '1\.[89]'; then
        print_warn "Rust 版本可能过旧，建议 1.80+"
    fi
else
    RUST_OK=false
    BINARY_ONLY=true
    echo -e "         ${YELLOW}安装 Rust: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh${NC}"
fi

if ! command -v curl &>/dev/null; then
    print_err "curl 未安装 —— 无法继续"
    echo "         请先安装 curl: brew install curl"
    exit 1
fi

# 检查 ~/.local/bin 是否在 PATH 中
if ! echo "$PATH" | tr ':' '\n' | grep -qxF "$BIN_DIR"; then
    print_warn "~/.local/bin 不在 PATH 中，将自动添加"
    NEED_PATH_FIX=true
else
    NEED_PATH_FIX=false
fi

echo ""

# ===== Phase 2: Codex 检测 =====
print_step 2 "检测 Codex 安装状态"
CODEX_CLI_OK=false
CODEX_DESKTOP_OK=false

# 检测 Codex CLI
if command -v codex &>/dev/null; then
    print_ok "Codex CLI 已安装"
    CODEX_CLI_OK=true
else
    print_warn "Codex CLI 未安装"
    echo -e "         安装: npm install -g @anthropic-ai/codex"
    print_url "https://github.com/anthropics/codex"
fi

# 检测 Codex 桌面版
CODEX_DESKTOP_PATHS=(
    "/Applications/Codex.app"
    "$HOME/Applications/Codex.app"
)
DESKTOP_FOUND=false
for p in "${CODEX_DESKTOP_PATHS[@]}"; do
    if [ -d "$p" ]; then
        print_ok "Codex 桌面版: $p"
        CODEX_DESKTOP_OK=true
        DESKTOP_FOUND=true
        break
    fi
done
if [ "$DESKTOP_FOUND" = false ]; then
    print_warn "Codex 桌面版未安装"
    print_url "https://github.com/anthropics/codex/releases"
fi

echo ""

# ===== Phase 3: 配置 .env =====
print_step 3 "配置环境变量"

mkdir -p "$CONFIG_DIR"

ENV_FILE="$CONFIG_DIR/.env"
SKIP_ENV=false

if [ -f "$ENV_FILE" ]; then
    echo -e "       ${YELLOW}.env 已存在于 $ENV_FILE${NC}"
    echo "       [K] 保留现有配置    [O] 覆盖为新模板    [U] 仅更新 API Key"
    read -r -p "       请选择 [K/o/u]: " env_choice
    env_choice="${env_choice:-K}"
    case "$env_choice" in
        [Oo]) rm -f "$ENV_FILE" ;;
        [Uu])
            SKIP_ENV=true
            ;;
        *) SKIP_ENV=true ;;
    esac
fi

if [ "$SKIP_ENV" = true ]; then
    print_ok "保留现有 .env"
    # 仍然检查 API Key
    if grep -q 'DEECODEX_API_KEY=sk-your-deepseek-api-key-here' "$ENV_FILE" 2>/dev/null || \
       ! grep -q 'DEECODEX_API_KEY=sk-' "$ENV_FILE" 2>/dev/null; then
        print_warn "检测到 API Key 可能未正确配置"
    fi
else
    # 写入 .env 模板
    cat > "$ENV_FILE" << 'ENVEOF'
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
DEECODEX_MODEL_MAP='{"GPT-5.5":"deepseek-v4-pro","gpt-5.5":"deepseek-v4-pro","gpt-5.4":"deepseek-v4-flash","gpt-5.4-mini":"deepseek-v4-flash","codex-auto-review":"deepseek-v4-flash"}'

# 日志级别
RUST_LOG=deecodex=info
ENVEOF
    print_ok "配置模板已写入 $ENV_FILE"
fi

# 交互式引导填写 API Key
echo ""
echo -e "  ${BOLD}请输入你的 DeepSeek API Key${NC}"
echo -e "  ${YELLOW}（从 https://platform.deepseek.com → API Keys 获取）${NC}"
echo -e "  ${YELLOW}不填写将导致服务启动后无法正常工作！${NC}"
echo ""
read -r -p "  API Key: " user_api_key

if [ -z "$user_api_key" ] || [ "$user_api_key" = "sk-your-deepseek-api-key-here" ]; then
    echo ""
    echo -e "  ${RED}╔══════════════════════════════════════════╗${NC}"
    echo -e "  ${RED}║  ⚠ 警告：未填写 API Key                  ║${NC}"
    echo -e "  ${RED}║  服务启动后将无法正常调用 LLM 接口        ║${NC}"
    echo -e "  ${RED}║  你可以在安装完成后编辑 .env 手动填入     ║${NC}"
    echo -e "  ${RED}╚══════════════════════════════════════════╝${NC}"
    echo ""
    if ! confirm "确认跳过 API Key 配置？（可稍后手动填入）"; then
        echo ""
        read -r -p "  请重新输入 API Key: " user_api_key
        if [ -n "$user_api_key" ] && [ "$user_api_key" != "sk-your-deepseek-api-key-here" ]; then
            print_ok "API Key 已记录"
        else
            print_warn "仍然为空，稍后可编辑 $ENV_FILE 手动填入"
        fi
    fi
else
    print_ok "API Key 已记录"
fi

# 写入 API Key
if [ -n "$user_api_key" ] && [ "$user_api_key" != "sk-your-deepseek-api-key-here" ]; then
    if [[ "$OSTYPE" == "darwin"* ]]; then
        sed -i '' "s|DEECODEX_API_KEY=.*|DEECODEX_API_KEY=$user_api_key|" "$ENV_FILE"
    else
        sed -i "s|DEECODEX_API_KEY=.*|DEECODEX_API_KEY=$user_api_key|" "$ENV_FILE"
    fi
fi

echo ""

# ===== Phase 4: 安装 deecodex =====
print_step 4 "安装 deecodex"

mkdir -p "$BIN_DIR"

echo "       获取最新版本..."
RELEASE_TAG=$(get_latest_tag)
echo "       版本: $RELEASE_TAG"

RELEASE_URL="https://github.com/$GH_REPO/releases/download/$RELEASE_TAG"

# 下载二进制
echo "       下载 deecodex 二进制..."
if curl -fSL --progress-bar -o "$BIN_DIR/deecodex" "$RELEASE_URL/deecodex" 2>/dev/null; then
    chmod +x "$BIN_DIR/deecodex"
    print_ok "deecodex → $BIN_DIR/deecodex"
else
    print_err "二进制下载失败"
    if [ "$RUST_OK" = true ]; then
        echo "       将尝试从源码编译..."
    else
        echo "       请确认 Release 中包含 macOS 二进制，或安装 Rust 后从源码编译"
        exit 1
    fi
fi

# 下载管理脚本
echo "       下载管理脚本..."
if curl -fSL --progress-bar -o "$CONFIG_DIR/deecodex.sh" "$RELEASE_URL/deecodex.sh" 2>/dev/null; then
    chmod +x "$CONFIG_DIR/deecodex.sh"
    print_ok "deecodex.sh → $CONFIG_DIR/deecodex.sh"
else
    print_warn "管理脚本下载失败，将使用已内置版本"
fi

# 添加 ~/.local/bin 到 PATH
if [ "$NEED_PATH_FIX" = true ]; then
    for rc in "$HOME/.zshrc" "$HOME/.bashrc" "$HOME/.bash_profile"; do
        if [ -f "$rc" ]; then
            if ! grep -q "$BIN_DIR" "$rc" 2>/dev/null; then
                echo "export PATH=\"$BIN_DIR:\$PATH\"" >> "$rc"
                print_ok "已添加 $BIN_DIR 到 $rc"
            fi
        fi
    done
    export PATH="$BIN_DIR:$PATH"
fi

echo ""

# ===== Phase 5: 启动服务 =====
print_step 5 "启动服务"

STARTED=false

if confirm "是否现在启动 deecodex？"; then
    # 检测端口是否被占用
    if lsof -i ":$PORT" &>/dev/null 2>&1; then
        print_warn "端口 $PORT 已被占用"
        if confirm "是否终止占用进程并继续？"; then
            kill "$(lsof -ti ":$PORT")" 2>/dev/null || true
            sleep 1
        else
            echo "       请修改 $ENV_FILE 中的 DEECODEX_PORT 后手动启动"
        fi
    fi

    # 检测是否已有实例运行
    PID_FILE="$CONFIG_DIR/deecodex.pid"
    if [ -f "$PID_FILE" ]; then
        OLD_PID=$(cat "$PID_FILE")
        if kill -0 "$OLD_PID" 2>/dev/null; then
            print_warn "deecodex 已在运行 (PID: $OLD_PID)"
            if confirm "是否重启？"; then
                kill "$OLD_PID" 2>/dev/null || true
                sleep 1
            else
                print_ok "跳过启动，服务已在运行"
                STARTED=true
            fi
        fi
    fi

    if [ "$STARTED" = false ]; then
        # 使用管理脚本启动
        if [ -f "$CONFIG_DIR/deecodex.sh" ]; then
            echo "       启动中..."
            cd "$CONFIG_DIR" && bash "$CONFIG_DIR/deecodex.sh" start 2>&1 | while IFS= read -r line; do
                echo "       $line"
            done
        else
            # 备用：直接启动
            cd "$CONFIG_DIR" && nohup "$BIN_DIR/deecodex" start --data-dir "$CONFIG_DIR" > "$CONFIG_DIR/deecodex.log" 2>&1 &
            echo $! > "$PID_FILE"
            print_ok "deecodex 已启动 (PID: $(cat "$PID_FILE"))"
        fi

        # 等待服务就绪
        echo "       等待服务就绪..."
        for i in $(seq 1 15); do
            if curl -s "http://127.0.0.1:$PORT/api/status" &>/dev/null; then
                print_ok "服务就绪"
                STARTED=true
                break
            fi
            sleep 1
        done

        if [ "$STARTED" = false ]; then
            print_warn "服务可能启动较慢，请稍后检查"
        fi
    fi
else
    print_ok "跳过启动（可稍后手动启动）"
fi

echo ""

# ===== Phase 6: 打开管理面板 =====
print_step 6 "完成安装"

PANEL_URL="http://127.0.0.1:$PORT"

if [ "$STARTED" = true ]; then
    if confirm "是否打开 Web 配置面板？"; then
        if [[ "$OSTYPE" == "darwin"* ]]; then
            open "$PANEL_URL" 2>/dev/null || print_warn "无法自动打开浏览器，请手动访问: $PANEL_URL"
        else
            xdg-open "$PANEL_URL" 2>/dev/null || print_warn "无法自动打开浏览器，请手动访问: $PANEL_URL"
        fi
    fi
fi

echo ""
echo -e "${GREEN}╔══════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║         🎉 deecodex 安装完成！            ║${NC}"
echo -e "${GREEN}╚══════════════════════════════════════════╝${NC}"
echo ""
echo -e "  ${BOLD}管理命令${NC}（在 $CONFIG_DIR 目录下执行）:"
echo -e "    ${CYAN}./deecodex.sh start${NC}    启动服务"
echo -e "    ${CYAN}./deecodex.sh stop${NC}     停止服务"
echo -e "    ${CYAN}./deecodex.sh restart${NC}  重启服务"
echo -e "    ${CYAN}./deecodex.sh status${NC}   查看状态"
echo -e "    ${CYAN}./deecodex.sh logs${NC}     查看日志"
echo -e "    ${CYAN}./deecodex.sh health${NC}   健康检查"
echo -e "    ${CYAN}./deecodex.sh update${NC}   一键升级"
echo ""
echo -e "  ${BOLD}配置面板${NC}: ${CYAN}$PANEL_URL${NC}"
echo -e "  ${BOLD}配置文件${NC}: $ENV_FILE"
echo -e "  ${BOLD}日志文件${NC}: $CONFIG_DIR/deecodex.log"
echo ""
echo -e "  ${YELLOW}提醒：如修改了 .env，需重启服务生效${NC}"
echo -e "  ${YELLOW}       ${CYAN}./deecodex.sh restart${NC}"
echo ""
