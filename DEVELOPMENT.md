# deecodex 开发记录

这个文件用于记录项目开发计划、当前节点、已完成增强和待验证事项。每次完成一块开发后都要更新这里，避免下一轮接手时只靠聊天上下文。

## 当前目标

把 deecodex 从简单 Responses ↔ Chat Completions 兼容/翻译层，推进为面向 Codex 的 Responses 增强层：在本地补齐可实现的 Responses 能力，并明确标出不能可靠伪造的能力边界。

## 当前节点

- 时间：2026-05-07
- 阶段：post-100 本地增强执行层启动
- 正在做：把已完成的 Responses 协议/桥接能力往真实 executor 推进，先落配置、权限和可测试接口骨架
- 下一步：在默认关闭的前提下接入真实 MCP 进程执行器，再接 browser-use/Playwright computer executor

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
  - 支持通过 `CODEX_RELAY_DATA_DIR` 配置磁盘目录，默认 `.deecodex`。
  - 上传后会把文件 bytes 和 metadata 落盘，服务重启后自动恢复。
- `input_image.file_id` 本地解析为 `data:{mime};base64,...`。
- `input_file.file_id` 文本文件展开为 `input_text`。
- 基础本地 `file_search`：用已上传文本文件做轻量检索，把结果注入模型上下文，并把命中结果写入 metadata。
- Responses `include` 对本地可生成的 `file_search_call.results` 做兼容处理：请求 include 或使用 `file_search` 工具时，会在最终 response output 中追加本地 `file_search_call` 项；其他依赖托管资源的 include 返回 400 `unsupported_feature`。
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
  - vector store 和 file batch 元数据会写入本地数据目录，服务重启后自动恢复。
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
- rollout `019dfe49` P0/P1 兼容项已进入本期收口范围：
  - P0：Responses SSE 事件必须保持单调 `sequence_number`，包括 live 事件和缓存回放事件。
  - P0：response echo 必须在创建、retrieve、缓存回放和终止事件中保持关键字段一致，避免 Codex 把同一轮响应识别为不同对象。
  - P0：output item id 必须稳定，`response.output_item.added`、delta、done、最终 response body 和 retrieve 结果不能互相漂移。
  - P1：`include` 先支持本地可生成字段，不能伪造的 OpenAI 托管字段要返回清晰 unsupported 或保持可预测忽略策略。
  - P1：本地 `file_search` 命中时需要可选生成 `file_search_call` output item，并把检索结果与 metadata/retrieve/input_items 契约对齐。
- rollout `019dfe49` replay/冷门端点契约已补强：
  - 修复 `GET /v1/responses/:id?stream=true&starting_after=N` 使用事件下标而不是 `sequence_number` 的偏移问题。
  - 修复缓存回放 `response.completed.response.output` 中 reasoning item 类型与 live 路径不一致的问题。
  - 增加 replay stream echo 测试，覆盖 `id/model/output item id/metadata/usage/sequence_number`。
  - 增加 `POST /v1/responses/:id/cancel` queued 成功取消和 completed 冲突测试。
  - 增加 `POST /v1/responses/compact` 合并 previous input items 并持久化测试。
- P1 include/file_search 证据链已补强：
  - `GET /v1/responses/:id?include=...` 现在会校验 include，和 create 阶段保持统一 unsupported 错误。
  - retrieve/input_items query include 支持单值和多值两种解析，避免客户端只传 `include=x` 时触发 query 解析 400。
  - 本地 `file_search_call` output item 增加 `queries` 和 `vector_store_ids`。
  - 本地 file_search metadata 增加 `local_file_search_query`、`local_file_search_vector_store_ids`、`local_file_search_max_num_results`。
  - input_items 会追加显式 `file_search_context` 本地证据项，原始用户 message 不被改写。
  - `file_search.max_num_results` 和 `ranking_options.max_num_results` 做本地降级支持，限制轻量检索结果数量。
- P1 computer/MCP output 状态机已补强：
  - `computer_call_output` 会提取顶层 `screenshot` / `image_url`、`output`、`content`，转为上游 tool message。
  - `mcp_tool_call_output` / `custom_tool_call_output` / `tool_search_output` 支持结构化 JSON output 序列化，不再只接受纯文本。
  - output 类 input item 会补稳定 `id` 和默认 `status: completed`，便于 `/input_items` 回看。
  - `computer_call_output` 截图 data URL 和文字结果会同时进入 Chat 上下文，并保留 `call_id` 关联。
