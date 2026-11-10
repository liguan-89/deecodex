#!/bin/bash
set -euo pipefail

# 发布 DEXAI Debug 插件（独立更新，不需要重发整个 app 版本）
# 用法：./scripts/publish-debug-plugin.sh <版本> [更新说明文件]
# 依赖：tauri CLI（签名）、rsync、SSH 密钥
# 部署目标通过环境变量提供：DEX_AI_DEBUG_SERVER（user@host）、DEX_AI_UPDATE_SSH_KEY（私钥路径）
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="${1:?用法: publish-debug-plugin.sh <版本> [更新说明文件]}"
NOTES_FILE="${2:-$ROOT/docs/releases/plugin-dexai-debug-$VERSION.md}"
PLUGIN_SRC="$ROOT/deecodex-plugins/plugins/deecodex-debug"
OUT="$ROOT/dist/plugin-dexai-debug/$VERSION"
SERVER="${DEX_AI_DEBUG_SERVER:?需要设置 DEX_AI_DEBUG_SERVER（格式 user@host）}"
SSH_KEY="${DEX_AI_UPDATE_SSH_KEY:?需要设置 DEX_AI_UPDATE_SSH_KEY（SSH 私钥路径）}"
REMOTE_BASE="/var/www/dex-ai/releases/dex-ai/plugins/dexai-debug"
PLUGIN_ID="dexai-debug"
MIN_DEEECODEX_VERSION="4.3.5"
ZIP="dexai-debug-$VERSION-darwin-aarch64.zip"

if [[ ! -d "$PLUGIN_SRC" ]]; then
  echo "插件目录不存在：$PLUGIN_SRC" >&2
  exit 1
fi

mkdir -p "$OUT"

# 1) 打包（排除运行时数据；zip 根直接是插件内容，不带头目录；
#    用临时副本并把 plugin.json version 设为发布版本，避免与更新清单不一致）
rm -f "$OUT/$ZIP"
TMP_PKG="$(mktemp -d)"
cp -R "$PLUGIN_SRC"/. "$TMP_PKG/"
rm -rf "$TMP_PKG/.diagnose-data" "$TMP_PKG/.git"
python3 - "$TMP_PKG/plugin.json" "$VERSION" <<'PY'
import json, sys
path, version = sys.argv[1], sys.argv[2]
d = json.load(open(path, encoding="utf-8"))
d["version"] = version
json.dump(d, open(path, "w", encoding="utf-8"), ensure_ascii=False, indent=2)
print("plugin.json version ->", version)
PY
cd "$TMP_PKG"
zip -qr "$OUT/$ZIP" .
cd "$ROOT"
rm -rf "$TMP_PKG"

SHA="$(shasum -a 256 "$OUT/$ZIP" | awk '{print $1}')"
SIZE="$(stat -f%z "$OUT/$ZIP")"

# 2) 签名（tauri updater 签名，写入 .sig 文件）
cd "$ROOT"
cargo tauri signer sign -f "$HOME/.tauri/dex-ai-updater.key" \
  -p "$(cat "$HOME/.tauri/dex-ai-updater.key.password")" "$OUT/$ZIP" > "$OUT/sign-output.txt" 2>/dev/null
SIGNATURE="$(awk '/^Public signature:/{found=1; next} found && NF {print; exit}' "$OUT/sign-output.txt" | tr -d ' \t\r\n')"
rm -f "$OUT/sign-output.txt"
if [[ -z "$SIGNATURE" ]]; then
  echo "签名失败：未从 signer 输出提取到 base64 签名" >&2
  exit 1
fi

# 3) 生成 latest.json
if [[ -f "$NOTES_FILE" ]]; then
  python3 - "$NOTES_FILE" > "$OUT/release-notes.json" <<'PY'
import json, sys
lines = [l.rstrip() for l in open(sys.argv[1], encoding="utf-8") if l.strip()]
json.dump(lines, ensure_ascii=False, fp=sys.stdout)
PY
  RELEASE_NOTES="$(cat "$OUT/release-notes.json")"
else
  RELEASE_NOTES='["DEXAI Debug '"$VERSION"'"]'
fi

cat > "$OUT/latest.json" <<EOF
{
  "schemaVersion": 1,
  "pluginId": "$PLUGIN_ID",
  "version": "$VERSION",
  "minDeecodexVersion": "$MIN_DEEECODEX_VERSION",
  "publishedAt": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "releaseNotes": $RELEASE_NOTES,
  "platforms": {
    "darwin-aarch64": {
      "url": "https://api.liguan.me/releases/dex-ai/plugins/dexai-debug/$VERSION/$ZIP",
      "sha256": "$SHA",
      "sizeBytes": $SIZE,
      "signature": "$SIGNATURE"
    },
    "windows-x86_64": {
      "url": "https://api.liguan.me/releases/dex-ai/plugins/dexai-debug/$VERSION/$ZIP",
      "sha256": "$SHA",
      "sizeBytes": $SIZE,
      "signature": "$SIGNATURE"
    }
  }
}
EOF

# 4) 上传
echo "上传 $VERSION 到服务器..."
rsync -avz -e "ssh -i \"$SSH_KEY\" -o IdentitiesOnly=yes" \
  "$OUT/" "$SERVER:$REMOTE_BASE/$VERSION/" 2>&1 | tail -3
rsync -avz -e "ssh -i \"$SSH_KEY\" -o IdentitiesOnly=yes" \
  "$OUT/latest.json" "$SERVER:$REMOTE_BASE/latest.json" 2>&1 | tail -1

echo "DEXAI Debug 插件 $VERSION 发布完成："
echo "  https://api.liguan.me/releases/dex-ai/plugins/dexai-debug/$VERSION/$ZIP"
echo "客户端在插件详情页「DEXAI Debug 更新」检查/立即更新即可。"
