#!/usr/bin/env bash
# 本机（Apple Silicon macOS）交叉编译 Windows x64 发布包。
#
# 依赖：
#   - rustup target add x86_64-pc-windows-msvc
#   - cargo install cargo-xwin
#   - brew install nsis（makensis；若 brew 前缀与 NSIS 编译前缀不一致，
#     本脚本通过 NSISDIR 显式指向 share/nsis 修复）
#
# 用法：
#   bash scripts/build-windows-release.sh            # 本地联调包（无 updater 签名）
#   CREATE_UPDATER=true bash scripts/build-windows-release.sh   # 出 updater 产物（需签名密钥）
#
# 说明：
#   - 第一方插件（mobile-codex / desktop-sync）的 Windows 产物缺省不打包，
#     且默认不把插件目录打进 resources（避免把 macOS 二进制塞进 NSIS 包）；
#     提供 .exe 产物后需把插件目录加回 tauri.windows.conf.json 的 resources，
#     并设置 DEECODEX_*_PLUGIN_SHA256 启用校验与打包。
#   - Windows updater 签名密钥（39109EA3AD54A4D4 对应）不在本机时，
#     不要设 TAURI_SIGNING_PRIVATE_KEY，脚本默认跳过 updater 产物。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# --- makensis 前缀修复：NSIS 编译前缀可能是 /opt/homebrew，与 brew 前缀不一致 ---
NSIS_HOME="$(brew --prefix nsis 2>/dev/null || true)"
if [ -n "$NSIS_HOME" ] && [ -d "$NSIS_HOME/share/nsis" ]; then
  export NSISDIR="${NSISDIR:-$NSIS_HOME/share/nsis}"
fi
echo "NSISDIR=$NSISDIR"

# --- LLVM 工具链：xwin 交叉编译需要 clang-cl/lld-link/llvm-lib/llvm-rc ---
LLVM_BIN="$(brew --prefix llvm 2>/dev/null || true)/bin"
if [ -n "$LLVM_BIN" ] && [ -x "$LLVM_BIN/llvm-lib" ]; then
  export PATH="$LLVM_BIN:$PATH"
  echo "LLVM 工具链: $LLVM_BIN"
fi

# --- makensis wrapper：tauri bundler 调用 makensis 时可能覆盖 NSISDIR，
# 用 ~/.local/bin/makensis 包装脚本强制 NSISDIR 指向真实安装目录 ---
if [ -x "$HOME/.local/bin/makensis" ]; then
  export PATH="$HOME/.local/bin:$PATH"
  echo "makensis wrapper: $HOME/.local/bin/makensis"
fi

CREATE_UPDATER="${CREATE_UPDATER:-false}"
echo "createUpdaterArtifacts=$CREATE_UPDATER"

CONFIG_ARGS=(
  --runner cargo-xwin
  --target x86_64-pc-windows-msvc
  --bundles nsis
  --config tauri.windows.conf.json
  --config "{\"bundle\":{\"createUpdaterArtifacts\":$CREATE_UPDATER}}"
)

echo ">>> cargo tauri build ${CONFIG_ARGS[*]}"
exec cargo tauri build "${CONFIG_ARGS[@]}"