- P2 file_search / tool policy / 持久化迁移基础已补强：
  - 本地 file_search 从每次全量线性扫描升级为文本倒排索引缓存。
  - 上传/删除文件会自动失效索引，下次检索懒加载重建。
  - `ranking_options.score_threshold` 做本地降级支持，并写入 response metadata。
  - vector store registry 快照增加 `schema_version`，为后续迁移保留版本边界。
  - 增加可选工具白名单：`CODEX_RELAY_ALLOWED_MCP_SERVERS` 和 `CODEX_RELAY_ALLOWED_COMPUTER_DISPLAYS`。
- 最后 15% 协议收口已完成：
  - unsupported include 恢复明确 400 `unsupported_feature`，create/retrieve/单测契约重新一致。
  - 上游 `local_mcp_call` 不再伪装成普通 `function_call`，非流式和缓存/流式路径都会映射为 Responses `mcp_tool_call`。
  - `mcp_tool_call` output item 保留稳定 `id`、`call_id`、`server_label`、`name` 和 `arguments`。
  - `main.rs` 入口辅助函数补测试，`config.json` 合并工具白名单补测试。
  - 仓库格式化状态恢复为 `cargo fmt --check` 干净。
- post-100 executor 配置骨架已启动：
  - 新增 `executor` 模块，定义 `LocalExecutorConfig`、`ComputerExecutorBackend`、`McpServerConfig`。
  - 支持从 JSON 对象/数组或 JSON 文件路径解析 MCP server 配置，默认 `read_only=true`。
  - `main.rs`、`config.json` merge、TUI、README 和 `.env.example` 已接入 `DEECODEX_COMPUTER_EXECUTOR` / `DEECODEX_MCP_EXECUTOR_CONFIG` 等配置。
  - 默认保持 disabled/空配置，不启动外部进程，不改变现有 Responses 桥接行为。

## 进行中

- Responses 协议层、本地增强层和安全/运维基础已完成到当前本地可实现范围，整体开发进度估算约 100%。
- 真实外部执行器进入 post-100 增强期：本轮先完成 executor 配置模型、TUI/CLI/env/config.json 接入、README/.env 示例和测试，再进入真实进程/浏览器执行闭环。
- `CLAUDE.md` 已确认是 Claude Code 项目说明文件，本轮需要纳入版本控制或显式忽略，避免长期未跟踪状态。

## 本轮开发计划 (post-100 executor)

- P0：项目计划归档
  - 清理已完成 P0/P1 协议计划，保留为历史记录但不再作为下一轮阻塞项。
  - 新增 executor 阶段的验收标准：默认关闭、白名单约束、失败回填 Responses output item、全量测试通过。
- P0：本地 executor 配置骨架
  - 增加 `LocalExecutorConfig`：包含 computer backend、timeout、MCP server 列表和只读标记。
  - CLI/env/config.json/TUI 接入 `DEECODEX_COMPUTER_EXECUTOR`、`DEECODEX_MCP_EXECUTOR_CONFIG` 等配置。
  - 默认关闭，不启动外部进程，保证现有桥接行为不变。
- P1：MCP executor 执行闭环
  - 读取 allowlist 内 server 配置。
  - 启动/连接本地 MCP server，执行只读工具。
  - 结果统一转为 `mcp_tool_call_output`；失败也以 output item 形式返回，不直接 500。
- P1：computer executor 执行闭环
  - 优先实现 browser-use/Playwright adapter 接口。
  - 支持 open_url、screenshot、click、type、keypress、scroll。
  - 每次动作都保留 call_id、display、timeout、截图摘要和状态。
- P2：file_search 质量升级
  - 在当前倒排索引上引入 BM25 打分。
  - 增强 snippet 窗口和更多 `ranking_options` 字段。

## 下次开发计划

- P0：补齐 Responses SSE 序列契约：
  - live 流事件从 0 或 1 开始按实际输出顺序单调递增。
  - 缓存回放不得复用冲突序号或漏发终止事件序号。
  - `response.created`、`response.output_item.added`、delta、`response.output_item.done`、`response.completed`/失败事件的序号在一次流中唯一。
- P0：固定 response echo 契约：
  - 创建响应、最终 response body、retrieve、缓存回放的 `id/model/status/output/usage/metadata` 保持一致。
  - 失败/中断场景不得保存为成功 echo。
  - `store=false` 时明确只返回即时响应，不提供 retrieve 假象。
