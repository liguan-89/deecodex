# 功能/插件系统

## 职责

deecodex 插件宿主运行时，管理第三方插件的安装、卸载、生命周期、进程管理、RPC 通信。

## 覆盖模块

| 文件/目录 | 职责 |
|-----------|------|
| `deecodex-plugins/src/lib.rs` | 公开模块声明，导出 PluginManager、PluginManifest、PluginInfo 等 |
| `deecodex-plugins/src/manager.rs` | PluginManager 核心：安装/卸载/启停/RPC 调用 |
| `deecodex-plugins/src/manifest.rs` | 插件清单定义与解析（id、name、version、entry、permissions） |
| `deecodex-plugins/src/process.rs` | 插件子进程管理、stdin/stdout JSON-RPC 管道 |
| `deecodex-plugins/src/rpc.rs` | JSON-RPC 2.0 消息定义 |
| `deecodex-plugins/src/protocol.rs` | 协议常量、方法名、PluginState、AccountStatus 等公开类型 |
| `deecodex-plugins/src/store.rs` | 插件注册表持久化（`~/.deecodex/plugins.json`） |
| `deecodex-plugins/plugins/deecodex-weixin/` | 内置微信通道插件（Node.js，iLink Bot API） |
| `deecodex-plugins/Cargo.toml` | 独立 crate `deecodex-plugin-host` |
| `deecodex-plugins/INTEGRATION.md` | 集成到主 deecodex 的指南 |

## 编译

```bash
cargo build -p deecodex-plugin-host
cargo build -p deecodex-plugin-host --release
cargo test -p deecodex-plugin-host
```

## 推送

```bash
git add deecodex-plugins/
git commit -m "<描述>"
git push deecodex-new 功能/插件系统
```

## 合入主线

```bash
cd /Users/liguan/deecodex
git merge 功能/插件系统
git push deecodex-new deecodex-gui
```

## 注意

- 修改 `rpc.rs` 或 `protocol.rs` 的协议定义会影响所有插件，需保持向后兼容
- 新增 manifest 字段时同步更新 `store.rs` 的持久化逻辑
- 微信插件（`deecodex-weixin/`）依赖外部 iLink Bot API，改动前确认 API 兼容性
