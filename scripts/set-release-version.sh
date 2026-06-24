#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <version>" >&2
  exit 1
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${1#v}"

if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z]+)*$ ]]; then
  echo "invalid version: $1" >&2
  exit 1
fi

python3 - "$ROOT_DIR" "$VERSION" <<'PY'
import json
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
version = sys.argv[2]


def replace_first(path: pathlib.Path, pattern: str, replacement: str) -> None:
    text = path.read_text(encoding="utf-8")
    new_text, count = re.subn(pattern, replacement, text, count=1, flags=re.MULTILINE)
    if count != 1:
        raise SystemExit(f"failed to update version in {path}")
    path.write_text(new_text, encoding="utf-8")


(root / "VERSION").write_text(version + "\n", encoding="utf-8")
replace_first(root / "Cargo.toml", r'^version\s*=\s*"[^"]+"', f'version = "{version}"')
replace_first(root / "deecodex-gui" / "Cargo.toml", r'^version\s*=\s*"[^"]+"', f'version = "{version}"')

tauri_path = root / "deecodex-gui" / "tauri.conf.json"
data = json.loads(tauri_path.read_text(encoding="utf-8"))
data["version"] = version
tauri_path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
PY

echo "release version set to $VERSION"