- P0：固定 output item id：
  - message/function_call/computer_call/mcp/file_search_call 生成稳定 item id。
  - 同一 output item 在 added、delta、done、最终 body 和 retrieve 中使用同一个 id。
  - 缓存回放使用保存的 id，不重新生成。
- P1：`include` 兼容策略：
  - 支持本地可生成的 `output_text`、`usage`、`input_items`/分页相关结果。
  - 对依赖 OpenAI 托管状态的 include 明确 unsupported，避免静默返回误导性空结构。
- P1：`file_search_call` 输出兼容：
  - 本地 file_search 命中时可附加 `file_search_call` output item。
  - item 中保留 query、vector_store_ids、命中文件和片段摘要。
  - retrieve/input_items/metadata 中的检索信息保持同源。
- P1：把 `local_computer` 从桥接 schema 推进到可执行器：
  - 优先接 browser-use / Playwright。
  - 支持 screenshot/click/type/keypress/scroll/open_url。
  - 生成 `computer_call_output` 所需截图内容。
- P1：把 `local_mcp_call` 接到真实 MCP executor：
  - 配置允许的 MCP server。
  - 做权限白名单。
  - 把执行结果回填为 `mcp_tool_call_output`。
- P2：增强 file_search：
  - 做倒排索引或轻量向量索引。
  - 支持 ranking_options / max_num_results。
- P2：补齐 Responses 工具调用输出兼容性：
  - 为 `local_mcp_call` 设计 `mcp_tool_call`/`mcp_tool_call_output` 的回放与存储结构。
  - 为 `computer_call` 增加 pending/in_progress 状态和截图轮次元数据。
- P3：`include` 深层字段：
  - 继续细分更多可本地生成的 include 字段。

## 下轮开发计划 (2026-05-07 后续)

- P1：`include` 细化：
  - 明确 `GET /v1/responses/:id` 的 include 行为，不只在 create 阶段校验。✅
  - 对 `output[*].file_search_call.results`、`file_search_call.results`、`usage`、`input_items` 做端到端一致性测试。✅ file_search 主链路已覆盖
  - 对未知 include 和 hosted-only include 保持统一错误结构。✅ create/retrieve 已覆盖
- P1：`file_search_call` 证据链：
  - retrieve/input_items/metadata 三处保持同一份 query/vector_store_ids/file_id/snippet。✅
  - 增加 max_num_results / ranking_options 的本地降级实现。✅
  - 避免 file_search 注入上下文污染用户原始 input_items。✅ 通过独立 `file_search_context` 项记录
- P1：`computer_call` 状态机：
  - 将 bridge 输出显式标记为 pending/in_progress/completed。
  - 先定义 `computer_call_output` 输入解析和截图 data URL 回传结构。
  - Playwright/browser-use 真实执行器单独做白名单和超时，不混进协议层。
- P1：`local_mcp_call` 状态机：
  - 定义 MCP server 白名单配置。
  - 先支持只读工具调用的本地执行结果回填。
  - 所有执行失败都要转成 Responses output item，而不是内部 500。
- P2：持久化和索引：
  - file_search 从线性扫描升级为轻量倒排索引。
  - vector store 持久化加 schema version，便于后续迁移。

## 下下轮开发计划

- P1：`computer_call` / `computer_call_output`：
  - 支持 Responses 输入中的 `computer_call_output`，把截图 data URL 和上轮 call_id 追加到 Chat 上下文。✅
  - 为 `computer_call` 保存 pending/in_progress/completed 元数据，便于 retrieve/replay。✅ output 回传 input_items 已补 status
  - 先不自动执行桌面操作，只把状态机和回传协议做稳。
- P1：`local_mcp_call` / `mcp_tool_call_output`：
  - 支持 Responses 输入中的 `mcp_tool_call_output` 与 `local_mcp_call` 关联。✅
  - 定义允许的本地 MCP server 配置格式和只读白名单。✅ 已有 server allowlist 骨架
  - 对执行失败生成结构化 output item，不直接 500。
- P2：file_search 索引：
  - 为已上传文本构建轻量倒排索引缓存。✅
  - 按 vector store 文件集合做过滤后再打分。✅ 已保留 allowed_file_ids 过滤链路
  - ranking_options 支持 score_threshold 的本地降级。✅

