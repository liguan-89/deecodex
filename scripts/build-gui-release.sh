#!/bin/zsh
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
export repo_root

eval "$(
  python3 - <<'PY'
import hashlib
import os
from pathlib import Path

root = Path(os.environ["repo_root"])


def package_hash(package: Path) -> str:
    digest = hashlib.sha256()
    entries = sorted(package.rglob("*"), key=lambda path: path)
    for path in entries:
        if ".git" in path.parts or path.name == ".DS_Store":
            continue
        relative = path.relative_to(package).as_posix().encode()
        digest.update(relative)
        if path.is_dir():
            digest.update(b"/")
        else:
            digest.update(b"\0")
            digest.update(path.read_bytes())
    return digest.hexdigest()


packages = {
    "DEECODEX_MOBILE_CODEX_PLUGIN_SHA256": root
    / "deecodex-plugins/plugins/deecodex-mobile-codex",
    "DEECODEX_DESKTOP_SYNC_PLUGIN_SHA256": root
    / "deecodex-plugins/plugins/deecodex-desktop-sync",
    "DEECODEX_DEBUG_PLUGIN_SHA256": root
    / "deecodex-plugins/plugins/deecodex-debug",
}

for variable, package in packages.items():
    if not package.is_dir():
        raise SystemExit(f"插件目录不存在：{package}")
    print(f'export {variable}="{package_hash(package)}"')
PY
)"

cd "$repo_root"
# 先构建 core 二进制（deecodex，src/main.rs）。只构建 GUI 不会更新它，
# 导致 target-local/release/deecodex 停留在旧版本，部署/验证时跑旧逻辑。
cargo build -p deecodex --release
exec cargo build -p deecodex-gui --release "$@"
