# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 交流语言

与本仓库的所有交互必须使用中文（简体中文），包括代码注释、commit 信息、PR 描述以及对话回复。

## 当前分区

你正在 **功能/使用帮助** 分区工作，负责使用帮助文档。

**只修改这些文件：**
- `deecodex-gui/gui/js/panels-core.js` — 帮助面板渲染逻辑
- `gui/nav/08-使用帮助.html` — 导航栏片段

**注意：** 本分区无核心 Rust 模块，主要维护帮助文档和导航片段。

**排查 bug 时可以阅读任何分区的代码。修改仅限本分区文件。**

**验证方式（编译通过不算完成，必须启动 GUI 看效果）：**
- 编译: `cargo build`
- **必须启动 GUI 看效果：** 前端变更需启动 GUI 确认页面渲染正确、无报错，不能只编译
- 导航片段变更可刷新 Tauri 窗口看到效果
## Build & Test

```
cargo build
cargo build --release
cargo test
cargo fmt --check
```

**注意：** 本分区无核心 Rust 模块，通常只需改 HTML。`cargo build` 用于确认改动不破坏编译。
## Architecture

deecodex is a proxy that translates Codex CLI's **OpenAI Responses API** requests into **Chat Completions API** calls for DeepSeek/OpenRouter. The core translation is two-way:

- `translate.rs` → Requests: Responses API → Chat Completions API
- `stream.rs` → Responses: Chat Completions SSE stream → Responses API SSE events

### Crate structure

- `src/lib.rs` — Library crate root. Exposes all public modules (`handlers`, `translate`, `stream`, `session`, `cache`, `files`, `prompts`, `vector_stores`, `executor`, `types`, `utils`, `sse`, `ratelimit`, `metrics`, `token_anomaly`). Integration tests import from `deecodex::`.
- `src/main.rs` — Separate binary crate. Owns: service management (start/stop/restart/status/logs subcommands), daemonization, tracing init, `.env` loading, TUI launch, Codex config injection.
- `deecodex-gui/` — Tauri 2 desktop GUI crate. Contains Tray, IPC commands (`src/commands.rs`, `src/commands/`), and webview frontend (`gui/`).
- `deecodex-plugins/` — Plugin host crate: install/uninstall/enable/disable, subprocess management, JSON-RPC communication.

Modules NOT in lib.rs (binary-only): `config`, `tui`, `codex_config`.

### Request flow

1. `main.rs` builds `AppState` (monolithic shared state via `Arc`), calls `handlers::build_router()`, serves via axum.
2. `handlers.rs` (`/v1/responses` POST) validates auth, checks cache, calls `translate::to_chat_request()`.
3. For streaming: `stream::translate_stream()` spawns a tokio task that reads upstream SSE, emits Responses SSE events through a `tokio::sync::mpsc::Sender`.
4. Non-streaming: collects all events, returns JSON response.
5. Vision requests (images) are optionally routed to a separate upstream.

### State management

- `AppState` in `handlers.rs` holds everything: sessions, upstream URL, API keys, model map, cache, vision config, files, vector stores, prompts, rate limiter, metrics, token tracker, tool policy, background tasks.
- Caches use `Arc<DashMap<K, V>>` for concurrent access.
- Sessions/reasoning are in-memory only (lost on restart; Codex replays full conversation).
- `EvictingMap` pattern: `Arc<Mutex<VecDeque<K>>>` for bounded LRU eviction.
- Local executor settings are parsed in `executor.rs` and stored on `AppState`; MCP stdio and computer execution are default-disabled, run only when configured/allowed.

### Key modules

| Module | Purpose |
|--------|---------|
| `handlers` | Axum router, all HTTP handlers, `AppState`, middleware |
| `translate` | Request direction: Responses → Chat translation |
| `stream` | Response direction: SSE stream translation |
| `files` | Local Files API with search index |
| `vector_stores` | Local Vector Store API |
| `prompts` | Hosted prompts registry |
| `sse` | SSE event builder helpers |
| `session` | In-memory session/conversation store |
| `types` | Request/response types |
| `cache` | LRU request cache |
| `utils` | Merge/truncation helpers |
| `token_anomaly` | Token usage anomaly detection |
| `config` | Args struct, config persistence, merge logic |
| `executor` | Local computer/MCP executor config, Playwright action adapter |
| `metrics` | Prometheus metrics |
| `codex_config` | Codex config.toml injection/removal |
| `ratelimit` | Sliding-window rate limiter |
| `accounts` | Multi-account management, presets, CRUD |
| `validate` | 15-item diagnostics engine |
| `cdp` | Chrome DevTools Protocol client |
| `inject` | CDP page injection (auth switching, session delete UI) |
| `codex_threads` | Codex thread aggregation, migration, restore |
| `request_history` | Request history store, monthly stats |

