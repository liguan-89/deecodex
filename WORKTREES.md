# deecodex 项目工作区说明

## 仓库分工

| Remote | 仓库 | 用途 | 可见性 |
|--------|------|------|--------|
| `deecodex-new` | `liguan-89/deecodex-new` | 源码仓库 | **私有**，不对外开放克隆 |
| `origin` | `liguan-89/deecodex` | 二进制发布 | **公开**，仅发布编译产物 |

宗旨：**不开放源码克隆，只发二进制**。

---

## 命名约定

| 场景 | 命名语言 | 示例 |
|------|----------|------|
| 源码功能分支 / 工作区 | 中文 | `功能/协议配置`、`功能/服务概览` |
| 编译 / 二进制分支 | 英文 | `编译-mac`、`编译-win`、`编译-linux` |
| 发布页 | 英文（对应二进制） | `release/v1.9.5` |

---

## 项目结构

```
/Users/liguan/deecodex/                       ← 主工作区（deecodex-gui 分支）
│
├── 功能/
│   ├── 服务概览/          ← 功能/服务概览（lib.rs + 服务生命周期 + 日志）
│   ├── 协议配置/          ← 功能/协议配置（核心翻译）
│   ├── 执行诊断/          ← 功能/执行诊断（本地能力 + 诊断引擎）
│   ├── 账号管理/          ← 功能/账号管理（多账号 + 配置 + 会话 + 注入）
│   ├── 请求历史/          ← 功能/请求历史
│   ├── 线程聚合/          ← 功能/线程聚合
│   ├── 插件管理/          ← 功能/插件管理（插件宿主 + GUI）
│   ├── 使用帮助/          ← 功能/使用帮助
│   ├── DEX助手/           ← 功能/DEX助手
│   ├── 个人中心/          ← 功能/个人中心
│   └── Windows兼容/       ← 功能/Windows兼容（跨平台修复）
│
└── 编译二进制/
    ├── 编译-mac/          ← macOS 编译
    ├── 编译-win/          ← Windows 编译
    └── 编译-linux/        ← Linux 编译
```

---

## 导航栏机制

导航栏采用**片段文件动态加载**，避免多分支编辑同一文件冲突：

```
gui/nav/
├── 01-服务概览.html  →  功能/服务概览
├── 02-协议配置.html  →  功能/协议配置
├── 03-执行诊断.html  →  功能/执行诊断
├── 04-账号管理.html  →  功能/账号管理
├── 05-请求历史.html  →  功能/请求历史
├── 06-线程聚合.html  →  功能/线程聚合
├── 07-插件管理.html  →  功能/插件管理
├── 08-使用帮助.html  →  功能/使用帮助
├── 09-DEX助手.html   →  功能/DEX助手
└── 10-个人中心.html  →  功能/个人中心
```

- `build.rs` 在编译时读取所有 `gui/nav/*.html`，生成 `fragments.js`
- `gui/js/app-shell.js` 中的 `loadNav()` 按顺序加载 10 个片段
- 每个功能分支**只维护自己的片段文件**，合入 `deecodex-gui` 时零冲突

---

## GUI 共享层

以下文件属于共享架构层，**不归单一业务分区独占**：

