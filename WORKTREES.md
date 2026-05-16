# deecodex 工作区说明

## 仓库分工

| Remote | 仓库 | 用途 | 可见性 |
|--------|------|------|--------|
| `deecodex-new` | `liguan-89/deecodex-new` | 源码仓库 | 私有，不对外开放克隆 |
| `origin` | `liguan-89/deecodex` | 二进制发布 | 公开，仅发布编译产物 |

宗旨：不开放源码克隆，只发二进制。

## 长期工作区

长期只保留这些 worktree：

```text
/Users/liguan/deecodex/                       主开发区，分支 deecodex-gui
/Users/liguan/deecodex/稳定版                 稳定验证/热修复区
/Users/liguan/deecodex/编译二进制/编译-mac     macOS 发版编译
/Users/liguan/deecodex/编译二进制/编译-win     Windows 发版编译
/Users/liguan/deecodex/编译二进制/编译-linux   Linux 发版编译
```

`功能/` 不再保存固定 11 个长期分区。需要隔离开发时，按任务临时创建；合入主区后删除。

## 临时任务 worktree

### 创建

```bash
cd /Users/liguan/deecodex
git worktree add -b 功能/<任务名> 功能/<任务名> deecodex-gui
```

任务名使用中文短语，表达真实目标，例如 `功能/修复请求历史筛选`、`功能/优化插件安装`。

### 开发

- 临时 worktree 只承载当前任务，不长期占用模块名。
- 排查问题可以阅读全仓库，修改范围应贴合任务目标。
- 需要修改 GUI 共享层、跨模块类型或公共接口时，优先回到主开发区完成，或尽快合入主区后再继续。
- 改 GUI 后必须通过 `cargo tauri dev` 实测；编译通过不算完成。

### 合入与清理

```bash
cd /Users/liguan/deecodex
git merge 功能/<任务名>
git push deecodex-new deecodex-gui
git worktree remove 功能/<任务名>
git branch -d 功能/<任务名>
```

若任务分支需要保留远程历史，可推送后再删除本地 worktree：

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

## 导航栏机制

导航栏采用片段文件动态加载：

```text
deecodex-gui/gui/nav/
├── 01-服务概览.html
├── 02-协议配置.html
├── 03-执行诊断.html
├── 04-账号管理.html
├── 05-请求历史.html
├── 06-线程聚合.html
├── 07-插件管理.html
├── 08-使用帮助.html
├── 09-DEX助手.html
└── 10-个人中心.html
```

- `build.rs` 在编译时读取 `gui/nav/*.html`，生成 `fragments.js`。
- `gui/js/app-shell.js` 中的 `loadNav()` 按顺序加载片段。
- 新增导航项时同时维护片段、面板渲染逻辑和 Tauri 实测。

## 编译工作区

三个编译工作区只用于发版构建，避免把安装包、staging 目录和大体积 target 写入功能开发区。

### 编译-mac

```bash
cd /Users/liguan/deecodex/编译二进制/编译-mac
cargo build --release
cargo tauri build --bundles dmg
```

### 编译-win

```bash
cd /Users/liguan/deecodex/编译二进制/编译-win
cargo build --release
cargo tauri build --bundles nsis
```

### 编译-linux

```bash
cd /Users/liguan/deecodex/编译二进制/编译-linux
cargo build --release
cargo tauri build --bundles deb
```

## 发版流程

1. 在主开发区合入所有修复，确认测试通过。
2. 同步版本号：`Cargo.toml`、`deecodex-gui/Cargo.toml`、`deecodex-gui/tauri.conf.json`、`VERSION`。
3. 提交版本号并打 tag。
4. 同步三个编译 worktree，分别构建安装包。
5. 推送私有源码分支到 `deecodex-new`，推送公开 tag 到 `origin`，上传安装包到 Releases。

升级检测读取 `origin` 远程 tag 并比较版本号，因此每次发版必须升版本号并推送 tag。

## 清理规则

- `功能/` 下不长期保留空闲 worktree。
- 临时 worktree 合入后立即 `git worktree remove`。
- 功能开发区不保留 `target-*`、`release/`、`installer-staging/`。
- 删除 worktree 前先确认 `git -C <路径> status --short` 为空。

## 已归档分支

以下旧分支已打 `archive/` 标签保留历史：

| 分支 | 归档标签 | 最后提交 |
|------|----------|----------|
| `build-v1.8.11-win` | `archive/build-v1.8.11-win` | Windows 兼容修复 |
| `download-page` | `archive/download-page` | README 下载页 |
| `deecodex-test` | `archive/deecodex-test` | CDP 注入 + 自动启动 |
| `deecodex-gui-pre-merge` | `archive/deecodex-gui-pre-merge` | CLAUDE.md + v1.4.1 |
| `deecodex-gui-rebuild` | `archive/deecodex-gui-rebuild` | .deecodex 排除 + v1.4.1 合并 |

查看归档标签：

```bash
git tag -l 'archive/*'
```
