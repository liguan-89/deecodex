# DEX AI 应用内更新发布

DEX AI GUI 使用 Tauri v2 updater 做应用内更新。更新源是静态 HTTPS 文件，可以放在阿里云 OSS、Nginx 或任意 HTTPS 静态站点。

## 更新源

默认更新清单：

```text
https://api.liguan.me/releases/dex-ai/latest.json
```

推荐服务器目录：

```text
/releases/dex-ai/latest.json
/releases/dex-ai/<version>/mac/*.app.tar.gz
/releases/dex-ai/<version>/mac/*.app.tar.gz.sig
/releases/dex-ai/<version>/mac/*.dmg
/releases/dex-ai/<version>/windows/*setup.exe
/releases/dex-ai/<version>/windows/*setup.exe.sig
```

`latest.json` 里的 `signature` 必须是 `.sig` 文件内容，不是 `.sig` 文件 URL。

## 本机签名 key

Tauri updater 需要签名校验。当前项目只保存公钥，私钥保存在本机：

```text
~/.tauri/dex-ai-updater.key
~/.tauri/dex-ai-updater.key.password
```

不要把私钥或密码提交到仓库。

### Windows 独立构建机

Windows 版本在独立机器上适配时，也必须使用同一把 updater 私钥签名，否则同一个 `latest.json` 清单无法同时服务 macOS 和 Windows 更新包。

推荐做法：

1. 线下安全拷贝这两个文件到 Windows 构建机，不经过 Git：

```text
%USERPROFILE%\.tauri\dex-ai-updater.key
%USERPROFILE%\.tauri\dex-ai-updater.key.password
```

2. PowerShell 构建时注入：

```powershell
cd C:\path\to\deecodex\deecodex-gui
$env:TAURI_SIGNING_PRIVATE_KEY=(Get-Content "$env:USERPROFILE\.tauri\dex-ai-updater.key" -Raw).Trim()
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD=(Get-Content "$env:USERPROFILE\.tauri\dex-ai-updater.key.password" -Raw).Trim()
cargo tauri build
```

3. 如果走 GitHub Actions 或其他 CI，只把私钥内容和密码放进 Secret，例如：

```text
TAURI_SIGNING_PRIVATE_KEY
TAURI_SIGNING_PRIVATE_KEY_PASSWORD
```

仓库只保存读取方式和公钥配置，不保存私钥本体。

## 发布纪律

正式版发布只允许走脚本，不手改 `latest.json`，不手工复制旧包进新版本目录。

发布前必须确认四个版本源一致：

```text
VERSION
Cargo.toml
deecodex-gui/Cargo.toml
deecodex-gui/tauri.conf.json
```

统一改版本号只使用：

```bash
cd /Users/liguan/deecodex
./scripts/set-release-version.sh 3.6.13
```

如果使用独立编译区发包，编译区也必须执行同一条命令，不能只改主区。

## 构建

构建 updater artifact 时需要设置签名私钥和密码：

```bash
cd /Users/liguan/deecodex
TAURI_SIGNING_PRIVATE_KEY="$(cat "$HOME/.tauri/dex-ai-updater.key")" \
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(cat "$HOME/.tauri/dex-ai-updater.key.password")" \
cargo tauri build
```

> 注：Tauri CLI 不支持 `--manifest-path`，构建必须在 `deecodex-gui/` 子目录执行，或在 workspace 根直接执行 `cargo tauri build`（cargo 会从根 `Cargo.toml` 找到 workspace 成员 `deecodex-gui` 的 `tauri.conf.json`）。

macOS updater 产物是 `.app.tar.gz` 和 `.app.tar.gz.sig`；DMG 只用于手动安装。

构建目录里的 `target-local/release/bundle/macos/DEX AI.app` 只是中间产物（路径由根目录 `.cargo/config.toml` 的 `target-dir` 决定）。发布脚本会给该目录打 `.metadata_never_index`，避免 Spotlight / Launchpad 把构建产物也当成一份已安装应用，导致 Launchpad 里出现多个 DEX AI 图标。

## 生成发布目录

```bash
cd /Users/liguan/deecodex
DEX_AI_UPDATE_BASE_URL="https://api.liguan.me/releases/dex-ai" \
DEX_AI_UPDATE_NOTES_FILE="docs/releases/3.6.13.md" \
./scripts/prepare-updater-release.sh 3.6.13
```

脚本会生成：

```text
dist/updater-release/<version>/
```

`prepare-updater-release.sh` 会强制检查：

- 输出目录先清空，避免旧版本残留。
- updater `.app.tar.gz` 内的 `CFBundleShortVersionString` 必须等于当前发布版本。
- `.sig` 必须是真实 Tauri updater 签名。
- `latest.json` 中的 URL 必须指向当前版本目录。

