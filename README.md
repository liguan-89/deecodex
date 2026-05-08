# deecodex

**DeepSeek API → Codex CLI 兼容代理 · 本地 Responses 增强层**

将 Codex CLI 发出的 **Responses API** 请求实时翻译为 **Chat Completions API**，使 Codex 可以原生对接 DeepSeek，同时保留思考模式、工具调用等完整功能。

内置本地增强层：Files/Vector Store API、file_search（BM25 chunk 级索引）、computer/MCP executor、配置诊断、视觉路由等。

```
Codex CLI  →  /v1/responses (gpt-5.5 / gpt-5.4)
                │
                ▼
          deecodex（协议翻译 + 模型映射 + 增强层 + 视觉路由）
                │
                ▼
          api.deepseek.com/v1/chat/completions
```

## 依赖要求

| 依赖 | 用途 | 必需 |
|------|------|------|
| Rust 1.80+ | 源码编译 | 仅源码安装 |
| `~/.local/bin` 在 PATH | 二进制存放 | 是 |

以下为可选依赖，仅在使用对应功能时需要：

| 依赖 | 用途 |
|------|------|
| Node.js（可 `import("playwright")`） | Playwright computer executor |
| `mcp-filesystem` 等 MCP server | 本地 MCP tool 执行 |

## 安装

### 方式一：下载预编译二进制（推荐）

