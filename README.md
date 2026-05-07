# deecodex

**DeepSeek API → Codex CLI 兼容代理**

将 Codex CLI 发出的 **Responses API** 请求实时翻译为 **Chat Completions API**，使 Codex 可以原生对接 DeepSeek 等第三方模型，同时保留思考模式、工具调用、Web Search 等完整功能。可选配 MiniMax 视觉路由支持多模态图片理解。

```
Codex CLI 发出 /v1/responses (gpt-5.5 / gpt-5.4 等模型名)
        │
        ▼
  deecodex (Responses ↔ Chat 协议翻译 + 模型映射 + 缓存 + 重试 + 视觉路由)
        │
        ▼
  api.deepseek.com/v1/chat/completions
```

## 安装

### 从源码编译

```bash
git clone https://github.com/liguan-89/deecodex.git
cd deecodex
cargo build --release
cp target/release/deecodex ~/.local/bin/
```

### 从 Release 下载

从 [Releases](https://github.com/liguan-89/deecodex/releases) 下载 `deecodex` 二进制：

```bash
chmod +x deecodex
mv deecodex ~/.local/bin/
```

### 验证

```bash
deecodex --help
```

## 快速开始

```bash
cp .env.example .env
vim .env                                # 填入 DEECODEX_API_KEY
./deecodex.sh start                     # 启动服务
./deecodex.sh health                    # 确认 healthy
```

Codex 桌面端 `~/.codex/config.toml`：

```toml
model = "deepseek-v4-pro"
model_provider = "custom"
model_reasoning_effort = "medium"

[model_providers.custom]
base_url = "http://127.0.0.1:4446/v1"
name = "custom"
requires_openai_auth = true
wire_api = "responses"
```

CC Switch 用户只需填 API 请求地址 `http://127.0.0.1:4446/v1` 和任意 API Key。

## 项目结构

## API 端点

deecodex 实现了完整的 Responses API 端点集合，远超出简单的请求翻译：

| 端点 | 方法 | 说明 |
|---|---|---|
| `/v1/responses` | POST | 创建响应（主入口，流式/非流式） |
| `/v1/responses/:id` | GET | 获取已存储的响应详情 |
| `/v1/responses/:id` | DELETE | 删除已存储的响应 |
| `/v1/responses/:id/cancel` | POST | 取消正在进行的流式响应 |
| `/v1/responses/:id/input_items` | GET | 获取响应的输入项列表 |
| `/v1/responses/compact` | POST | 压缩存储中的响应 |
| `/v1/responses/input_tokens` | POST | 计算输入 token 数 |
| `/v1/conversations` | POST | 创建会话 |
| `/v1/conversations/:id` | GET | 获取会话详情 |
| `/v1/conversations/:id` | DELETE | 删除会话 |
| `/v1/conversations/:id/items` | GET | 获取会话消息列表 |
| `/v1/models` | GET | 模型列表（透传上游） |
| `/v1/health` | GET | 健康检查 |

### 本地增强层能力

deecodex 在 Responses ↔ Chat 翻译之外，内置了一层面向 Codex 的本地 Responses 增强能力。目标是尽量补齐 Codex 客户端依赖、且不伪造无法可靠实现的 OpenAI 托管状态。

| 能力 | 支持情况 |
|---|---|
| Hosted prompts | 支持 `prompt: "id"` 和 `prompt: {id, version, variables}`；从本地 `prompts/` 读取 JSON/Markdown 模板并注入 `instructions` / `input_prefix` |
| Files API | 支持上传、列表、读取 metadata、读取内容、删除；默认持久化到 `CODEX_RELAY_DATA_DIR` |
| Vector stores | 支持本地 vector store、文件关联和 file batch 壳层；持久化快照带 schema version，用于约束本地 file_search 范围 |
| file_search | 对已上传文本文件维护轻量倒排索引，把命中内容注入模型上下文，并输出 `file_search_call` / metadata / input_items 证据链 |
| Computer bridge | `computer_use` / `computer_use_preview` 转换为 `local_computer` bridge；上游返回 local computer 调用时映射为 Responses `computer_call`，可用 `DEECODEX_ALLOWED_COMPUTER_DISPLAYS` 限制 display，并已预留 `DEECODEX_COMPUTER_EXECUTOR` 执行器配置 |
| MCP bridge | `mcp` / `remote_mcp` 转换为 `local_mcp_call` bridge；上游返回 local MCP 调用时映射为 Responses `mcp_tool_call`；启用 `DEECODEX_MCP_EXECUTOR_CONFIG` 后会通过本地 stdio MCP server 执行并回填 `mcp_tool_call_output` |
| input_tokens | `/v1/responses/input_tokens` 使用 `tiktoken-rs` 做本地 token 计数 |
| Client auth | `/v1/*` 可用独立 `DEECODEX_CLIENT_API_KEY` 校验；`/health` 和 `/v1` 探活豁免，显式留空可关闭本地鉴权 |

### 会话存储

deecodex 内置内存会话存储，支持响应的生命周期管理：

- 响应创建后自动存储到 `SessionStore`
- `background: true` 时放入后台任务队列，返回 `queued` 状态
- 支持通过 `conversation` 字段关联响应到会话
- 存储包含完整响应体和用量数据
- 服务重启后数据丢失（Codex 会重放历史）


```
deecodex/
├── Cargo.toml            # Rust 项目配置
├── src/
│   ├── main.rs           # 服务入口 + HTTP 路由 + 视觉路由
│   ├── translate.rs      # Responses ↔ Chat 核心翻译（类型转换、工具转发、思考映射）
│   ├── stream.rs         # SSE 流式响应翻译 + 缓存回放 + 重试
│   ├── session.rs        # 会话管理 + reasoning_content 三级恢复
│   ├── types.rs          # 35+ 类型定义（请求/响应/流/用量/缓存）
│   ├── cache.rs          # LRU 请求缓存（128 条目哈希）
│   └── lib.rs            # 模块导出
├── deecodex.sh           # 管理脚本
├── .env.example          # 环境变量模板
└── tests/                # 63 个测试
```

### 新增字段透传（0.4.0）

| Responses API 字段 | 说明 |
|---|---|
| `background` | 后台完成模式（返回 `queued` 状态） |
| `store` | 存储响应到会话（透传到 upstream 存储） |
| `conversation` | 关联会话 ID |
| `text.format` | 输出格式控制 |
| `include` | 响应包含的字段列表（`output_text`/`usage` 等） |
| `include[]` (input_items) | 列出存储的输入项 |
| `parallel_tool_calls` | 并行工具调用 |
| `max_tool_calls` | 最大工具调用次数 |
| `top_logprobs` | Top-logprobs 采样 |
| `user` | 用户标识 |
| `safety_identifier` | 安全标识 |
| `prompt_cache_key` | 提示缓存键 |
| `prompt_cache_retention` | 缓存保留策略 |
| `service_tier` | 服务层级（`auto`/`default`） |
| `input_items` 响应 | 列出输入消息项 |
| `text.response_format` | 结构化输出格式 |

### 请求方向（Codex → DeepSeek）

## 协议转换全表

deecodex 在企业级精度下完成了 Responses API ↔ Chat Completions API 的双向翻译：

### 请求方向（Codex → DeepSeek）

| Responses API 字段 | 转换为 | 说明 |
|---|---|---|
| `model` | `model` | 通过 `DEECODEX_MODEL_MAP` 映射 |
| `input` (text) | `messages[{role:"user", content}]` | 纯文本输入 |
| `input` (messages) | `messages[]` | 消息数组，逐项转换 |
| `input[].type = "message"` | `messages[{role, content}]` | 常规消息，`developer` → `system` |
| `input[].type = "function_call"` | 单个 `messages[{role:"assistant", tool_calls}]` | **连续 function_call 自动合并为一条 assistant 消息** |
| `input[].type = "function_call_output"` | `messages[{role:"tool", content, tool_call_id}]` | 失败时前缀 `[FAILED]` |
| `input[].type = "mcp_tool_call_output"` | `messages[{role:"tool"}]` | MCP 工具输出 |
| `input[].type = "custom_tool_call_output"` | `messages[{role:"tool"}]` | 自定义工具输出 |
| `input[].type = "tool_search_output"` | `messages[{role:"tool"}]` | 搜索工具输出 |
| `content` (String) | `content` (String) | 纯文本 |
| `content` (Array) | `content` (Array) | 多模态内容数组 |
| `type = "input_image"` | `type = "image_url"` | 图片格式转换 |
| Base64 内嵌文本 | 自动切割为 text + image_url | 从文本中提取 `data:image/` |
| `instructions` | 优先于 `system` 作为 system prompt | `system` 字段兼容 |
| `previous_response_id` | 会话历史拼接 | 自动从本地 store 恢复历史 |
| `conversation` | 本地 conversation 历史拼接 | 支持 string id / object id；不能与 `previous_response_id` 同用 |
| `tools[].type = "function"` | `tools[{type:"function", function:{...}}]` | 标准函数 |
| `tools[].type = "custom"` | `tools[{type:"function", function:{...}}]` | **自定义工具 → 带参数 schema 的 function** |
| `tools[].type = "namespace"` | 展开为多个 function tools | **MCP 命名空间工具展开** |
| `tools[].name = "apply_patch"` | `exec_command` | **维持行为但改名，避免上游拒绝** |
| 同名 tool 去重 | 保留首个，删除重复 | **DeepSeek 强制要求工具名唯一** |
| `tools.web_search_preview` | `web_search_options` | **DeepSeek web_search 激活** |
| `stream: true` | `stream: true` + `stream_options.include_usage` | 流式输出 + 用量统计 |
| `temperature` | `temperature` | 透传 |
| `top_p` | `top_p` | 透传 |
| `max_output_tokens` | `max_tokens` | 字段名适配 |
| `tool_choice` | `tool_choice` | 透传 |
| `parallel_tool_calls` | `parallel_tool_calls` | 透传给兼容上游 |
| `store` | 本地 response/input/history 存储开关 | `false` 时生成结果但不可 retrieve |
| `metadata` | Responses response metadata | 随 response 保存和返回 |
| `truncation` | Responses response truncation | 记录到 response 对象 |
| `background` | 后台非流式任务 | 先返回 `queued`，后台完成后可 retrieve；cancel 会 abort 本地任务 |
| `text.format` | `response_format` | 支持 `json_object` / `json_schema` |
| `prompt_cache_key` / `safety_identifier` / `user` | `user` / 本地缓存命名空间 | 优先 `user`，其次 `safety_identifier`，再其次 `prompt_cache_key` |
| `max_tool_calls` | 本地 output 限制 | 超出后标记 `incomplete` |
| `top_logprobs` | 400 unsupported | Chat 兼容上游无法提供 Responses logprobs |
| `reasoning.effort` | `reasoning_effort` + `thinking` | 六级映射（见下文） |
| `reasoning.summary` | — | 透传 |
| — | 中文思考指令（可选） | `DEECODEX_CHINESE_THINKING=true` 时注入 |

### 响应方向（DeepSeek → Codex）

| Chat API 字段 | 转换为 Responses API 字段 |
|---|---|
| `choices[0].message.content` | `output[0].content[0].text` |
| `choices[0].message.reasoning_content` | 流式 → `response.reasoning_text.delta` |
| `choices[0].message.tool_calls` | `output[{type:"function_call", call_id, name, arguments}]` |
| `choices[0].finish_reason` | `output[0].type: "message"` |
| `usage.prompt_tokens` | `usage.input_tokens` |
| `usage.completion_tokens` | `usage.output_tokens` |
| `usage.completion_tokens_details.reasoning_tokens` | 日志打印 |
| `usage.prompt_cache_hit_tokens` | 日志打印 |
| `usage.prompt_cache_miss_tokens` | 日志打印 |
| `id` (chat cmpl id) | `id` (response id, 格式适配) |
| `model` | `model` |

### Responses 管理端点

| 端点 | 支持情况 |
|---|---|
| `GET /v1/responses/{response_id}` | 读取本地保存的 response；`stream=true` 会回放最小 SSE |
| `DELETE /v1/responses/{response_id}` | 删除 response、history 和 input_items |
| `POST /v1/responses/{response_id}/cancel` | 标记 `queued` / `in_progress` response 为 `cancelled`，并 abort 后台任务 |
| `GET /v1/responses/{response_id}/input_items` | 支持 `after`、`limit`、`order=asc/desc`，非法 cursor/order 返回 400 |
| `POST /v1/responses/compact` | 返回简化 `response.compacted` |
| `POST /v1/responses/input_tokens` | 返回本地近似 token 数 |
| `POST /v1/conversations` | 创建本地内存 conversation |
| `GET /v1/conversations/{conversation_id}` | 读取本地 conversation |
| `DELETE /v1/conversations/{conversation_id}` | 删除本地 conversation |
| `GET /v1/conversations/{conversation_id}/items` | 列出本地 conversation items |

### 思考等级映射（六级）

| Codex `reasoning.effort` | DeepSeek `reasoning_effort` | DeepSeek `thinking` |
|---|---|---|
| `none` | `low` | `disabled` |
| `minimal` | `low` | `disabled` |
| `low` | — | `disabled` |
| `medium` | `high` | `enabled` |
| `high` | `high` | `enabled` |
| `xhigh` | `max` | `enabled` |
| 无字段（工具调用等） | `high` | `enabled` |

## 功能详解

### 工具转发引擎

Codex 发送的工具定义（tools）与 OpenAI Chat API 格式不同，deecodex 在 `translate.rs` 中实现了一套完整的工具转换管道：

1. **类型识别** — 按 `type` 字段分类：`function` / `custom` / `namespace`
2. **namespace 展开** — MCP 命名空间工具（如 `mcp__filesystem__read`）拆分为独立 function tool，子工具名前缀命名空间
3. **工具名去重** — 展开后按 `function.name` 去重（DeepSeek 要求工具名唯一）
4. **自定义工具包装** — `apply_patch` → 映射为 `exec_command`（参数 schema 一致），其他自定义工具生成通用参数 schema
5. **Web Search 检测** — `web_search_preview` 类型 → 开启 `web_search_options`

### 会话与思维链恢复

`session.rs` 实现三级 reasoning_content 恢复机制：

1. **call_id 精确匹配** — `function_call` 的 `call_id` → 上次响应的 `reasoning_content`
2. **turn 指纹匹配** — 根据 assistant 消息内容 + tool_call_ids 组合指纹查找
3. **历史扫描** — 扫描整个对话历史中最近匹配的内容

全部为内存存储，服务重启后 Codex 会重放完整历史，deecodex 从重放中重建。

### 请求缓存

`cache.rs` 基于请求体 JSON 哈希值的 LRU 缓存：

- 最大 128 条目，超限时淘汰最早条目
- 缓存内容包括：完整 SSE 事件序列、tool call 数据、用量统计
- 命中时直接回放缓存流，不请求上游
- 日志标注 `cache hit` / `cache miss`

### 流式处理

`stream.rs` 处理 DeepSeek SSE 流，完成以下翻译：

- **Delta 合并** — 将 DeepSeek 的 `choices[0].delta` 聚合为完整消息
- **reasoning_content 流式输出** — 通过 `response.reasoning_text.delta` SSE 事件发送
- **Tool call delta 流式重建** — 按 `index` 分组增量参数，补齐 `id` 和 `name`
- **名称透明替换** — `apply_patch` → `exec_command` 名称映射贯穿流
- **用量恢复** — 从最终 chunk 的 `usage` 字段和 `include_usage` 流事件中重建
- **缓存回放** — 从缓存读取完整 SSE 事件序列，按原始顺序重放

### 自动重试

在 `stream.rs` 中内置重试逻辑：

- **触发条件** — HTTP 429/502/503 + `reasoning_content must be passed back` 错误
- **退避策略** — 固定延迟重试（非指数，简化实现）
- **最大次数** — 3 次
- **特殊处理** — `reasoning_content` 丢失时禁用 thinking 重新发送

### 视觉路由

多模态请求路由逻辑：

1. 检测 `has_images` 标志（从 content 数组中识别 `image_url`/`input_image`/base64）
2. 判断条件：有图片 + 配置了视觉上游 + **首回合**（消息数 ≤ 3） + **非轻量模型**（非 gpt-5.4/auto-review）
3. 符合条件 → 构建 VLM 请求体并发送到 `CODEX_RELAY_VISION_ENDPOINT`
4. 不符合条件 → 自动剥离所有图片内容，走 DeepSeek

### 中文思考注入

`DEECODEX_CHINESE_THINKING=true` 时：

- 系统指令前置注入 "【核心指令：你的所有推理、思考和分析过程必须全程使用中文..."
- 最后一条 user 消息前置注入 "【你的推理过程必须使用中文。】"
- 不影响正常对话流程

## 环境变量

| 变量 | 说明 | 默认值 |
|---|---|---|
| `DEECODEX_UPSTREAM` | DeepSeek API 地址 | `https://api.deepseek.com/v1` |
| `DEECODEX_API_KEY` | DeepSeek API Key | **（必填）** |
| `DEECODEX_PORT` | 监听端口 | `4446` |
| `DEECODEX_MODEL_MAP` | 模型映射 JSON | 见下 |
| `DEECODEX_CHINESE_THINKING` | 中文思考注入 | `false` |
| `DEECODEX_ALLOWED_MCP_SERVERS` | MCP 工具 server 白名单，逗号分隔 | `""` |
| `DEECODEX_ALLOWED_COMPUTER_DISPLAYS` | computer_use display/environment 白名单，逗号分隔 | `""` |
| `DEECODEX_COMPUTER_EXECUTOR` | 本地 computer 执行器后端：`disabled` / `playwright` / `browser-use` | `disabled` |
| `DEECODEX_COMPUTER_EXECUTOR_TIMEOUT_SECS` | 本地 computer 单步超时秒数 | `30` |
| `DEECODEX_MCP_EXECUTOR_CONFIG` | MCP server JSON 对象/数组，或 JSON 文件路径；为空则不执行本地 MCP | `""` |
| `DEECODEX_MCP_EXECUTOR_TIMEOUT_SECS` | MCP 单次工具调用超时秒数 | `30` |
| `CODEX_RELAY_MAX_BODY_MB` | 请求体上限 | `100` |
| `CODEX_RELAY_VISION_UPSTREAM` | MiniMax API 地址 | `""` |
| `CODEX_RELAY_VISION_API_KEY` | MiniMax API Key | `""` |
| `CODEX_RELAY_VISION_MODEL` | 视觉模型名 | `MiniMax-M1` |
| `CODEX_RELAY_VISION_ENDPOINT` | 视觉端点路径 | `v1/coding_plan/vlm` |

### 模型映射

```json
{
  "GPT-5.5": "deepseek-v4-pro",
  "gpt-5.5": "deepseek-v4-pro",
  "gpt-5.4": "deepseek-v4-flash",
  "gpt-5.4-mini": "deepseek-v4-flash",
  "codex-auto-review": "deepseek-v4-flash"
}
```

## 日常管理

### 调试端点

```bash
# 获取已存储的响应
curl http://127.0.0.1:4446/v1/responses/<response_id>

# 获取响应输入项
curl http://127.0.0.1:4446/v1/responses/<response_id>/input_items

# 取消正在进行的响应
curl -X POST http://127.0.0.1:4446/v1/responses/<response_id>/cancel

# 健康检查
curl http://127.0.0.1:4446/v1/health

# 模型列表
curl http://127.0.0.1:4446/v1/models
```


```bash
./deecodex.sh start      # 启动（自动日志轮转）
./deecodex.sh stop       # 停止（10s 优雅超时）
./deecodex.sh restart    # 重启
./deecodex.sh status     # PID + 端口
./deecodex.sh logs       # 实时日志
./deecodex.sh health     # 健康检查
```

## 日志解读

```
← codex: model=gpt-5.5 reasoning.effort=Some("medium")    ← 原始请求
→ upstream: model=deepseek-v4-pro effort=high thinking=on msgs=12  ← 转换后参数
↑ done in=41067 out=171 hit=40576 miss=491                  ← 流完成 + 用量
📷 routing to vision upstream: https://api.minimaxi.com     ← 视觉路由
cache hit for hash=0xabcd1234                               ← 缓存命中
```

## 故障排查

| 问题 | 原因 | 解决 |
|---|---|---|
| connection refused | deecodex 未启动 | `./deecodex.sh start` |
| model not found | 映射表缺失/DeepSeek 模型名变更 | 更新 `DEECODEX_MODEL_MAP` |
| image_url 错误 | 历史消息含图片 | 已自动剥离，仍出现则重启 |
| reasoning_content 错误 | 思维链恢复失败 | 自动重试，仍出现则减少上下文 |
| 413 请求体过大 | 图片太大 | `CODEX_RELAY_MAX_BODY_MB=200` |

## License

MIT License. 初始代码基于 [codex-relay](https://github.com/MetaFARS/codex-relay) (MIT)，后续功能已全面重写（Rust 源码 2,920 行，相对上游增加 58%，每文件 40-70% 重写）。
