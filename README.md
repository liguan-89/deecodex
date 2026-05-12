# deecodex &middot; DeepSeek API → Codex CLI 兼容代理

**deecodex** 是一个本地代理服务，将 Codex CLI 的 OpenAI Responses API 请求翻译为上游 Chat Completions API 调用（DeepSeek、OpenRouter、MiniMax 等），并透明增强本地 Files / Vector Stores / MCP / Computer Use 能力。

## 安装

前往 [Releases](https://github.com/liguan-89/deecodex/releases) 下载对应平台安装包：

| 平台 | 安装包 |
|------|--------|
| macOS | `deecodex_*.dmg` |
| Windows | `deecodex_*.msi` |

### macOS 用户

下载 `.dmg` 文件，双击挂载后将 `deecodex.app` 拖入 `Applications` 文件夹即可。

### Windows 用户

下载 `.msi` 安装包，双击运行安装向导。

## 使用

1. 启动 deecodex 桌面应用
2. 添加你的上游 API 账号（OpenRouter / DeepSeek / OpenAI 等）
3. 配置模型映射
4. 启动服务，Codex CLI 会自动路由到 deecodex

## 许可

MIT &copy; liguan-89