## deecodex-gui 前端规则

**deecodex-gui 是 Tauri 桌面应用，不支持将 `gui/index.html` 当作独立网页使用。** `file://` 打开仅用于静态排版检查，不能作为功能测试依据。

真实功能测试必须使用：

```bash
cd /Users/liguan/deecodex
cargo tauri dev
```

### 前端结构

```
gui/
  index.html              # 只保留页面骨架和 <script> 资源加载，不写业务逻辑
  css/
    app.css               # 全局样式
  js/
    tauri-api.js          # Tauri IPC 边界：DeeCodexTauri 环境判断 + invoke 封装
    ui-core.js            # toast、confirm、转义、deeStorage 封装
    app-shell.js          # 初始化、loadNav()、renderPanel() 分发
    theme-config.js       # 主题、配置 schema
    service-management.js # 服务启停、重启、CDP、升级
    log-viewer.js         # 日志弹窗、刷新、清空、解析
    panels-core.js        # 状态、配置、诊断、帮助面板
    setup-wizard.js       # 首次配置引导
    formatters.js         # 通用格式化函数
    request-history.js    # 请求历史面板
    threads.js            # 线程聚合面板
    accounts.js           # 账号管理面板
    plugins.js            # 插件管理面板
    placeholder-pages.js  # DEX助手、个人中心占位
    startup.js            # DOM 事件入口
```

### 前端约定

- **不把大段 JS/CSS 写回 `index.html`**，放到 `gui/js/<feature>.js` 或 `gui/css/app.css`
- **不直接调用 `window.__TAURI__`**，统一走 `DeeCodexTauri.invoke(name, args)`
- **不直接访问 `localStorage`**，统一走 `deeStorage`（浏览器安全策略下自动降级为内存存储）
- **不为 `file://` 预览模式牺牲正式 Tauri GUI 逻辑**
- **非 Tauri 环境直接显示阻断页**，不做假数据或静默降级
- **新增 Tauri command 优先拆到 `src/commands/<feature>.rs`**（已有 `logs.rs`）
- **改完代码必须启动 GUI 实际测试**，编译通过不算完成

## Configuration System

Three config sources, merged at startup:

1. **Environment variables** — `DEECODEX_*` (backward compat `CODEX_RELAY_*` → `DEECODEX_*` mapping in main.rs)
2. **CLI args** — via clap derive (`Args` in `config.rs`)
3. **`config.json`** — persisted to `~/.deecodex/config.json`

Merge rule: CLI/env values override file values only when they differ from hardcoded defaults (see `pick()`, `pick_str()`, `pick_f64()` in `config.rs`). `.env` is loaded from CWD, `~/.deecodex/`, or exe directory.

Service management subcommands (`start`, `stop`, `restart`, `status`, `logs`) are handled before tracing init and before config merge. They fork the process as a daemon, write a PID file to the data dir, and manage the Codex config injection lifecycle.

## Codex Config Injection

On startup, `codex_config::inject()` writes into `~/.codex/config.toml` to route Codex through deecodex:
- Sets `model_provider = "custom"` and `[model_providers.custom]` section with `base_url = http://127.0.0.1:{port}/v1`
- Uses `toml_edit` for non-destructive TOML editing (preserves other config).
- On shutdown/SIGTERM/SIGINT, `codex_config::remove()` cleans up the injected section.

## Testing Conventions

- Integration tests in `tests/integration.rs`: Build the Axum router via `build_router()`, send requests with `tower::ServiceExt::oneshot`, mock upstreams with raw `tokio::net::TcpListener` + `tokio::io` write.
- Unit tests inline in source modules (e.g., `ratelimit.rs`).
- No mocking framework — raw TCP sockets simulate upstream responses.
- Test helper `test_state()` constructs a fully wired `AppState`.
- `tests/compat_deepseek_v4_pro.rs` requires `DEEPSEEK_API_KEY` env var to run.

## Conventions

- **Concurrency:** `DashMap` for shared maps, `Arc<Mutex<VecDeque>>` for bounded queues. Tokio async runtime.
- **Error handling:** `anyhow` for internal errors. Custom error types in `files.rs`, `prompts.rs`, `vector_stores.rs` implement `IntoResponse`.
- **Logging:** `tracing` with `tracing-subscriber` env-filter. Daemon mode writes to log file; foreground writes to stderr. Default filter: `deecodex=info`.
- **Dynamic JSON:** `serde_json::Value` used extensively for API translation — fields are manipulated dynamically rather than through strict struct deserialization.
- **路径绝对化：** `data_dir` 等所有目录配置在使用前必须转为绝对路径。clap `default_value` 可能产生相对路径（如 `.deecodex`），不同启动目录下指向不同位置，导致账号/配置/日志静默分离。`load_args()` 中统一用 `config::home_dir()` 转换。
- **静默失败加日志：** 关键分支（如托盘菜单构建、账号加载、文件读取）返回空结果时要打 `tracing::warn!`，不静默跳过。
- **跨目录启动测试：** 编译后从不同目录启动二进制验证路径解析是否正常。

