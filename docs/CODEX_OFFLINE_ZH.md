# Codex 离线中文支持

Codex 桌面版的 UI 标签（`New chat / Search / Plugins / Pinned / Projects / Chats` 等）
**hardcode 在 app.asar 里**，没有内置中文 i18n 字典。即使 `navigator.language=zh-CN` 且
IntlProvider 拿到 `locale: "zh-CN"`，但 `messages={}`（空字典），system labels 仍显示英文。

deecodex 通过**双层兜底**让 Codex 在 ab.chatgpt.com 屏蔽、Statsig 拿不到、messages 字典空
的情况下也能显示中文 UI：

## 工作机制（三层）

| 层 | 组件 | 作用 |
|---|---|---|
| L1: Statsig 离线 | Rust `src/inject.rs` + `static/inject.js` | CDP 拦截 ab.chatgpt.com 调用，serve 本地缓存（`statsig_init_zh.json`），让 Codex 拿到合法 Statsig 响应 |
| L2: DOM 兜底 | `static/inject.js` installDomTranslation | MutationObserver 把 sidebar/顶部的英文系统标签（New chat / Search / Plugins / Pinned / Projects / Chats 等）实时替换为中文，不依赖 IntlProvider |
| L3: navigator.language | 系统/OS | macOS 系统语言为中文时，Chromium `--lang=zh-CN` 会让部分 UI 默认走中文（仅在 Codex 启用此 fallback 的部分生效） |

**实测效果**（ab.chatgpt.com 屏蔽 + 全清 Codex 存储重启）：
- L1（Statsig 离线）：deecodex 拦截 /v1/rgstr 并回填 242KB cache（如果 Codex 真的调了网络）
- L2（DOM 兜底）：8 个 sidebar 系统标签从英文实时翻译为中文
- L3：用户 macOS 系统语言为中文时，部分内容自动跟随后

**L2 兜底是核心**——L1 在某些 Codex 版本下不触发（Statsig SDK 直接用 internal cache），
L3 取决于系统语言；只有 L2 在所有情况下都可靠生效。

## 端到端流程

### 第一次启动（需要 api.statsigcdn.com 可达）

1. 启动 deecodex：`deecodex --codex-launch-with-cdp`
2. Codex 启动 → 注入脚本运行 → 检查本地缓存（不存在）→ 安装 fetch/XHR 捕获 hook
3. Codex 渲染层调用 fetch('/v1/initialize') → CDP Fetch 拦截 → 放行（无缓存）→ 真实请求发出
4. 响应回到 fetch 调用 → 注入脚本捕获响应体 → POST 到 `/statsig-init`
5. Rust 端把响应体写入 `~/.deecodex/statsig_init_zh.json`
6. 日志输出：`Statsig 初始化响应已捕获并保存到本地（XXX 字节）`

### 第二次及以后启动（api.statsigcdn.com 任意状态）

1. 启动 deecodex
2. Codex 启动 → 注入脚本运行 → 检查本地缓存（已存在）→ 啥也不做
3. Codex 渲染层调用 fetch('/v1/initialize') → CDP Fetch 拦截 → fulfillRequest 用本地缓存回填
4. Codex 拿到伪造的响应，误以为 Statsig 返回 `locale=zh-CN`，加载中文 UI

## 缓存文件

| 项 | 值 |
|---|---|
| 路径 | `~/.deecodex/statsig_init_zh.json` |
| 格式 | 原始 JSON 响应体（Statsig `/v1/initialize` 的标准输出） |
| 大小 | 约 10–30 KB |
| 删除后果 | 下次启动时自动重新捕获（需要 api.statsigcdn.com 可达） |

## 手动准备缓存（备选）

如果无法在 deecodex 环境下联网启动 Codex（比如持续屏蔽 api.statsigcdn.com），
但你在另一台机器上能访问：

1. 在能访问 api.statsigcdn.com 的机器上启动 Codex
2. 用 Charles/Proxyman/mitmproxy 抓取 `POST https://api.statsigcdn.com/v1/initialize` 的响应体
3. 把响应 JSON 原样保存为 `~/.deecodex/statsig_init_zh.json`
4. 重新启动 deecodex + Codex，CDP 层会自动用本地缓存回填

或者用任意语言一行脚本（curl + jq）：

```bash
curl -X POST 'https://api.statsigcdn.com/v1/initialize' \
  -H 'Content-Type: application/json' \
  -H 'Statsig-Client-Start-Time: 0' \
  -H 'Statsig-SDK-Type: js-client' \
  --data '{"sinceTime":0,"user":{"customIDs":{},"statsigEnvironment":{"tier":"production"}},"statsigMetadata":{},"context":{}}' \
  > ~/.deecodex/statsig_init_zh.json
```

注意：上述 curl 输出的字段可能与 Codex 实际请求的字段不匹配（Codex 会在
请求里带会话 ID、SDK 版本等），但只要响应体是合法的 Statsig 初始化响应
（含 `dynamic_configs`、`feature_gates` 等标准字段），CDP 拦截就能工作。

## 验证

启动 deecodex 时观察日志：

```
[INFO  CDP 注入成功 (端口 9222)：插件解锁 + 模型选择器扩展 + Statsig 离线回退已激活
[INFO  已加载本地 Statsig 配置（XXX 字节），api.statsigcdn.com 请求将由 CDP 直接回填
```

启动 Codex 之后：

- 打开 DevTools（Codex 菜单 → View → Toggle Developer Tools）
- 在 Network 标签过滤 `api.statsigcdn.com`
- 第一次 `/v1/initialize` 请求的 status 应该是 `(failed)` 或 `(blocked)` —— 这是预期的
- 但 UI 应当是中文

如果 UI 仍是英文：

1. 检查 `~/.deecodex/statsig_init_zh.json` 是否存在
2. 用 `head -c 500` 看下文件是否包含 `"locale":"zh-CN"` 之类的字段
3. 启动时 deecodex 日志是否出现 `Statsig 离线回退：使用本地缓存回填` 字样
4. Codex 是否真的在访问 `/v1/initialize`（用 DevTools Network 标签确认）

## 已知限制

1. **首次启动需要网络**。如果 api.statsigcdn.com 在首次启动时就不可达，需要手动准备缓存文件。
2. **响应体有时效性**。Statsig 配置中的 feature flag / 动态参数会随时间变化。本地缓存可能在数月后过期，Codex 部分新功能可能受影响。删除 `statsig_init_zh.json` 即可触发重新捕获。
3. **不会捕获非 `/v1/initialize` 的请求**。如 `/v1/log_event` 等其他 Statsig 调用仍会发往 api.statsigcdn.com，被防火墙挡掉后只是日志事件丢失，不影响 UI。
4. **CDP 拦截只在主进程**。如果 Codex 启用了多 renderer 进程（如未来版本），每个 renderer 需要独立的 CDP 连接 —— 当前 deecodex 只连第一个 Codex 页面目标。
