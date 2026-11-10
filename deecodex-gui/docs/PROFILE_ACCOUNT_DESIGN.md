# 个人中心登录注册系统设计（接入 Mobile Codex 服务器）

## 目标

个人中心（`profile` 面板）提供完整的 Mobile Codex 账号登录注册能力：

- 未登录：登录、邀请码注册、邮箱验证、忘记密码 / 重置密码。
- 已登录：账号信息（邮箱、账号 ID、设备 ID、平台）、Companion 连接与服务状态、绑定设备管理（重命名 / 撤销）、退出登录。
- 保留现有“匿名使用统计”设置，不因账号系统改动丢失。

认证后端复用 Mobile Codex control-plane（`https://api.liguan.me/mobile-codex`），
通过本机 `deecodex-mobile-codex` 插件 JSON-RPC 中转；前端不直连服务器、不落盘凭证。

## 现状与复用

### 后端 API（control-plane）

| 动作 | 端点 | 请求体 | 说明 |
|------|------|--------|------|
| 注册 | `POST /v1/auth/register` | `{invitationCode, email, password}` | 密码至少 12 位；返回 `pendingVerification`，开发环境可回显 `developmentToken` |
| 验证邮箱 | `POST /v1/auth/verify-email` | `{token}` | 注册后必须验证才能登录 |
| 找回密码 | `POST /v1/auth/password/forgot` | `{email}` | 发送重置邮件 |
| 重置密码 | `POST /v1/auth/password/reset` | `{token, password}` | 成功后撤销全部设备会话 |
| 登录 | `POST /v1/auth/login` | 设备/平台/公钥等 | 由插件封装，GUI 只传 `{email, password, deviceName?}` |

服务端规则（GUI 需对齐提示）：

- 密码长度：≥ 12 字节（`hash_password` 校验）。
- 注册限流：每邮箱 5 次 / 15 分钟；登录限流：每邮箱 10 次 / 15 分钟。
- 未验证邮箱登录返回 `EmailUnverified`，UI 应引导去“验证邮箱”。
- 服务端错误文案为中文，可直接透传到 toast。

### 本机桥接

- 插件 `deecodex-mobile-codex` 的 `plugin.json` 声明 `account.methods`，把 `register / verify_email / forgot_password / reset_password / login / logout / refresh / current_user / list_devices / rename_device / revoke_device / status / start / stop` 映射到 `mobile_codex.auth.*` 与 `mobile_codex.devices.*`。
- Rust 命令 `invoke_plugin_account_action(pluginId, action, params)` 已注册（`src/lib.rs`），前端通过 `window.DeeCodexTauri.invoke` 调用。
- 登录成功后插件会启动 Companion、持久化登录态；退出/撤销本机会清理登录态并停止服务。

### 与插件详情页的关系

插件详情页已有同源账号面板。个人中心是用户主入口，这里实现独立的一套（状态、DOM id 均以 `pf`/`profile` 前缀隔离），
不修改插件详情页，避免回归其现有测试；两处共用 `gui/css/plugins.css` 的 `mc-*` 组件样式。
后续如需收敛，可把两处抽取为共享的 `mobile-codex-account.js` 模块。

## 交互设计

### 状态模型

- `profileMobileCodexState`：`{ loading, loaded, user, service, devices, error }`
- 首次进入面板：`hydrateProfile()` 依次调用 `current_user` → `status` →（已登录时）`list_devices`。
- 任何写操作（登录/注册/验证/重置/退出/设备操作）成功后重置 `loaded=false` 并重新 `hydrateProfile()`。

### 未登录视图

- 登录卡（主）：邮箱 + 密码 + “登录并启动 Companion”。
- 折叠区（次）：
  - 邀请码注册：邀请码 + 邮箱 + 密码（≥12 位）+ 提交；成功后若返回 `developmentToken` 自动填入验证框。
  - 验证邮箱：Token + 完成验证。
  - 忘记 / 重置密码：邮箱 + 发送重置邮件；重置 Token + 新密码 + 重置密码。
- 安全提示：`密码仅进入本机第一方插件`。

### 已登录视图

- 连接状态卡：Companion 运行 / 局域网 / P2P 网络状态。
- 账号卡：头像（邮箱首字母）、邮箱、绑定设备数、账号 ID / 设备 ID / 平台。
- 服务控制：启动 / 停止 Companion；外部系统管理时禁用并提示。
- 设备列表：每台设备显示在线状态、平台、版本；本机标记；支持重命名 / 撤销。
- 退出登录（撤销本机会先弹确认，说明会停止 Companion）。

### 错误与安全

- 服务端中文错误透传 toast；不静默吞错。
- 密码输入框不回显；前端不保存密码；登录态由插件持久化在私有凭证目录。
- 撤销当前设备 / 退出登录前必须二次确认。

## 文件变更

- `gui/js/placeholder-pages.js`：个人中心渲染 + Mobile Codex 账号区（渲染、hydrate、动作处理）+ 保留匿名统计。
- `gui/js/app-shell.js`：`profile` 面板挂载 `hydrateProfile()`。
- `gui/js/profile-render-smoke.test.js`：新增渲染冒烟测试（登录 / 已登录 / 动作调用链）。
- 样式复用 `gui/css/plugins.css` 的 `mc-*`、`plugin-config-input`、`btn-apply`，不新增独立 CSS 文件。

## 验收标准

1. 未登录时个人中心显示登录卡与注册 / 验证 / 找回 / 重置折叠区。
2. 登录成功后显示账号信息、连接状态、设备列表与退出按钮；退出后回到登录视图。
3. 所有动作只通过 `invoke_plugin_account_action`，前端无直接 HTTP 调用。
4. `node gui/js/profile-render-smoke.test.js` 通过；现有 `plugins-render-smoke` / `tauri-api-redaction` 不回归。
5. 在局域网 Mac mini（<LAN_MAC_MINI>:22）上 `cargo tauri dev` 实测个人中心面板（登录 / 已登录两种状态）。
