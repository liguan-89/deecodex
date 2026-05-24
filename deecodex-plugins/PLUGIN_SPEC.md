# DEX AI 插件规范

插件是 DEX AI 的通用扩展单元。通讯通道、模型供应商、数据源、自动化任务、工作区工具都使用同一套 `plugin.json` 和 JSON-RPC stdio 协议。

## 插件类型

`kind` 描述插件的主类型：

- `tool`：向 DEX 助手暴露工具。
- `provider`：接入模型、端点或认证供应商。
- `datasource`：接入可搜索、可读取的数据源。
- `automation`：提供可执行、可查询状态的自动化任务。
- `channel`：接入消息通道或外部会话入口。
- `workspace`：扩展工作区、文件、项目操作。
- `integration`：其他第三方服务集成。

## 能力声明

`features` 描述插件贡献的具体能力。每个能力可以声明可被宿主调用的 `methods`：

```json
{
  "id": "docs-search",
  "kind": "datasource",
  "label": "文档搜索",
  "description": "搜索本地或远程文档索引",
  "methods": {
    "search": "datasource.search",
    "read": "datasource.read",
    "status": "datasource.status"
  },
  "params_schema": {
    "search": {
      "type": "object",
      "properties": {
        "query": { "type": "string", "title": "查询" },
        "limit": { "type": "integer", "default": 10 }
      },
      "required": ["query"]
    }
  }
}
```

`params_schema` 可选，用动作名映射到 JSON Schema object。插件中心会用它把能力动作渲染成表单；未声明时退回 JSON 参数输入。

常用 `features.kind`：

- `dex_tool`：DEX 助手动态工具。
- `model_provider`：模型供应商能力。
- `datasource`：数据源搜索/读取能力。
- `automation`：自动化执行/状态能力。
- `workspace`：工作区能力。
- `channel`：消息通道能力。
- `connection`：连接、认证或账号状态能力。
- `ui_panel`：未来 UI 面板扩展。

## 标准工作流

插件中心会按 `features.kind` 优先排列常见动作，并显示中文动作名：

- `datasource`：`search` / `read` / `status`
- `automation`：`run` / `status` / `stop`
- `workspace`：`list` / `read` / `write` / `status`
- `model_provider`：`models` / `status` / `test`
- `channel` / `connection`：`login` / `status` / `start` / `stop`

不在标准列表里的动作仍会显示，但会排在标准动作之后。建议新插件优先使用这些动作名，方便宿主生成一致的 UI 和 DEX 工具语义。

## DEX 工具

`dex_tools` 会被宿主转换成 DEX 助手可调用工具：

```json
{
  "name": "echo_message",
  "description": "回显一段文本",
  "level": 1,
  "method": "echo.message",
  "capability": "plugin.echo-test",
  "parameters": {
    "type": "object",
    "properties": {
      "message": { "type": "string" }
    },
    "required": ["message"]
  }
}
```

`level` 分级：

- `0`：只读、安全信息。
- `1`：低风险操作。
- `2`：会修改配置、启动进程或访问外部服务。
- `3`：高风险操作，需要确认。

## 可选连接能力

连接/账号不是插件主结构，只是可选能力。需要登录、扫码、启动网关的插件可以声明 `account`：

```json
{
  "account": {
    "enabled": true,
    "label": "微信连接",
    "methods": {
      "login": "account.login",
      "cancel_login": "account.cancel_login",
      "status": "account.status",
      "start": "account.start",
      "stop": "account.stop"
    }
  }
}
```

旧插件的 `weixin.login / weixin.status / weixin.start / weixin.stop` 仍被兼容。

连接资产由宿主独立保存，不属于普通 `config_schema` 配置项。插件中心新增或删除连接时写入注册表的 `account_assets`，运行期会为了兼容旧插件临时合成到 `config.accounts` 中。

## 资产目录

每个插件有独立资产根目录：

- `plugin-assets/<plugin_id>/data`：插件长期数据。
- `plugin-assets/<plugin_id>/cache`：可重建缓存。
- `plugin-assets/<plugin_id>/secrets`：认证文件、令牌和敏感连接资料。

宿主启动插件时会通过 `initialize` 参数传入：

- `asset_paths.install_dir`
- `asset_paths.data_dir`
- `asset_paths.cache_dir`
- `asset_paths.secrets_dir`

同时也会注入环境变量：

- `DEECODEX_PLUGIN_INSTALL_DIR`
- `DEECODEX_PLUGIN_DATA_DIR`
- `DEECODEX_PLUGIN_CACHE_DIR`
- `DEECODEX_PLUGIN_SECRETS_DIR`

生命周期规则：

- 安装插件会创建隔离资产目录。
- 更新插件会替换插件文件，但保留配置、启用状态、连接资产和资产目录。
- 卸载插件会删除插件文件和隔离资产目录。
- 插件详情页会展示资产占用、连接资产数量和目录路径。

插件应优先通过宿主受控 API 访问资产，而不是直接拼接目录路径。受控 API 会限制路径只能落在当前插件自己的资产目录内，并按 `permissions` 拦截：

