# 功能/核心翻译

## 职责

OpenAI Responses API ↔ Chat Completions API 双向协议翻译，是 deecodex 代理的核心引擎。

## 覆盖模块

| 文件 | 行数 | 职责 |
|------|------|------|
| `src/translate.rs` | ~1,900 | 请求方向：Responses → Chat Completions 转换 |
| `src/stream.rs` | ~1,870 | 响应方向：Chat SSE → Responses SSE 转换 |
| `src/handlers.rs` | ~3,470 | Axum HTTP 路由、所有请求处理器、AppState |
| `src/sse.rs` | ~676 | Responses API SSE 事件构建器 |
| `src/types.rs` | ~547 | 请求/响应数据结构定义 |
| `src/utils.rs` | ~224 | 响应合并、function_call 输出限制 |

## 编译

```bash
cargo build
cargo build --release
cargo test
cargo clippy -- -D warnings
```

## 推送

```bash
git add src/translate.rs src/stream.rs src/handlers.rs src/sse.rs src/types.rs src/utils.rs
git commit -m "<描述>"
git push deecodex-new 功能/核心翻译
```

## 合入主线

```bash
cd /Users/liguan/deecodex
git merge 功能/核心翻译
git push deecodex-new deecodex-gui
```

## 注意

- 修改 `types.rs` 会影响所有下游模块，需同步检查 `translate.rs` 和 `stream.rs`
- 改动 `handlers.rs` 的 `AppState` 字段时，确认 `main.rs` 中的初始化代码同步更新
- SSE 事件序列号由 `SseState` 维护，不要手动修改序列逻辑
