# deecodex

**Codex CLI 多模型代理 · 桌面 GUI 版**

deecodex 是一个运行在本地的轻量级代理服务，将 Codex CLI 的 **Responses API** 协议实时翻译为上游模型厂商的 **Chat Completions API**，让 Codex 无缝对接 DeepSeek、OpenRouter、OpenAI、Anthropic、Google AI、MiniMax 等主流模型平台，同时完整保留思考链（reasoning）、工具调用（tool calls）、视觉理解等多模态能力。

```
Codex CLI  →  /v1/responses
                │
                ▼
          deecodex（协议翻译 + 模型路由 + 扩展层 + 视觉路由）
                │
                ▼
          DeepSeek / OpenRouter / OpenAI / Anthropic / Google AI / MiniMax …
```

---

## 核心能力

### 协议引擎

- **双向实时翻译** — Responses API ↔ Chat Completions API，请求方向输入重写（`translate.rs`）、响应方向 SSE 流式转换（`stream.rs`），全程保留 `reasoning_content` / `thinking` 字段
- **流式优先** — 基于 Tokio 异步任务逐块转发上游 SSE 流，零缓冲延迟
- **视觉路由** — 图片请求可选定向到专用视觉模型上游
- **多模型映射表** — 灵活的模型名称 → 上游路由规则，支持按模型、按账号精细分流

### 桌面控制台（Tauri 2）

- **系统托盘常驻** — macOS 菜单栏 / Windows 系统托盘，右键快速控制服务启停、账号切换
- **可视化面板** — 服务状态概览、请求历史检索、线程会话聚合、账号管理、模型路由配置、插件管理、配置诊断、使用帮助
- **实时日志查看器** — 结构化日志过滤与搜索
- **配置热更新** — 修改路由表、账号后无需重启服务

### 本地扩展层

| 模块 | 说明 |
|------|------|
| **Files API** | 本地文件系统搜索索引，BM25 全文检索，chunk 级切分 |
| **Vector Store API** | 本地向量存储与检索 |
| **MCP Executor** | 本地 computer executor，支持 Playwright 浏览器自动化 |
| **CDP 集成** | Chrome DevTools Protocol 客户端，支持页面注入与控制 |
| **Hosted Prompts** | 内置提示词模板注册与分发 |

### 工程与运维

| 模块 | 说明 |
|------|------|
| **诊断引擎** | 15 项自动诊断（端口占用、配置文件完整性、上游连通性、TLS 证书等） |
| **限流保护** | 滑动窗口限流，按账号/模型维度控制 QPS |
| **LRU 缓存** | 请求级缓存，减少重复上游调用 |
| **Token 异常检测** | 实时监控 token 用量，发现异常用量自动告警 |
| **请求历史** | 全量请求记录持久化，支持时间范围检索和回放 |
| **线程聚合** | 跨会话线程聚合，支持导出和迁移 |
| **多账号隔离** | 多上游账号独立配置，互不干扰 |
| **Codex 配置注入** | 自动将 deecodex 注册为 Codex 默认 provider，停止服务时自动还原 |
| **Prometheus 指标** | 标准 `/metrics` 端点暴露服务运行指标 |
| **插件子系统** | 支持第三方插件安装/卸载/启用/停用，JSON-RPC 进程通信 |

### 跨平台

| 平台 | 格式 | 状态 |
|------|------|------|
| macOS 12+ (Apple Silicon) | `.dmg` | ✅ |
| macOS 12+ (Intel) | `.dmg` | ✅ |
| Windows 10+ (x64) | NSIS `.exe` 安装包 | ✅ |
| Linux (x64) | CLI 二进制 | ✅ |

---

## 交流群

欢迎加入 deecodex 交流群，一起讨论、反馈、共建。

<img src="https://github.com/liguan-89/deecodex/releases/download/v2.0.0/deecodex-qr-202605.jpg" width="200" alt="deecodex 交流群" />

---

## 安装

