#!/bin/bash
# deecodex 管理脚本
# 用法: ./deecodex.sh {start|stop|restart|status|logs|health}

set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")" && pwd)"
ENV_FILE="$PROJECT_DIR/.env"
PID_FILE="$PROJECT_DIR/deecodex.pid"
LOG_DIR="$PROJECT_DIR/logs"
LOG_FILE="$LOG_DIR/deecodex.log"
MAX_LOG_SIZE_MB=50
MAX_LOG_FILES=5
BIN="deecodex"
GRACEFUL_TIMEOUT=35

# === Codex 配置管理 ===
CODEX_CONFIG="$HOME/.codex/config.toml"
CODEX_CONFIG_OPENAI="${CODEX_CONFIG}.openai.txt"
CODEX_CONFIG_DEECODEX="${CODEX_CONFIG}.deecodex.txt"
_DEECODEX_CONFIG_ACTIVE=0

codex_config_init() {
    local port="${DEECODEX_PORT:-4446}"
    [ ! -f "$CODEX_CONFIG" ] && return 0

    local is_openai_version=true
    if grep -q "# === 以下由 deecodex 自动管理 ===" "$CODEX_CONFIG" 2>/dev/null; then
        is_openai_version=false
    fi

    if [ ! -f "$CODEX_CONFIG_OPENAI" ]; then
        cp "$CODEX_CONFIG" "$CODEX_CONFIG_OPENAI"
        echo "已创建 $CODEX_CONFIG_OPENAI"
    elif $is_openai_version; then
        cp "$CODEX_CONFIG" "$CODEX_CONFIG_OPENAI"
        echo "已同步 $CODEX_CONFIG → $CODEX_CONFIG_OPENAI"
    fi

    rm -f "$CODEX_CONFIG_DEECODEX"
    cp "$CODEX_CONFIG_OPENAI" "$CODEX_CONFIG_DEECODEX"

    sed -i '' "/^model_reasoning_effort/a\\
model_provider = \"custom\"
" "$CODEX_CONFIG_DEECODEX"

    local requires_openai_auth="false"
    if [ -n "${DEECODEX_CLIENT_API_KEY:-}" ]; then
        requires_openai_auth="true"
    fi

    cat >> "$CODEX_CONFIG_DEECODEX" << 'CODEX_EOF'

# === 以下由 deecodex 自动管理 ===
[model_providers.custom]
base_url = "http://127.0.0.1:__DEECODEX_PORT__/v1"
name = "custom"
requires_openai_auth = __DEECODEX_REQUIRES_OPENAI_AUTH__
api_key = "__DEECODEX_CLIENT_API_KEY__"
wire_api = "responses"
CODEX_EOF
    sed -i '' "s|__DEECODEX_PORT__|$port|g" "$CODEX_CONFIG_DEECODEX"
    sed -i '' "s|__DEECODEX_REQUIRES_OPENAI_AUTH__|${requires_openai_auth}|g" "$CODEX_CONFIG_DEECODEX"
    sed -i '' "s|__DEECODEX_CLIENT_API_KEY__|${DEECODEX_CLIENT_API_KEY}|g" "$CODEX_CONFIG_DEECODEX"
    echo "已更新 $CODEX_CONFIG_DEECODEX (端口: $port)"
}

codex_config_switch_to_deecodex() {
    [ -f "$CODEX_CONFIG_DEECODEX" ] || return 0
    cp "$CODEX_CONFIG_DEECODEX" "$CODEX_CONFIG"
    _DEECODEX_CONFIG_ACTIVE=1
    echo "Codex 配置 → deecodex"
}

codex_config_switch_to_openai() {
    [ -f "$CODEX_CONFIG_OPENAI" ] || return 0
    cp "$CODEX_CONFIG_OPENAI" "$CODEX_CONFIG"
    _DEECODEX_CONFIG_ACTIVE=0
    echo "Codex 配置 → OpenAI"
}

cleanup_config() {
    if [ "${_CLEANUP_RUN:-0}" -eq 1 ]; then
        return
    fi
    _CLEANUP_RUN=1
    if [ "${_DEECODEX_CONFIG_ACTIVE:-0}" -eq 1 ]; then
        echo "中断信号，正在还原配置..."
        codex_config_switch_to_openai
    fi
    if is_running; then
        kill "$(cat "$PID_FILE" 2>/dev/null)" 2>/dev/null || true
        rm -f "$PID_FILE"
    fi
}

