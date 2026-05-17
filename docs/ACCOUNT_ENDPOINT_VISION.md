# 多账户、端点隔离与视觉策略

本文说明账号管理 v2 的配置模型、迁移行为、端点协议和视觉策略。GUI 实测与真实上游冒烟见 [验收清单](ACCEPTANCE_CHECKLIST.md)。

## 配置模型

`accounts.json` 当前版本为 v2。核心结构分四层：

- `AccountStore`：保存账号列表、活跃账号和活跃端点。
- `Account`：保存账号名称、供应商、API Key，以及该账号下的端点列表。
- `EndpointConfig`：保存协议类型、上游 URL、端点路径、模型映射、模型能力覆盖、视觉策略和高级请求参数。
- `VisionConfig`：保存图片请求如何处理，包括关闭、原生多模态、胶水多模态。

主要字段：

```json
{
  "version": 2,
  "active_account_id": "account_id",
  "active_endpoint_id": "endpoint_id",
  "accounts": [
    {
      "id": "account_id",
      "name": "OpenRouter",
      "provider": "openrouter",
      "api_key": "sk-...",
      "endpoints": [
        {
          "id": "endpoint_id",
          "name": "Chat 兼容端点",
          "kind": "open_ai_chat",
          "base_url": "https://openrouter.ai/api/v1",
          "path": "chat/completions",
          "model_map": {
            "gpt-5": "deepseek/deepseek-chat"
          },
          "model_profiles": {
            "deepseek/deepseek-chat": { "vision_mode": "off" }
          },
          "vision": {
            "mode": "glue",
            "unsupported_image_policy": "reject",
            "glue_strategy": "caption_then_main",
            "adapter_id": "minimax_coding_plan_vlm",
            "base_url": "https://api.minimax.chat",
            "api_key": "minimax-key",
            "model": "MiniMax-M1",
            "path": "v1/coding_plan/vlm"
          }
        }
      ]
    }
  ]
}
```

## 端点协议

`kind` 决定 deecodex 如何把 Codex 的 Responses 请求转给上游：

- `open_ai_chat`：Responses 转 Chat Completions。
- `open_ai_responses`：Responses 直连。
- `anthropic_messages`：转 Anthropic Messages，当前支持非流式。
- `custom_chat`：自定义 Chat 兼容端点。
- `custom_responses`：自定义 Responses 兼容端点。

`path` 为空时使用协议默认路径：

- Chat：`chat/completions`
- Responses：`responses`
- Anthropic：`messages`

## 视觉策略

端点默认视觉模式：

- `off`：不向上游发送图片。默认拒绝图片请求。
- `native`：保留图片内容，交给当前上游模型原生处理。
- `glue`：调用胶水视觉适配器。

不支持图片时的策略：

- `reject`：返回 `vision_disabled`，提示用户配置视觉能力。
- `strip_with_warning`：剥离图片后继续请求，并写入 warn 日志。

模型级覆盖：

- `inherit`：继承端点默认视觉模式。
- `off`：该模型关闭视觉。
- `native`：该模型原生支持图片。
- `glue`：该模型走胶水视觉。

这用于 OpenRouter 这类聚合端点：同一端点下不同模型的图片能力可以不一致。

## 胶水视觉

第一版只实现 MiniMax `coding_plan/vlm`：

- `adapter_id`: `minimax_coding_plan_vlm`
- 默认路径：`v1/coding_plan/vlm`
- 请求体：`{ "prompt": "...", "image_url": "data:image/..." }`

胶水策略：

- `final_answer`：视觉模型直接回答，deecodex 把结果包装为 Responses 响应。
- `caption_then_main`：视觉模型先生成图片描述，deecodex 剥离原图后把描述追加给主模型继续回答。

胶水模式是严格的：

- 未配置视觉上游 URL 会返回 `vision_glue_not_configured`。
- 视觉 URL 非法会返回 `vision_glue_invalid_url`。
- 非 MiniMax 适配器会返回 `vision_adapter_reserved`。
- 不会静默回退到主模型，也不会回退到 localhost。

## 迁移行为

加载 `accounts.json` 时会自动归一化为 v2：

- 旧账号对象会生成一个默认端点。
- 旧账号数组格式会迁移为 `AccountStore`。
- `translate_enabled = true` 迁移为 `open_ai_chat`。
- `translate_enabled = false` 迁移为 `open_ai_responses`。
- 旧 MiniMax 多模态配置迁移为 `vision.mode = glue` 和 `adapter_id = minimax_coding_plan_vlm`。
- GUI 启动加载已有账号文件时会安全写回规范 v2。

旧字段仍保留反序列化兼容，新保存统一写 v2。

## 运行时同步

服务启动和账号/端点切换会同步以下端点字段到运行时：

- 上游 URL
- 模型映射
- 自定义请求头
- 请求超时
- 最大重试次数
- reasoning effort 覆盖
- thinking token 预算
- 视觉上游、视觉 key、视觉模型、视觉路径
- 上下文窗口覆盖

## 已知限制

- Anthropic Messages 当前只支持非流式请求；流式和后台模式会返回明确的未实现错误。
- 胶水视觉第一版只实现 MiniMax。
- API Key 仍沿用当前持久化方式，未接入系统钥匙串。
- GUI 实测和真实上游 Key 冒烟需要在本机完成。
