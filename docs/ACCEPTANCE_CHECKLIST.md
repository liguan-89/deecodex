# 多账户端点视觉验收清单

本文用于 GUI 和真实上游验收。自动化验证命令不替代 GUI 实测。

## GUI 实测

- 打开开发版 GUI，不关闭安装版。
- 新建账号并保存。
- 编辑已有账号，确认旧字段不会丢失。
- 同一账号下新增端点。
- 复制端点。
- 删除非唯一端点。
- 应用端点后，账号列表显示活跃端点、协议和视觉模式。
- 切换账号后，活跃端点跟随目标账号生效。
- 上游 URL、端点路径、API Key、余额 URL 可以保存并回显。
- 模型映射可以保存并回显。
- 模型能力覆盖可以新增、保存、删除并回显。
- 视觉能力三段切换可用：关闭、原生、胶水。
- 关闭视觉时仍能看到“不支持图片时”的策略选项。
- 胶水模式下展示 MiniMax 模板、策略、视觉 URL、视觉 Key、视觉模型、视觉路径和测试按钮。
- 高级参数可以保存并回显：自定义 headers、timeout、retries、reasoning effort、thinking tokens。
- 诊断页能显示活跃账号、活跃端点、协议、视觉模式、胶水完整性和模型视觉覆盖检查。

## 真实上游冒烟

- OpenAI Responses 直连：文本请求成功。
- OpenAI Responses 直连：原生图片请求成功。
- OpenAI Chat 翻译：文本请求成功。
- DeepSeek 或 OpenRouter 的 Chat 兼容端点：文本请求成功。
- OpenRouter 聚合端点：不同模型使用不同视觉覆盖策略。
- MiniMax 胶水 `final_answer`：图片请求只打到视觉端点，返回视觉模型答案。
- MiniMax 胶水 `caption_then_main`：先打视觉端点，再打主模型端点，主模型请求中不含原始图片。
- 视觉关闭 + `reject`：图片请求返回 `vision_disabled`。
- 视觉关闭 + `strip_with_warning`：图片被剥离，请求继续，日志出现 warn。
- Anthropic Messages：非流式文本请求成功，请求头包含 `x-api-key` 和 `anthropic-version`。
- Anthropic Messages：流式或后台模式返回明确的未实现错误。

## 迁移验收

- 旧 v1 账号文件启动后写回 `version = 2`。
- 旧账号数组格式启动后写回 `version = 2`。
- `translate_enabled = true` 的旧账号迁移为 Chat 端点。
- `translate_enabled = false` 的旧账号迁移为 Responses 端点。
- 旧 MiniMax 视觉配置迁移为胶水视觉端点。
- 活跃账号和活跃端点无效时会修复为第一个可用项。

## 自动化验证

构建或测试前先确认没有并发 cargo：

```bash
pgrep -x cargo || true
```

推荐完整验证：

```bash
cargo fmt --check
node --check deecodex-gui/gui/js/accounts.js
node --check deecodex-gui/gui/js/app-shell.js
node --check deecodex-gui/gui/js/panels-core.js
git diff --check
cargo test --all-targets
cargo clippy -- -D warnings
cargo build
cargo test -p deecodex-gui --all-targets
cargo clippy -p deecodex-gui --all-targets -- -D warnings
cargo test --manifest-path deecodex-plugins/Cargo.toml
cargo clippy --manifest-path deecodex-plugins/Cargo.toml --all-targets -- -D warnings
cargo build --release
```

## 交付前边界确认

- 未实现 Anthropic 流式和后台模式。
- 未实现 MiniMax 以外的胶水视觉适配器。
- 未接入系统钥匙串。
- 未使用真实用户 Key 的自动化线上回归。
