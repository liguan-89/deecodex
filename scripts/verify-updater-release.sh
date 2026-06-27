#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION_FILE="$ROOT_DIR/VERSION"
VERSION=""
VERIFY_REMOTE=false

for arg in "$@"; do
  case "$arg" in
    --remote)
      VERIFY_REMOTE=true
      ;;
    *)
      if [[ -n "$VERSION" ]]; then
        echo "unexpected argument: $arg" >&2
        exit 1
      fi
      VERSION="$arg"
      ;;
  esac
done

VERSION="${VERSION:-$(tr -d '[:space:]' < "$VERSION_FILE")}"
VERSION="${VERSION#v}"

BASE_URL="${DEX_AI_UPDATE_BASE_URL:-https://api.liguan.me/releases/dex-ai}"
OUT_DIR="${DEX_AI_UPDATE_OUT_DIR:-$ROOT_DIR/dist/updater-release/$VERSION}"
SSH_KEY="${DEX_AI_UPDATE_SSH_KEY:-$HOME/Desktop/aliyun.pem}"
REMOTE_TARGET="${DEX_AI_UPDATE_REMOTE_TARGET:-}"

if [[ ! -f "$OUT_DIR/latest.json" ]]; then
  echo "missing release manifest: $OUT_DIR/latest.json" >&2
  exit 1
fi

python3 - "$ROOT_DIR" "$VERSION" "$OUT_DIR" "$BASE_URL" "$VERIFY_REMOTE" "$REMOTE_TARGET" "$SSH_KEY" <<'PY'
import base64
import json
import pathlib
import plistlib
import re
import subprocess
import sys
import tarfile
import tempfile
import urllib.parse

root = pathlib.Path(sys.argv[1])
version = sys.argv[2].strip().lstrip("v")
out_dir = pathlib.Path(sys.argv[3])
base_url = sys.argv[4].rstrip("/")
verify_remote = sys.argv[5] == "true"
remote_target = sys.argv[6]
ssh_key = sys.argv[7]


def read_text(path: pathlib.Path) -> str:
    return path.read_text(encoding="utf-8").strip()


def expect_equal(label: str, actual: str, expected: str) -> None:
    if actual != expected:
        raise SystemExit(f"{label} mismatch: expected {expected}, got {actual}")


def cargo_version(path: pathlib.Path) -> str:
    text = path.read_text(encoding="utf-8")
    match = re.search(r'(?m)^version\s*=\s*"([^"]+)"\s*$', text)
    if not match:
        raise SystemExit(f"missing version in {path}")
    return match.group(1).strip().lstrip("v")


def tauri_version(path: pathlib.Path) -> str:
    data = json.loads(path.read_text(encoding="utf-8"))
    return str(data.get("version", "")).strip().lstrip("v")


def tarball_version(path: pathlib.Path) -> str:
    with tarfile.open(path, "r:gz") as tf:
        plist_name = next((name for name in tf.getnames() if name.endswith("Contents/Info.plist")), None)
        if plist_name is None:
            raise SystemExit(f"missing Info.plist in {path}")
        plist = plistlib.loads(tf.extractfile(plist_name).read())
        return str(plist.get("CFBundleShortVersionString", "")).strip().lstrip("v")


def validate_signature(sig: str) -> None:
    try:
        decoded = base64.b64decode(sig.strip(), validate=True).decode("utf-8", "replace")
    except Exception as exc:
        raise SystemExit(f"invalid updater signature: {exc}")
    if "signature from tauri secret key" not in decoded:
        raise SystemExit("invalid updater signature: not a Tauri updater signature")


def curl_bytes(url: str) -> bytes:
    parsed = urllib.parse.urlsplit(url)
    url = urllib.parse.urlunsplit((
        parsed.scheme,
        parsed.netloc,
        urllib.parse.quote(parsed.path, safe="/"),
        parsed.query,
        parsed.fragment,
    ))
    try:
        return subprocess.check_output(["curl", "-fsSL", url], timeout=180)
    except subprocess.CalledProcessError as exc:
        raise SystemExit(f"curl failed for {url}: exit {exc.returncode}") from exc
    except subprocess.TimeoutExpired as exc:
        raise SystemExit(f"curl timed out for {url}") from exc


