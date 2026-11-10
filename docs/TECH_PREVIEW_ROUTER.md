# DEX AI 技术预览版路线

## 目标

技术预览版用于验证“官方登录态 + 第三方模型路由”的新链路，不影响稳定版 DEX AI。

核心目标：

- 保留 Codex 官方登录态，用 free / plus / pro 账号作为能力锚点。
- 通过本地路由层接管模型请求，优先按账号模型直选调用第三方或中转模型。
- 保留 Codex 插件、语音、ChatGPT 远程、官方能力入口等功能。
- 将账号资产、路由模型、运行时 provider 选择拆开，避免全局活跃账号污染。

## 预览版隔离

- 应用名：`DEX AI Preview`
- Bundle ID：`com.deecodex.app.preview`
- 版本号：`3.3.0-preview.1`
- 技术预览版可以和正式版 `DEX AI` 并存安装。

## 关键假设

- Codex provider 写入 `requires_openai_auth = true` 时，Codex 会继续按官方登录态启用能力入口。
- 本地 router 可以接收 Codex 的 Responses 请求，并在不泄露官方 token 给第三方的情况下转发到模型执行账号。
- 官方登录态账号不等于模型执行账号；前者负责能力锚定，后者负责推理算力。

## 第一阶段最小闭环

1. 新增预览专用 provider：`dex_router`。
2. 注入 `model_provider = "dex_router"`，并写入 `requires_openai_auth = true`。
3. 新增 `/codex-router/v1/responses`，先只做请求诊断和敏感头打码。
4. 验证 Codex UI 是否仍显示官方能力入口。
5. 将账号模型直选请求路由到一个第三方 Responses 账号。
6. 增加路由诊断：登录态存在、provider 命中、直选模型、工具降级原因。

## 后续能力

- 官方模型 passthrough：保留官方模型真实调用能力。
- 中转模型接管：在账号模型直选路径中直接使用第三方真实模型名。
- 工具过滤：对不支持的 `image2`、`web_search`、`web_fetch` 做屏蔽或降级。
- 线程可见性：路由模型切换后保持 Codex 会话可见。
- 路由市场：把可复用 provider 模板沉淀为插件或市场项。

## 不做

- 不迁移正式版用户数据目录。
- 不改变正式版 Bundle ID。
- 不让全局活跃账号参与路由决策。
- 不把官方登录态 token 透传给第三方模型服务。

## 当前落地状态

- `3.3.0-preview.1` 会以 `DEX AI Preview` / `com.deecodex.app.preview` 和正式版并存。
- Codex 注入会保留 `deecodex`、`deecodex_cli`、`deecodex_desktop`，并额外写入 `dex_router`。
- 技术预览版默认把 `model_provider` 指向 `dex_router`。
- `dex_router` 的 `base_url` 是 `/codex-router/v1`，`requires_openai_auth = true`，用于验证官方登录态能力入口是否保留。
- `/codex-router/v1/responses` 会记录打码后的请求头，然后以 Codex 桌面版活跃账号作为“锚点”确定账号池。
- 执行账号从同一个账号池中选择，不再固定使用当前活跃账号：
  - 只选择 Codex 桌面版 surface 的账号。
  - 账号池必须启用，且 `pool` 与锚点账号一致。
  - 优先级 `priority` 最高的候选先参与。
  - 同优先级内按 `weight` 轮询。
  - 冷却、额度耗尽或当前模型不可用的账号会被跳过。
  - Chat / Anthropic 翻译端点使用账号模型直选的真实模型名；Responses 直连和 Codex 官方端点允许原模型名透传。
- 路由执行结果会写回账号运行态：
  - 成功请求清理当前模型的失败/冷却状态。
  - HTTP 非 2xx、连接失败、解析失败、SSE 中断会记录到执行账号的 `runtime_state`。
  - `Retry-After` 会进入运行态，下一次路由选择会自动跳过冷却账号。
  - 反馈只写执行账号，不写官方登录态锚点账号。