trap cleanup_config INT TERM

usage() {
    echo "用法: $0 {start|stop|restart|status|logs|health}"
    exit 1
}

load_env() {
    if [ ! -f "$ENV_FILE" ]; then
        echo "错误: 找不到 .env 文件 ($ENV_FILE)"
        exit 1
    fi
    set -a
    # shellcheck disable=SC1090
    source "$ENV_FILE"
    set +a
}

# 将 DEECODEX_* 变量映射到 CODEX_RELAY_*（二进制原生变量）
map_env() {
    DEECODEX_UPSTREAM="${DEECODEX_UPSTREAM:-${CODEX_RELAY_UPSTREAM:-}}"
    DEECODEX_API_KEY="${DEECODEX_API_KEY:-${CODEX_RELAY_API_KEY:-}}"
    DEECODEX_PORT="${DEECODEX_PORT:-${CODEX_RELAY_PORT:-4446}}"
    DEECODEX_MODEL_MAP="${DEECODEX_MODEL_MAP:-${CODEX_RELAY_MODEL_MAP:-}}"
    DEECODEX_CLIENT_API_KEY="${DEECODEX_CLIENT_API_KEY-${CODEX_RELAY_CLIENT_API_KEY-}}"
    DEECODEX_PROMPTS_DIR="${DEECODEX_PROMPTS_DIR:-${CODEX_RELAY_PROMPTS_DIR:-prompts}}"

    # 反向导出 CODEX_RELAY_* 供二进制使用（二进制通过 clap env 属性读取 CODEX_RELAY_*）
    export CODEX_RELAY_UPSTREAM="${DEECODEX_UPSTREAM}"
    export CODEX_RELAY_API_KEY="${DEECODEX_API_KEY}"
    export CODEX_RELAY_PORT="${DEECODEX_PORT}"
    export CODEX_RELAY_MODEL_MAP="${DEECODEX_MODEL_MAP}"
    export CODEX_RELAY_CLIENT_API_KEY="${DEECODEX_CLIENT_API_KEY}"
    export CODEX_RELAY_PROMPTS_DIR="${DEECODEX_PROMPTS_DIR}"
    export CODEX_RELAY_VISION_UPSTREAM="${DEECODEX_VISION_UPSTREAM:-}"
    export CODEX_RELAY_VISION_API_KEY="${DEECODEX_VISION_API_KEY:-}"
    export CODEX_RELAY_VISION_MODEL="${DEECODEX_VISION_MODEL:-MiniMax-M1}"
    export CODEX_RELAY_VISION_ENDPOINT="${DEECODEX_VISION_ENDPOINT:-v1/coding_plan/vlm}"
    export DEECODEX_PLAYWRIGHT_STATE_DIR="${DEECODEX_PLAYWRIGHT_STATE_DIR:-}"
    export DEECODEX_BROWSER_USE_BRIDGE_URL="${DEECODEX_BROWSER_USE_BRIDGE_URL:-}"
    export DEECODEX_BROWSER_USE_BRIDGE_COMMAND="${DEECODEX_BROWSER_USE_BRIDGE_COMMAND:-}"
}

is_running() {
    if [ -f "$PID_FILE" ]; then
        local pid
        pid=$(cat "$PID_FILE")
        if kill -0 "$pid" 2>/dev/null; then
            return 0
        fi
    fi
    return 1
}

get_port() {
    load_env
    map_env
    echo "${DEECODEX_PORT:-4446}"
}

rotate_logs() {
    if [ ! -f "$LOG_FILE" ]; then
        return
    fi
    local size_bytes
    size_bytes=$(wc -c < "$LOG_FILE" 2>/dev/null || echo 0)
    local max_bytes=$((MAX_LOG_SIZE_MB * 1024 * 1024))
    if [ "$size_bytes" -lt "$max_bytes" ]; then
        return
    fi
    rm -f "$LOG_FILE.$MAX_LOG_FILES"
    for i in $(seq $((MAX_LOG_FILES - 1)) -1 1); do
        [ -f "$LOG_FILE.$i" ] && mv "$LOG_FILE.$i" "$LOG_FILE.$((i + 1))"
    done
    mv "$LOG_FILE" "$LOG_FILE.1"
    touch "$LOG_FILE"
    echo "$(date -u +"%Y-%m-%dT%H:%M:%SZ") log rotated (was >= ${MAX_LOG_SIZE_MB}MB)" >> "$LOG_FILE"
}

