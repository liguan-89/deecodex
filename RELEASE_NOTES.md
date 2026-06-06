# DEX AI 版本更新说明

## 3.4.1

- 修复正式版从 Preview 转正后无法继承预览版账号登录态的问题。
- 正式版首次启动时会从 `~/.deecodex-preview/accounts.json` 合并缺失账号到 `~/.deecodex/accounts.json`。
- OAuth 登录态、Codex 桌面版账号选择和 DEX 助手账号选择会随账号迁移一起恢复。
- 迁移不会覆盖正式版已有账号和正式版全局活跃账号。
- 迁移完成后会写入 `preview_accounts_migrated.json` 标记，避免后续反复用 Preview 旧数据覆盖正式版。

## 3.4.0

- 技术预览功能回归主线，发布为正式版。
- Tauri 打包身份从 `DEX AI Preview` 切回 `DEX AI`。
- 正式版使用 `com.deecodex.app`、`deecodex-gui` 和默认端口 `4446`。
- 保留 Preview 版隔离逻辑：Preview 继续使用独立数据目录和端口，便于并行测试。
