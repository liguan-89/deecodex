# deecodex 开发记录

这个文件用于记录项目开发计划、当前节点、已完成增强和待验证事项。每次完成一块开发后都要更新这里，避免下一轮接手时只靠聊天上下文。

## 当前目标

把 deecodex 从简单 Responses ↔ Chat Completions 兼容/翻译层，推进为面向 Codex 的 Responses 增强层：在本地补齐可实现的 Responses 能力，并明确标出不能可靠伪造的能力边界。

## 当前节点

- 时间：2026-05-07
- 阶段：增强层第一轮恢复合并 + v0.6 路由结构适配
- 正在做：本轮合并验证收尾
- 下一步：按“下次开发计划”继续推进可执行器、持久化和检索增强

## 已完成

- 修复流式中断被伪装成成功完成的问题：上游 SSE 解析错误或提前中断会走失败事件，不再保存残缺成功响应。
- 修复非流式工具调用丢失：`message.tool_calls` 会转换成 Responses `function_call` output item。
- 收紧本地 `.env` 权限到 `0600`，并建议下游客户端使用独立 `DEECODEX_CLIENT_API_KEY`。
- 修复重试误关 thinking：只在 reasoning_content 兼容错误时降级，不因限流/5xx/连接错误静默关 thinking。
- 补齐 live reasoning `output_item.done`，让 live SSE 和缓存回放契约一致。
- 非流式请求不再发送 `stream_options`。
- JSON 解析失败日志改为 body 长度和 hash，不记录请求体前缀。
- `/v1` 路由增加客户端 Authorization 校验，`/health` 和 `/v1` 保持探活豁免。
- 本地 Responses 管理端点已覆盖：retrieve/delete/cancel/input_items/compact/input_tokens/conversations。
- 接入本地 hosted prompts registry：
  - 支持 `prompt: "id"`。
  - 支持 `prompt: { "id": "...", "version": "...", "variables": {...} }`。
  - 支持 `prompts/{id}.json`、`prompts/{id}.{version}.json`、`.md`。
  - 支持 `instructions` 和 `input_prefix` 注入。
  - 新增 `GET /v1/prompts` 和 `GET /v1/prompts/:id`。
- 本地 Files API：
  - `POST /v1/files`
  - `GET /v1/files`
  - `GET /v1/files/:id`
  - `GET /v1/files/:id/content`
  - `DELETE /v1/files/:id`
  - 文件存储当前为内存态，服务重启后丢失。
- `input_image.file_id` 本地解析为 `data:{mime};base64,...`。
- `input_file.file_id` 文本文件展开为 `input_text`。
- 基础本地 `file_search`：用已上传文本文件做轻量检索，把结果注入模型上下文，并把命中结果写入 metadata。
- 本地 vector store / file batch 壳层：
  - `POST/GET /v1/vector_stores`
  - `GET/DELETE /v1/vector_stores/:id`
  - `POST/GET /v1/vector_stores/:id/files`
  - `GET/DELETE /v1/vector_stores/:id/files/:file_id`
  - `POST /v1/vector_stores/:id/file_batches`
  - `GET /v1/vector_stores/:id/file_batches/:batch_id`
  - `POST /v1/vector_stores/:id/file_batches/:batch_id/cancel`
  - `GET /v1/vector_stores/:id/file_batches/:batch_id/files`
  - `file_search.vector_store_ids` 会限制检索范围。
- `computer_use` / `computer_use_preview` 转为 `local_computer` bridge，上游返回 `local_computer` tool call 时映射为 Responses `computer_call`。
- `mcp` / `remote_mcp` 转为 `local_mcp_call` bridge，为本地 MCP executor 保留结构化入口。
- `/v1/responses/input_tokens` 接入 `tiktoken-rs`，替换原字符近似估算。
- `deecodex.sh` 增加 Codex config 自动注入/还原：
  - 首次启动时创建 `~/.codex/config.toml.{openai,deecodex}.txt` 双模板。
  - `start` 自动将 `config.toml` 切换为含 custom provider 的版本，`api_key` 从 `.env` 自动读取注入。
  - `stop` 自动还原到原始 OpenAI 配置。
  - `trap INT TERM` 信号触发时自动还原，防止 Crash/中断 导致配置残留。
  - 启动失败时也还原，确保配置不污染。
  - 修复 `model_provider` TOML 位置：从文件末尾移到 root 表（`model_reasoning_effort` 之后），符合 TOML 规范。
  - 修复停服期间编辑丢失：检测 `config.toml` 为 openai 版本时先同步到 `openai.txt`。
  - `requires_openai_auth` 随 `DEECODEX_CLIENT_API_KEY` 动态生成：本地鉴权启用时为 `true`，显式留空时为 `false`。
