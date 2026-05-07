# Codex CLI 功能兼容对照表

> deecodex v1.0.0 · 最后更新 2026-05-07

## 图例

| 标记 | 含义 |
|------|------|
| ✅ | 完整支持 |
| ⚠️ | 有限支持（见说明） |
| ❌ | 明确拒绝（返回 400 `unsupported_feature`） |
| — | 不适用（DeepSeek 无此能力，不可实现） |

---

## 1. API 端点

| 端点 | 方法 | 状态 | 说明 |
|------|------|------|------|
| `/v1/responses` | POST | ✅ | 流式 + 非流式，含请求缓存 |
| `/v1/responses/:id` | GET | ✅ | 含 `include` 查询参数 |
| `/v1/responses/:id` | DELETE | ✅ | |
| `/v1/responses/:id/cancel` | POST | ✅ | queued/completed 状态冲突处理 |
| `/v1/responses/:id/input_items` | GET | ✅ | 含分页 |
| `/v1/responses/compact` | POST | ✅ | previous input items 合并持久化 |
| `/v1/responses/input_tokens` | POST | ✅ | tiktoken-rs 精确计数 |
| `/v1/conversations` | POST | ✅ | 创建会话 |
| `/v1/conversations/:id` | GET | ✅ | 获取会话 |
| `/v1/conversations/:id` | DELETE | ✅ | 删除会话 |
| `/v1/conversations/:id/items` | GET | ✅ | 会话消息列表 |
| `/v1/files` | GET | ✅ | 文件列表 |
| `/v1/files` | POST | ✅ | 上传文件（multipart） |
| `/v1/files/:id` | GET | ✅ | 文件元数据 |
| `/v1/files/:id` | DELETE | ✅ | 删除文件 |
| `/v1/files/:id/content` | GET | ✅ | 文件原始内容 |
| `/v1/vector_stores` | POST | ✅ | 创建 vector store |
| `/v1/vector_stores` | GET | ✅ | 列表 |
| `/v1/vector_stores/:id` | GET | ✅ | 获取 |
| `/v1/vector_stores/:id` | DELETE | ✅ | 删除 |
| `/v1/vector_stores/:id/files` | POST | ✅ | 添加文件到 store |
| `/v1/vector_stores/:id/files` | GET | ✅ | 列出 store 文件 |
| `/v1/vector_stores/:id/files/:id` | GET | ✅ | 获取 store 中文件 |
| `/v1/vector_stores/:id/files/:id` | DELETE | ✅ | 从 store 移除文件 |
| `/v1/vector_stores/:id/file_batches` | POST | ✅ | 创建文件批次 |
| `/v1/vector_stores/:id/file_batches` | GET | ✅ | 批次列表 |
| `/v1/vector_stores/:id/file_batches/:id` | GET | ✅ | 批次状态 |
| `/v1/vector_stores/:id/file_batches/:id/cancel` | POST | ✅ | 取消批次 |
| `/v1/vector_stores/:id/file_batches/:id/files` | GET | ✅ | 批次文件列表 |
| `/v1/prompts` | GET | ✅ | 本地 prompts 列表 |
| `/v1/prompts/:id` | GET | ✅ | 含 version/variables 解析 |
| `/v1/models` | GET | ✅ | 代理上游 /models |
| `/health` | GET | ✅ | 探活（免鉴权） |
| `/v1` | GET | ✅ | 状态探测（免鉴权） |

**覆盖率**：33/33 = 100%

---

## 2. 请求参数

### 核心参数

| 参数 | 状态 | 说明 |
|------|------|------|
| `model` | ✅ | 经 `DEECODEX_MODEL_MAP` 映射后转发 |
| `input` (text) | ✅ | 纯文本输入 |
| `input` (messages) | ✅ | 多轮对话 + function_call/mcp/computer 完整 input items |
| `stream` | ✅ | `true`=SSE 流式，`false`=JSON 响应 |
| `instructions` | ✅ | 合并为 system message |
| `system` | ✅ | 合并为 system message |
| `temperature` | ✅ | 透传上游 |
| `top_p` | ✅ | 透传上游 |
| `max_output_tokens` | ✅ | 透传上游 |
| `tool_choice` | ✅ | 透传上游 |
| `parallel_tool_calls` | ✅ | 透传上游 |
| `reasoning.effort` | ✅ | 映射为 `reasoning_effort` + thinking 开关 |
| `reasoning.summary` | ✅ | 映射为 `summary` 参数 |
| `store` | ✅ | `true`=持久化，`false`=即时响应不保存 |
| `metadata` | ✅ | 透传并回显在 response.metadata |
| `truncation` | ✅ | 透传上游 |
| `max_tool_calls` | ✅ | 供下游参考 |
| `conversation` | ✅ | 关联/创建 conversation |
| `previous_response_id` | ✅ | 继续之前的响应 |
| `prompt` (string) | ✅ | 从本地 prompts 注册表加载 instructions |
| `prompt` (object) | ✅ | 含 `id`/`version`/`variables` 解析 |
| `text.format` | ✅ | 透传上游 `response_format` |
| `user` | ✅ | 透传上游（优先于 `safety_identifier`/`prompt_cache_key`） |
| `stream_options` | ✅ | 流式时强制 `include_usage: true`，非流式不发送 |

