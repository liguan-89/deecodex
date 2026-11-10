# DEX 助手 opencode 引擎验收清单（v1）

> 范围：`deecodex-gui` 的 DEX 助手面板由 opencode 1.18.16（pin）驱动。
> 无头部分已由 `scripts/verify-opencode-engine.sh` 与自动化测试覆盖；本清单覆盖
> **需要在局域网 Mac mini（<LAN_MAC_MINI>:22）上人工确认**的 GUI 行为与跨环境项。

## 0. 前置

- 已构建 `cargo build`（或 `cargo tauri dev`），二进制存在。
- opencode 二进制可用（`DEECODEX_OPENCODE_BIN` 或 `scripts/fetch-opencode.sh` 生成的捆绑文件）。
- 账号管理里有一个可用账号（余额充足；注意 DeepSeek 账号可能欠费 401）。
- 快速无头自检：`bash scripts/verify-opencode-engine.sh --config ~/.deecodex/opencode/config/opencode.json`

## 1. GUI 冒烟（在局域网 Mac mini <LAN_MAC_MINI>:22 上执行）

| # | 步骤 | 期望 |
|---|---|---|
| 1.1 | 启动应用（`cargo tauri dev` 或安装包），打开「DEX 助手」面板 | 面板正常渲染，无阻断页 |
| 1.2 | 输入一条诊断消息并发送（如“汇总服务状态”） | 出现思考动画；几秒后返回真实回答 |
| 1.3 | 观察工具卡片 | 出现 tool-start → tool-result 卡片（如 get_service_status），含耗时/等级徽标 |
| 1.4 | 让模型调用一个 L3 工具（如“执行 shell 命令 pwd”） | 弹出内联确认；点“确认”后执行，点“取消”后拒绝且不执行 |
| 1.5 | 回复过程中点“停止” | 停止生成，不再新增内容 |
| 1.6 | 切换账号后再次发送 | 提示引擎已重建；新消息使用新账号上游（可在配置/日志确认） |

## 2. 引擎侧确认（可选，无头）

- `pgrep -fl "opencode serve"`：存在一个子进程，端口随机。
- `~/.deecodex/opencode/config/opencode.json`：provider 名为 `dex_<provider>`，`agent` 含 diag/fix/workspace/plugins/cost/logs，`permission` 含 L3 ask 列表。
- MCP 端口 `initialize` 返回 200 且 `serverInfo.name == "deecodex-mcp"`（用 `scripts/verify-opencode-engine.sh --mcp http://127.0.0.1:<port>/mcp/dex_ops --token <token>`）。

## 3. 跨环境

- **Mac mini（macOS arm64）**：已在 `~/deecodex-verify` 完成原生构建 + 103 测试 + 真实链路验证（本清单 1.x 仍需人工执行一次）。
- **Windows x64**：未验证（无环境）。至少应验证：二进制 SHA256（`dadee463…`）、serve 启动、MCP 端口、GUI 面板。

## 4. 已知限制（v1）

- opencode 1.18.16 不支持按 agent 过滤 MCP 工具：所有启用能力包的工具（约 115 个）都会进入上下文；已禁用 opencode 内置 bash/edit/read 等宿主工具。
- 引擎直连上游账号：deecodex「请求历史/成本」面板不再统计 DEX 助手引擎调用。
- 旧 deeStorage 会话历史不迁移，opencode 会话全新开始。
- 引擎是唯一自动路径：不可用时面板直接显示错误，不再静默回退旧循环。
- 显式旧循环开关：`deeStorage.dex_opencode_engine = "legacy"` 才启用旧 `dex_chat` 循环（仅排查用）。

## 5. 验收通过标准

1.1–1.6 全部符合期望，且 `scripts/verify-opencode-engine.sh` 全 PASS；Windows（若有）完成 3.1。
