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

## 安装

### 1. 安装 deecodex 二进制

**pipx 安装（推荐）：**

```bash
pipx install codex-relay
ln -sf ~/.local/pipx/venvs/codex-relay/bin/codex-relay ~/.local/bin/deecodex
```

验证安装：

```bash
deecodex --version
pipx list | grep codex-relay
```

二进制名为 `deecodex`，底层基于 [codex-relay](https://github.com/MetaFARS/codex-relay) (MIT) 深度修改。

### 2. 克隆配置仓库

```bash
cd ~
git clone https://github.com/liguan-89/deecodex.git
cd deecodex
```

### 3. 配置环境变量

```bash
cp .env.example .env
vim .env
```

填入你的 DeepSeek API Key：

```
DEECODEX_API_KEY=sk-your-real-key-here
```

其他配置保持默认即可。获取 Key：登录 [platform.deepseek.com](https://platform.deepseek.com/) → API Keys

### 4. 启动服务

```bash
./deecodex.sh start
./deecodex.sh health    # 确认返回 healthy
```

首次启动可能等 1-2s，确认日志正常：

```bash
./deecodex.sh logs
# 看到 ← codex 和 → upstream 行说明成功
```

### 5. 配置 Codex 桌面版

编辑 `~/.codex/config.toml`（如文件不存在则新建）：

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

保存后重启 Codex 桌面版，在设置中选择 `custom` provider。

**常见配置错误：**
- `base_url` 写成了 `http://127.0.0.1:4446/v1/`（末尾多斜杠） → 去掉末尾 `/`
- `wire_api` 拼写错误或漏写 → 必须为 `"responses"`
- 端口号写成了 `4444`（默认值） → 改为 `4446`
- `requires_openai_auth = false` 或漏写 → 必须为 `true`

### 6. 验证连通

在 Codex 里随便发一条消息。如果日志里出现 `← codex:` 和 `→ upstream:` 两行，说明一切正常。

---

## 项目文件结构

```
~/deecodex/
├── .env               # （已 gitignore）你的 API Key 等配置
├── .env.example       # 配置模板，直接复制使用
├── deecodex.sh        # 管理脚本（启动/停止/日志/健康检查）
├── logs/              # （已 gitignore）日志目录，50MB 自动轮转，保留 5 份
└── deecodex.pid       # （已 gitignore）运行 PID，自动生成
```

## 环境变量

编辑 `.env`（或复制 `.env.example` 修改）：

| 变量 | 说明 | 默认值 |
|------|------|--------|
| `DEECODEX_UPSTREAM` | DeepSeek API 地址 | `https://api.deepseek.com/v1` |
| `DEECODEX_API_KEY` | DeepSeek API Key | （必填） |
| `DEECODEX_PORT` | 本地监听端口 | `4446` |
| `DEECODEX_MODEL_MAP` | 模型名映射 JSON | 见下方 |
| `RUST_LOG` | 日志级别 | `codex_relay=info` |

兼容旧名 `CODEX_RELAY_*`（二进制原生变量），`DEECODEX_*` 优先级更高。

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

如果 DeepSeek 更新了模型名，需要同时更新这个映射。**键名大小写敏感**，大小写都要加。

验证当前可用的 DeepSeek 模型名：

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
./deecodex.sh logs       # 实时日志（Ctrl+C 退出）
./deecodex.sh health     # 健康检查（GET /v1/models）
```

## 思考等级映射

deecodex 捕获 Codex Responses API 的 `reasoning.effort` 字段，映射到 DeepSeek 参数：

| Codex `reasoning.effort` | 传递给 DeepSeek |
|--------------------------|----------------|
| `low` | `thinking: {"type":"disabled"}`（关闭思考） |
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

- `←` 行：Codex 发来的原始请求（模型 + 思考等级）
- `→` 行：转换后发给 DeepSeek 的实际参数（`msgs`=消息数）

流结束时打印 token 统计：

```
↑ done in=41067 out=171 hit=40576 miss=491
```

**日志里常见的 WARN 行（正常现象）：**

```
dropping 3 unsupported tool(s): ["apply_patch", "web_search", ...]
```

这是 deecodex 在过滤 Codex 独有的非标准工具，不影响使用。

## 故障排查

### 1. 服务无法启动

```bash
# 检查端口是否被占用
lsof -i :4446

# 检查二进制是否存在
which deecodex

# 查看启动日志
./deecodex.sh logs
```

### 2. Codex 报错 "connection refused" 或 404

- deecodex 没启动 → 执行 `./deecodex.sh start && ./deecodex.sh health`
- Codex 配置中 `base_url` 写错了 → 必须为 `http://127.0.0.1:4446/v1`（末尾无 `/`）
- 端口不匹配 → `.env` 里 `DEECODEX_PORT` 和 `config.toml` 里的端口号必须一致

### 3. Codex 报 "model not found" 或 "gpt-5.5 not recognized"

- DeepSeek 的模型名变了 → 用 `curl` 查最新模型名，更新 `DEECODEX_MODEL_MAP`
- 映射表里缺少 Key 的大小写变体 → 大小写都加一遍
- Codex 用了映射表里没有的模型名 → 参考日志里 `← codex:` 行显示的模型名，补一条

### 4. Codex 一直在转圈但不返回

- 检查日志，看是 `←` 行都没出现（Codex 没发请求），还是只有 `←` 没 `→`（deecodex 处理失败）
- DeepSeek API 不可达 → 检查网络，检查 API Key 余额
- `lsof -i :4446` 确认服务确实在监听

### 5. 413 Payload Too Large

图片太大，已设 100MB 上限。如果还是超限，在 `.env` 加：

```env
CODEX_RELAY_MAX_BODY_MB=200
```

### 6. 其他已知问题

- **reasoning_content must be passed back**：思考模式下工具调用链回传不完整，部分场景偶发
- **image_url unknown variant**：DeepSeek V4 不支持图片，deecodex 已自动丢弃，日志会打印丢弃记录
- **content null 错误**：空内容用 `""` 代替 `null`，避免 DeepSeek 400

---

## 功能支持矩阵

| DeepSeek 功能 | 支持 | 说明 |
|-------------|------|------|
| 思考模式 | ✅ | 自动注入 `thinking: {type: enabled}`，透传 `reasoning_effort` |
| 模型名映射 | ✅ | GPT-5.5→v4-pro, gpt-5.4/gpt-5.4-mini→v4-flash |
| Tool Calls | ✅ | 格式转换 + reasoning_content 回传 |
| 流式输出 | ✅ | SSE 流透传 |
| 多模态（图片） | ⚠️ 自动丢弃 | DeepSeek V4 不支持，保留文字丢弃图片 |
| JSON Output | — | Codex 用 Tool Calls 实现结构化输出 |
| FIM 补全 | — | Codex 不走 `/v1/completions` |

## 与上游的关系

基于 [codex-relay](https://github.com/MetaFARS/codex-relay) (MIT) 深度修改：

1. **模型名映射** — `--model-map` 参数 + 环境变量
2. **思考模式** — 适配 Codex `reasoning.effort` → DeepSeek 参数
3. **图片自动丢弃** — 过滤 image 内容
4. **请求体上限** — 2MB → 100MB
5. **调试日志** — 打印原始/转换后的关键参数
6. **content null 修复** — 空内容用 `""` 代替 `null`
7. **安全入口** — 移除 `--thinking` CLI 参数，思考逻辑统一在 translate.rs 处理