### 显式拒绝

| 参数 | 状态 | 说明 |
|------|------|------|
| `top_logprobs` | ❌ | 返回 400 `unsupported_feature` |
| `background=true` + `stream=true` | ❌ | 后台异步流式不支持，返回 400 |

### 静默忽略（OpenAI 托管功能，不影响运行）

| 参数 | 状态 | 说明 |
|------|------|------|
| `safety_identifier` | ⚠️ | 接受但不作为，降级为 `user` 参数传递 |
| `prompt_cache_key` | ⚠️ | 接受但不作为，降级为 `user` 参数传递 |
| `prompt_cache_retention` | ⚠️ | 接受但不作为 |
| `service_tier` | ⚠️ | 接受但不作为 |
| `include_obfuscation` | ⚠️ | 接受但不作为 |

---

## 3. 工具类型

### Codex 原生工具

| 工具 | 状态 | 翻译目标 | 说明 |
|------|------|----------|------|
| `function` | ✅ | Chat `tools[].function` | 标准函数调用 |
| `web_search` / `web_search_preview` | ✅ | DeepSeek `web_search_options` | 启用 `search_context` |
| `file_search` / `file_search_preview` | ✅ | 本地 BM25 chunk 级倒排索引 | 1200 字符窗口，200 重叠，文件名 2.5x 加权 |
| `computer_use` / `computer_use_preview` | ✅ | `local_computer` bridge | Playwright / browser-use executor |
| `mcp` / `remote_mcp` | ✅ | `local_mcp_call` bridge | MCP stdio executor |
| `code_interpreter` | — | — | 需要 OpenAI 托管沙箱，DeepSeek 不具备 |
| `image_generation` | — | — | 需要图像生成模型，DeepSeek 不具备 |

### 扩展工具（deecodex 本地实现）

| 工具 | 状态 | 说明 |
|------|------|------|
| `custom` / `apply_patch` | ✅ | 映射为 `exec_command` 兼容格式 |
| `namespace` | ✅ | MCP namespace 展开为独立工具 |
| `local_shell` | ✅ | 本地 Shell 命令执行 |

### 工具策略

| 能力 | 状态 | 说明 |
|------|------|------|
| MCP server 白名单 | ✅ | `DEECODEX_ALLOWED_MCP_SERVERS` |
| computer display 白名单 | ✅ | `DEECODEX_ALLOWED_COMPUTER_DISPLAYS` |
| MCP read_only 默认 | ✅ | 拒绝写入/删除/修改类工具 |
| MCP `tools/list` metadata 探测 | ✅ | 优先使用 `readOnlyHint`/`destructiveHint` |
| 失败回填 output item | ✅ | 执行失败不 500，统一返回 `*_output` item |

---

## 4. 流式 SSE 事件

| 事件 | 状态 | 说明 |
|------|------|------|
| `response.created` | ✅ | 含 `id`/`model`/`status`/`conversation`/`metadata` |
| `response.completed` | ✅ | 含完整 `output`/`usage` |
| `response.failed` | ✅ | 上游错误/中断时不伪装成功 |
| `output_item.added` | ✅ | 所有 output 类型 |
| `output_item.done` | ✅ | 含 `status: completed` |
| `output_text.delta` | ✅ | 增量文本 |
| `reasoning_summary_text.delta` | ✅ | 推理摘要增量 |
| `function_call_arguments.delta` | ✅ | 函数参数增量 |
| `computer_call` added/done | ✅ | `local_computer` → Responses `computer_call` |
| `computer_call_output` added/done | ✅ | executor 结果回填 |
| `mcp_tool_call` added/done | ✅ | `local_mcp_call` → Responses `mcp_tool_call` |
| `mcp_tool_call_output` added/done | ✅ | executor 结果回填 |
| `file_search_call` added/done | ✅ | 本地检索结果 |
| `sequence_number` | ✅ | 单调递增，live + 缓存回放全程一致 |

**覆盖率**：14/14 = 100%

---

## 5. Include 字段