- `assets.list`：列出 `data` 目录，参数 `{ "path": "" }`，需要 `fs.read`。
- `assets.read`：读取 `data` 文本文件，参数 `{ "path": "state.json" }`，需要 `fs.read`。
- `assets.write`：写入 `data` 文本文件，参数 `{ "path": "state.json", "content": "..." }`，需要 `fs.write`。
- `assets.delete`：删除 `data` 文件或子目录，参数 `{ "path": "state.json" }`，需要 `fs.write`。
- `cache.read` / `cache.write`：读取或写入 `cache` 文本文件，需要对应 `fs.read` / `fs.write`。
- `cache.clear`：清空当前插件 `cache` 目录，需要 `fs.write`。
- `secrets.set` / `secrets.get` / `secrets.delete`：写入、读取、删除密钥，需要 `secrets`、`secrets.read` 或 `secrets.write`。

受控 API 的 `path` / `key` 必须是相对路径，不能包含 `..`、根路径或符号链接跳转。每次资产操作会进入插件详情页的“运行事件”。

## 模板内置 SDK

`templates/node-*` 和 `templates/python-datasource` 都带有一个轻量 SDK 文件，插件作者优先从模板开始改：

- Node：`const { createPlugin } = require('./deecodex-plugin')`
- Python：`from deecodex_plugin import create_plugin`

SDK 会处理 stdio JSON-RPC、初始化、配置更新、关闭通知、宿主请求回包和错误响应。插件只需要声明：

- `initialize(params, host, req)`：初始化插件，读取 `params.config`。
- `notifications["config.update"]`：响应配置热更新。
- `methods["xxx.method"]`：实现 `features.methods` 或 `dex_tools.method` 暴露的方法。

Node 模板的 `host` 提供：

- `host.request(method, params)`：调用任意宿主受控 API。
- `host.log(level, message)`：写入运行事件。
- `host.llm.call(params)`：请求 DEX AI 模型链路。
- `host.assets.read/write/list/delete(...)`
- `host.cache.read/write/clear(...)`
- `host.secrets.set/get/delete(...)`

Python 模板提供对应的 `host.request()`、`host.log()`、`host.asset_*()`、`host.cache_*()`、`host.secret_*()` 方法。

## 权限

`permissions` 是安装预览和后续执行拦截的基础：

- `http` / `network`：网络访问。
- `llm.call`：调用 DEX AI 模型链路。
- `media.download` / `media.upload`：媒体处理。
- `fs.read` / `fs.write`：文件读取或写入。
- `secrets` / `secrets.read` / `secrets.write`：读取或写入插件密钥资产。
- `shell` / `exec` / `process`：本机进程或命令执行。
- `account` / `account.*`：连接、认证或账号状态。

宿主会在安装预览中标出低/中/高风险，并记录来源路径和 SHA-256 指纹。插件包含高风险权限时，通用能力动作需要确认；DEX 动态工具会被提升为 L3 确认操作。

## 更新插件

同 ID 插件再次导入时进入更新流程：

- 预览会展示当前版本、新版本、当前 SHA、新 SHA 和权限变化。
- 更新会停止运行中的旧插件，替换插件文件和 `plugin.json`。
- 更新会保留用户配置、启用状态、账号/连接资产和安装时间。
- 如果插件 ID 未安装，应走安装流程而不是更新流程。

## JSON-RPC 生命周期

插件进程通过 stdout/stdin 逐行收发 JSON-RPC：

1. 宿主启动插件入口脚本。
2. 宿主发送 `initialize` 请求。
3. 插件返回初始化结果。
4. 宿主发送 `initialized` 通知。
5. 宿主按 `dex_tools.method` 或 `features.methods` 调用插件方法。
6. 关闭时宿主发送 `shutdown` 通知。

插件可以主动发送通知：

- `log`：写入宿主日志。
- `status`：更新连接状态。
- `qr_code`：上报二维码。
- 插件 stderr、无法解析的 stdout 消息、初始化失败、启动失败和异常退出会被宿主转换成运行事件，显示在插件详情页的“运行事件”区域。

插件也可以主动请求：

- `llm.call`：通过 DEX AI 当前活跃账号调用模型。

## 运行事件

插件中心会保留最近一段运行事件，用于排查插件状态：

- `log`：插件主动日志或宿主捕获的 stderr。
- `status_changed`：连接/账号状态变化。
- `qr_code`：认证二维码，详情页会展示最近一张二维码。
- `error`：启动、初始化或进程退出等不可恢复错误。

事件日志是运行期诊断信息，不是长期审计数据库；插件需要长期记录时应自己写入受控数据目录，并在 `permissions` 中声明相应权限。

## 启用与运行状态

插件安装后默认是“已启用、已停止”。这两个状态不要混用：

- `enabled=false`：插件被停用，宿主不会把它的 `dex_tools` 暴露给 DEX 助手，也不会通过 `features.methods` 自动拉起插件。
- `state=stopped`：插件已启用但进程未运行，可以由用户启动，也可以由低风险动态工具按需启动。
- 停用插件时，宿主会先停止正在运行的插件，但会保留插件文件、配置和账号信息。

插件作者不需要在 `plugin.json` 里声明 `enabled`；这是宿主注册表的运行期字段。

## 最小 plugin.json

```json
{
  "id": "example-tool",
  "name": "Example Tool",
  "version": "1.0.0",
  "description": "示例工具插件",
  "author": "DEX AI",
  "kind": "tool",
  "tags": ["example"],
  "entry": {
    "runtime": "node",
    "script": "index.js"
  },
  "features": [
    {
      "id": "example-tools",
      "kind": "dex_tool",
      "label": "示例工具"
    }
  ],
  "dex_tools": [],
  "permissions": []
}
```
