# deecodex

**DeepSeek API → Codex CLI 兼容代理 · 桌面 GUI 版**

将 Codex CLI 发出的 **Responses API** 请求实时翻译为 **Chat Completions API**，使 Codex 可以原生对接 DeepSeek / OpenRouter / MiniMax / OpenAI / Anthropic / Google AI 等上游，同时保留思考模式、工具调用等完整功能。

内置本地增强层：Files / Vector Store API、file_search（BM25 chunk 级索引）、computer / MCP executor、多账号管理、请求历史、线程聚合、配置诊断、视觉路由等。

```
Codex CLI  →  /v1/responses
                │
                ▼
          deecodex（协议翻译 + 模型映射 + 增强层 + 视觉路由）
                │
                ▼
          DeepSeek / OpenRouter / MiniMax / OpenAI / Anthropic / Google AI
```

## 交流群

欢迎加入 deecodex 交流群，一起讨论、反馈、共建。

<img src="https://github.com/liguan-89/deecodex/releases/download/v1.0.0/deecodex.0515.jpg" width="200" alt="deecodex 交流群1" /> <img src="https://github.com/liguan-89/deecodex/releases/download/v1.0.0/_20260508195435_79_22.jpg" width="200" alt="deecodex 交流群2" />

## 安装

前往 [Releases](https://github.com/liguan-89/deecodex/releases) 下载对应平台安装包：

| 平台 | 安装包 |
|------|--------|
| macOS 12+ | `deecodex_*.dmg` — 双击挂载，拖入 Applications |
| Windows | `deecodex_*.msi` — 双击运行安装向导 |

### 最新版 v1.8.11

| 平台 | 下载 |
|------|------|
| macOS (Apple Silicon) | [deecodex_1.8.11_aarch64.dmg](https://github.com/liguan-89/deecodex/releases/download/v1.8.11/deecodex_1.8.11_aarch64.dmg) |
| Windows (MSI) | [deecodex_1.8.11_x64_zh-CN.msi](https://github.com/liguan-89/deecodex/releases/download/v1.8.11/deecodex_1.8.11_x64_zh-CN.msi) |
| Windows (EXE) | [deecodex_1.8.11_x64-setup.exe](https://github.com/liguan-89/deecodex/releases/download/v1.8.11/deecodex_1.8.11_x64-setup.exe) |

### 兜底版本 v1.6.0

如 v1.8.11 在 Windows 上遇到问题，可回退至稳定版：

| 平台 | 下载 |
|------|------|
| Windows (MSI) | [deecodex_1.6.0_x64_zh-CN.msi](https://github.com/liguan-89/deecodex/releases/download/v1.6.0/deecodex_1.6.0_x64_zh-CN.msi) |
| Windows (EXE) | [deecodex_1.6.0_x64-setup.exe](https://github.com/liguan-89/deecodex/releases/download/v1.6.0/deecodex_1.6.0_x64-setup.exe) |

## 依赖要求

| 依赖 | 用途 | 必需 |
|------|------|------|
| Codex CLI 桌面版 | AI 编程助手 | 是 |

以下为可选依赖，仅在使用对应功能时需要：

| 依赖 | 用途 |
|------|------|
| Node.js（可 `import("playwright")`） | Playwright computer executor |
| `mcp-filesystem` 等 MCP server | 本地 MCP tool 执行 |

## 使用

1. 启动 deecodex 桌面应用（macOS 菜单栏 / Windows 系统托盘可见菱形图标）
2. 添加上游 API 账号（支持 OpenRouter、DeepSeek、OpenAI、Anthropic、Google AI、MiniMax、自定义）
3. 配置模型映射表
4. 启动服务，Codex CLI 自动路由到 deecodex

## 许可

MIT &copy; liguan-89