- 修复 client auth 回退链导致无法关闭鉴权：
  - Rust: `client_api_key` 为空时不再回退到 `api_key`（DeepSeek key），两个 key 独立。
  - Shell: `DEECODEX_CLIENT_API_KEY` 的 `${var:-default}` 改为 `${var-default}`，使显式空值不会走回退链变成 DeepSeek key。
  - `.env` 设 `DEECODEX_CLIENT_API_KEY=` 留空即可关闭本地 client auth，设为非空值则启用鉴权。
- 修复消息每轮翻倍 bug：`handle_responses_inner` 中从 session 加载 `get_history`/`get_conversation` 的 history 与 Codex `input` 数组重放的完整对话叠加，导致消息数指数增长最终触发上游 413。修复：`history` 固定为空 Vec，Codex 的 `input` 重放已包含完整上下文。

## 进行中

- 本轮已把 stash 中丢失的增强模块合入当前 v0.6 架构，剩余为真实执行器和持久化增强。

## 下次开发计划

- P1：把 `local_computer` 从桥接 schema 推进到可执行器：
  - 优先接 browser-use / Playwright。
  - 支持 screenshot/click/type/keypress/scroll/open_url。
  - 生成 `computer_call_output` 所需截图内容。
- P1：把 `local_mcp_call` 接到真实 MCP executor：
  - 配置允许的 MCP server。
  - 做权限白名单。
  - 把执行结果回填为 `mcp_tool_call_output`。
- P2：持久化本地 Files/vector stores：
  - 支持磁盘目录存储。
  - 重启后恢复 file/vector_store/batch 元数据。
- P2：增强 file_search：
  - 做倒排索引或轻量向量索引。
  - 支持 ranking_options / max_num_results。
  - 可选输出 `file_search_call` 兼容项。
- P2：补齐 Responses 工具调用输出兼容性：
  - 为本地 file_search 生成可选 `file_search_call` output item。
  - 为 `local_mcp_call` 设计 `mcp_tool_call`/`mcp_tool_call_output` 的回放与存储结构。
  - 为 `computer_call` 增加 pending/in_progress 状态和截图轮次元数据。
- P3：`include` 深层字段：
  - 明确支持本地可生成的 include。
  - 对必须依赖 OpenAI 托管资源的 include 返回清晰 unsupported。

## 验证记录

- 2026-05-06：hosted prompts registry 完成后通过 `cargo test`、`cargo clippy --all-targets -- -D warnings`。
- 2026-05-06：Files API / `file_id` / `file_search` 已接入，并通过 `cargo fmt --check`、`cargo test`、`cargo clippy --all-targets -- -D warnings`。
- 2026-05-06：vector store / file batch 壳层已接入，并通过 `cargo fmt --check`、`cargo test`、`cargo clippy --all-targets -- -D warnings`。
- 2026-05-06：六大项补齐到本地增强/桥接层，并通过 `cargo fmt --check`、`cargo test`、`cargo clippy --all-targets -- -D warnings`。
- 2026-05-06：`deecodex.sh` Codex config 自动注入/还原已测试通过：`bash -n` 语法检查、start/stop/restart 全流程、Ctrl+C trap 还原、启动失败还原。
- 2026-05-06：修复消息翻倍 bug（`src/main.rs`），`cargo test` 49/49 通过，`cargo build --release` 编译成功。
- 2026-05-07：从 stash 恢复并适配 `prompts/files/vector_stores` 模块到当前 `handlers.rs` 架构，补回 `computer_use`、`remote_mcp` 桥和 tokenizer 计数，通过 `cargo test`、`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`git diff --check`。