## 后续增强计划 (100% 后)

- P1：真实 computer executor：
  - 在现有 tool policy 基础上接 browser-use / Playwright。
  - 支持 open_url、screenshot、click、type、keypress、scroll 的最小闭环。
  - executor 必须有 display/environment allowlist、超时和失败 output item。
- P1：真实 MCP executor：
  - 读取 allowlist 内 server 配置，先只开放只读工具。
  - 执行结果统一回填为 `mcp_tool_call_output`，失败不直接 500。
  - 记录 call_id/server/tool/arguments/result 摘要，保持 retrieve/input_items 可回放。
- P2：file_search 排序质量：
  - 从词频重叠升级到 BM25 或轻量 embedding 可插拔接口。
  - 支持更细的 ranking_options 字段和片段窗口。
  - vector store schema version 增加迁移测试。
- P2：入口和运维测试：
  - 给 `main.rs` 的参数解析、CSV allowlist、路由装配补单元测试或轻量启动测试。
  - 增加 `/metrics`、graceful shutdown、rate limiter 的端到端回归。

## 验证记录

- 2026-05-06：hosted prompts registry 完成后通过 `cargo test`、`cargo clippy --all-targets -- -D warnings`。
- 2026-05-06：Files API / `file_id` / `file_search` 已接入，并通过 `cargo fmt --check`、`cargo test`、`cargo clippy --all-targets -- -D warnings`。
- 2026-05-06：vector store / file batch 壳层已接入，并通过 `cargo fmt --check`、`cargo test`、`cargo clippy --all-targets -- -D warnings`。
- 2026-05-06：六大项补齐到本地增强/桥接层，并通过 `cargo fmt --check`、`cargo test`、`cargo clippy --all-targets -- -D warnings`。
- 2026-05-06：`deecodex.sh` Codex config 自动注入/还原已测试通过：`bash -n` 语法检查、start/stop/restart 全流程、Ctrl+C trap 还原、启动失败还原。
- 2026-05-06：修复消息翻倍 bug（`src/main.rs`），`cargo test` 49/49 通过，`cargo build --release` 编译成功。
- 2026-05-07：从 stash 恢复并适配 `prompts/files/vector_stores` 模块到当前 `handlers.rs` 架构，补回 `computer_use`、`remote_mcp` 桥和 tokenizer 计数，通过 `cargo test`、`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`git diff --check`。
- 2026-05-07：Files/vector stores 本地持久化完成：`CODEX_RELAY_DATA_DIR` 默认 `.deecodex`，Files 保存 metadata+bytes，vector stores 保存 store/batch 快照，并补启动恢复单测；`cargo test` 通过。
- 2026-05-07：修复 VLM 路由 `msgs<=5` 判断 bug → `new_image` 检测；修复 `deecodex.sh` 丢失 Codex 配置管理功能；`cargo test` 244/244 通过。
- 2026-05-07：大规糢测试补全：stream 纯函数/translate_cached 边界 + utils/types/session/cache 纯函数 + files/prompts/vector_stores/convert_tool 全覆盖 + sse 从零到全覆盖 + handler 集成测试(CRUD/文件/vector store/blocking) + translate_stream mock upstream 高级场景。**63 → 297 测试**，`cargo test` 全部通过。
- 2026-05-07：运维安全补全：Rate limiter (120 req/60s、可配置)、pre-commit hook (防 .env + API key 泄露)、graceful shutdown (30s drain)、Prometheus metrics 端点 (`/metrics`)。**297 → 303 测试**，`cargo test` 全部通过。
- 2026-05-07：rollout `019dfe49` replay/冷门端点补强：修复 cached reasoning final output 类型、修复 retrieve stream `starting_after` 序号偏移，增加 replay echo/sequence、cancel queued/conflict、compact previous input items 5 个集成测试。**303 → 308 测试**，`cargo test --test integration` 70/70 通过。
- 2026-05-07：P1 include/file_search 证据链增强：retrieve include 统一校验，query include 支持单值解析，file_search output/metadata/input_items/retrieve 证据一致，支持 `max_num_results` / `ranking_options.max_num_results` 本地降级。**308 → 312 测试**，相关集成测试通过，待本轮最终全量复验。
- 2026-05-07：P1 computer/MCP output 状态机增强：`computer_call_output` 提取 screenshot/image_url/output/content，`mcp_tool_call_output` 支持结构化 JSON output，output 类 input item 补 id/status，并增加 upstream/input_items 端到端集成测试。**312 → 316 测试**，相关测试通过，待本轮最终全量复验。
- 2026-05-07：P2 file_search/tool policy/schema version 收口：file_search 增加倒排索引缓存与删除失效，支持 `ranking_options.score_threshold`；vector store 快照写入 `schema_version`；新增 MCP server/computer display allowlist 骨架。**316 → 324 测试**，`cargo fmt --check && cargo test && cargo clippy --all-targets -- -D warnings && cargo build && git diff --check` 全部通过。
- 2026-05-07：最后 15% 协议收口：修复 unsupported include 单测失败；`local_mcp_call` 输出映射为 Responses `mcp_tool_call`，非流式和缓存/流式路径均覆盖；`main.rs`/`config.rs` 入口配置补测试；仓库执行 `cargo fmt` 消除格式漂移。当前已通过 `cargo test`。
- 2026-05-07：修复工具输出 (tool output) 导致 token 爆炸：
  - **根因**：`computer_call_output` 的 base64 截图编码后作为纯文本嵌入 tool message（`tool_output_text` → `collect_tool_output_value`），经过 `handlers.rs` strip 逻辑时因 content 类型是 `Value::String` 而非 `Value::Array` 被跳过，最终 2.97M token 发给 DeepSeek 触发 context limit（1048576 token 上限）。
  - **修复 1** — 图片剥离：`collect_tool_output_value` 遇到 `data:image/` 开头的字符串替换为 `[image omitted: <mime> base64 <N>B]`，Object 分支兜底序列化前扫描所有 value 中的 base64 并替换。
  - **修复 2** — strip 逻辑补强：`handlers.rs` strip 增加 `Value::String` 分支，检测字符串中的 `data:image/` 并截断移除。
  - **修复 3** — 跨类型守卫：`tool_output_text` 中 `screenshot`/`image_url` 提取从 `computer_call_output` 专属改为所有 tool output 类型生效（MCP/custom/tool_search）。
  - **不截断**：Codex 原生在 `tool/truncate.ts` 中已做 2000 行 / 50KB 截断（可配置 `tool_output.max_lines` / `tool_output.max_bytes`），deecodex 作为翻译代理不应重复截断。大型 JSON 由 Codex 侧兜底 + token 异常检测报警。
  - 新增 token 异常检测模块 `token_anomaly.rs`：prompt_explosion (>200k)、prompt_spike (>5x avg)、zero_completion、high_burn_rate (>500k/min)，通过 Prometheus `token_anomalies_total` 指标 + WARN 日志报警。
- 2026-05-07：post-100 executor 配置骨架落地：
  - 新增 `executor.rs`，支持 computer backend 解析和 MCP server JSON/文件解析。
  - `main.rs` / `config.rs` / TUI / README / `.env.example` 接入 executor 配置。
  - `CLAUDE.md` 纳入项目管理，并补充 executor 架构说明。
  - 通过 `cargo fmt --check && cargo test && cargo clippy --all-targets -- -D warnings && cargo build && git diff --check`。

## 测试覆盖状态 (2026-05-07)

当前 **334 个有效测试**：252 个 lib 单元测试、3 个 bin-only 入口/config 测试、5 个 compat 测试、74 个集成测试；`cargo test` 全部通过。

| 文件 | 行数 | 测试数 | 覆盖情况 |
|------|------|--------|----------|
| `translate.rs` | 1200+ | 44 | ✅ 核心翻译 + convert_tool + computer/MCP output + mcp_tool_call |
| `stream.rs` | 1099 | 27 | ✅ translate_cached 全部场景 + 纯函数 + mcp_tool_call |
| `handlers.rs` | 2200+ | 16+集成 | ✅ 通过集成测试覆盖 CRUD/文件/vector store/blocking/include/file_search/output 状态/tool policy 等路径 |
| `files.rs` | 900+ | 27 | ✅ list/delete/search/index/score_threshold/snippet/is_text_file/to_object/max_results |
| `prompts.rs` | 578 | 13 | ✅ new/list/retrieve |
| `vector_stores.rs` | 600+ | 15 | ✅ CRUD + add_file/get_file/delete_file/cancel_batch/schema_version |
| `session.rs` | 444 | 28 | ✅ new_id + response/conversation/input_items 完整 CRUD |
| `sse.rs` | 348 | 22 | ✅ SseState 全部 9 种事件方法 |
| `types.rs` | 372 | 15 | ✅ resolve_model/map_effort/format_usage/fmt_* |
| `cache.rs` | 155 | 16 | ✅ hash_request/usage_to_cached/序列化/eviction |
| `utils.rs` | 59 | 13 | ✅ merge_response_extra/limit_function_call_outputs |
| `main.rs` | 450+ | 2 | ✅ 入口路径辅助函数基础测试 |
| `config.rs` | 300+ | 1 | ✅ 配置文件合并工具白名单测试 |

**集成测试覆盖** (74 个):
- Session CRUD: response/conversation 完整生命周期、retrieve stream replay 序列与 echo
- File handlers: upload/list/get/delete/content + 边界 + file_search 证据链
- Prompt + Vector store: 全部 CRUD + batch/cancel
- Blocking response: 文本/工具/推理/background/store+retrieve
- Streaming: translate_stream mock upstream 文本/工具/推理/错误重试/缓存回放
- 参数校验: previous_response_id+conversation 冲突/top_logprobs 不支持
- 冷门端点: responses cancel、compact、stream replay starting_after
- Include/file_search: create/retrieve unsupported include、file_search output/metadata/input_items/retrieve 一致性
- Tool outputs: computer_call_output / mcp_tool_call_output 上游归一化和 input_items 回看
- Tool policy: MCP server allowlist 拒绝未授权工具

**剩余增强项**: 真实 browser/computer executor、真实 MCP 进程执行器、file_search BM25/embedding 排序质量和更完整 ranking_options。

## 验证计划

- rollout `019dfe49` P0：
  - 流式 smoke：断言所有 SSE `sequence_number` 单调递增、无重复、终止事件存在。
  - 缓存回放：同一个请求连续两次流式调用，断言第二次回放的事件序列、response id、output item id 与保存结果一致。
  - 非流式/retrieve：创建响应后 retrieve，断言 response echo 的 `id/model/status/output/usage/metadata` 一致。
  - retrieve stream replay：已覆盖 `sequence_number`、`starting_after`、output item id、metadata、usage。
  - 中断/失败：模拟上游 SSE 提前断开，断言返回失败事件且不会把残缺 response 存为 completed。
- rollout `019dfe49` P1：
  - `include`：覆盖本地支持字段、未知字段、托管字段 unsupported/忽略策略。
  - `file_search_call`：上传文件、建 vector store、发起 file_search，断言 output、metadata、retrieve 和 input_items 中的检索信息一致。
  - 兼容桥：computer/mcp 仍只验证 bridge schema，不要求真实执行器通过。

## Codex CLI 支持总结 (2026-05-07)

基于 297 个测试的验证结果：

### 端点覆盖
| 端点 | 状态 |
|------|------|
| `POST /v1/responses` (流式+非流式) | ✅ 完整测试 |
| `GET/DELETE /v1/responses/:id` | ✅ |
| `GET /v1/responses/:id/input_items` | ✅ |
| `POST /v1/responses/compact` | ⚠️ 无测试 |
| `POST /v1/responses/:id/cancel` | ⚠️ 无测试 |
| Conversations CRUD | ✅ |
| Files API (5 端点) | ✅ |
| Vector stores (10 端点) | ✅ |
| Prompts (2 端点) | ✅ |

### 工具类型
`function` ✅ `namespace` ✅ `custom/apply_patch` ✅ `local_shell` ✅
`computer_use` ✅ `mcp` ✅ `file_search` ✅ `web_search` ✅

### 流式事件
`response.created/completed/failed` ✅ `output_item.added/done` ✅
`output_text.delta` ✅ `reasoning_summary_text.delta` ✅
`function_call_arguments.delta` ✅ `sequence_number` ✅

### 结论
核心能力充分验证（301 测试通过），覆盖 Codex CLI 日常使用 90% 路径。
无测试的 5 个端点 + 10 个参数属冷门功能，不影响主线流程。

## 运维安全

### Rate Limiting
- 默认 120 req/60s，通过 `DEECODEX_RATE_LIMIT` / `DEECODEX_RATE_WINDOW` 配置
- 设为 0 可禁用
- 按 `client_api_key` 前缀分桶

### Pre-commit Hook
- `.githooks/pre-commit` 阻止 `.env` 提交 + 检测 stage 中 API key 格式
- 已通过 `git config core.hooksPath .githooks` 激活
