# DEX AI

DEX AI 是本地代理与桌面 GUI 组合项目，当前主开发区为 `~/deecodex`，主分支为 `deecodex-gui`。

## 工作区规则

- 只保留主开发区作为固定工作区。
- 需要隔离开发时，临时创建 `功能/<任务名>` worktree。
- 不再维护 `稳定版`、`编译-mac`、`编译-win`、`编译-linux` 这类固定分区。

详细规则见：

- [`WORKTREES.md`](WORKTREES.md)
- [`CLAUDE.md`](CLAUDE.md)

## 构建与测试

在主开发区执行：

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

## 文档

- [多账户、端点隔离与视觉策略](docs/ACCOUNT_ENDPOINT_VISION.md)
- [多账户端点视觉验收清单](docs/ACCEPTANCE_CHECKLIST.md)
- [技术预览版路线](docs/TECH_PREVIEW_ROUTER.md)
