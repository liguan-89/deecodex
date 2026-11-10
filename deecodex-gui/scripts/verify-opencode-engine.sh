#!/usr/bin/env bash
# 无头验证 opencode 引擎（DEX 助手 v1）：
#   1) 二进制存在 + 版本 pin + SHA256
#   2) 生成的 opencode.json 结构（provider 前缀 / agents / mcp 分组 / L3 ask）
#   3) MCP 端点 initialize 可达（可选，需引擎已运行）
# 用法：
#   bash scripts/verify-opencode-engine.sh                 # 默认检查本地/捆绑二进制
#   bash scripts/verify-opencode-engine.sh --config <path>  # 额外校验指定配置
#   bash scripts/verify-opencode-engine.sh --mcp <url> --token <tok>  # 额外探活 MCP
set -uo pipefail

PIN="1.18.16"
EXPECTED_DOC="c3a9f94af0c3324d97b482b14c692e810ce7ccac3136319ba46334de972b4cf1"
SHA_DARWIN_ARM64="a41776bf64c75786d6baf531b840ffb873c090d7c44793ae2dd4b1896de56a1f"
SHA_WINDOWS_X64="dadee463adc9eaeeab9b79d5c5b4557a372a33af70b2742fff76d5507fccc0ac"

CONFIG=""
MCP_URL=""
MCP_TOKEN=""
while [ $# -gt 0 ]; do
  case "$1" in
    --config) CONFIG="$2"; shift 2;;
    --mcp) MCP_URL="$2"; shift 2;;
    --token) MCP_TOKEN="$2"; shift 2;;
    *) echo "未知参数: $1" >&2; exit 2;;
  esac
done

PASS=0; FAIL=0
ok()   { echo "  [OK]   $1"; PASS=$((PASS+1)); }
bad()  { echo "  [FAIL] $1"; FAIL=$((FAIL+1)); }

echo "== 1) opencode 二进制 =="
BIN="${DEECODEX_OPENCODE_BIN:-}"
if [ -z "$BIN" ]; then
  for c in \
    "$(dirname "$0")/../resources/opencode/opencode-darwin-arm64" \
    "$(dirname "$0")/../resources/opencode/opencode-windows-x64.exe" \
    "$HOME/.local/opencode/$PIN/opencode" ; do
    [ -f "$c" ] && BIN="$c" && break
  done
fi
if [ -z "$BIN" ] || [ ! -f "$BIN" ]; then
  bad "未找到 opencode 二进制（设置 DEECODEX_OPENCODE_BIN 或先跑 scripts/fetch-opencode.sh）"
else
  ok "二进制: $BIN"
  VER=$("$BIN" --version 2>/dev/null | tr -d 'v' | tr -d '\n')
  if [ "$VER" = "$PIN" ]; then ok "版本 = $PIN"; else bad "版本 = $VER（期望 $PIN）"; fi
  ACTUAL=$(shasum -a 256 "$BIN" | awk '{print $1}')
  case "$(uname -s)" in
    Darwin) EXPECTED="$SHA_DARWIN_ARM64";;
    MINGW*|MSYS*|CYGWIN*) EXPECTED="$SHA_WINDOWS_X64";;
    *) EXPECTED="";;
  esac
  if [ -n "$EXPECTED" ] && [ "$ACTUAL" = "$EXPECTED" ]; then ok "SHA256 匹配"
  elif [ -n "$EXPECTED" ]; then bad "SHA256 不匹配（$ACTUAL）"
  else ok "平台未内置 SHA256，跳过（$ACTUAL）"; fi
fi

if [ -n "$CONFIG" ]; then
  echo "== 2) 生成配置: $CONFIG =="
  if [ ! -f "$CONFIG" ]; then bad "配置文件不存在"; else
    python3 - "$CONFIG" <<'PY'
import json, sys
cfg = json.load(open(sys.argv[1]))
prov = cfg.get("provider", {})
if not prov: raise SystemExit("FAIL no provider")
pid = next(iter(prov))
checks = []
checks.append(("provider id 带 dex_ 前缀", pid.startswith("dex_")))
checks.append(("默认 agent 存在", cfg.get("default_agent") in cfg.get("agent", {})))
agents = cfg.get("agent", {})
for m in ("diag","fix","workspace","plugins","cost","logs"):
    if m not in agents: checks.append((f"agent {m}", False))
mcp = cfg.get("mcp", {})
for g in ("dex_diag","dex_workspace","dex_ops","dex_plugins"):
    if g not in mcp or "headers" not in mcp[g]: checks.append((f"mcp {g}", False))
l3 = [k for k,v in cfg.get("permission", {}).items() if v == "ask"]
checks.append(("L3 ask 数量 > 0", len(l3) > 0))
checks.append(("autoupdate=false", cfg.get("autoupdate") is False))
checks.append(("snapshot=false", cfg.get("snapshot") is False))
for name, ok in checks:
    print(("  [OK]   " if ok else "  [FAIL] ") + name)
sys.exit(0 if all(ok for _, ok in checks) else 1)
PY
  [ $? -eq 0 ] && ok "配置结构" || bad "配置结构"
  fi
fi

if [ -n "$MCP_URL" ]; then
  echo "== 3) MCP 端点: $MCP_URL =="
  HDR=()
  [ -n "$MCP_TOKEN" ] && HDR=(-H "Authorization: Bearer $MCP_TOKEN")
  CODE=$(curl -s -o /tmp/oc-mcp-init.json -w "%{http_code}" -X POST "$MCP_URL" \
    "${HDR[@]}" -H "Content-Type: application/json" \
    -H "Accept: application/json, text/event-stream" \
    -d '{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"verify","version":"1"}}}')
  if [ "$CODE" = "200" ] && grep -q "deecodex-mcp" /tmp/oc-mcp-init.json; then ok "initialize 200 (deecodex-mcp)"
  else bad "initialize -> $CODE"; fi
fi

echo
echo "结果: PASS=$PASS FAIL=$FAIL"
[ "$FAIL" -eq 0 ]
