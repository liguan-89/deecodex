# 功能/集成与会话

## 职责

配置系统、Codex 桌面端集成（配置注入/CDP/线程聚合）、多账号管理、会话存储、请求缓存与历史、Token 异常检测、速率限制、指标暴露。

## 覆盖模块

| 文件 | 行数 | 职责 |
|------|------|------|
| `src/config.rs` | ~555 | CLI 参数定义、config.json 持久化、三源合并逻辑 |
| `src/validate.rs` | ~858 | 启动前配置诊断（API key、model map、executor 路径等） |
| `src/codex_config.rs` | ~724 | Codex `config.toml` 注入/移除/修复（toml_edit 非破坏性编辑） |
| `src/cdp.rs` | ~167 | Chrome DevTools Protocol 客户端（WebSocket、JS 执行） |
| `src/inject.rs` | ~404 | CDP 注入编排：插件解锁、会话删除 UI、CDP 桥接 |
| `src/codex_threads.rs` | ~526 | Codex 线程聚合/迁移/还原（SQLite 解析） |
| `src/accounts.rs` | ~242 | 多账号数据结构与存储 |
| `src/backup_store.rs` | ~77 | 会话数据 JSON 备份 |
| `src/session.rs` | ~788 | 内存会话/对话存储（DashMap + LRU 淘汰） |
| `src/cache.rs` | ~418 | LRU 请求结果缓存 |
| `src/request_history.rs` | ~193 | 请求历史持久化（SQLite）+ 月度统计 |
| `src/token_anomaly.rs` | ~200 | 滑动窗口 Token 用量异常检测 |
| `src/ratelimit.rs` | ~90 | 滑动窗口速率限制 |
| `src/metrics.rs` | ~187 | Prometheus 指标注册与暴露 |
| `src/main.rs` | ~776 | 二进制入口：服务管理、tracing、配置合并、启动 HTTP |

## 编译

```bash
cargo build
cargo test
cargo clippy -- -D warnings
```

## 推送

```bash
git add src/
git commit -m "<描述>"
git push deecodex-new 功能/集成与会话
```

## 合入主线

```bash
cd /Users/liguan/deecodex
git merge 功能/集成与会话
git push deecodex-new deecodex-gui
```

## 注意

- `codex_config.rs` 操作 `~/.codex/config.toml`，修改注入逻辑需在 Windows 和 macOS 上分别验证编码检测
- `inject.rs` + `cdp.rs` 依赖 Codex Electron 的远程调试端口（9222-9250），端口扫描逻辑勿随意修改
- `session.rs` 全内存存储，重启丢失——这是设计如此，Codex 会重放完整对话
- `config.rs` 的合并优先级：CLI > env > config.json，不要改变此顺序
- `token_anomaly.rs` 的阈值调整会影响 Token 盗刷误报率