```
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

**修改共享层时：**
- 优先在父区 `deecodex-gui` 做；
- 提交说明写清影响范围；
- 合入后立刻同步所有功能 worktree。

---

## 一、功能工作区

每个功能工作区均基于 `deecodex-gui` 分支创建，可独立开发、编译、测试，互不干扰。

**每个分区的覆盖范围同时包含后端 Rust 模块和前端 JS 文件。** 排查 bug 时可以阅读任何分区代码，修改仅限本分区覆盖的文件。

### 1. 功能/服务概览

**原分支：** 功能/桌面端

**覆盖模块：**
- `deecodex-gui/src/lib.rs` — Tauri app 主结构、托盘、窗口、IPC 注册
- `deecodex-gui/src/commands.rs` 中服务启停/状态/CDP/升级/logs 相关命令
- `deecodex-gui/src/commands/logs.rs` — 日志命令
- `deecodex-gui/gui/js/service-management.js` — 服务管理面板
- `deecodex-gui/gui/js/log-viewer.js` — 日志弹窗
- `deecodex-gui/gui/nav/01-服务概览.html` — 导航栏片段

**职责：** 系统托盘、控制台窗口、服务启停/状态/CDP、日志查看/清空、升级检测。

**编译：**
```bash
cd 功能/服务概览
cargo build -p deecodex-gui
cargo build -p deecodex-gui --release
cargo test -p deecodex-gui
```

**推送：**
```bash
cd 功能/服务概览
git add deecodex-gui/src/lib.rs deecodex-gui/src/commands.rs deecodex-gui/src/commands/ deecodex-gui/gui/js/service-management.js deecodex-gui/gui/js/log-viewer.js deecodex-gui/gui/nav/01-服务概览.html
git commit -m "<描述>"
git push deecodex-new 功能/服务概览
```

**验证：改完代码必须启动 GUI 实际测试，编译通过不算完成。**

---

### 2. 功能/协议配置

**原分支：** 功能/核心翻译

**覆盖模块：**
- `src/translate.rs` — 请求方向：Responses → Chat 翻译
- `src/stream.rs` — 响应方向：SSE 流翻译
- `src/handlers.rs` — HTTP 路由与请求处理
- `src/sse.rs` — SSE 事件构建
- `src/types.rs` — 请求/响应类型定义
- `src/utils.rs` — 合并/截断工具函数
- `gui/nav/02-协议配置.html` — 导航栏片段

**职责：** OpenAI Responses API ↔ Chat Completions API 双向协议翻译，HTTP 路由与 SSE 流处理。

**编译：**
```bash
cd 功能/协议配置
cargo build
cargo test
cargo clippy -- -D warnings
```

---

### 3. 功能/执行诊断

**原分支：** 功能/本地能力

**覆盖模块：**
- `src/files.rs` — 本地 Files API
- `src/vector_stores.rs` — 本地 Vector Stores API
- `src/prompts.rs` — Prompts 注册表
- `src/executor.rs` — Computer Use / MCP 本地执行器
- `deecodex-gui/gui/js/panels-core.js` — 诊断面板渲染逻辑
- `gui/nav/03-执行诊断.html` — 导航栏片段

**职责：** 本地 Files API、Vector Stores API、Prompts 注册表、Computer Use / MCP 本地执行器诊断。

---

### 4. 功能/账号管理

**原分支：** 功能/集成与会话

**覆盖模块：**
- `src/accounts.rs` — 多账号管理
- `src/config.rs` — 配置系统（Args、合并逻辑）
- `src/validate.rs` — 诊断引擎
- `src/codex_config.rs` — Codex 配置注入
- `src/cdp.rs` — CDP 注入
- `src/inject.rs` — 注入逻辑
- `src/session.rs` — 会话存储
- `src/cache.rs` — 请求缓存
- `src/backup_store.rs` — 备份存储
- `src/ratelimit.rs` — 速率限制
- `src/metrics.rs` — Prometheus 指标
- `src/token_anomaly.rs` — Token 异常检测
- `deecodex-gui/gui/js/accounts.js` — 账号管理面板
- `gui/nav/04-账号管理.html` — 导航栏片段

**职责：** 多账号管理、配置系统、Codex 配置注入/CDP 注入、会话存储、请求缓存、Token 异常检测、速率限制、Prometheus 指标。

---

### 5. 功能/请求历史

**覆盖模块：**
- `src/request_history.rs` — 请求历史存储与查询
- `deecodex-gui/gui/js/request-history.js` — 请求历史面板
- `gui/nav/05-请求历史.html` — 导航栏片段

---

### 6. 功能/线程聚合

**覆盖模块：**
- `src/codex_threads.rs` — 线程聚合逻辑
- `deecodex-gui/gui/js/threads.js` — 线程聚合面板
- `gui/nav/06-线程聚合.html` — 导航栏片段

---

### 7. 功能/插件管理

**原分支：** 功能/插件系统

**覆盖模块：**
- `deecodex-plugins/` — 插件宿主 crate（安装/卸载/启停、JSON-RPC 通信）
- `deecodex-gui/src/commands.rs` 中插件相关 Tauri commands
- `deecodex-gui/gui/js/plugins.js` — 插件管理面板
- `gui/nav/07-插件管理.html` — 导航栏片段

---

### 8. 功能/使用帮助

**覆盖模块：**
- `deecodex-gui/gui/js/panels-core.js` — 帮助面板渲染逻辑
- `gui/nav/08-使用帮助.html` — 导航栏片段

**职责：** 使用帮助文档、常见问题与故障排查。本分区无核心 Rust 模块。

---

### 9. 功能/DEX助手

**覆盖模块：**
- `deecodex-gui/gui/js/placeholder-pages.js` — DEX助手占位
- `gui/nav/09-DEX助手.html` — 导航栏片段

**职责：** 智能诊断与辅助工具。本分区无核心 Rust 模块。

---

### 10. 功能/个人中心

**覆盖模块：**
- `deecodex-gui/gui/js/placeholder-pages.js` — 个人中心占位
- `gui/nav/10-个人中心.html` — 导航栏片段

**职责：** 账户信息与偏好设置。本分区无核心 Rust 模块。

### 11. 功能/Windows兼容

**覆盖模块：**
- `deecodex.bat` — Windows 启动脚本
- `install.ps1` — Windows 安装脚本
- `deecodex-gui/tauri.conf.json` — Windows 打包配置部分
- `deecodex-gui/icons/` — 应用图标
- 源码中 `#[cfg(target_os = "windows")]` 代码块

