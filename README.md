# deecodex

**DeepSeek API → Codex CLI 兼容代理**

将 OpenAI Codex CLI 发出的 **Responses API** 请求实时翻译为 **Chat Completions API**，使 Codex 可以原生对接 DeepSeek，同时保留思考模式、工具调用等完整功能。

```
Codex CLI  →  /v1/responses (gpt-5.5 / gpt-5.4 等模型名)
                │
                ▼
          deecodex（协议翻译 + 模型映射 + 思考等级适配）
                │
                ▼
          api.deepseek.com/v1/chat/completions
```

## 安装

### 方式一：下载预编译二进制（推荐）

从 [Releases](https://github.com/liguan-89/deecodex/releases) 下载对应平台的二进制：

```bash
# macOS ARM64 示例
curl -L https://github.com/liguan-89/deecodex/releases/download/v0.3.0/deecodex -o deecodex
chmod +x deecodex
mv deecodex ~/.local/bin/

# 下载管理脚本和环境变量模板
curl -L https://github.com/liguan-89/deecodex/releases/download/v0.3.0/deecodex.sh -o deecodex.sh
curl -L https://github.com/liguan-89/deecodex/releases/download/v0.3.0/env.example -o .env.example
chmod +x deecodex.sh
```

### 方式二：源码编译

```bash
git clone https://github.com/liguan-89/deecodex.git
cd deecodex
cargo build --release
cp target/release/deecodex ~/.local/bin/
```

## 配置

```bash
cp .env.example .env
vim .env
```

填入 DeepSeek API Key（[获取地址](https://platform.deepseek.com/)）：

```bash
DEECODEX_API_KEY=sk-your-real-key-here
```

其他配置保持默认即可。

## 启动

```bash
./deecodex.sh start     # 启动服务（后台运行）
./deecodex.sh health    # 确认返回 healthy
./deecodex.sh logs      # 查看实时日志
```

## 配置 Codex 桌面端

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

保存后重启 Codex 桌面端，在设置中选择 `custom` provider。

> ⚠️ `base_url` 末尾不要加 `/`，端口号必须与 `.env` 中 `DEECODEX_PORT` 一致。

### CC Switch 用户

如果你使用 [CC Switch](https://github.com/YiNNx/cc-switch)，只需添加 provider：

| 字段 | 值 |
|------|-----|
| API 请求地址 | `http://127.0.0.1:4446/v1` |
| API Key | 任意字符串（不会被校验） |

## 验证

在 Codex 中发一条消息，日志中出现 `← codex:` 和 `→ upstream:` 即表示正常。

```bash
./deecodex.sh logs
# ← codex: model=gpt-5.5 reasoning.effort=Some("medium")
# → upstream: model=deepseek-v4-pro stream=true msgs=5
```

## 日常管理

```bash
./deecodex.sh start      # 启动
./deecodex.sh stop       # 停止（10s 优雅超时）
./deecodex.sh restart    # 重启
./deecodex.sh status     # 查看 PID / 端口
./deecodex.sh logs       # 实时日志（Ctrl+C 退出）
./deecodex.sh health     # 健康检查
```

## 环境变量

| 变量 | 说明 | 默认值 |
|------|------|--------|
| `DEECODEX_UPSTREAM` | DeepSeek API 地址 | `https://api.deepseek.com/v1` |
| `DEECODEX_API_KEY` | DeepSeek API Key | （必填） |
| `DEECODEX_PORT` | 监听端口 | `4446` |
| `DEECODEX_MODEL_MAP` | 模型名映射 JSON | 见下方 |
| `RUST_LOG` | 日志级别 | `deecodex=info` |

兼容旧名 `CODEX_RELAY_*`，`DEECODEX_*` 优先级更高。

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

如果 DeepSeek 更新模型名，同步更新此映射。键名大小写敏感，大小写都要加。

验证当前可用模型：

```bash
curl -s https://api.deepseek.com/v1/models \
  -H "Authorization: Bearer $DEECODEX_API_KEY" \
  | jq '.data[].id'
```

## 思考等级映射

| Codex `reasoning.effort` | DeepSeek 参数 |
|--------------------------|---------------|
| `low` | `thinking: {"type":"disabled"}` |
| `medium` | `reasoning_effort: "high"` + `thinking: enabled` |
| `high` | `reasoning_effort: "high"` + `thinking: enabled` |
| `xhigh` | `reasoning_effort: "max"` + `thinking: enabled` |
| 无此字段 | `reasoning_effort: "high"` + `thinking: enabled` |

## 功能支持

| 功能 | 状态 | 说明 |
|------|------|------|
| 思考模式 | ✅ | 自动注入 `thinking`，透传 `reasoning_effort` |
| 模型名映射 | ✅ | GPT-5.5→v4-pro, gpt-5.4→v4-flash |
| Tool Calls | ✅ | 格式转换 + reasoning_content 回传 |
| 流式输出 | ✅ | SSE 流透传 |
| 多模态（图片） | ⚠️ 自动丢弃 | DeepSeek V4 不支持图片输入 |

## 故障排查

### connection refused / 404

deecodex 未启动或配置错误：
```bash
./deecodex.sh start && ./deecodex.sh health
```
检查 `config.toml` 中 `base_url` 为 `http://127.0.0.1:4446/v1`（末尾无 `/`），端口与 `.env` 一致。

### model not found

DeepSeek 模型名变更，用 `curl` 查最新模型名后更新 `DEECODEX_MODEL_MAP`。

### 一直转圈不返回

检查日志中是否出现 `← codex:` 行。未出现说明 Codex 没连上 deecodex；有 `←` 无 `→` 说明 DeepSeek 不可达或 API Key 无效。

### 413 Payload Too Large

图片过大，在 `.env` 中提高上限：
```bash
CODEX_RELAY_MAX_BODY_MB=200
```

### 日志中的 WARN 行

```
dropping 3 unsupported tool(s): ["apply_patch", "web_search", ...]
```

这是 deecodex 过滤 Codex 非标准工具的提示，不影响使用。
