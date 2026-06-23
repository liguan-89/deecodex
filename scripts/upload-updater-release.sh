#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION_FILE="$ROOT_DIR/VERSION"
VERSION="${1:-$(tr -d '[:space:]' < "$VERSION_FILE")}"
VERSION="${VERSION#v}"

OUT_DIR="${DEX_AI_UPDATE_OUT_DIR:-$ROOT_DIR/dist/updater-release/$VERSION}"
SSH_KEY="${DEX_AI_UPDATE_SSH_KEY:-$HOME/Desktop/aliyun.pem}"
REMOTE_TARGET="${DEX_AI_UPDATE_REMOTE_TARGET:-}"

if [[ -z "$REMOTE_TARGET" ]]; then
  echo "missing DEX_AI_UPDATE_REMOTE_TARGET, example:" >&2
  echo "  DEX_AI_UPDATE_REMOTE_TARGET='root@1.2.3.4:/var/www/dex-ai/releases/dex-ai' $0 $VERSION" >&2
  exit 1
fi

if [[ ! -d "$OUT_DIR" ]]; then
  echo "missing release directory: $OUT_DIR" >&2
  echo "run ./scripts/prepare-updater-release.sh $VERSION first" >&2
  exit 1
fi

if [[ ! -f "$SSH_KEY" ]]; then
  echo "missing ssh key: $SSH_KEY" >&2
  exit 1
fi

chmod 600 "$SSH_KEY"

REMOTE_HOST="${REMOTE_TARGET%%:*}"
REMOTE_DIR="${REMOTE_TARGET#*:}"

ssh -i "$SSH_KEY" "$REMOTE_HOST" "mkdir -p '$REMOTE_DIR/$VERSION'"
rsync -avz -e "ssh -i '$SSH_KEY'" "$OUT_DIR/" "$REMOTE_HOST:$REMOTE_DIR/$VERSION/"
rsync -avz -e "ssh -i '$SSH_KEY'" "$OUT_DIR/latest.json" "$REMOTE_HOST:$REMOTE_DIR/latest.json"

echo "Uploaded updater release:"
echo "  $REMOTE_TARGET/$VERSION"
echo "  $REMOTE_TARGET/latest.json"
