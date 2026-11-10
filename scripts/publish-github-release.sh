#!/usr/bin/env bash
set -euo pipefail

# 发布到公开仓库 GitHub Release（安装包分发渠道）。
# 依赖：gh CLI（已登录 liguan-89）、已运行 prepare-updater-release.sh。
#
# 用法：
#   ./scripts/publish-github-release.sh <version>
# 可选环境变量：
#   DEXAI_GITHUB_REPO=liguan-89/DEXAI            公开仓库（默认）
#   DEXAI_RELEASE_TITLE="DEX AI <version> — 标题"  Release 标题（默认 "DEX AI <version>"）
#   DEXAI_RELEASE_NOTES_FILE=docs/releases/<v>.md 更新说明（默认 docs/releases/<v>.md）
#   DEXAI_UPDATE_OUT_DIR=dist/updater-release/<v> 产物目录（默认 dist/updater-release/<v>）

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION_FILE="$ROOT_DIR/VERSION"
VERSION="${1:-$(tr -d '[:space:]' < "$VERSION_FILE")}"
VERSION="${VERSION#v}"

GITHUB_REPO="${DEXAI_GITHUB_REPO:-liguan-89/DEXAI}"
OUT_DIR="${DEXAI_UPDATE_OUT_DIR:-$ROOT_DIR/dist/updater-release/$VERSION}"
TITLE="${DEXAI_RELEASE_TITLE:-DEX AI $VERSION}"
NOTES_FILE="${DEXAI_RELEASE_NOTES_FILE:-$ROOT_DIR/docs/releases/$VERSION.md}"

if ! command -v gh >/dev/null 2>&1; then
  echo "缺少 gh CLI，请先安装并登录 GitHub（gh auth login）" >&2
  exit 1
fi

if [[ ! -d "$OUT_DIR/mac" ]]; then
  echo "缺少发布产物目录：$OUT_DIR/mac" >&2
  echo "请先运行 ./scripts/prepare-updater-release.sh $VERSION" >&2
  exit 1
fi

if [[ ! -f "$NOTES_FILE" ]]; then
  echo "缺少更新说明文件：$NOTES_FILE" >&2
  exit 1
fi

ASSETS=()
for f in "$OUT_DIR"/mac/*.app.tar.gz "$OUT_DIR"/mac/*.app.tar.gz.sig "$OUT_DIR"/mac/*.dmg; do
  [[ -f "$f" ]] && ASSETS+=("$f")
done

if [[ ${#ASSETS[@]} -eq 0 ]]; then
  echo "产物目录中未找到 tar.gz / sig / dmg 安装包：$OUT_DIR/mac" >&2
  exit 1
fi

TAG="v$VERSION"
if gh release view "$TAG" -R "$GITHUB_REPO" >/dev/null 2>&1; then
  echo "Release $TAG 已存在：https://github.com/$GITHUB_REPO/releases/tag/$TAG" >&2
  echo "如需覆盖请先手动删除旧 Release 或改用 --force" >&2
  exit 1
fi

echo "发布 $VERSION 到 ${GITHUB_REPO}（${#ASSETS[@]} 个资产）："
printf '  %s\n' "${ASSETS[@]}"
gh release create "$TAG" -R "$GITHUB_REPO" \
  --title "$TITLE" \
  --notes-file "$NOTES_FILE" \
  "${ASSETS[@]}"

echo
echo "公开仓库 Release：https://github.com/$GITHUB_REPO/releases/tag/$TAG"