- 如果没有选择 Codex 桌面版锚点账号，或同池没有可用执行账号，路由入口会直接返回配置错误，不回退全局活跃账号。
- 官方 Codex 账号的路由身份已经拆分：
  - `anchor_enabled` 只表示“作为登录态锚点”，用于保留 Codex Desktop 官方登录态和能力入口。
  - `execution_enabled` 才表示“参与模型执行”，官方账号默认关闭，避免官方号被误当作模型响应账号。
  - 第三方 / 直连执行账号仍按同池、模型直选、能力矩阵和冷却状态参与候选。
- 如果 Codex Desktop 已经在客户端内部完成官方登录，但 DEX 账号库里没有对应官方账号，Router 会复用请求头里的官方登录态作为临时锚点：
  - 只使用 `Authorization` / `Chatgpt-Account-Id` 判断“官方登录态存在”，不保存、不转发给第三方执行账号。
  - 临时锚点使用默认池 `codex-official`，执行账号仍需要在 DEX 账号库中加入同池并开启 `execution_enabled`。
  - 如果 DEX 中已经配置了显式桌面版锚点账号，则优先使用显式锚点。
- 新增 `/api/router/status?model=gpt-5.5` 诊断接口，返回锚点账号、候选账号、跳过原因和当前按 `cursor=0` 预估的候选。
- 路由诊断会区分账号级冷却、账号级额度冷却、模型级冷却、模型级额度冷却，并带上下一恢复时间与最近错误说明。
- 每条经过 `dex_router` 的请求会在请求历史写入 `route_trace`：
  - 记录锚点账号、账号池、cursor、候选数量、可用/跳过数量。
  - 记录最终执行账号、端点、执行模型、优先级和权重。
  - 记录每个候选账号的跳过原因，方便复盘“为什么这次选它”。
- 路由选择开始使用轻量健康评分：
  - 优先级仍然是主控，同优先级候选之间再比较健康分。
  - 健康分来自最近一小时成功/失败桶；无记录账号按中性分处理。
  - 评分会进入路由诊断和 `route_trace`，便于观察智能路由是否符合预期。
- 路由诊断开始暴露并执行候选能力矩阵：
  - 协议层：Chat 翻译、Responses 直连、Anthropic Messages、Codex 官方。
  - 工具层：原生工具、翻译工具、Anthropic 工具或不支持。
  - 能力标签：web、vision、image2、reasoning、stream usage。
  - 每次 Codex Router 请求会先分析工具需求，再按候选能力过滤执行账号。
  - 不满足工具需求的候选会以 `capability_mismatch` 进入 `route_trace`，并记录具体缺口。
  - 最终执行账号会写入工具决策：原生保留、翻译转接、本地处理或过滤原因。
  - 已补入口层集成测试，覆盖原生工具走 Responses、web_search 走 Chat 翻译、remote_mcp 走本地桥接、无可用能力时拒绝请求。
- 账号页和 `/api/router/status` 开始提供场景化诊断：
  - `tools=web,image,computer,mcp,file,function` 可以模拟指定工具需求。
  - 账号页展示文本、Web、文件、MCP、图片/电脑几个关键场景的执行账号和候选数量。
- Router 已开始记录并展示真实链路：
  - 非流式请求遇到 429 / 5xx / 网关类可重试错误时，会在同池内排除失败账号并尝试下一个候选，最多 3 次；不可重试的 400 / 参数错误不降级。
  - 流式请求不能半路重放，所以发送前会检查候选近期失败率、健康分、账号错误和模型错误；如果有健康替代候选，会在请求前改走替代账号。
  - `route_trace` 会写入 `fallback_attempts`、`fallback_count` 和 `stream_preflight`，请求历史页展示降级链路与最终执行账号。
  - `/api/router/status` 和账号管理页同步展示 `recent_success`、`recent_failed`、`failure_rate_percent` 与 `stream_preflight_risk`，能在发请求前看到“流式预选避开”的原因。
