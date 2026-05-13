# 功能/桌面端

## 职责

Tauri 2 桌面应用（deecodex 控制台），包含系统托盘、配置窗口、IPC 命令、插件生命周期管理。

## 覆盖模块

| 文件/目录 | 职责 |
|-----------|------|
| `deecodex-gui/src/main.rs` | 应用入口，调用 `deecodex_gui_lib::run()` |
| `deecodex-gui/src/lib.rs` | Tauri 应用主逻辑：系统托盘、插件安装/自启动、命令注册 |
| `deecodex-gui/src/commands.rs` | 全部 Tauri IPC 命令（40+）：服务启停、配置、账号、会话、请求历史、线程、文件、插件 |
| `deecodex-gui/tauri.conf.json` | 窗口配置、打包配置（DMG/NSIS） |
| `deecodex-gui/Cargo.toml` | 依赖 `deecodex` + `deecodex-plugin-host` |
| `deecodex-gui/gui/` | 前端 Web 页面 |

## 编译

```bash
cargo build -p deecodex-gui
cargo build -p deecodex-gui --release
cargo test -p deecodex-gui
```

## 推送

```bash
git add deecodex-gui/
git commit -m "<描述>"
git push deecodex-new 功能/桌面端
```

## 合入主线

```bash
cd /Users/liguan/deecodex
git merge 功能/桌面端
git push deecodex-new deecodex-gui
```

## 注意

- 修改 `commands.rs` 中的 IPC 命令签名时，确认前端 `gui/index.html` 中的调用同步更新
- 系统托盘图标文件路径在 `lib.rs` 中硬编码，勿随意移动资源文件
- `tauri.conf.json` 中 CSP 当前为无限制，如需收紧安全策略需同步测试前端功能
