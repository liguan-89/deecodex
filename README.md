# 功能/本地能力

## 职责

本地 API 实现：文件管理、向量存储、提示词注册、本地执行器（Computer Use / MCP）。

## 覆盖模块

| 文件 | 行数 | 职责 |
|------|------|------|
| `src/files.rs` | ~1,450 | 本地 Files API（`/v1/files`）：上传/列出/删除/搜索（BM25 索引） |
| `src/vector_stores.rs` | ~761 | 本地 Vector Stores API（`/v1/vector_stores/*`）：创建/列出/删除/文件批量操作 |
| `src/prompts.rs` | ~711 | 本地 Prompts API（`/v1/prompts/*`）：注册/解析/变量替换 |
| `src/executor.rs` | ~1,060 | 本地执行器：Computer Use（Playwright/BrowserUse 后端）+ MCP stdio JSON-RPC |

## 编译

```bash
cargo build
cargo test
cargo clippy -- -D warnings
```

## 推送

```bash
git add src/files.rs src/vector_stores.rs src/prompts.rs src/executor.rs
git commit -m "<描述>"
git push deecodex-new 功能/本地能力
```

## 合入主线

```bash
cd /Users/liguan/deecodex
git merge 功能/本地能力
git push deecodex-new deecodex-gui
```

## 注意

- `files.rs` 的文件内容存储在 `~/.deecodex/files/`，修改存储路径时需迁移已有数据
- `executor.rs` 的 MCP 和 Computer Use 默认禁用，需在配置中显式启用
- `vector_stores.rs` 的数据持久化为 JSON 快照，修改数据结构时需考虑兼容性
- Playwright 后端依赖外部 `playwright_cli` 可执行文件