| 字段 | 状态 | 说明 |
|------|------|------|
| `file_search_call.results` | ✅ | 本地生成 |
| `output[*].file_search_call.results` | ✅ | 本地生成 |
| `usage` | ✅ | 透传上游 usage |
| `input_items` | ✅ | 本地存储，含 `file_search_context` 证据项 |
| `reasoning.encrypted_content` | ⚠️ | 安全忽略（OpenAI 加密专有字段，relay 不可实现） |
| `output[*].reasoning.encrypted_content` | ⚠️ | 安全忽略 |
| `reasoning.encrypted_content_summary` | ⚠️ | 安全忽略 |
| `output[*].reasoning.encrypted_content_summary` | ⚠️ | 安全忽略 |
| 其他一切字段 | ❌ | 返回 400 `unsupported_feature` |

---

## 6. Output Item 类型

| 类型 | 状态 | 说明 |
|------|------|------|
| `output_text` | ✅ | 普通文本输出 |
| `reasoning` | ✅ | 推理内容（含 `summary`） |
| `function_call` | ✅ | 含稳定 `id`/`call_id`/`name`/`arguments` |
| `computer_call` | ✅ | `local_computer` 映射，含 `action`/`call_id`/`display`/`status` |
| `computer_call_output` | ✅ | executor 结果，含 `screenshot`/`output`/`content` |
| `mcp_tool_call` | ✅ | `local_mcp_call` 映射，含 `server_label`/`name`/`arguments` |
| `mcp_tool_call_output` | ✅ | executor 结果，支持结构化 JSON |
| `custom_tool_call_output` | ✅ | 含稳定 `id` 和 `status: completed` |
| `tool_search_output` | ✅ | |
| `file_search_call` | ✅ | 含 `queries`/`vector_store_ids`/`results[]`/`chunk_id`/稳定 id |
| `file_search_context` | ✅ | 附加在 input_items 中作为本地证据 |

---

## 7. 协议契约

| 契约 | 状态 | 说明 |
|------|------|------|
| Response echo 一致 | ✅ | `id`/`model`/`status`/`output`/`usage`/`metadata` 在 create/retrieve/缓存回放中一致 |
| Output item id 稳定 | ✅ | `file_search_call`/`file_search_context` 使用 query+results 哈希；其余用 UUID |
| `sequence_number` 单调 | ✅ | live 事件 + 缓存回放事件全程单调递增无重复 |
| 非流式 tool_calls → output | ✅ | `message.tool_calls` 转为 `function_call` output item |
| 中断/错误 → `response.failed` | ✅ | SSE 解析错误或提前中断不保存残缺成功响应 |
| 缓存回放 SSE 契约 | ✅ | reasoning item 类型、output item id、metadata、usage 与 live 一致 |
| `starting_after` 游标 | ✅ | 基于 `sequence_number` 偏移，不基于事件下标 |
| thinking 降级保护 | ✅ | 仅在 reasoning_content 兼容错误时降级，不限流/5xx/连接错误静默关 thinking |
| 非流式不发 `stream_options` | ✅ | |
| VLM 路由 image 检测 | ✅ | 基于 `new_image` 检测而非 `msgs<=5` 判断 |
| 工具输出图片剥离 | ✅ | `data:image/` base64 在发给上游前替换为省略标记，防止 token 爆炸 |
| token 异常检测 | ✅ | prompt_explosion(>200k)、spike(>5x avg)、zero_completion、burn_rate(>500k/min) |

---

## 8. 本地增强能力

### Files API

| 能力 | 状态 | 说明 |
|------|------|------|
| 文件上传/列表/获取/删除 | ✅ | 磁盘持久化（`DEECODEX_DATA_DIR`） |
| `input_image.file_id` 解析 | ✅ | 本地展开为 `data:{mime};base64,...` |
| `input_file.file_id` 解析 | ✅ | 文本文件展开为 `input_text` |

### File Search

| 能力 | 状态 | 说明 |
|------|------|------|
| 倒排索引 | ✅ | 按 1200 字符 chunk（200 重叠）分块 |
| BM25 打分 | ✅ | `k1=1.2, b=0.75, scale=12.0` |
| 文件名加权 | ✅ | `FILENAME_MATCH_BOOST=2.5`，词项 3x 重复 |
| `max_num_results` | ✅ | 默认 5，可配置 |
| `ranking_options.score_threshold` | ✅ | 本地降级支持 |
| `vector_store_ids` 过滤 | ✅ | 限定检索范围 |
| chunk 级标注 | ✅ | `chunk_id`/`start_char`/`end_char` |
| 稳定 output item id | ✅ | 基于 query+vector+results 哈希 |
| 索引缓存/惰性重建 | ✅ | 上传/删除文件自动失效索引 |

### Computer Executor