## 功能分区与提交规范

项目通过 git worktree 划分为 11 个功能分区 + 3 个编译分区，每个分区独立开发、独立提交。

### 11 个功能分区（后端归属 + GUI 归属）

| 分区 | 后端归属 | GUI 归属 |
|------|---------|---------|
| 功能/服务概览 | `deecodex-gui/src/lib.rs`；服务启停/状态/CDP/升级/logs 相关 commands | `gui/js/service-management.js`；`gui/js/log-viewer.js`；`gui/nav/01-服务概览.html` |
| 功能/协议配置 | `src/translate.rs`；`src/stream.rs`；`src/handlers.rs`；`src/sse.rs`；`src/types.rs`；`src/utils.rs` | `gui/nav/02-协议配置.html`；必要时 `panels-core.js` 中配置面板 |
| 功能/执行诊断 | `src/files.rs`；`src/vector_stores.rs`；`src/prompts.rs`；`src/executor.rs` | `gui/js/panels-core.js` 中诊断页；`gui/nav/03-执行诊断.html` |
| 功能/账号管理 | `src/accounts.rs`；`src/config.rs`；`src/validate.rs`；`src/codex_config.rs`；`src/cdp.rs`；`src/inject.rs`；`src/session.rs`；`src/cache.rs`；`src/backup_store.rs`；`src/ratelimit.rs`；`src/metrics.rs`；`src/token_anomaly.rs` | `gui/js/accounts.js`；`gui/nav/04-账号管理.html` |
| 功能/请求历史 | `src/request_history.rs` | `gui/js/request-history.js`；`gui/nav/05-请求历史.html` |
| 功能/线程聚合 | `src/codex_threads.rs` | `gui/js/threads.js`；`gui/nav/06-线程聚合.html` |
| 功能/插件管理 | `deecodex-plugins/`；插件相关 Tauri commands | `gui/js/plugins.js`；`gui/nav/07-插件管理.html` |
| 功能/使用帮助 | — | `gui/js/panels-core.js` 中帮助页；`gui/nav/08-使用帮助.html` |
| 功能/DEX助手 | — | `gui/js/placeholder-pages.js`；`gui/nav/09-DEX助手.html` |
| 功能/个人中心 | — | `gui/js/placeholder-pages.js`；`gui/nav/10-个人中心.html` |
| 功能/Windows兼容 | `deecodex.bat`；`install.ps1`；`#[cfg(target_os = "windows")]` 代码块 | `tauri.conf.json` Windows 打包；icons；无导航片段 |

### GUI 共享层

以下文件属于共享架构层，**不归单一业务分区独占**：

- `deecodex-gui/gui/index.html`
- `deecodex-gui/gui/css/app.css`
- `deecodex-gui/gui/js/ui-core.js`
- `deecodex-gui/gui/js/tauri-api.js`
- `deecodex-gui/gui/js/theme-config.js`
- `deecodex-gui/gui/js/app-shell.js`
- `deecodex-gui/gui/js/startup.js`
- `deecodex-gui/gui/js/panels-core.js`
- `deecodex-gui/gui/js/formatters.js`
- `deecodex-gui/gui/js/setup-wizard.js`
- `deecodex-gui/build.rs`
- `deecodex-gui/tauri.conf.json`

**修改共享层时：**
- 优先在父区 `deecodex-gui` 做；
- 提交说明写清影响范围；
- 合入后立刻同步所有功能 worktree。

### 提交前缀

- `feat:` — 新功能
- `fix:` — 修复
- `refactor:` — 重构
- `docs:` — 文档
- `chore:` — 杂项/构建
- `release:` — 发版

### 工作流

1. 在对应 worktree 目录开发，只改自己分区覆盖的文件（排查 bug 时可以阅读任何分区代码）
2. 提交使用中文，前缀 + 简短描述
3. 推到自己的分支：`git push deecodex-new 功能/<分区名>`
4. 回主工作区 `cd /Users/liguan/deecodex` 合入：`git merge 功能/<分区名>`
5. 推送主干：`git push deecodex-new deecodex-gui`
6. 同步其他 worktree：`for b in ...; do git -C "功能/$b" merge deecodex-gui; done`

### 导航栏修改

导航栏采用 `build.rs` 自动拼接 `gui/nav/*.html` 生成 `fragments.js`。每个分区只改自己的片段文件，合入时不冲突。不要在功能分区中修改 `gui/js/app-shell.js` 的 `loadNav()`/`renderPanel()` 分发逻辑，除非正在做 GUI 架构调整。