cmd_start() {
    if is_running; then
        echo "deecodex 已在运行中 (PID: $(cat "$PID_FILE"))"
        return 1
    fi
    if ! command -v "$BIN" > /dev/null 2>&1; then
        echo "错误: 找不到二进制 $BIN"
        echo "      创建符号链接: ln -sf \$(which codex-relay) \$(dirname \$(which $BIN))/deecodex"
        echo "      或安装: pipx install codex-relay"
        exit 1
    fi
    load_env
    map_env
    codex_config_init
    codex_config_switch_to_deecodex
    rotate_logs
    local port="${DEECODEX_PORT:-4446}"
    echo "启动 deecodex (端口: $port, 二进制: $(command -v "$BIN"))..."
    nohup "$BIN" \
        --port "$port" \
        --upstream "${DEECODEX_UPSTREAM}" \
        --model-map "${DEECODEX_MODEL_MAP:-}" \
        >> "$LOG_FILE" 2>&1 &
    local pid=$!
    echo "$pid" > "$PID_FILE"
    local attempts=0
    while [ $attempts -lt 5 ]; do
        sleep 1
        if is_running; then
            echo "deecodex 已启动 (PID: $pid, 端口: $port)"
            return 0
        fi
        attempts=$((attempts + 1))
    done
    echo "启动失败，查看日志: tail -20 $LOG_FILE"
    rm -f "$PID_FILE"
    codex_config_switch_to_openai
    return 1
}

cmd_stop() {
    if ! is_running; then
        echo "deecodex 未运行"
        codex_config_switch_to_openai
        rm -f "$PID_FILE"
        return 0
    fi
    local pid
    pid=$(cat "$PID_FILE")
    echo "停止 deecodex (PID: $pid)..."
    kill "$pid" 2>/dev/null || true
    local waited=0
    while [ $waited -lt "$GRACEFUL_TIMEOUT" ]; do
        if ! kill -0 "$pid" 2>/dev/null; then
            echo "已停止 (优雅退出, 耗时 ${waited}s)"
            codex_config_switch_to_openai
            rm -f "$PID_FILE"
            return 0
        fi
        sleep 1
        waited=$((waited + 1))
    done
    echo "优雅退出超时 (${GRACEFUL_TIMEOUT}s), 强制终止..."
    kill -9 "$pid" 2>/dev/null || true
    sleep 1
    if kill -0 "$pid" 2>/dev/null; then
        echo "警告: 无法终止进程 $pid"
        return 1
    fi
    echo "已强制停止"
    codex_config_switch_to_openai
    rm -f "$PID_FILE"
}

cmd_restart() {
    cmd_stop
    sleep 1
    cmd_start
}

cmd_status() {
    if is_running; then
        local pid
        pid=$(cat "$PID_FILE")
        local port_line
        port_line=$(lsof -iTCP -sTCP:LISTEN -a -p "$pid" 2>/dev/null | grep LISTEN | awk '{print $9}' | head -1 || echo '未知')
        echo "deecodex 运行中"
        echo "  PID:    $pid"
        echo "  端口:   $port_line"
        echo "  日志:   $LOG_FILE (${MAX_LOG_SIZE_MB}MB 轮转, 保留 ${MAX_LOG_FILES} 份)"
    else
        echo "deecodex 未运行"
        rm -f "$PID_FILE"
    fi
}

cmd_logs() {
    if [ -f "$LOG_FILE" ]; then
        tail -f "$LOG_FILE"
    else
        echo "暂无日志 ($LOG_FILE)"
    fi
}

cmd_health() {
    local port
    port=$(get_port)
    local code
    code=$(curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:${port}/v1/models" 2>/dev/null || echo "000")
    if [ "$code" = "200" ]; then
        echo "healthy (GET /v1/models -> $code)"
    elif [ "$code" = "000" ]; then
        echo "unreachable (端口 $port 无响应)"
    else
        echo "degraded (GET /v1/models -> $code)"
    fi
}

case "${1:-}" in
    start)   cmd_start ;;
    stop)    cmd_stop ;;
    restart) cmd_restart ;;
    status)  cmd_status ;;
    logs)    cmd_logs ;;
    health)  cmd_health ;;
    *)       usage ;;
esac
