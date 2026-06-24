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
NOTES="${DEX_AI_UPDATE_NOTES:-}"
NOTES_FILE="${DEX_AI_UPDATE_NOTES_FILE:-}"

mark_macos_build_artifacts_noindex() {
  if [[ "$(uname -s)" != "Darwin" ]]; then
    return 0
  fi

  local macos_dir="$BUNDLE_DIR/macos"
  if [[ -d "$macos_dir" ]]; then
    touch "$macos_dir/.metadata_never_index" 2>/dev/null || true
    find "$macos_dir" -maxdepth 1 -type d -name "*.app" -exec touch "{}/.metadata_never_index" \; 2>/dev/null || true
  fi
}

if [[ -n "$NOTES_FILE" ]]; then
  NOTES="$(cat "$NOTES_FILE")"
fi

mkdir -p "$OUT_DIR/mac" "$OUT_DIR/windows"
mark_macos_build_artifacts_noindex
shopt -s nullglob
rm -f "$OUT_DIR"/latest.json
rm -f "$OUT_DIR"/mac/*
rm -f "$OUT_DIR"/windows/*
shopt -u nullglob

select_matching_app_tarball() {
  local desired_version="$1"
  local file
  while IFS= read -r file; do
    [[ -n "$file" ]] || continue
    local bundle_version
    bundle_version="$(
      python3 - "$file" <<'PY'
import plistlib
import sys
import tarfile

path = sys.argv[1]
with tarfile.open(path, "r:gz") as tf:
    plist_name = next((name for name in tf.getnames() if name.endswith("Contents/Info.plist")), None)
    if plist_name is None:
        raise SystemExit(2)
    plist = plistlib.loads(tf.extractfile(plist_name).read())
    print(plist.get("CFBundleShortVersionString", ""))
PY
    )"
    if [[ "${bundle_version#v}" == "$desired_version" ]]; then
      printf '%s\n' "$file"
      return 0
    fi
  done < <(find "$BUNDLE_DIR" -type f -name '*.app.tar.gz' | sort)
  return 1
}

find_artifact() {
  local pattern="$1"
  local optional="${2:-false}"
  local file
  file="$(find "$BUNDLE_DIR" -type f -name "$pattern" | sort | grep -F "$VERSION" | tail -n 1 || true)"
  if [[ -z "$file" ]]; then
    file="$(find "$BUNDLE_DIR" -type f -name "$pattern" | sort | tail -n 1 || true)"
  fi
  if [[ -z "$file" && "$optional" != "true" ]]; then
    echo "missing bundle artifact: $pattern under $BUNDLE_DIR" >&2
    exit 1
  fi
  printf '%s\n' "$file"
}

copy_first() {
  local pattern="$1"
  local dest_dir="$2"
  local optional="${3:-false}"
  local file
  if [[ "$pattern" == "*.app.tar.gz" ]]; then
    file="$(select_matching_app_tarball "$VERSION" || true)"
  else
    file="$(find_artifact "$pattern" "$optional")"
  fi
  if [[ -z "$file" ]]; then
    if [[ "$optional" == "true" ]]; then
      return 0
    fi
    echo "missing bundle artifact: $pattern under $BUNDLE_DIR" >&2
    exit 1
  fi
  cp "$file" "$dest_dir/"
}

MAC_APP_TAR="$(select_matching_app_tarball "$VERSION" || true)"
if [[ -z "$MAC_APP_TAR" ]]; then
  echo "missing updater tarball for version $VERSION under $BUNDLE_DIR" >&2
  exit 1
fi
MAC_APP_SIG="$MAC_APP_TAR.sig"
if [[ ! -f "$MAC_APP_SIG" ]]; then
  echo "missing updater signature paired with tarball: $MAC_APP_SIG" >&2
  exit 1
fi
cp "$MAC_APP_TAR" "$OUT_DIR/mac/"
cp "$MAC_APP_SIG" "$OUT_DIR/mac/"
copy_first "*.dmg" "$OUT_DIR/mac" true
copy_first "*setup.exe" "$OUT_DIR/windows" true
copy_first "*setup.exe.sig" "$OUT_DIR/windows" true

MAC_TAR="$(basename "$(find "$OUT_DIR/mac" -type f -name '*.app.tar.gz' | sort | tail -n 1)")"
MAC_SIG_FILE="$OUT_DIR/mac/$MAC_TAR.sig"
MAC_SIG="$(tr -d '\r\n' < "$MAC_SIG_FILE")"

python3 - "$MAC_SIG" <<'PY'
import base64
import sys

signature = sys.argv[1].strip()
try:
    decoded = base64.b64decode(signature, validate=True).decode("utf-8", "replace")
except Exception as exc:
    raise SystemExit(f"invalid updater signature: {exc}")

if "signature from tauri secret key" not in decoded:
    raise SystemExit("invalid updater signature: not a Tauri updater signature")
PY

python3 - "$OUT_DIR/mac/$MAC_TAR" "$VERSION" <<'PY'
import plistlib
import sys
import tarfile

tarball, desired = sys.argv[1:3]
with tarfile.open(tarball, "r:gz") as tf:
    plist_name = next((name for name in tf.getnames() if name.endswith("Contents/Info.plist")), None)
    if plist_name is None:
        raise SystemExit("missing Info.plist in updater tarball")
    plist = plistlib.loads(tf.extractfile(plist_name).read())
    actual = str(plist.get("CFBundleShortVersionString", "")).lstrip("v")
    if actual != desired:
        raise SystemExit(f"updater tarball version mismatch: expected {desired}, got {actual}")
PY

python3 - "$OUT_DIR/latest.json" "$VERSION" "$BASE_URL" "$MAC_TAR" "$MAC_SIG" "$MAC_TARGETS" "$NOTES" <<'PY'
import json
import pathlib
import sys
import urllib.parse

out, version, base_url, mac_tar, mac_sig, mac_targets, notes = sys.argv[1:8]
base_url = base_url.rstrip("/")
mac_tar_url = urllib.parse.quote(mac_tar)

manifest = {
    "version": version,
    "notes": notes,
    "pub_date": None,
    "platforms": {},
}

for target in [item.strip() for item in mac_targets.split(",") if item.strip()]:
    manifest["platforms"][target] = {
        "signature": mac_sig,
        "url": f"{base_url}/{version}/mac/{mac_tar_url}",
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
