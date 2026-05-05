# deecodex

**DeepSeek API → Codex CLI 兼容代理**

将 Codex CLI 发出的 **Responses API** 请求实时翻译为 **Chat Completions API**，使 Codex 可以原生对接 DeepSeek 等第三方模型，同时保留思考模式、工具调用、Web Search 等完整功能。可选配 MiniMax 视觉路由支持多模态图片理解。

```
Codex CLI 发出 /v1/responses (gpt-5.5 / gpt-5.4 等模型名)
        │
        ▼
  deecodex (协议翻译 + 模型映射 + 思考适配 + 缓存 + 重试)
        │
        ▼
  api.deepseek.com/v1/chat/completions
```

## 安装

### 从源码编译（推荐）

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

### 验证安装

```bash
deecodex --help
```

## 快速开始

### 1. 配置环境变量

```bash
cp .env.example .env
vim .env
```

填入你的 DeepSeek API Key（登录 [platform.deepseek.com](https://platform.deepseek.com) → API Keys）：

```
DEECODEX_API_KEY=sk-your-real-key-here
```

### 2. 启动服务

```bash
./deecodex.sh start
./deecodex.sh health    # 确认返回 healthy
```

### 3. 配置 Codex 桌面版

编辑 `~/.codex/config.toml`：

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

重启 Codex，选择 `custom` provider 即可使用。

### 4. 验证连通

在 Codex 发一条消息。日志出现以下两行说明成功：

```
← codex: model=gpt-5.5 reasoning.effort=Some("medium")
→ upstream: model=deepseek-v4-pro stream=true effort=Some("high") msgs=12
```

## 项目文件

```
deecodex/
├── Cargo.toml          # Rust 项目配置
├── src/
│   ├── main.rs         # 服务入口 + HTTP 路由
│   ├── translate.rs    # Responses ↔ Chat 协议翻译
│   ├── stream.rs       # SSE 流式响应翻译
│   ├── session.rs      # 会话管理 + reasoning_content 回传
│   ├── types.rs        # 请求/响应类型定义
│   ├── cache.rs        # 请求缓存（LRU 128 条目）
│   └── lib.rs          # 模块导出
├── deecodex.sh         # 管理脚本
├── .env.example        # 环境变量模板
└── tests/              # 33 个集成测试
```

## 环境变量

编辑 `.env`（或复制 `.env.example`）：

| 变量 | 说明 | 默认值 |
|------|------|--------|
| `DEECODEX_UPSTREAM` | DeepSeek API 地址 | `https://api.deepseek.com/v1` |
| `DEECODEX_API_KEY` | DeepSeek API Key | **（必填）** |
| `DEECODEX_PORT` | 本地监听端口 | `4446` |
| `DEECODEX_MODEL_MAP` | 模型名映射 JSON | 见下方 |
| `DEECODEX_CHINESE_THINKING` | 中文思考提示 | `false` |
| `CODEX_RELAY_*` 前缀 | 兼容上游变量名 | — |

### 视觉路由（可选）

| 变量 | 说明 | 默认值 |
|------|------|--------|
| `CODEX_RELAY_VISION_UPSTREAM` | MiniMax API 地址 | `""`（不配置则丢弃图片） |
| `CODEX_RELAY_VISION_API_KEY` | MiniMax API Key | `""` |
| `CODEX_RELAY_VISION_MODEL` | 视觉模型名 | `MiniMax-M1` |
| `CODEX_RELAY_VISION_ENDPOINT` | 视觉端点路径 | `v1/coding_plan/vlm` |

### 模型映射

Codex 硬编码的 OpenAI 模型名 → DeepSeek 实际模型名：

```json
{
  "GPT-5.5": "deepseek-v4-pro",
  "gpt-5.5": "deepseek-v4-pro",
  "gpt-5.4": "deepseek-v4-flash",
  "gpt-5.4-mini": "deepseek-v4-flash",
  "codex-auto-review": "deepseek-v4-flash"
}
```

键名**大小写敏感**，建议大小写都加。验证当前可用模型：

```bash
curl -s https://api.deepseek.com/v1/models \
  -H "Authorization: Bearer $DEECODEX_API_KEY" \
  | jq '.data[].id'
```

## 日常管理

```bash
./deecodex.sh start      # 启动
./deecodex.sh stop       # 停止（10s 优雅超时后强杀）
./deecodex.sh restart    # 重启
./deecodex.sh status     # 查看 PID / 端口
./deecodex.sh logs       # 实时日志
./deecodex.sh health     # 健康检查
```

### CC Switch 配置

在 CC Switch 中新增 Codex provider，只需填写：

| 字段 | 值 |
|------|-----|
| API 请求地址 | `http://127.0.0.1:4446/v1` |
| API Key | 任意字符串（如 `sk-123456`） |

`wire_api = "responses"` 和 `requires_openai_auth = true` 由 deecodex 自动处理。

## 功能特性

### 协议翻译

- **Responses → Chat 翻译** — 实时双向协议转换，保持流式响应完整
- **思考等级六级映射** — Codex `reasoning.effort`（none/minimal/low/medium/high/xhigh）→ DeepSeek 参数
- **`tool_choice` / `parallel_tool_calls` / `top_p` 透传** — 从 Responses API 透传到 Chat API
- **`content null` 修复** — 空内容用 `""` 代替 `null`，避免 DeepSeek 400 错误

### 工具调用

- **自定义工具转发** — `apply_patch` 等非标准工具转为标准 function 类型，带参数 schema
- **MCP 命名空间展开** — 命名空间工具展开为独立 function tools
- **工具名自动去重** — MCP 展开后按 `function.name` 去重，修复 DeepSeek 400
- **Web Search 适配** — Codex `web_search_preview` → DeepSeek `web_search_options`
- **工具丢弃日志去重** — 同名工具首次 WARN，后续 DEBUG，不刷屏

### 多模态视觉（可选）

- **自动路由** — 检测图片内容，首回合路由 MiniMax VLM，后续回合走 DeepSeek
- **智能决策** — gpt-5.4 / auto-review 跳过 VLM；历史图片自动剥离避免 DeepSeek 400
- **双格式支持** — MiniMax VLM 端点 + Anthropic 格式自动转换
- **Base64 内嵌检测** — 支持 `input_image` 和 `image_url` 两种图片格式

### 可靠性

- **请求缓存** — 基于 ChatRequest JSON 哈希，LRU 128 条目，命中时直接回放缓存的 SSE 流
- **通用重试** — 429/502/503 + `reasoning_content` 丢失错误，指数退避最多 3 次
- **reasoning 流式输出** — 通过 `response.reasoning_text.delta` SSE 事件发送给 Codex，UI 可见思考过程
- **Health 端点** — `GET /health` → `{"status":"ok","uptime_secs":N}`

### 安全

- **`--api-key` CLI 参数移除** — 仅从环境变量读取，避免 `ps aux` 泄露
- **请求体上限** — 100MB，支持大图片/大上下文

## 日志解读

每请求打印两行关键信息：

```
← codex: model=gpt-5.5 reasoning.effort=Some("medium")
→ upstream: model=deepseek-v4-pro stream=true effort=Some("high") thinking=Some({"type":"enabled"}) msgs=12
```

- `←` 行：Codex 原始请求（模型 + 思考等级）
- `→` 行：转换后发给 DeepSeek 的参数（`msgs`=消息数）

流结束时打印 token 统计：

```
↑ done in=41067 out=171 hit=40576 miss=491
```

## 思考等级映射

| Codex `reasoning.effort` | 传递给 DeepSeek |
|--------------------------|----------------|
| `none` | 等效 `low`，关闭思考 |
| `minimal` | 等效 `low`，关闭思考 |
| `low` | `thinking: {"type":"disabled"}` |
| `medium` | `reasoning_effort: "high"` + `thinking: enabled` |
| `high` | `reasoning_effort: "high"` + `thinking: enabled` |
| `xhigh` | `reasoning_effort: "max"` + `thinking: enabled` |
| 无此字段（工具调用等） | `reasoning_effort: "high"` + `thinking: enabled` |

## 故障排查

### 服务无法启动
```bash
lsof -i :4446           # 检查端口占用
which deecodex          # 检查二进制
./deecodex.sh logs      # 查看启动日志
```

### Codex "connection refused"
- deecodex 没启动 → `./deecodex.sh start && ./deecodex.sh health`
- `base_url` 末尾多 `/` → 去掉末尾 `/`
- 端口不匹配 → 检查 `DEECODEX_PORT` 和 CC Switch 配置

### Codex "model not found"
- DeepSeek 模型名更新 → 用 `curl` 查最新模型名
- 映射表缺少大小写变体 → 大小写都加
- Codex 用了映射表外的模型名 → 看日志 `←` 行，补一条映射

### 图片发送后一直转圈
- 没配视觉上游 → 图片自动丢弃走 DeepSeek
- 已配视觉上游 → 检查 API Key / 端点地址
- 日志看是否有 `📷 routing to vision upstream`

### 413 Payload Too Large
```env
CODEX_RELAY_MAX_BODY_MB=200
```

## 与上游的关系

基于 [codex-relay](https://github.com/MetaFARS/codex-relay) (MIT) 深度重写，改动量超过 60%：

1. **模型名映射** — `--model-map` 参数 + 环境变量
2. **思考模式适配** — Codex `reasoning.effort` → DeepSeek 参数，六级映射
3. **图片自动丢弃 / 视觉路由** — MiniMax VLM 可选路由
4. **请求体上限** — 2MB → 100MB
5. **工具转发** — MCP 展开 + 工具去重 + Web Search
6. **请求缓存** — LRU 128 条目哈希缓存
7. **通用重试** — 指数退避最多 3 次
8. **Security** — `--api-key` 从 CLI 移除
9. **日志增强** — 原始/转换参数对比打印
10. **`content null` 修复**、**Health 端点**、**中文思考提示**
