#!/bin/bash
DIR="$(cd "$(dirname "$0")" && pwd)"
APP="$DIR/../MacOS/deecodex-gui"
if [ ! -f "$APP" ]; then
    # 如果在 .app bundle 外部运行，尝试从 /Applications 找
    APP="/Applications/deecodex.app/Contents/MacOS/deecodex-gui"
fi
if [ -f "$APP" ]; then
    exec "$APP"
else
    echo "未找到 deecodex-gui，请确保已安装到 /Applications"
    read -p "按回车键退出..."
    exit 1
fi
