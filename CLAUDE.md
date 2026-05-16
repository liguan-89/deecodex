# CLAUDE.md

## 交流语言

与本仓库的所有交互使用中文。

## 当前工作区

你正在 **主开发区** `/Users/liguan/deecodex` 工作，对应主分支 `deecodex-gui`。

主开发区用于日常开发、共享层修改、集成验证和发版前合入。不要再把长期功能 worktree 当作固定模块边界使用；需要隔离开发时，按任务临时创建 `功能/<任务名>` worktree，完成合入后删除。

## 修改边界

- 小改动、共享层改动、跨模块修复优先在主开发区完成。
- 大功能或高风险实验可临时创建 `功能/<任务名>` worktree。
- 编译产物和安装包只放在 `编译二进制/` 工作区或发布目录，不回写源码区。
- 排查 bug 可以阅读全仓库；提交前确认变更范围与任务目标一致。

## Build & Test

```bash
cargo build
cargo build --release
cargo test --all-targets
cargo clippy -- -D warnings
cargo fmt --check
```

运行 `cargo build` 前先检查并发构建：

```bash
pgrep -x cargo
```

若已有 cargo 进程在运行，等待结束后再构建。

## deecodex-gui 前端规则

`deecodex-gui` 是 Tauri 桌面应用，不支持将 `gui/index.html` 当作独立网页使用。

真实功能测试必须使用：

```bash
cd /Users/liguan/deecodex
cargo tauri dev
```

前端约定：

- 不把大段 JS/CSS 写回 `index.html`。
- 不直接调用 `window.__TAURI__`，统一走 `DeeCodexTauri.invoke(name, args)`。
- 不直接访问 `localStorage`，统一走 `deeStorage`。
- 不为 `file://` 预览模式做兜底或假数据。
- 非 Tauri 环境直接显示阻断页。
- 改完 GUI 代码必须启动 GUI 实测，编译通过不算完成。

## 模块归属

详细模块归属、临时 worktree 流程和发版编译工作区见 `WORKTREES.md`。

高层规则：

- 协议翻译：`src/translate.rs`、`src/stream.rs`、`src/handlers.rs`、`src/sse.rs`、`src/types.rs`、`src/utils.rs`。
- 本地能力与诊断：`src/files.rs`、`src/vector_stores.rs`、`src/prompts.rs`、`src/executor.rs`、`src/validate.rs`。
- 账号、配置与会话：`src/accounts.rs`、`src/config.rs`、`src/codex_config.rs`、`src/session.rs`、`src/cache.rs`、`src/ratelimit.rs`、`src/metrics.rs`、`src/token_anomaly.rs`。
- GUI：`deecodex-gui/src/`、`deecodex-gui/gui/js/`、`deecodex-gui/gui/nav/`、`deecodex-gui/gui/css/`。
- 插件：`deecodex-plugins/` 与插件相关 GUI/IPC。

## 提交

中文 commit，前缀使用：

- `feat:` — 新功能
- `fix:` — 修复
- `refactor:` — 重构
- `docs:` — 文档
- `chore:` — 杂项/构建
- `release:` — 发版
