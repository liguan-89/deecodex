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

deecodex is a proxy that translates Codex CLI's **OpenAI Responses API** requests into **Chat Completions API** calls for DeepSeek/OpenCode. The core translation is two-way:

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

真实功能测试使用（GUI 验证不强制在本机执行；如确需 GUI 实测，在局域网 Mac mini `<LAN_MAC_MINI>:22` 上进行）：

```bash
cd ~/deecodex
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
- **GUI 验证不强制在本机执行，也不禁止使用 GUI 验证**：编译通过不算视觉验证完成；如确需 GUI 实测，使用局域网 Mac mini（<LAN_MAC_MINI>:22）验证

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

## Windows 兼容性

涉及 Windows 分支的改动，先读 `docs/WINDOWS_COMPAT.md`（11 类兼容性清单与修复模式）。
核心原则：用户目录用 `home_dir()`、PATH 用 `split_paths`、临时目录用 `temp_dir()`、行切分用
`.lines()`；Unix-only 命令必须 `#[cfg]` 门控或提供 Windows 等价；spawn 一律 `CREATE_NO_WINDOW`
（除非要显示终端）；PowerShell 输出强制 UTF-8、Node 输出 GBK 回退；长阻塞操作不进主线程/setup。
改动后除主机 `cargo check` 外，需跑 `cargo xwin check --target x86_64-pc-windows-msvc` 验证。

## 工作区

项目当前只保留主开发区 `deecodex-gui`。功能开发按任务临时创建 `功能/<任务名>` worktree，合入后删除；不再长期维护固定分区。详细模块归属、临时 worktree 流程和 GUI 共享层规则见 `CLAUDE.md` 与 `WORKTREES.md`。

### 提交前缀

- `feat:` — 新功能
- `fix:` — 修复
- `refactor:` — 重构
- `docs:` — 文档
- `chore:` — 杂项/构建
- `release:` — 发版