def ssh_check_remote_tarball(remote_path: str, desired: str) -> str:
    host, directory = remote_target.split(":", 1)
    code = r'''
set -euo pipefail
python3 - <<'REMOTE_PY'
import os
import plistlib
import tarfile

tarball = os.environ["DEX_AI_REMOTE_TARBALL"]
desired = os.environ["DEX_AI_REMOTE_VERSION"]
with tarfile.open(tarball, "r:gz") as tf:
    plist_name = next((name for name in tf.getnames() if name.endswith("Contents/Info.plist")), None)
    if plist_name is None:
        raise SystemExit("missing Info.plist in remote updater tarball")
    plist = plistlib.loads(tf.extractfile(plist_name).read())
    actual = str(plist.get("CFBundleShortVersionString", "")).strip().lstrip("v")
    if actual != desired:
        raise SystemExit(f"remote updater tarball version mismatch: expected {desired}, got {actual}")
    print(actual)
REMOTE_PY
'''
    try:
        output = subprocess.check_output(
            [
                "ssh",
                "-i",
                ssh_key,
                host,
                f"DEX_AI_REMOTE_TARBALL={remote_path!r} DEX_AI_REMOTE_VERSION={desired!r} bash -s",
            ],
            input=code.encode("utf-8"),
            timeout=60,
        )
    except subprocess.CalledProcessError as exc:
        raise SystemExit(f"remote ssh verification failed: exit {exc.returncode}") from exc
    except subprocess.TimeoutExpired as exc:
        raise SystemExit("remote ssh verification timed out") from exc
    return output.decode("utf-8", "replace").strip()


expect_equal("VERSION", read_text(root / "VERSION").lstrip("v"), version)
expect_equal("Cargo.toml version", cargo_version(root / "Cargo.toml"), version)
expect_equal("GUI Cargo.toml version", cargo_version(root / "deecodex-gui" / "Cargo.toml"), version)
expect_equal("Tauri version", tauri_version(root / "deecodex-gui" / "tauri.conf.json"), version)

manifest_path = out_dir / "latest.json"
manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
expect_equal("manifest version", str(manifest.get("version", "")).lstrip("v"), version)

force_update = manifest.get("force_update", False)
if not isinstance(force_update, bool):
    raise SystemExit("manifest force_update must be boolean when present")
if "force_update_reason" in manifest and not isinstance(manifest.get("force_update_reason"), str):
    raise SystemExit("manifest force_update_reason must be string when present")
if "minimum_supported_version" in manifest:
    minimum_supported_version = str(manifest.get("minimum_supported_version", "")).strip().lstrip("v")
    if not re.match(r"^\d+(?:\.\d+){1,3}(?:[-+][0-9A-Za-z.-]+)?$", minimum_supported_version):
        raise SystemExit(f"invalid minimum_supported_version: {manifest.get('minimum_supported_version')}")

platforms = manifest.get("platforms") or {}
mac = platforms.get("darwin-aarch64") or platforms.get("darwin-x86_64")
if not isinstance(mac, dict):
    raise SystemExit("missing mac platform in latest.json")

url = str(mac.get("url", ""))
signature = str(mac.get("signature", ""))
validate_signature(signature)

expected_prefix = f"{base_url}/{version}/mac/"
if not url.startswith(expected_prefix):
    raise SystemExit(f"manifest url mismatch: expected prefix {expected_prefix}, got {url}")

local_tar_name = urllib.parse.unquote(pathlib.PurePosixPath(urllib.parse.urlparse(url).path).name)
local_tar = out_dir / "mac" / local_tar_name
if not local_tar.exists():
    raise SystemExit(f"missing local updater tarball: {local_tar}")
expect_equal("local updater tarball version", tarball_version(local_tar), version)

sig_path = pathlib.Path(f"{local_tar}.sig")
if not sig_path.exists():
    raise SystemExit(f"missing local updater signature: {sig_path}")
validate_signature(sig_path.read_text(encoding="utf-8").strip())

if verify_remote:
    remote_manifest_url = f"{base_url}/latest.json"
    remote_manifest = json.loads(curl_bytes(remote_manifest_url).decode("utf-8"))
    expect_equal("remote manifest version", str(remote_manifest.get("version", "")).lstrip("v"), version)
    remote_mac = (remote_manifest.get("platforms") or {}).get("darwin-aarch64") or {}
    remote_url = str(remote_mac.get("url", ""))
    if remote_url != url:
        raise SystemExit(f"remote manifest url mismatch: expected {url}, got {remote_url}")
    validate_signature(str(remote_mac.get("signature", "")))
    if remote_target and ":" in remote_target:
        remote_tar_name = urllib.parse.unquote(pathlib.PurePosixPath(urllib.parse.urlparse(remote_url).path).name)
        remote_path = f"{remote_target.split(':', 1)[1].rstrip('/')}/{version}/mac/{remote_tar_name}"
        expect_equal("remote updater tarball version", ssh_check_remote_tarball(remote_path, version), version)
    else:
        with tempfile.NamedTemporaryFile(suffix=".app.tar.gz") as tmp:
            tmp.write(curl_bytes(remote_url))
            tmp.flush()
            expect_equal("remote updater tarball version", tarball_version(pathlib.Path(tmp.name)), version)

print(f"updater release verified: {version}")
PY