**导航片段：** 无（跨平台底层修复，不涉及前端面板）

**特殊规则：** 优先用 `#[cfg(target_os = "windows")]` 在源码中隔离，避免影响其他平台。本分区可能修改共享文件，需在提交说明中注明影响范围。

---

## 二、编译工作区

三个编译工作区均基于 `deecodex-gui` 分支创建，各自独立编译目录，可**同时编译不同平台**互不冲突。

### 编译-mac

```bash
cd 编译二进制/编译-mac
cargo build --release          # 产物在 target-mac/release/
cargo tauri build --bundles dmg # 打包 DMG
```

### 编译-win

```bash
cd 编译二进制/编译-win
cargo build --release          # 产物在 target-win/release/
cargo tauri build --bundles nsis # 打包 NSIS 安装包
```

### 编译-linux

```bash
cd 编译二进制/编译-linux
cargo build --release          # 产物在 target-linux/release/
cargo tauri build --bundles deb  # 打包 deb
```

### 发布二进制

发布流程涉及 3 个步骤：

| 步骤 | 分区 | 操作 |
|------|------|------|
| 1 | **deecodex-gui**（父区） | 合入所有修复后，升级版本号并打 tag |
| 2 | **编译二进制/** | 从父区同步，编译各平台安装包 |
| 3 | **origin**（公开仓库） | tag 推到这里，上传安装包到 Releases |

**第一步：父区升级版本号并打 tag**

```bash
cd /Users/liguan/deecodex

# 1. 升级版本号（四处同步）
#    Cargo.toml → version = "1.9.7"
#    deecodex-gui/Cargo.toml → version = "1.9.7"
#    deecodex-gui/tauri.conf.json → "version": "1.9.7"
#    VERSION → v1.9.7

# 2. 提交版本号
git add Cargo.toml deecodex-gui/Cargo.toml deecodex-gui/tauri.conf.json VERSION
git commit -m "chore: 版本号 → 1.9.7"

# 3. 打 tag 并推送
git tag v1.9.7
git push deecodex-new deecodex-gui
git push origin v1.9.7          # ← origin 是公开仓库，check_upgrade 读它
```

**升级检测原理**

用户 GUI 中的 `check_upgrade` 命令读取 `origin` 远程的所有 tag，比较版本号。版本号变更才会触发更新提示。因此每次发版必须升版本号。

---

## 三、开发工作流

### 新功能开发

```bash
cd /Users/liguan/deecodex
git worktree add -b 功能/<新功能名> 功能/<新功能名> deecodex-gui
```

### 合入主分支

```bash
cd /Users/liguan/deecodex
git merge 功能/<功能名>
git push deecodex-new deecodex-gui
```

### 同步其他工作区

```bash
for b in 服务概览 协议配置 执行诊断 账号管理 请求历史 线程聚合 插件管理 使用帮助 DEX助手 个人中心 Windows兼容; do
  git -C "功能/$b" merge deecodex-gui -m "merge: 同步 deecodex-gui"
done
git push deecodex-new 功能/服务概览 功能/协议配置 ...
```

### 冲突预防三原则

1. **严格按分区改文件** — 每个分区只改本文档中列出的模块和前端文件。需要改共享模块时，去对应的分区改，或在父区改后同步。
2. **跨分区改动先合** — 动了共享结构体/接口（如 `Args`、`AppState`、`GuiConfig`）的改动优先合入主干，其他分支同步后再继续。
3. **勤同步，逐个合** — 每合完一个分支马上同步其他 worktree，冲突摊到每次。

---

## 四、已归档分支

### 旧功能分支（远程已删除，本地保留重命名）

| 旧分支 | 新分支 |
|--------|--------|
| `功能/核心翻译` | `功能/协议配置` |
| `功能/桌面端` | `功能/服务概览` |
| `功能/本地能力` | `功能/执行诊断` |
| `功能/集成与会话` | `功能/账号管理` |
| `功能/插件系统` | `功能/插件管理` |

### 归档标签

以下旧分支已打 `archive/` 标签保留历史：

| 分支 | 归档标签 | 最后提交 |
|------|----------|----------|
| `build-v1.8.11-win` | `archive/build-v1.8.11-win` | Windows 兼容修复 |
| `download-page` | `archive/download-page` | README 下载页 |
| `deecodex-test` | `archive/deecodex-test` | CDP 注入 + 自动启动 |
| `deecodex-gui-pre-merge` | `archive/deecodex-gui-pre-merge` | CLAUDE.md + v1.4.1 |
| `deecodex-gui-rebuild` | `archive/deecodex-gui-rebuild` | .deecodex 排除 + v1.4.1 合并 |

```bash
# 查看归档标签
git tag -l 'archive/*'
```
