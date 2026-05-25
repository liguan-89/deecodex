# 插件市场验收清单

本文用于 DEX AI 插件市场、已安装插件和开发入口的回归验收。自动化检查只能覆盖语法和部分数据流，真实行为仍需要在 Tauri 开发版里确认。

## 启动前

- 执行 `pgrep -x cargo || true`，确认没有并发构建。
- 用 `DEECODEX_GUI_ALLOW_MULTI_INSTANCE=1 cargo tauri dev` 启动开发版，不关闭安装版。
- 确认 `.deecodex-dev/` 和 `deecodex-gui/.deecodex-dev/` 不进入提交。

## 市场扫描

- 插件市场能扫描内置插件、开发模板和个人市场目录。
- 无效插件目录会被跳过并写日志，不阻塞整个市场列表。
- 市场卡片展示名称、版本、来源、分类、风险和兼容状态。
- 同版本同来源 hash 的已安装插件不显示可更新。
- 版本变化或来源 hash 明确变化时才显示可更新。

## 安装与更新

- 未安装插件点击卡片打开安装预览。
- 已安装且无更新的插件点击卡片进入已安装详情页。
- 有更新的插件点击卡片打开更新预览。
- 安装预览展示权限、能力、来源 hash、安装目录、资产目录和兼容性。
- 安装成功后进入已安装详情页。
- 更新成功后保留配置、启用状态、长期数据、密钥和连接资产。

## 启停与状态

- 已安装列表能切换启用、停用、启动、停止。
- 插件运行状态能从停止、启动中、运行中、异常正确刷新。
- 自动刷新开关默认按界面状态工作，不强制启动插件。
- 停止和卸载按钮使用统一危险按钮样式，没有内联重色样式。

## 账号资产

- 支持添加插件连接账号。
- 支持启动连接、停止连接、认证和删除连接。
- 插件连接状态能显示未连接、连接中、已连接、登录过期和异常。
- 插件连接资产独立保存，不能被其他客户端账号列表接管。
- 卸载插件前明确提示会删除插件文件和隔离资产目录。

## 缓存与事件

- 资产区展示总占用、数据、缓存、密钥和连接资产数量。
- 清理缓存只清理缓存，不删除长期数据、密钥或连接资产。
- 运行事件能展示日志、状态、二维码、错误和资产操作。
- 最新二维码能正确预览，刷新事件不影响插件详情表单。

## 开发入口

- 开发入口默认折叠。
- 可从模板创建插件，并写入新的 `plugin.json`。
- 可选择插件目录或插件包进行校验。
- 可将插件目录打包为 zip。
- 可打开个人市场目录和插件开发目录。
- 模板创建、校验、打包、打开目录能力保持可用。

## 自动化检查

```bash
node --check deecodex-gui/gui/js/plugins.js
node --check deecodex-gui/gui/js/plugins-events.js
node --check deecodex-gui/gui/js/plugins-dev.js
node --check deecodex-gui/gui/js/plugins-detail.js
node --check deecodex-gui/gui/js/plugins-market.js
node --check deecodex-gui/gui/js/plugins-exports.js
node deecodex-gui/gui/js/plugins-render-smoke.test.js
cargo test --manifest-path deecodex-plugins/Cargo.toml
cargo clippy --manifest-path deecodex-plugins/Cargo.toml --all-targets -- -D warnings
git diff --check
```
