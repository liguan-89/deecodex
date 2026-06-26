# Codex 桌面版模型选择器 CDP 扩展方案

## 背景

Codex 桌面版（Electron 应用）默认只显示 5 个 GPT 模型（gpt-5.5、gpt-5.4、gpt-5.4-mini、gpt-5.3-codex、gpt-5.2）。这些模型是 Codex 自己的 React 组件从 ChatGPT 后端拉取的硬编码白名单，与 deecodex 配置无关。

本文档说明如何通过 CDP 注入把 deecodex 的 78 个模型扩展进 Codex 桌面版的模型选择器。

## 关键发现（通过 React fiber 探测）

Codex 桌面版模型选择器组件的 React fiber 链：

```
oUt（混淆名）— 关键组件
  props: {
    model: "gpt-5.4",                    ← 当前选中的模型 id
    models: [                            ← 模型列表（Codex 默认 5 个）
      { id: "gpt-5.5", displayName: "GPT-5.5", ... },
      ...
    ],
    modelOptionsDisabled: false,
    onSelectModel: function(modelId)     ← 选中后调用
  }
```

`oUt` 在 React fiber 链深度约 25-32 处。其父组件是 menu/dropdown 容器，displayName 含 `oUt`。

## 实现

### 1. 桥接路径 `/models`

`src/inject.rs` 新增桥接路径返回 `~/.codex/models_deecodex.json` 内容：

```rust
"/models" => handle_models(state).await,
```

`handle_models` 读取 `models_deecodex.json`（deecodex daemon 启动时生成的 78 模型目录），返回 `{status: "ok", models: [...]}`。

### 2. inject.js 的 patchModelPicker

`static/inject.js` 新增以下函数：

- `loadDeecodexModels()` — 通过 `window.__deecodexBridge('/models', {})` 拉取模型列表，结果缓存到 `deecodexModelsCache`。
- `findPickerFiberByText()` — 在所有 span 里找符合 picker 文本格式（`5.x` / `DeepSeek*` / `MiMo*` / `自定义` / `GPT-*`），沿 `__reactFiber` 往上爬到含 `models + onSelectModel` 的 fiber。
- `deecodexModelToPickerEntry(m)` — 把 deecodex 模型目录项转换为 Codex 选择器期望的字段：
  ```js
  {
    id: m.slug,
    model: m.slug,
    displayName: <去掉 "桌面版 账号 / " 前缀>,
    description: <完整 displayName>,
    supportedReasoningEfforts: [...],
    defaultReasoningEffort: "medium",
    inputModalities: ["text"],
    hidden: false,
    ...
  }
  ```
- `patchPickerModels(fiber, models)` — 用 existingIds 去重后 push 到 `fiber.memoizedProps.models`。返回 true 表示新增了模型。
- `tryPatchPicker()` — 调用 `findPickerFiberByText` + `patchPickerModels`。
- `startPickerRetryLoop()` — 每 1.5s 轮询调用 `tryPatchPicker`。**不停止**：因为 React 重渲染时 picker fiber 是新实例，patch 需要重新跑（existingIds 去重保证无副作用）。

### 3. 时机

`scan()` 中调用 `startPickerRetryLoop()` 和 `patchModelPicker()`。

**关键观察**：picker fiber 在以下情况**不渲染**：
- Codex 仍在侧边栏未进入聊天界面
- 空 chat 状态（新建 thread 还没输入任何内容）
- 项目页面 / 设置页面

picker fiber 在以下情况**渲染**：
- 用户进入已有 thread 的聊天界面

因此 polling 必须持续运行直到用户进聊天界面触发 picker 挂载。实测从 Codex 启动到 picker 出现约需 2-5 秒。

## 验证

通过 CDP 验证最终状态：
```
picker fiber 在 depth 25-32 找到
models 总数: 78（5 GPT + 73 deecodex）
当前选中: dexacct.MThiNGUxZjAwNDI0MjViODAwMDA.ZW5kcG9pbnRfMTllODFlYWI1MjZfYTkzOTcy.ZGVlcHNlZWstdjQtZmxhc2g
picker button 文本: "DeepSeek V4 Flash低"
```

Codex 桌面版接受 deecodex 模型作为当前选中，picker 下拉菜单显示完整 78 个选项。

## 已知限制

1. **Codex Rust CLI 可能校验模型白名单**：picker 接受了 deecodex 模型，但 Rust CLI 在收到 fetch 请求时可能校验 model 字段是否合法。需要实测验证（用户已确认能成功发请求）。
2. **Picker fiber 可能找不到**：取决于 Codex 当前页面状态，polling 时机不确定时可能错过。改进方向：监听 React 路由切换事件主动触发 patch。
3. **React 重渲染导致 picker fiber 实例变更**：fiber 上的标记属性会丢失，必须用 `existingIds` 去重，polling 持续运行。
4. **Codex 桌面版未来升级可能改 fiber 结构**：当前依赖混淆名 `oUt` 不稳定。

## 相关文件

- `src/inject.rs` — 桥接路径 `/models`
- `static/inject.js` — patchModelPicker 函数族
- `~/.codex/models_deecodex.json` — deecodex 生成的 78 模型目录
- `docs/MODEL_UNLOCK_CDP_SOLUTION.md` — 旧方案（Statsig hook，已废弃）

## 调试技巧

通过 CDP 手动验证 picker 状态：

```python
# 找到 picker fiber
span = [...document.querySelectorAll('span')].find(s =>
    /DeepSeek|MiMo|Kimi|自定义|^5\.|^GPT-/.test(s.textContent.trim()));
fiber = span[Object.keys(span).find(k => k.startsWith('__reactFiber'))];
while (fiber) {
    if (fiber.memoizedProps?.models && fiber.memoizedProps?.onSelectModel) {
        return fiber;  // picker fiber
    }
    fiber = fiber.return;
}
```

手动强制 patch：

```javascript
const bridge = await window.__deecodexBridge('/models', {});
const picker = findPickerFiberByText();
picker.memoizedProps.models.push(...bridge.models.map(deecodexModelToPickerEntry));
```