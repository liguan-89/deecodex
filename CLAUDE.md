# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

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
- Local executor settings are parsed in `executor.rs` and stored on `AppState`; MCP stdio execution is default-disabled, runs only for configured servers, and returns `mcp_tool_call_output` instead of surfacing executor failures as HTTP 500.

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
| `executor` | ~400 | Local computer/MCP executor config plus stdio MCP JSON-RPC tool execution |
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
