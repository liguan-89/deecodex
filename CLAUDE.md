# CLAUDE.md

## 交流语言

与本仓库的所有交互使用中文。

## 当前分区

你正在 **功能/账号管理** 分区工作，负责多账号、配置、会话、缓存、速率限制等基础设施。

**只修改这些文件：**
- `src/accounts.rs` — 多账号管理
- `src/config.rs` — 配置系统
- `src/validate.rs` — 诊断引擎
- `src/codex_config.rs` / `src/cdp.rs` / `src/inject.rs` — Codex/CDP 注入
- `src/session.rs` / `src/cache.rs` / `src/backup_store.rs` — 会话/缓存
- `src/ratelimit.rs` / `src/metrics.rs` / `src/token_anomaly.rs` — 限流/指标
- `deecodex-gui/gui/js/accounts.js` — 账号管理面板
- `gui/nav/04-账号管理.html` — 导航栏片段

排查 bug 时可以阅读任何分区代码。修改仅限本分区文件。

## Build & Test

```
cargo build
cargo build --release
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```


## 前端规则

- **Tauri-only**，不测 `file://`。非 Tauri 环境直接阻断
- **不把 JS/CSS 写回 `index.html`**，放 `gui/js/<feature>.js` 或 `gui/css/app.css`
- 统一走 `DeeCodexTauri.invoke()`，所有 IPC 自动 trace
- 统一走 `window.deeStorage`（即 `localStorage`）
- `confirm()` 在 WebView 中不可靠，用 `showConfirm()`
- **改完必须启动 GUI 实测**，编译通过不算完成

## Bug 定位速查（按顺序）

1. `invoke('debug_gui_state')` 确认环境
2. 控制台看 `[ipc:start]` / `[ipc:ok]` / `[ipc:error]` trace
3. 检查 `generate_handler![]` 是否注册了对应 command
4. 检查文件/DB 是否真的被修改
5. 检查前端过滤/解析是否误判（空状态、BOM 等）

## 提交

中文 commit，前缀: `feat:` / `fix:` / `refactor:` / `docs:` / `chore:` / `release:`
只改本分区覆盖的文件。改共享层去父区 `/Users/liguan/deecodex` 做。

完整架构、配置系统、模块说明、测试约定见父区: /Users/liguan/deecodex/CLAUDE.md
