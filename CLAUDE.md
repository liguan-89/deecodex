# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 交流语言

与本仓库的所有交互必须使用中文（简体中文），包括代码注释、commit 信息、PR 描述以及对话回复。

## 当前分区

你正在 **功能/使用帮助** 分区工作，负责使用帮助文档。

**只修改这些文件：**
- `gui/nav/08-使用帮助.html` — 导航栏片段

**注意：** 本分区无核心 Rust 模块，主要维护帮助文档和导航片段。如需修改前端展示内容，在 `deecodex-gui/gui/index.html` 中「使用帮助」面板区域修改。

**禁止修改其他分区的导航片段。**

**验证方式：**
- 编译: `cargo build`
- 前端变更无需编译，刷新 Tauri 窗口即可看到效果

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
- `src/main.rs` — Separate binary crate. Owns: service management (start/stop/restart/status/logs subcommands), daemonization, tracing init, `.env` loading, TUI launch, Codex config injection. Has its own `mod` declarations that include lib.rs modules plus `codex_config`, `config`, and `tui`.

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
- Local executor settings are parsed in `executor.rs` and stored on `AppState`; MCP stdio and computer execution are default-disabled, run only when configured/allowed, and return `mcp_tool_call_output` / `computer_call_output` instead of surfacing executor failures as HTTP 500.

### Key modules (largest → smallest)

| Module | Lines | Purpose |
|--------|-------|---------|
| `handlers` | 2,561 | Axum router, all HTTP handlers, `AppState`, middleware |
| `translate` | 1,580 | Request direction: Responses → Chat translation |
| `stream` | 1,477 | Response direction: SSE stream translation |
| `files` | 1,203 | Local Files API with search index |
| `tui` | 1,160 | Terminal UI config menu (ratatui) |
| `vector_stores` | 761 | Local Vector Store API |
| `prompts` | 711 | Hosted prompts registry |
| `sse` | 676 | SSE event builder helpers |
| `session` | 644 | In-memory session/conversation store |
| `types` | 545 | Request/response types |
| `cache` | 418 | LRU request cache |
| `utils` | 224 | Merge/truncation helpers |
| `token_anomaly` | 205 | Token usage anomaly detection |
| `config` | 204 | Args struct, config persistence, merge logic |
| `executor` | ~650 | Local computer/MCP executor config, Playwright action adapter, and stdio MCP JSON-RPC tool execution |
| `metrics` | 180 | Prometheus metrics |
| `codex_config` | 106 | Codex config.toml injection/removal |
| `ratelimit` | 90 | Sliding-window rate limiter |

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

- Integration tests in `tests/integration.rs` (3,235 lines): Build the Axum router via `build_router()`, send requests with `tower::ServiceExt::oneshot`, mock upstreams with raw `tokio::net::TcpListener` + `tokio::io` write.
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

项目通过 git worktree 划分为 10 个功能分区，每个分区独立开发、独立提交。

### 10 个功能分区

| 分区 | 覆盖模块 | 导航片段 |
|------|---------|---------|
| 功能/服务概览 | `deecodex-gui/` Tauri 应用 | `gui/nav/01-服务概览.html` |
| 功能/协议配置 | `translate.rs`, `stream.rs`, `handlers.rs`, `sse.rs`, `types.rs`, `utils.rs` | `gui/nav/02-协议配置.html` |
| 功能/执行诊断 | `files.rs`, `vector_stores.rs`, `prompts.rs`, `executor.rs` | `gui/nav/03-执行诊断.html` |
| 功能/账号管理 | `accounts.rs`, `config.rs`, `validate.rs`, `codex_config.rs`, `cdp.rs`, `inject.rs`, `session.rs`, `cache.rs`, `backup_store.rs`, `ratelimit.rs`, `metrics.rs`, `token_anomaly.rs` | `gui/nav/04-账号管理.html` |
| 功能/请求历史 | `request_history.rs` | `gui/nav/05-请求历史.html` |
| 功能/线程聚合 | `codex_threads.rs` | `gui/nav/06-线程聚合.html` |
| 功能/插件管理 | `deecodex-plugins/` | `gui/nav/07-插件管理.html` |
| 功能/使用帮助 | — | `gui/nav/08-使用帮助.html` |
| 功能/DEX助手 | — | `gui/nav/09-DEX助手.html` |
| 功能/个人中心 | — | `gui/nav/10-个人中心.html` |
| 功能/Windows兼容 | `deecodex.bat`, `install.ps1`, `tauri.conf.json` 等 | 无（跨平台底层修复） |

### 提交前缀

- `feat:` — 新功能
- `fix:` — 修复
- `refactor:` — 重构
- `docs:` — 文档
- `chore:` — 杂项/构建
- `release:` — 发版

### 工作流

1. 在对应 worktree 目录开发，只改自己分区覆盖的文件
2. 提交使用中文，前缀 + 简短描述
3. 推到自己的分支：`git push deecodex-new 功能/<分区名>`
4. 回主工作区 `cd /Users/liguan/deecodex` 合入：`git merge 功能/<分区名>`
5. 推送主干：`git push deecodex-new deecodex-gui`
6. 同步其他 worktree：`for b in ...; do git -C "功能/$b" merge deecodex-gui; done`

### 导航栏修改

导航栏采用 `build.rs` 自动拼接 `gui/nav/*.html` 生成 `fragments.js`。每个分区只改自己的片段文件，合入时不冲突。不要修改其他分区的片段文件，不要修改 `index.html` 中的 `loadNav()` 逻辑。
