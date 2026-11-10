# DEX AI 工作区说明

## 当前固定工作区

| 路径 | 分支 | 用途 |
|------|------|------|
| `~/deecodex/` | `deecodex-gui` | 主开发区、集成验证、发版前合入 |

当前只保留这一套固定工作区。  
`稳定版`、`编译-mac`、`编译-win`、`编译-linux` 等长期分区已取消，不再作为固定目录维护。

## 远程约定

- 只保留 `deecodex-new` 作为源码主远程。
  - 注：`deecodex-new` 是本地 remote 别名，2026-08-19 起实际指向
    `github.com/liguan-89/dexai-core`（原 `deecodex-new` 已更名）。
- `origin` 已移除，不再作为日常开发或发版目标。

## 开发原则

- 日常开发、共享层修改、跨模块修复、集成验证都在主开发区完成。
- 需要隔离风险或并行排查时，才临时创建 `功能/<任务名>` worktree。
- 发版构建也从主开发区完成，不再维护固定编译 worktree。
- 排查问题可以阅读全仓库，但提交前必须确认变更范围与任务目标一致。

## 临时任务 worktree

### 创建

```bash
cd ~/deecodex
git worktree add -b 功能/<任务名> 功能/<任务名> deecodex-gui
```

任务名使用中文短语，表达真实目标，例如 `功能/修复请求历史筛选`、`功能/优化插件安装`。

### 开发

- 临时 worktree 只承载当前任务，不长期占用模块名。
- 需要修改 GUI 共享层、跨模块类型或公共接口时，优先回到主开发区完成，或尽快合入主区后再继续。
- 改 GUI 后不强制在本机启动 `cargo tauri dev` 实测；编译通过不算视觉验证完成。如确需 GUI 验证，在局域网 Mac mini（<LAN_MAC_MINI>:22）上执行 `cargo tauri dev`。

### 合入与清理

```bash
cd ~/deecodex
git merge 功能/<任务名>
git push deecodex-new deecodex-gui
git worktree remove 功能/<任务名>
git branch -d 功能/<任务名>
```

若任务分支需要保留远端历史，可先推送再删除本地 worktree：

```bash
git push deecodex-new 功能/<任务名>
git worktree remove 功能/<任务名>
```

## 模块归属表

模块归属用于控制修改边界，不再对应长期 worktree 目录。

| 模块 | 主要路径 | 说明 |
|------|----------|------|
| 协议翻译 | `src/translate.rs`、`src/stream.rs`、`src/handlers.rs`、`src/sse.rs`、`src/types.rs`、`src/utils.rs` | Responses API 与 Chat Completions API 双向转换 |
| 本地能力 | `src/files.rs`、`src/vector_stores.rs`、`src/prompts.rs`、`src/executor.rs` | Files、Vector Stores、Prompts、本地执行器 |
| 账号与配置 | `src/accounts.rs`、`src/config.rs`、`src/codex_config.rs`、`src/session.rs`、`src/cache.rs` | 多账号、配置合并、Codex 注入、会话与缓存 |
| 运行保护 | `src/ratelimit.rs`、`src/metrics.rs`、`src/token_anomaly.rs`、`src/validate.rs` | 限流、指标、异常检测、诊断 |
| 线程与历史 | `src/codex_threads.rs`、`src/request_history.rs` | Codex 线程聚合与请求历史 |
| GUI 桌面壳 | `deecodex-gui/src/lib.rs`、`deecodex-gui/src/commands/` | Tauri app、托盘、窗口、IPC 命令 |
| GUI 前端 | `deecodex-gui/gui/js/`、`deecodex-gui/gui/css/`、`deecodex-gui/gui/nav/` | WebView UI、导航片段、样式 |
| 插件系统 | `deecodex-plugins/`、`deecodex-gui/gui/js/plugins.js` | 插件宿主、插件管理 UI |
| Windows 兼容 | `deecodex.bat`、`install.ps1`、`#[cfg(target_os = "windows")]` 代码块 | Windows 启动、安装、平台隔离 |

## GUI 共享层

以下文件属于共享架构层，不归单一业务模块独占：

```text
deecodex-gui/gui/index.html
deecodex-gui/gui/css/app.css
deecodex-gui/gui/js/ui-core.js
deecodex-gui/gui/js/tauri-api.js
deecodex-gui/gui/js/theme-config.js
deecodex-gui/gui/js/app-shell.js
deecodex-gui/gui/js/startup.js
deecodex-gui/gui/js/panels-core.js
deecodex-gui/gui/js/formatters.js
deecodex-gui/gui/js/setup-wizard.js
deecodex-gui/build.rs
deecodex-gui/tauri.conf.json
```

修改共享层时，提交说明写清影响范围；若在临时 worktree 中完成，应尽快合入主区，避免其他任务基于旧共享结构继续开发。

## 发版流程

1. 在主开发区完成修复并确认测试通过。
2. 同步版本号：`Cargo.toml`、`deecodex-gui/Cargo.toml`、`deecodex-gui/tauri.conf.json`、`VERSION`。
3. 提交版本号并打 tag。
4. 在主开发区直接构建安装包和发布产物。
5. 按当前仓库策略推送源码与发布物。

## 清理规则

- `功能/` 下不长期保留空闲 worktree。
- 临时 worktree 合入后立即 `git worktree remove`。
- 不再创建固定的 `稳定版`、`编译-*` 分区。
- 删除 worktree 前先确认 `git -C <路径> status --short` 为空。
