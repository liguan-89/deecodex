#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION_FILE="$ROOT_DIR/VERSION"
VERSION="${1:-$(tr -d '[:space:]' < "$VERSION_FILE")}"
VERSION="${VERSION#v}"

BASE_URL="${DEX_AI_UPDATE_BASE_URL:-https://api.liguan.me/releases/dex-ai}"
OUT_DIR="${DEX_AI_UPDATE_OUT_DIR:-$ROOT_DIR/dist/updater-release/$VERSION}"
BUNDLE_DIR="$ROOT_DIR/target-mac/release/bundle"
MAC_TARGETS="${DEX_AI_UPDATE_MAC_TARGETS:-darwin-aarch64}"

mkdir -p "$OUT_DIR/mac" "$OUT_DIR/windows"

copy_first() {
  local pattern="$1"
  local dest_dir="$2"
  local optional="${3:-false}"
  local file
  file="$(find "$BUNDLE_DIR" -type f -name "$pattern" | sort | tail -n 1 || true)"
  if [[ -z "$file" ]]; then
    if [[ "$optional" == "true" ]]; then
      return 0
    fi
    echo "missing bundle artifact: $pattern under $BUNDLE_DIR" >&2
    exit 1
  fi
  cp "$file" "$dest_dir/"
}

copy_first "*.app.tar.gz" "$OUT_DIR/mac"
copy_first "*.app.tar.gz.sig" "$OUT_DIR/mac"
copy_first "*.dmg" "$OUT_DIR/mac" true
copy_first "*setup.exe" "$OUT_DIR/windows" true
copy_first "*setup.exe.sig" "$OUT_DIR/windows" true

MAC_TAR="$(basename "$(find "$OUT_DIR/mac" -type f -name '*.app.tar.gz' | sort | tail -n 1)")"
MAC_SIG_FILE="$OUT_DIR/mac/$MAC_TAR.sig"
MAC_SIG="$(tr -d '\r\n' < "$MAC_SIG_FILE")"

python3 - "$OUT_DIR/latest.json" "$VERSION" "$BASE_URL" "$MAC_TAR" "$MAC_SIG" "$MAC_TARGETS" <<'PY'
import json
import pathlib
import sys

out, version, base_url, mac_tar, mac_sig, mac_targets = sys.argv[1:7]
base_url = base_url.rstrip("/")

manifest = {
    "version": version,
    "notes": "",
    "pub_date": None,
    "platforms": {},
}

for target in [item.strip() for item in mac_targets.split(",") if item.strip()]:
    manifest["platforms"][target] = {
        "signature": mac_sig,
        "url": f"{base_url}/{version}/mac/{mac_tar}",
    }

win_files = sorted(pathlib.Path(out).parent.joinpath("windows").glob("*setup.exe"))
if win_files:
    exe = win_files[-1].name
    sig_path = pathlib.Path(out).parent / "windows" / f"{exe}.sig"
    if sig_path.exists():
        sig = sig_path.read_text(encoding="utf-8").strip()
        manifest["platforms"]["windows-x86_64"] = {
            "signature": sig,
            "url": f"{base_url}/{version}/windows/{exe}",
        }

pathlib.Path(out).write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
PY

echo "Prepared updater release:"
echo "  $OUT_DIR"
echo
echo "Upload example:"
echo "  rsync -avz \"$OUT_DIR/\" user@server:/var/www/dex-ai/releases/dex-ai/$VERSION/"
echo "  rsync -avz \"$OUT_DIR/latest.json\" user@server:/var/www/dex-ai/releases/dex-ai/latest.json"
