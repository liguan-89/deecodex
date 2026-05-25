# 项目健康记录

记录日期：2026-05-25

本记录用于保存本轮项目巡检发现的结构债和后续拆分建议。当前轮次只修阻断和高频踩坑点，不做大范围 UI 或架构拆分。

## 本轮已处理

- 主项目、GUI 和插件宿主的 clippy 阻断项已收口到局部结构调整。
- 用户可见品牌继续迁移到 `DEX AI`，内部兼容名继续保留 `deecodex`。
- Codex 额度查询 User-Agent 改为随当前包版本生成，移除旧的 `3.0beta` 硬编码。
- GUI 初始化移除开发机本机绝对路径，只保留仓库相对路径和打包资源路径。
- 插件市场卡片点击行为按未安装、已安装、可更新三类区分。
- 插件市场前端已从单个 `plugins.js` 拆成状态、事件、开发入口、详情、市场和导出模块。
- 插件市场前端新增加载顺序、重复函数和基础渲染 smoke test。
- 后端插件 Tauri 命令已从 `commands/mod.rs` 拆到 `commands/plugins.rs`，外部命令名保持不变。
- DEX Markdown 链接只允许 `http:` 和 `https:` 协议渲染成可点击链接。
- DEX Markdown 渲染器已从页面文件抽出，并补链接、表格和代码块 smoke test。
- DEX 助手前端主体已从 `placeholder-pages.js` 拆到 `dex-assistant.js`，个人中心占位页保持独立。
- DEX 助手附件输入和对话搜索已拆成独立脚本，避免继续堆进主 Agent 文件。
- DEX 助手运行态补丁样式已从 JS 注入迁移到 `app.css`，减少页面初始化副作用。
- DEX 助手快捷键绑定已拆成独立脚本，主 Agent 文件继续减负。
- DEX 助手消息气泡、工具结果、确认卡片和思考状态已拆成 `dex-assistant-messages.js`。

## 保留的兼容边界

- crate 名、二进制名、配置目录 `~/.deecodex` 不迁移。
- Codex provider id `deecodex` 不迁移。
- 已有账号字段、历史配置字段和数据目录结构不迁移。
- 文档中的命令名和路径示例继续使用现有兼容名称。

## 大文件风险

- `deecodex-gui/gui/css/app.css` 约 18922 行，主题覆盖和页面样式混在一起，后续 UI 调整容易互相污染。
- `deecodex-gui/src/commands/mod.rs` 约 6776 行，插件命令已移出，但账号、线程、额度等命令仍集中在同一文件。
- `deecodex-gui/src/commands/dex.rs` 约 5131 行，DEX 助手工具、诊断和环境读取逻辑需要继续分层。
- 插件市场前端已拆分为多个 `plugins-*` 模块，后续继续避免把新增能力回灌进单个大文件。
- `src/handlers.rs` 约 8071 行，HTTP handler、图片代理、官方账号和历史记录逻辑继续膨胀。

## 样式债

- `app.css` 中亮色、暗色和页面专用覆盖层较多，后续需要按页面 shell、通用控件、主题覆盖拆分。
- 插件管理、线程聚合、账号管理已经多次经历局部覆盖，继续修改前应先确认是否存在旧规则压过新规则。
- 危险按钮应统一走 `btn-danger` 或页面既有 danger 类，不再写内联红色背景。
- 新页面应复用统一内容框，不把局部实验结构反向扩散到全局页面。

## 插件市场后续建议

- 保持市场扫描、安装预览、已安装详情、开发入口和事件流的前端模块边界。
- 保持后端插件命令在 `commands/plugins.rs` 的模块边界，新增插件 IPC 不再回灌到 `commands/mod.rs`。
- 市场更新判断和插件账号资产隔离已有基础测试，后续新增插件资产类型时继续扩展覆盖。
- 插件风险等级和权限说明需要形成稳定文档，减少前端写死解释。

## DEX 助手后续建议

- DEX Markdown 渲染器已独立为 `dex-render-markdown.js`，后续继续补更完整的 Markdown 边界测试。
- DEX 助手前端主体已独立为 `dex-assistant.js`，附件、搜索、快捷键和消息列表也已拆出；后续可继续拆 agent 与模型状态。
- DEX 输出区、输入区和工具调用 UI 应继续保持透明结构层，避免再次叠出多层内容框。
- DEX 工具执行结果需要统一轻量样式，避免输出块比主内容更重。

## 验收提醒

- GUI 代码改完必须启动 Tauri 开发版实测，不能只用浏览器打开 `gui/index.html`。
- 构建或测试前先用 `pgrep -x cargo || true` 检查并发 cargo。
- 不提交 `.deecodex-dev/` 和 `deecodex-gui/.deecodex-dev/`。
