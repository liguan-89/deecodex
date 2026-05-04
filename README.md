# deecodex

**DeepSeek API → Codex CLI 兼容代理**

将 OpenAI Codex CLI 发出的 **Responses API** 请求实时翻译为 **Chat Completions API**，使 Codex 可以原生对接 DeepSeek 等第三方模型，同时保留思考模式、工具调用等完整功能。

```
Codex CLI 发出 /v1/responses (gpt-5.5 / gpt-5.4 等模型名)
        │
        ▼
  deecodex (Responses → Chat 翻译 + 模型映射 + 思考等级适配)
        │
        ▼
  api.deepseek.com/v1/chat/completions
```

## 项目结构

```
~/deecodex/
├── .env               # 环境变量配置（含 API Key）
├── deecodex.sh        # 管理脚本
├── logs/              # 日志目录（50MB 自动轮转，保留 5 份）
└── deecodex.pid       # 运行 PID（自动生成）
```

二进制：`~/.local/bin/deecodex`（基于 [codex-relay](https://github.com/MetaFARS/codex-relay) 深度修改编译）

## 快速开始

```bash
# 1. 编辑配置
vim .env

# 2. 启动
./deecodex.sh start

# 3. Codex 桌面版配置（~/.codex/config.toml）
: '
model = "deepseek-v4-pro"
model_provider = "custom"
model_reasoning_effort = "medium"

[model_providers.custom]
base_url = "http://127.0.0.1:4446/v1"
name = "custom"
requires_openai_auth = true
wire_api = "responses"
'
```

## 环境变量

编辑 `.env`：

| 变量 | 说明 | 默认值 |
|------|------|--------|
| `DEECODEX_UPSTREAM` | DeepSeek API 地址 | `https://api.deepseek.com/v1` |
| `DEECODEX_API_KEY` | API Key | — |
| `DEECODEX_PORT` | 监听端口 | `4446` |
| `DEECODEX_MODEL_MAP` | 模型名映射 JSON | 见下方 |
| `RUST_LOG` | 日志级别 | `codex_relay=info` |

兼容旧名 `CODEX_RELAY_*`（二进制原生变量），`DEECODEX_*` 优先级更高。

### 模型映射

Codex 硬编码了 OpenAI 模型名，需要映射到 DeepSeek 实际模型：

```json
{
  "GPT-5.5": "deepseek-v4-pro",
  "gpt-5.5": "deepseek-v4-pro",
  "gpt-5.4": "deepseek-v4-flash",
  "gpt-5.4-mini": "deepseek-v4-flash",
  "codex-auto-review": "deepseek-v4-flash"
}
```

键名**大小写敏感**，建议大小写都加。

## 日常管理

```bash
./deecodex.sh start      # 启动
./deecodex.sh stop       # 停止（10s 优雅超时后强杀）
./deecodex.sh restart    # 重启
./deecodex.sh status     # 查看 PID / 端口
./deecodex.sh logs       # 实时日志
./deecodex.sh health     # 健康检查（GET /v1/models）
```

## 思考等级映射

deecodex 捕获 Codex Responses API 的 `reasoning.effort` 字段，映射到 DeepSeek 参数：

| Codex `reasoning.effort` | 传递给 DeepSeek |
|--------------------------|----------------|
| `low` | `thinking: {"type":"disabled"}` |
| `medium` | `reasoning_effort: "high"` + `thinking: enabled` |
| `high` | `reasoning_effort: "high"` + `thinking: enabled` |
| `xhigh` | `reasoning_effort: "max"` + `thinking: enabled` |
| 无此字段（工具调用等） | `reasoning_effort: "high"` + `thinking: enabled` |

v4-pro 和 v4-flash 使用统一的思考参数。

## 日志解读

每请求打印两行关键信息：

```
← codex: model=gpt-5.5 reasoning.effort=Some("medium")
→ upstream: model=deepseek-v4-pro stream=true effort=Some("high") thinking=Some({"type":"enabled"}) msgs=12
```

- `←` 行：Codex 发来的原始请求
- `→` 行：转换后发给 DeepSeek 的实际参数（`msgs`=消息数）

流结束时打印 token 统计：

```
↑ done in=41067 out=171 hit=40576 miss=491
```

## 功能支持

| DeepSeek 功能 | 支持 | 说明 |
|-------------|------|------|
| 思考模式 | ✅ | 自动注入 `thinking: {type: enabled}`，透传 `reasoning_effort` |
| 模型名映射 | ✅ | GPT-5.5→v4-pro, gpt-5.4/gpt-5.4-mini→v4-flash |
| Tool Calls | ✅ | 格式转换 + reasoning_content 回传 |
| 流式输出 | ✅ | SSE 流透传 |
| 多模态（图片） | ⚠️ 自动丢弃 | DeepSeek V4 不支持，保留文字丢弃图片 |
| JSON Output | — | Codex 用 Tool Calls 实现结构化输出 |
| FIM 补全 | — | Codex 不走 `/v1/completions` |

## 故障排查

- **413 Payload Too Large**：图片过大，已设 100MB 上限
- **gpt-5.5 not recognized**：模型映射表缺少该条目（检查大小写）
- **reasoning_content must be passed back**：思考模式下工具调用链回传不完整
- **image_url unknown variant**：DeepSeek V4 不支持图片，deecodex 已自动丢弃
- **content null 错误**：空内容用 `""` 代替 `null`，避免 DeepSeek 400

## 与上游的关系

基于 [codex-relay](https://github.com/MetaFARS/codex-relay) (MIT) 深度修改：

1. **模型名映射** — `--model-map` 参数 + 环境变量
2. **思考模式** — 适配 Codex `reasoning.effort` → DeepSeek 参数
3. **图片自动丢弃** — 过滤 image 内容
4. **请求体上限** — 2MB → 100MB
5. **调试日志** — 打印原始/转换后的关键参数
6. **content null 修复** — 空内容 `""` 代替 `null`
7. **安全入口** — 移除 `--thinking` CLI 参数，思考逻辑统一在 translate.rs 处理
