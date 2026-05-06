# deecodex 质量改进计划

> 基于 v0.5.1 质量审计报告，按优先级排列

---

## 🔴 P0 — 紧急（影响稳定性/安全性）

### 1. 添加上游请求超时

**问题：** `reqwest::Client` 未配置 timeout，上游 hang 时请求永久阻塞。

**方案：** 在 `Client::builder()` 添加 `.timeout(Duration::from_secs(300))`，可通过环境变量配置。

**影响范围：** `main.rs:181-184`  
**预估工时：** 0.5h

### 2. stream.rs 补齐单元测试

**问题：** 核心流式翻译模块（793 行）零测试覆盖。

**方案：** 至少覆盖以下场景：
- 正常 SSE 流生命周期（reasoning → text → tool_calls → completed）
- 上游返回非 2xx 时 emit `response.failed`
- 流中断（`[DONE]` 未到达）时 emit `response.failed`
- `apply_patch` → `exec_command` 名称翻译
- reasoning_content 持久化到 session store
- 缓存命中时 `translate_cached` 正确回放

**影响范围：** `src/stream.rs` 新增 `#[cfg(test)] mod tests`  
**预估工时：** 4h

### 3. 修复重复代码

**问题：** `merge_response_extra` 和 `limit_function_call_outputs` 在 `main.rs` 和 `stream.rs` 中重复。

**方案：** 将公共函数提取到 `src/utils.rs` 或保留一份在 `main.rs`，`stream.rs` 通过 `crate::` 引用。

**影响范围：** `main.rs:1412-1501`, `stream.rs:758-793`  
**预估工时：** 0.5h

---

## 🟡 P1 — 重要（提升质量/可维护性）

### 4. 拆分 main.rs

**问题：** 1810 行单体文件包含路由、handler、VLM、工具函数等。

**方案：**
```
src/
├── main.rs          # 仅入口 + AppState + Router 组装 (~100行)
├── routes/
│   ├── mod.rs
│   ├── responses.rs  # handle_responses + handle_responses_inner
│   ├── retrieve.rs   # handle_get_response + handle_delete_response + handle_cancel
│   ├── items.rs      # handle_input_items + list_items_response
│   ├── conversations.rs
│   └── models.rs     # handle_models
├── vlm.rs            # build_vlm_body + handle_vlm
├── utils.rs          # merge_response_extra, limit_function_call_outputs, now_unix_secs 等
```

**影响范围：** 全部 `src/main.rs`  
**预估工时：** 3h

### 5. 清理 dead_code 和 clippy 警告

**问题：** 4 个 clippy 警告 + 多处 `#[allow(dead_code)]`。

**方案：**
| 位置 | 处理方式 |
|------|----------|
| `sse.rs:1` | 移除 `#![allow(dead_code)]`，所有方法均已使用 |
| `sse.rs:18` | 添加 `impl Default for SseState` |
| `sse.rs:244` | `match` → `if let` |
| `main.rs:406` | `.map_or(false, ...)` → `.is_some_and(...)` |
| `cache.rs:94-107` | `len()`/`is_empty()` 在测试中保留，`stats()` 移除或使用 |
| `session.rs:196,271` | 移除未使用的 `get_history()` / `get_conversation()` |

**预估工时：** 1h

### 6. LRU eviction 改为真正 LRU

**问题：** `DashMap::iter().next()` 不保证顺序，淘汰的是随机条目。

**方案：** 
- 使用 `std::collections::HashMap` + `Mutex` 配合访问时间戳实现真 LRU
- 或换用 `lru` crate（已在社区广泛使用）
- 评估后发现 `RequestCache` 和 `SessionStore` 的读写比极低（proxy 场景以读为主），`Mutex` 开销可接受

**影响范围：** `cache.rs:79-92`, `session.rs` 多处 eviction  
**预估工时：** 2h

### 7. 添加 HTTP 集成测试

**问题：** 无 endpoint 级别的端到端测试。

