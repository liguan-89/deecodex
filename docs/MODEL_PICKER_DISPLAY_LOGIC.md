# Codex 模型选择器显示逻辑（API 模式 vs 智能路由模式）

> 目的：说明 DEX AI 控制台两种路由模式下，Codex 桌面版模型选择器如何显示 dex 直选
> 模型，以及各自的关键依赖和排查路径。遇到“dex 模型不显示 / 分组不对”先读本文档。

## 1. 关键概念

- **model_catalog_json**：写入 `~/.codex/config.toml` 的
  `model_catalog_json = "~/.codex/models_deecodex.json"`，Codex 读它获得模型目录。
- **catalog（models_deecodex.json）**：deecodex 启动时由 `src/codex_config.rs`
  （`generate_context_catalog`）生成，结构为 `{"models": [...]}`。
- **账号模型条目**：slug 形如 `dexacct.<base64 account>.<base64 endpoint>.<base64 model>`。
- **分组占位条目**：slug 形如 `dexacct.group.<base64 account>`，`display_name` = 账号名，
  是“分组标题”的可选项；选中它 deecodex 会返回错误（`dex_router_group_placeholder`），
  不执行实际调用。

## 2. API 模式（CDP 注入驱动）

### 机制

1. Codex 从 `https://ab.chatgpt.com`（Statsig）拉取 `use_hidden_models`：
   - `true` → 只显示 5 个 GPT 白名单，**dex 模型被过滤**（API 模式的默认表现）。
2. deecodex 通过 **CDP 注入**（`static/inject.js`）让 dex 模型显示：
   - `patchModelPicker` 把 catalog 模型 push 进 Codex 选择器的 React fiber
     `props.models`；
   - hook Statsig `getDynamicConfig` 强制 `use_hidden_models=false`（模型列表解锁）。
3. **分组**：注入脚本 `applyModelMenuGrouping` 在菜单渲染后按 catalog 账号映射
   （slug → 账号）做 DOM 重排，插入分组标题（14px 加粗），组内模型去账号前缀。

### 前提

- **Codex 必须以调试端口启动**（`--remote-debugging-port=9222`，由 DEX 控制台
  `codex_launch_with_cdp` 自动带），deecodex 才能注入。
- 没有 CDP 注入时，API 模式 dex 模型不显示（只剩 5 个白名单 GPT）。

### 关键文件

- `static/inject.js`：模型 push、Statsig 解锁、DOM 分组。
- `src/inject.rs`：CDP 桥接 `/models`。
- `docs/API_MODE_MODEL_FILTER_ROOT_CAUSE.md`：API 过滤根因调查。

## 3. 智能路由模式（原生读 catalog）

### 机制

1. Codex 桌面版**内部官方登录态**有效（ChatGPT.app 内登录），`dex_ai` provider
   （`requires_openai_auth=true`）走官方登录态轨道，**不受 `use_hidden_models` 过滤**。
2. Codex **原生读取 `model_catalog_json`** 显示 dex 模型——**不需要 CDP 注入**。
3. **分组**：catalog 里的**占位条目**（`dexacct.group.*`，`display_name`=账号名）作为
   分组标题显示；组内账号模型的 `display_name` 是**纯模型名**（无账号前缀），
   避免“账号 / 模型”与分组标题重复。

### Catalog 关键字段（缺失会导致不显示）

- **`shell_type`（必填）**：Codex 解析 catalog 要求每个条目带 `shell_type`。
  缺失时整个 catalog 解析失败（日志：
  `failed to parse model_catalog_json ... missing field shell_type`），
  **dex 模型全部不显示**，只剩官方 GPT。deecodex 在
  `ensure_model_catalog_schema` 中统一补 `shell_command`。
- **`display_name`**：账号模型 = 纯模型名；占位条目 = 账号名。
- **占位条目**：每组一个，slug 前缀 `dexacct.group.`。

### 前提

- Codex 桌面版内部官方登录态有效（UI 显示已登录，Codex Framework 官方会话建立）。
- Codex **不需要调试端口**（无注入，纯原生）。

### 已知原生限制

- Codex 原生按模型名/类别把 `GPT-*` 模型归入“内置 GPT”组。因此中转账号（如 `88`）
  的 GPT 模型在原生显示时可能出现在“内置 GPT”组；要严格归到对应账号分组，
  需要进一步调整 catalog 或接受原生分组行为。

## 4. 排查路径（dex 模型不显示时）

1. **看 Codex 启动日志**是否有：
   `failed to parse model_catalog_json ... missing field shell_type`
   → 是 catalog 缺 `shell_type`（跑一次 DEX 重新生成即可，检查 `shell_type` 缺失数应为 0）。
2. 检查 `models_deecodex.json`：
   - `shell_type` 缺失数（应为 0）；
   - 占位条目数量（账号数）；
   - 账号模型 `display_name` 是否纯模型名（不应含“账号 / ”前缀）。
3. 判断当前模式：
   - **API**：确认 Codex 是否带调试端口（`codex_launch_with_cdp`），注入是否生效。
   - **smart**：确认 Codex 桌面版内部官方登录态有效；确认 config.toml 的
     `model_provider=dex_ai` + `requires_openai_auth=true` + `model_catalog_json` 完整。
4. 若分组标题缺失/重复：确认 catalog 有占位条目、`display_name` 是纯模型名。

## 5. 相关提交

- `3e4ae3e8`：catalog 补齐 `shell_type` + 账号模型 `display_name` 改纯模型名。
- `ce1626a2`：分组占位条目（原生分组标题 + 选中返回错误）。
- `1bd32646` / `50534bbe`：注入侧分组（catalog 账号映射 + 版本重建）。
- `fad195d2`：`do_remove` 清理时移除 dex 账号 `model` 行。
- `b4a00d41` / `d6274bb5`：API 模式 CDP 注入模型解锁。