| 能力 | 状态 | 说明 |
|------|------|------|
| Playwright 后端 | ✅ | Node.js `playwright` 模块，headless Chromium |
| persistent context | ✅ | `DEECODEX_PLAYWRIGHT_STATE_DIR` 按 display 复用 cookies/localStorage/上次 URL |
| 动作支持 | ✅ | `open_url`, `screenshot`, `click`, `double_click`, `type`, `keypress`, `scroll`, `wait` |
| 截图上限 | ✅ | 1.5MB，超限替换为省略标记 + `screenshot_bytes`/`screenshot_limit_bytes` metadata |
| browser-use HTTP bridge | ✅ | `DEECODEX_BROWSER_USE_BRIDGE_URL` |
| browser-use 命令 bridge | ✅ | `DEECODEX_BROWSER_USE_BRIDGE_COMMAND` |
| browser-use 输出归一化 | ✅ | `normalize_browser_use_output()` 统一产出格式 |
| 无 bridge 时显式失败 | ✅ | 返回明确 `computer_call_output` 失败项，不静默忽略 |
| 单步超时 | ✅ | `DEECODEX_COMPUTER_EXECUTOR_TIMEOUT_SECS` |

### MCP Executor

| 能力 | 状态 | 说明 |
|------|------|------|
| stdio JSON-RPC | ✅ | `initialize` → `notifications/initialized` → `tools/call` |
| 按需启动 | ✅ | 工具调用时启动进程 |
| 只读默认 | ✅ | `read_only=true`，拒绝写入/删除/修改 |
| 超时控制 | ✅ | `DEECODEX_MCP_EXECUTOR_TIMEOUT_SECS` |
| 配置来源 | ✅ | JSON 对象/数组 或 `.json` 文件路径 |

### Prompts

| 能力 | 状态 | 说明 |
|------|------|------|
| `prompts/{id}.json` | ✅ | 含 `instructions`/`input_prefix` 注入 |
| `prompts/{id}.{version}.json` | ✅ | 版本化 prompt |
| `prompts/{id}.md` | ✅ | Markdown prompt 文件 |
| `variables` 替换 | ✅ | `{{variable}}` 模板替换 |

### Vector Stores

| 能力 | 状态 | 说明 |
|------|------|------|
| CRUD + files + file_batches | ✅ | 磁盘持久化 |
| `schema_version` | ✅ | 为后续迁移保留版本边界 |

---

## 9. 安全与运维

| 能力 | 状态 | 说明 |
|------|------|------|
| Client 鉴权 | ✅ | `DEECODEX_CLIENT_API_KEY`，空值=关闭，非空=启用 |
| Rate Limiting | ✅ | 默认 120 req/60s，可配置，按 client_api_key 分桶 |
| Graceful Shutdown | ✅ | SIGINT/SIGTERM 30s drain |
| Prometheus Metrics | ✅ | `/metrics` 端点 |
| token 异常检测 | ✅ | 4 种告警类型 + Prometheus 指标 |
| 审计日志 | ✅ | computer/MCP executor 脱敏事件（backend/display/tool/action/status/elapsed_ms） |
| 配置诊断 | ✅ | 启动前校验 Playwright/node/browser-use/MCP 配置 |
| Pre-commit Hook | ✅ | 阻止 `.env` + API key 泄露 |
| `.env` 权限 | ✅ | 启动时收紧到 `0600` |
| 日志安全 | ✅ | JSON 解析失败不记录请求体前缀，只记录长度+hash |

---

## 10. 汇总

| 类别 | 覆盖率 |
|------|--------|
| API 端点 | 33/33 = **100%** |
| 核心请求参数 | 23/25 = **92%** |
| 工具类型 | 7/9 = **78%**（剩余 2 项 DeepSeek 不可实现） |
| SSE 流式事件 | 14/14 = **100%** |
| Include 字段 | 4 本地可实现 + 4 安全忽略（其余显式拒绝） |
| Output Item 类型 | 11/11 = **100%** |
| 协议契约 | 13/13 = **100%** |
| 测试覆盖 | **362** 个（268 单元 + 80 集成 + 3 bin + 5 compat + 8 validate） |

### 不可实现项（共 2 项）

| 功能 | 原因 |
|------|------|
| `code_interpreter` 工具 | 需要 OpenAI 托管沙箱执行环境 |
| `image_generation` 工具 | 需要 DALL·E 等图像生成模型 |

两项均属 DeepSeek API 能力边界，翻译代理层面不可实现。

### 主动拒绝项（共 3 项）

| 功能 | 原因 |
|------|------|
| `top_logprobs` | DeepSeek 不返回 token 级别 logprobs |
| `background=true` + 流式 | 后台异步流式实现复杂且 Codex 极少使用 |
| 不支持 `include` 字段 | 4 本地可实现 + 4 安全忽略（如 `reasoning.encrypted_content`），其余显式 400 |