**方案：** 使用 `axum-test` 或 `tower::ServiceExt` 编写：
- `GET /health` 返回 200 + uptime
- `POST /v1/responses` 非流式正常请求
- `GET /v1/responses/:id` 404 响应
- `POST /v1/responses` JSON 解析错误返回 422
- `DELETE /v1/responses/:id` 已存在的响应

**影响范围：** `tests/integration.rs` 新建  
**预估工时：** 3h

---

## 🟢 P2 — 优化（体验/长期健康）

### 8. 统一配置常量

**问题：** 硬编码常量散落各处。

**方案：** 新建 `src/config.rs`：
```rust
pub const MAX_SESSIONS: usize = 256;
pub const MAX_REASONING: usize = 512;
pub const MAX_TURN_REASONING: usize = 256;
pub const CACHE_ENTRIES: usize = 128;
pub const MAX_RETRIES: u32 = 3;
pub const RETRY_BASE_DELAY_MS: u64 = 500;
pub const DEFAULT_PORT: u16 = 4444;
pub const DEFAULT_MAX_BODY_MB: usize = 100;
```

**预估工时：** 1h

### 9. 添加 `/metrics` 端点

**问题：** 无可观测性端点，无法监控 QPS/延迟/错误率。

**方案：** 轻量级方案（不引入 Prometheus 依赖）：
- 使用 `Arc<AtomicU64>` 计数器记录：`requests_total`, `errors_total`, `cache_hits`, `cache_misses`
- `GET /metrics` 返回 Prometheus text 格式
- 可选：请求延迟直方图（`Arc<DashMap>` 分桶）

**影响范围：** `AppState` 新增字段 + `main.rs` 新增路由  
**预估工时：** 2h

### 10. 添加 `#[tracing::instrument]` 请求级 span

**问题：** 日志无 request_id 关联，多请求并发时难以追踪。

**方案：**
- 在 `handle_responses` 上添加 `#[tracing::instrument(skip(state, body), fields(response_id))]`
- 在 `handle_responses_inner` 中 `tracing::Span::current().record("response_id", &response_id)`
- 日志输出中自动带上 `response_id` 字段

**预估工时：** 1h

### 11. 补全 CI/CD

**方案：** 新建 `.github/workflows/ci.yml`：
```yaml
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test --all-targets
      - run: cargo clippy -- -D warnings
      - run: cargo fmt --check
```

**预估工时：** 0.5h

### 12. Session store 职责拆分

**问题：** `SessionStore` 一个 struct 管理 7 个 DashMap。

**方案：** 保持对外接口不变，内部委托给 sub-store：

```rust
pub struct SessionStore {
    sessions: ResponseStore,
    conversations: ConversationStore,
    reasoning: ReasoningStore,
}
```

**影响范围：** `session.rs`, `main.rs`, `stream.rs`, `translate.rs` 调用点  
**预估工时：** 2h

---

## 汇总

| # | 任务 | 优先级 | 工时 |
|---|------|--------|------|
| 1 | 上游请求超时 | 🔴 P0 | 0.5h |
| 2 | stream.rs 单元测试 | 🔴 P0 | 4h |
| 3 | 修复重复代码 | 🔴 P0 | 0.5h |
| 4 | 拆分 main.rs | 🟡 P1 | 3h |
| 5 | 清理 dead_code/clippy | 🟡 P1 | 1h |
| 6 | LRU 真淘汰 | 🟡 P1 | 2h |
| 7 | HTTP 集成测试 | 🟡 P1 | 3h |
| 8 | 统一配置常量 | 🟢 P2 | 1h |
| 9 | /metrics 端点 | 🟢 P2 | 2h |
| 10 | tracing span | 🟢 P2 | 1h |
| 11 | CI/CD | 🟢 P2 | 0.5h |
| 12 | Session store 拆分 | 🟢 P2 | 2h |
| **合计** | | | **20.5h** |
