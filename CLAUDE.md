# CLAUDE.md

## 交流语言

与本仓库的所有交互使用中文。

## 当前工作区

你正在 **主开发区** `~/deecodex` 工作，对应主分支 `deecodex-gui`。

当前只保留这一套固定工作区。`稳定版`、`编译-mac`、`编译-win`、`编译-linux` 等长期分区已取消，不再作为固定目录维护。

主开发区用于日常开发、共享层修改、跨模块修复、集成验证和发版前合入。需要隔离开发时，按任务临时创建 `功能/<任务名>` worktree，完成合入后删除。

## 修改边界

- 小改动、共享层改动、跨模块修复优先在主开发区完成。
- 大功能或高风险实验可临时创建 `功能/<任务名>` worktree。
- 发版构建也从主开发区完成，不再维护固定编译 worktree。
- 排查 bug 可以阅读全仓库；提交前确认变更范围与任务目标一致。

## Build & Test

```bash
cargo build
cargo build --release
cargo test --all-targets
cargo clippy -- -D warnings
cargo fmt --check
```

运行 `cargo build` 前先检查并发构建：

```bash
pgrep -x cargo
```

若已有 cargo 进程在运行，等待结束后再构建。

## deecodex-gui 前端规则

`deecodex-gui` 是 Tauri 桌面应用，不支持将 `gui/index.html` 当作独立网页使用。

真实功能测试使用（GUI 验证不强制在本机执行；如确需 GUI 实测，在局域网 Mac mini `<LAN_MAC_MINI>:22` 上进行）：

```bash
cd ~/deecodex
cargo tauri dev
```

前端约定：

- 不把大段 JS/CSS 写回 `index.html`。
- 不直接调用 `window.__TAURI__`，统一走 `DeeCodexTauri.invoke(name, args)`。
- 不直接访问 `localStorage`，统一走 `deeStorage`。
- 不为 `file://` 预览模式做兜底或假数据。
- 非 Tauri 环境直接显示阻断页。
- GUI 验证不强制在本机执行，也不禁止使用 GUI 验证：编译通过不算视觉验证完成；如确需 GUI 实测，使用局域网 Mac mini（<LAN_MAC_MINI>:22）验证。

## 跨平台适配与验证

本项目为单仓库单主分支跨平台（macOS 本机开发 + Windows 适配同分支），平台差异必须隔离：

- Rust 平台专属逻辑一律用 `#[cfg(target_os = "macos")]` / `#[cfg(target_os = "windows")]` 隔离；
  Windows 专属依赖放在 `deecodex-gui/Cargo.toml` 的 `[target.'cfg(windows)'.dependencies]`。
- 平台专属文件独立存放（如 `tauri.windows.conf.json`、`icons/icon.ico`、`icons/tray-dex-win.rgba`、
  `scripts/build-gui-release.ps1`）；不把 Windows 逻辑塞进共享文件。
- 共享前端文件（`gui/index.html`、`gui/css/*`、`gui/js/*`）里的平台差异用
  `html[data-platform="windows"]` / `navigator.userAgent` 判断，且只加平台分支，不改共用流程；
  CSS 中平台专属样式（如拖拽条留位）必须限定在平台选择器下，避免影响 macOS。
- Windows 适配由另一台机器直推同一分支：本机开发前/提交前先 `git pull --ff-only`，
  避免共享文件叠加冲突；合入前确认 merge-base 关系，能 fast-forward 时优先 fast-forward。

平台验证门禁（合入前必须满足对应平台验证）：

- Windows 改动：在 Windows 上 `cargo build -p deecodex-gui` 编译通过，并启动冒烟
  （无边框窗口/自定义窗口控制、托盘、单实例互斥体、插件进程不弹黑框）。
- macOS 改动：本机 `cargo build` / `cargo tauri dev` 验证，必要时用局域网 Mac mini 实测。
- 共享文件改动：两端平台各回归一次，确认平台分支生效且对方平台不受影响。

本机 debug 与安装版共存规则（Mac mini 不可达时的默认验证路径）：

- debug 构建（`cargo tauri dev` / `target/debug/deecodex-gui`）默认允许多实例，
  可与已安装的 DEX AI.app 共存，无需设置环境变量（`lib.rs` 单实例判定：
  `allow_multi_instance = cfg!(debug_assertions) || env(...)`）。
- 端口：`port_auto` 开启时网关自动避让安装版占用的端口，不冲突。
- 数据目录：debug 非 preview 模式共用 `~/.deecodex`，验证时只做只读操作，
  **不得**在 debug 版里执行停止服务/重启/卸载等破坏性操作，避免影响安装版实例。
- Mac mini 不可达（`<LAN_MAC_MINI>:22` 不通）时，用本机 debug 版作为 GUI 验证 fallback：
  `cd ~/deecodex && cargo tauri dev`，确认窗口启动与目标面板渲染。

注意：`tauri.windows.conf.json` 使用独立的 updater 签名公钥（Windows/macOS 不同密钥对），
Windows 私钥必须妥善备份，丢失后无法向 Windows 客户端发布更新。

## 模块归属

详细模块归属、临时 worktree 流程和发版编译工作区见 `WORKTREES.md`。

高层规则：

- 协议翻译：`src/translate.rs`、`src/stream.rs`、`src/handlers.rs`、`src/sse.rs`、`src/types.rs`、`src/utils.rs`。
- 本地能力与诊断：`src/files.rs`、`src/vector_stores.rs`、`src/prompts.rs`、`src/executor.rs`、`src/validate.rs`。
- 账号、配置与会话：`src/accounts.rs`、`src/config.rs`、`src/codex_config.rs`、`src/session.rs`、`src/cache.rs`、`src/ratelimit.rs`、`src/metrics.rs`、`src/token_anomaly.rs`。
- GUI：`deecodex-gui/src/`、`deecodex-gui/gui/js/`、`deecodex-gui/gui/nav/`、`deecodex-gui/gui/css/`。
- 插件：`deecodex-plugins/` 与插件相关 GUI/IPC。

## 发版与清理

- 发版时先同步 `Cargo.toml`、`deecodex-gui/Cargo.toml`、`deecodex-gui/tauri.conf.json`、`VERSION`。
- 只保留 `deecodex-new` 作为源码主远程；`origin` 已移除，不再作为日常开发或发版目标。
  - 注：`deecodex-new` 是本地 remote 别名，2026-08-19 起实际指向
    `github.com/liguan-89/dexai-core`（原 `deecodex-new` 已更名）。
- 发版完成后必须同步公开仓库 GitHub Release（安装包分发）：
  `./scripts/publish-github-release.sh <version>`，目标仓库 `liguan-89/DEXAI`。
  完整流程见 `docs/APP_UPDATER_RELEASE.md`。
- `功能/` 下不长期保留空闲 worktree，合入后立即清理。
- 不再创建固定的 `稳定版`、`编译-*` 分区。

## 提交

中文 commit，前缀使用：

- `feat:` — 新功能
- `fix:` — 修复
- `refactor:` — 重构
- `docs:` — 文档
- `chore:` — 杂项/构建
- `release:` — 发版

## GUI Release 构建

GUI release 构建需要校验第一方插件包哈希。使用仓库脚本会按
`deecodex-gui/build.rs` 的同一算法自动计算并注入哈希：

```bash
./scripts/build-gui-release.sh
```
