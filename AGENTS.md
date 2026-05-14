# AGENTS.md

This file provides guidance to Codex (Codex.ai/code) when working with code in this repository.

## 交流语言

与本仓库的所有交互必须使用中文（简体中文），包括代码注释、commit 信息、PR 描述以及对话回复。

## Build & Test

```
cargo build
cargo build --release
cargo test --all-targets
cargo clippy -- -D warnings
cargo fmt --check
```

Run a specific test: `cargo test <test_name>`

**并发构建避免：** `cargo build` 前先 `pgrep -x cargo` 检查是否有其他 cargo 进程在运行。若有，等其结束后再执行。

## Architecture

deecodex is a proxy that translates Codex CLI's **OpenAI Responses API** requests into **Chat Completions API** calls for DeepSeek/OpenRouter. The core translation is two-way:

- `translate.rs` → Requests: Responses API → Chat Completions API
- `stream.rs` → Responses: Chat Completions SSE stream → Responses API SSE events

### Crate structure

- `src/lib.rs` — Library crate root. Exposes all public modules.
- `src/main.rs` — Binary crate. Service management, daemonization, tracing init, `.env` loading, TUI launch, Codex config injection.
- `deecodex-gui/` — Tauri 2 desktop GUI crate. Tray, IPC commands, webview frontend.
- `deecodex-plugins/` — Plugin host crate. Install/uninstall/enable/disable, subprocess management, JSON-RPC communication.

### Request flow

1. `main.rs` builds `AppState`, calls `handlers::build_router()`, serves via axum.
2. `handlers.rs` (`/v1/responses` POST) validates auth, checks cache, calls `translate::to_chat_request()`.
3. For streaming: `stream::translate_stream()` spawns a tokio task that reads upstream SSE, emits Responses SSE events.
4. Non-streaming: collects all events, returns JSON response.
5. Vision requests are optionally routed to a separate upstream.

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
| `executor` | Local computer/MCP executor config |
| `metrics` | Prometheus metrics |
| `codex_config` | Codex config.toml injection/removal |
| `ratelimit` | Sliding-window rate limiter |
| `accounts` | Multi-account management |
| `validate` | 15-item diagnostics engine |
| `cdp` | Chrome DevTools Protocol client |
| `inject` | CDP page injection |
| `codex_threads` | Codex thread aggregation, migration |
| `request_history` | Request history store |

## deecodex-gui 前端规则

**deecodex-gui 是 Tauri 桌面应用，不支持将 `gui/index.html` 当作独立网页使用。**

真实功能测试必须使用：

```bash
cd /Users/liguan/deecodex
cargo tauri dev
```

### 前端结构

```
gui/
  index.html              # 页面骨架和资源加载
  css/app.css             # 全局样式
  js/
    tauri-api.js          # Tauri IPC 边界
    ui-core.js            # toast、confirm、deeStorage
    app-shell.js          # 初始化、loadNav()、renderPanel()
    service-management.js # 服务启停
    log-viewer.js         # 日志弹窗
    panels-core.js        # 状态、配置、诊断、帮助
    request-history.js    # 请求历史
    threads.js            # 线程聚合
    accounts.js           # 账号管理
    plugins.js            # 插件管理
    placeholder-pages.js  # DEX助手、个人中心
```

### 前端约定

- **不把大段 JS/CSS 写回 `index.html`**
- **不直接调用 `window.__TAURI__`**，统一走 `DeeCodexTauri.invoke(name, args)`
- **不直接访问 `localStorage`**，统一走 `deeStorage`
- **不为 `file://` 预览模式做兜底或假数据**
- **非 Tauri 环境直接显示阻断页**
- **改完 GUI 代码必须启动实测**，编译通过不算完成

## Configuration System

Three config sources merged at startup: environment variables (`DEECODEX_*`), CLI args, and `~/.deecodex/config.json`. CLI/env override file values only when they differ from hardcoded defaults.

## Conventions

- **Concurrency:** `DashMap` for shared maps, `Arc<Mutex<VecDeque>>` for bounded queues.
- **Error handling:** `anyhow` for internal errors. Custom error types implement `IntoResponse`.
- **Logging:** `tracing` with `tracing-subscriber` env-filter. Default filter: `deecodex=info`.
- **Dynamic JSON:** `serde_json::Value` used extensively for API translation.
- **路径绝对化：** `data_dir` 等目录配置在使用前必须转为绝对路径。
- **静默失败加日志：** 关键分支返回空结果时要打 `tracing::warn!`，不静默跳过。
- **跨目录启动测试：** 编译后从不同目录启动二进制验证路径解析是否正常。

## 功能分区

项目通过 git worktree 划分为 11 个功能分区，每个分区独立开发、独立提交。详细分区定义和 GUI 共享层规则见 `CLAUDE.md`。

### 提交前缀

- `feat:` — 新功能
- `fix:` — 修复
- `refactor:` — 重构
- `docs:` — 文档
- `chore:` — 杂项/构建
- `release:` — 发版