前往 [Releases](https://github.com/liguan-89/deecodex/releases) 下载对应平台安装包。

### 最新版 v2.0.0-lock

| 平台 | 下载 |
|------|------|
| Windows (x64) | [deecodex_2.0.0-lock_x64-setup.exe](https://github.com/liguan-89/deecodex/releases/download/v2.0.0-lock/deecodex_2.0.0-lock_x64-setup.exe) |

### 稳定版 v2.0.0

| 平台 | 下载 |
|------|------|
| macOS (Apple Silicon) | [deecodex_2.0.0_aarch64.dmg](https://github.com/liguan-89/deecodex/releases/download/v2.0.0/deecodex_2.0.0_aarch64.dmg) |
| macOS (Intel) | [deecodex_2.0.0_x64.dmg](https://github.com/liguan-89/deecodex/releases/download/v2.0.0/deecodex_2.0.0_x64.dmg) |
| Windows (x64) | [deecodex_2.0.0_x64-setup.exe](https://github.com/liguan-89/deecodex/releases/download/v2.0.0/deecodex_2.0.0_x64-setup.exe) |
| Linux (CLI) | [deecodex-linux-x64](https://github.com/liguan-89/deecodex/releases/download/v2.0.0/deecodex-linux-x64) |

### 兜底版本 v1.8.11

如最新版遇到问题，可回退：

| 平台 | 下载 |
|------|------|
| macOS (Apple Silicon) | [deecodex_1.8.11_aarch64.dmg](https://github.com/liguan-89/deecodex/releases/download/v1.8.11/deecodex_1.8.11_aarch64.dmg) |
| Windows (x64) | [deecodex_1.8.11_x64-setup.exe](https://github.com/liguan-89/deecodex/releases/download/v1.8.11/deecodex_1.8.11_x64-setup.exe) |

---

## 快速开始

1. **安装并启动** deecodex 桌面应用（系统托盘可见菱形图标）
2. **添加 API 账号** — 支持 OpenRouter、DeepSeek、OpenAI、Anthropic、Google AI、MiniMax 及自定义端点
3. **配置模型路由表** — 将 Codex 使用的模型名映射到上游实际模型
4. **启动代理服务** — 点击「启动服务」，deecodex 自动注入 Codex 配置
5. **开始使用 Codex CLI** — 所有请求自动通过 deecodex 路由

---

## 功能界面

<details open>
<summary><b>📸 控制台截图（点击展开）</b></summary>
<br>

**服务概览**
<img src="https://github.com/liguan-89/deecodex/releases/download/v2.0.0/screenshots-01-overview.png" width="800">

**请求历史**
<img src="https://github.com/liguan-89/deecodex/releases/download/v2.0.0/screenshots-02-history.png" width="800">

**会话聚合**
<img src="https://github.com/liguan-89/deecodex/releases/download/v2.0.0/screenshots-03-threads.png" width="800">

**账户管理**
<img src="https://github.com/liguan-89/deecodex/releases/download/v2.0.0/screenshots-04-accounts.png" width="800">

**插件管理**
<img src="https://github.com/liguan-89/deecodex/releases/download/v2.0.0/screenshots-05-plugins.png" width="800">

**高级设置**
<img src="https://github.com/liguan-89/deecodex/releases/download/v2.0.0/screenshots-06-settings.png" width="800">

**执行诊断**
<img src="https://github.com/liguan-89/deecodex/releases/download/v2.0.0/screenshots-07-diagnostics.png" width="800">

**使用帮助**
<img src="https://github.com/liguan-89/deecodex/releases/download/v2.0.0/screenshots-08-help.png" width="800">

**DEX助手**
<img src="https://github.com/liguan-89/deecodex/releases/download/v2.0.0/screenshots-09-dex.png" width="800">
<img src="https://github.com/liguan-89/deecodex/releases/download/v2.0.0/screenshots-10-dex2.png" width="800">

</details>

---

## 依赖要求

| 依赖 | 用途 | 必需 |
|------|------|------|
| Codex CLI 桌面版 | AI 编程助手 | 是 |

以下为可选依赖，仅在使用对应功能时需要：

| 依赖 | 用途 |
|------|------|
| Node.js（`import("playwright")`） | Playwright computer executor |
| MCP server（`mcp-filesystem` 等） | 本地 MCP tool 执行 |

---

## 许可

MIT &copy; liguan-89