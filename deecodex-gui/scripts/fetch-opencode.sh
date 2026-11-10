#!/usr/bin/env bash
# 拉取 pin 版本 opencode 二进制到 deecodex-gui/resources/opencode/（gitignore）。
# 用法：bash scripts/fetch-opencode.sh
# 之后在 tauri.conf.json 的 bundle.resources 里把对应文件打进安装包：
#   macOS  -> resources/opencode/opencode-darwin-arm64 -> opencode（落在 Contents/Resources/opencode）
#   Windows -> resources/opencode/opencode-windows-x64.exe -> opencode.exe
set -euo pipefail

VERSION="1.18.16"
BASE_URL="https://github.com/anomalyco/opencode/releases/download/v${VERSION}"
OUT_DIR="$(cd "$(dirname "$0")/.." && pwd)/resources/opencode"

# 1.18.16 解压后二进制的 SHA256（防篡改/版本漂移）
SHA_DARWIN_ARM64="a41776bf64c75786d6baf531b840ffb873c090d7c44793ae2dd4b1896de56a1f"
SHA_WINDOWS_X64="dadee463adc9eaeeab9b79d5c5b4557a372a33af70b2742fff76d5507fccc0ac"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64|Darwin-arm64-*|Darwin-aarch64)
    ASSET="opencode-darwin-arm64.zip"; BIN="opencode"; TARGET="opencode-darwin-arm64"; EXPECTED="$SHA_DARWIN_ARM64";;
  MINGW*|MSYS*|CYGWIN*)
    ASSET="opencode-windows-x64.zip"; BIN="opencode.exe"; TARGET="opencode-windows-x64.exe"; EXPECTED="$SHA_WINDOWS_X64";;
  *)
    echo "暂不支持的平台: $(uname -s)-$(uname -m)（仅 darwin-arm64 / windows-x64）" >&2
    exit 1;;
esac

mkdir -p "$OUT_DIR"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "下载 opencode ${VERSION} (${ASSET}) ..."
curl -fsSL -o "$TMP/$ASSET" "$BASE_URL/$ASSET"
unzip -o -q "$TMP/$ASSET" -d "$TMP/x"

SRC="$TMP/x/$BIN"
[ -f "$SRC" ] || { echo "解压后未找到 $BIN" >&2; exit 1; }

ACTUAL="$(shasum -a 256 "$SRC" | awk '{print $1}')"
if [ "$ACTUAL" != "$EXPECTED" ]; then
  echo "SHA256 校验失败：期望 $EXPECTED，实际 $ACTUAL" >&2
  exit 1
fi

install -m 0755 "$SRC" "$OUT_DIR/$TARGET"
echo "OK: $OUT_DIR/$TARGET"
echo "提示：在 tauri.conf.json bundle.resources 中引用该文件即可随安装包分发。"