默认 macOS 更新目标只写入 `darwin-aarch64`。如果已经构建 universal 包，发布时再显式指定：

```bash
DEX_AI_UPDATE_MAC_TARGETS="darwin-aarch64,darwin-x86_64" ./scripts/prepare-updater-release.sh
```

更新说明可以用环境变量直接传入：

```bash
DEX_AI_UPDATE_NOTES=$'新增应用内更新\\n优化支持项目入口' ./scripts/prepare-updater-release.sh
```

## 强制更新

普通版本不要开启强制更新。只有严重 bug、安全问题、路由/会话数据兼容问题这类必须让旧版停止继续使用的版本，才在生成发布目录时显式开启：

```bash
DEX_AI_UPDATE_BASE_URL="https://api.liguan.me/releases/dex-ai" \
DEX_AI_UPDATE_NOTES_FILE="docs/releases/3.10.0.md" \
DEX_AI_FORCE_UPDATE=1 \
DEX_AI_FORCE_UPDATE_REASON="修复严重路由和会话索引问题，旧版本需要立即升级。" \
DEX_AI_MIN_SUPPORTED_VERSION="3.9.10" \
./scripts/prepare-updater-release.sh 3.10.0
```

字段含义：

- `DEX_AI_FORCE_UPDATE=1`：把本次 `latest.json` 标记为强制更新。
- `DEX_AI_FORCE_UPDATE_REASON`：给客户端展示的强制更新原因。
- `DEX_AI_MIN_SUPPORTED_VERSION`：低于该版本的客户端必须更新；等于或高于该版本不强制。为空时，所有低于最新版本的客户端都会被强制更新。

客户端行为：

- 启动后自动检查 `latest.json`。
- 命中强制更新时显示不可关闭的关键更新弹窗。
- 自动下载并安装更新。
- 安装完成后自动重启 DEX AI。
- 下载或安装失败时只允许重试或退出，不进入主流程继续使用旧版。

## 本地验证

发布目录生成后必须先本地验证：

```bash
./scripts/verify-updater-release.sh 3.6.13
```

验证内容包括版本源一致、`latest.json` 版本一致、本地 updater tar 包内版本一致、签名格式有效。

## 上传

```bash
DEX_AI_UPDATE_REMOTE_TARGET="root@39.96.198.228:/var/www/dex-ai/releases/dex-ai" \
./scripts/upload-updater-release.sh 3.6.13
```

默认 SSH key 路径是：

```text
~/Desktop/aliyun.pem
```

需要改 key 时：

```bash
DEX_AI_UPDATE_SSH_KEY="/path/to/key.pem" \
DEX_AI_UPDATE_REMOTE_TARGET="root@your-server:/var/www/dex-ai/releases/dex-ai" \
./scripts/upload-updater-release.sh
```

`upload-updater-release.sh` 会在上传前自动执行本地验证，上传后自动执行远端验证。

远端验证会通过公开更新源检查：

```text
https://api.liguan.me/releases/dex-ai/latest.json
https://api.liguan.me/releases/dex-ai/<version>/mac/DEX AI.app.tar.gz
```

只有远端 `latest.json`、远端 updater tar 包内版本、当前发布版本三者一致，发布才算完成。

上传后，旧版本 DEX AI 的服务概览页点击“检查更新”，应能看到新版本并执行“下载并安装”。

## 用户下载页

除了应用内更新源，服务器还对外提供一个用户下载页，给首次安装 / 手动重装的用户提供 macOS 和 Windows 的下载入口：

```text
https://api.liguan.me/download/
```

- 页面源码在仓库 `site/download/index.html`，单文件零依赖（自带 CSS / Markdown 渲染，无第三方 JS）。
- 部署位置：`/var/www/dex-ai/download/index.html`，由 nginx `location ^~ /download/` 提供。
- 页面运行时按用户 UA 判断平台，并按平台分别拉对应的 Tauri updater 清单：
  - macOS：`/releases/dex-ai/latest.json`，从 `platforms["darwin-aarch64"].url` 取 `.dmg` 链接。
  - Windows：`/releases/dex-ai/windows/latest.json`，从 `platforms["windows-x86_64"].url` 取 `setup.exe` 链接。
- macOS / Windows 用两份独立的 `latest.json`，因为历史上 Windows 包发布在 `/releases/dex-ai/windows/` 子树、Tauri updater 也独立指向这条清单；下载页只是把两边的最新版本号和 notes 拼到一起展示。
- 版本号、release notes、下载链接全部从 `latest.json` 动态读取，发新版本不需要改这个页面。
- 修改页面后只需要把 `site/download/index.html` 覆盖上传到 `/var/www/dex-ai/download/index.html` 即可，nginx 不需要 reload。