从 [Releases](https://github.com/liguan-89/deecodex/releases) 下载对应平台版本：

**macOS / Linux：**

```bash
curl -L https://github.com/liguan-89/deecodex/releases/download/v1.0.0/deecodex -o deecodex
chmod +x deecodex && mv deecodex ~/.local/bin/

# 管理脚本和配置模板
curl -L https://github.com/liguan-89/deecodex/releases/download/v1.0.0/deecodex.sh -o deecodex.sh
curl -L https://github.com/liguan-89/deecodex/releases/download/v1.0.0/env.example -o .env.example
chmod +x deecodex.sh
```

**Windows（PowerShell 一键安装）：**

```powershell
irm https://raw.githubusercontent.com/liguan-89/deecodex/main/install.ps1 | iex
```

脚本自动完成：下载文件 → 添加 PATH → 生成配置模板。安装后编辑 `.env` 填入 API Key 即可使用。

**Windows 便携版（免安装）：**

1. 下载 [`deecodex-windows-portable.zip`](https://github.com/liguan-89/deecodex/releases/download/v1.0.0/deecodex-windows-portable.zip)
2. 解压到任意目录
3. 将 `.env.example` 重命名为 `.env`，用记事本填入 DeepSeek API Key
4. 双击 `deecodex.bat` 或在命令行运行 `deecodex.bat start`

### 方式二：源码编译

```bash
git clone https://github.com/liguan-89/deecodex.git
cd deecodex
cargo build --release
# macOS/Linux
cp target/release/deecodex ~/.local/bin/
# Windows
copy target\release\deecodex.exe C:\Users\%USERNAME%\AppData\Local\Programs\deecodex\
```

验证安装：

```bash
deecodex --help
```

## 快速开始

**macOS / Linux：**

```bash
cp .env.example .env
vim .env                                # 填入 DEECODEX_API_KEY
./deecodex.sh start                     # 启动服务
./deecodex.sh health                    # 确认 healthy
```

**Windows：**

```cmd
copy env.example .env
notepad .env                            # 填入 DEECODEX_API_KEY
deecodex.bat start                      # 启动服务
deecodex.bat health                     # 确认 healthy
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

> ⚠️ `base_url` 末尾不要加 `/`，端口须与 `.env` 中 `DEECODEX_PORT` 一致。

CC Switch 用户只需填 API 请求地址 `http://127.0.0.1:4446/v1` 和任意 API Key。

## 日常管理

| 命令 | macOS / Linux | Windows |
|------|--------------|---------|
| 启动 | `./deecodex.sh start` | `deecodex.bat start` |
| 停止 | `./deecodex.sh stop` | `deecodex.bat stop` |
| 重启 | `./deecodex.sh restart` | `deecodex.bat restart` |
| 状态 | `./deecodex.sh status` | `deecodex.bat status` |
| 日志 | `./deecodex.sh logs` | `deecodex.bat logs` |
| 健康检查 | `./deecodex.sh health` | `deecodex.bat health` |

启动时自动注入 Codex 配置，停止时自动还原。

## v1.0.0 核心功能

### 协议翻译

完整的 Responses API ↔ Chat Completions API 双向翻译，覆盖 13 个端点、9 种工具类型、11 种流式事件。

| 端点 | 说明 |
|------|------|
| `POST /v1/responses` | 创建响应（流式/非流式） |
| `GET /v1/responses/:id` | 获取已存储的响应 |
| `DELETE /v1/responses/:id` | 删除响应 |
| `POST /v1/responses/:id/cancel` | 取消进行中的响应 |
| `GET /v1/responses/:id/input_items` | 获取输入项列表 |
| `POST /v1/responses/compact` | 压缩响应 |
| `POST /v1/responses/input_tokens` | token 计数 |
| Conversations CRUD | 会话管理（4 端点） |
| Files API | 文件上传/列表/读取/删除（5 端点） |
| Vector Stores API | 向量存储管理（10 端点） |
| Hosted Prompts | 本地模板注册表（2 端点） |
| `/v1/models` | 模型列表透传 |
| `/v1/health` | 健康检查 |

### 本地增强层

| 能力 | 说明 |
|------|------|
| **file_search** | chunk 级倒排索引 + BM25 排序，文件名独立加权 |
| **Computer executor** | Playwright/browser-use 后端，支持 `open_url`/`screenshot`/`click`/`type`/`keypress`/`scroll` |
| **MCP executor** | stdio JSON-RPC，read-only 保护 |
| **Files/Vector Stores** | 本地持久化，约束 file_search 范围 |
| **配置诊断** | 启动前校验 executor 配置、Playwright/node 可用性、MCP 命令存在性 |
| **中文思考注入** | `DEECODEX_CHINESE_THINKING=true` 自动注入 |
| **Token 异常检测** | prompt_explosion/spike/zero_completion/high_burn_rate |
| **视觉路由** | 多模态图片路由至 MiniMax VLM（可选配置） |

### 请求翻译要点

- **连续 function_call 自动合并**为单条 assistant 消息
- **MCP namespace 展开**为独立 function tools，按名称去重
- **apply_patch** → `exec_command` 名称映射
- **Web Search** 激活 DeepSeek `web_search_options`
- **图片检测**（`new_image`）替代启发式判断，精准路由视觉请求
- 六级思考等级映射（none/minimal/low/medium/high/xhigh）

### 响应流处理

- Delta 合并 + reasoning_content 流式输出
- Tool call delta 按 index 增量重建
- 三级 reasoning_content 恢复（call_id 匹配 / turn 指纹 / 历史扫描）
- LRU 请求缓存（128 条目）
- 自动重试（429/502/503 + reasoning_content 丢失，最多 3 次）

## 环境变量

| 变量 | 说明 | 默认值 |
|------|------|--------|
| `DEECODEX_UPSTREAM` | DeepSeek API 地址 | `https://api.deepseek.com/v1` |
| `DEECODEX_API_KEY` | DeepSeek API Key | **必填** |
| `DEECODEX_PORT` | 监听端口 | `4446` |
| `DEECODEX_MODEL_MAP` | 模型映射 JSON | 见 .env.example |
| `DEECODEX_CLIENT_API_KEY` | 本地 Bearer Token | 留空关闭鉴权 |
| `CODEX_RELAY_VISION_UPSTREAM` | MiniMax VLM 地址 | 留空关闭视觉路由 |
| `CODEX_RELAY_VISION_API_KEY` | MiniMax API Key | — |
| `DEECODEX_COMPUTER_EXECUTOR` | computer 后端：`disabled`/`playwright`/`browser-use` | `disabled` |
| `DEECODEX_MCP_EXECUTOR_CONFIG` | MCP server JSON 配置 | 留空不启用 |
| `DEECODEX_CHINESE_THINKING` | 中文思考注入 | `false` |
| `RUST_LOG` | 日志级别 | `deecodex=info` |

完整列表见 `.env.example`。

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

键名大小写敏感。更新模型名后需同步此映射。

## 日志解读

```
← codex: model=gpt-5.5 reasoning.effort=Some("medium")
→ upstream: model=deepseek-v4-pro effort=high thinking=on msgs=12
↑ done in=41067 out=171 hit=40576 miss=491
📷 routing to vision upstream
cache hit for hash=0xabcd1234
```

## 故障排查

| 问题 | 原因 | 解决 |
|------|------|------|
| connection refused | deecodex 未启动 | `./deecodex.sh start` |
| model not found | 映射表缺失/模型名变更 | 更新 `DEECODEX_MODEL_MAP` |
| 一直转圈 | DeepSeek 不可达或 API Key 无效 | 检查日志 `→ upstream` 行 |
| reasoning_content 错误 | 思维链恢复失败 | 自动重试，仍出现则减少上下文 |
| 413 Payload Too Large | 图片过大 | `CODEX_RELAY_MAX_BODY_MB=200` |
| 日志出现 WARN | 过滤 Codex 非标准工具 | 正常现象，不影响使用 |

## 项目结构

```
src/
├── main.rs          # 服务入口 + 服务管理
├── handlers.rs      # Axum HTTP handlers + AppState
├── translate.rs     # Responses → Chat 请求翻译
├── stream.rs        # Chat SSE → Responses SSE 流翻译
├── executor.rs      # computer/MCP 本地执行器
├── validate.rs      # 启动前配置诊断
├── files.rs         # 本地 Files API + file_search
├── vector_stores.rs # 本地 Vector Store API
├── session.rs       # 内存会话/对话存储
├── prompts.rs       # Hosted Prompts 注册表
├── types.rs         # 请求/响应类型定义
├── cache.rs         # LRU 请求缓存
├── sse.rs           # SSE 事件构建
├── token_anomaly.rs # Token 异常检测
├── ratelimit.rs     # 滑动窗口限流
├── metrics.rs       # Prometheus 指标
├── codex_config.rs  # Codex config.toml 注入/还原
├── config.rs        # 配置合并
├── tui.rs           # 中文 TUI 配置菜单
├── utils.rs         # 工具函数
└── lib.rs           # 库 crate 模块导出
```

测试：364 个（270 lib + 9 bin + 5 compat + 80 integration）

## 交流群

欢迎加入 deecodex 交流群，一起讨论、反馈、共建。

<img src="https://github.com/liguan-89/deecodex/releases/download/v1.0.0/deecodex.0515.jpg" width="200" alt="deecodex 交流群1" /> <img src="https://github.com/liguan-89/deecodex/releases/download/v1.0.0/_20260508195435_79_22.jpg" width="200" alt="deecodex 交流群2" />

## License

MIT License. 基于 [codex-relay](https://github.com/MetaFARS/codex-relay) (MIT) 深度修改，Rust 源码 ~14,000 行，18 个模块。
