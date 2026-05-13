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
│   ├── 服务概览/          ← 功能/服务概览（原桌面端）
│   ├── 协议配置/          ← 功能/协议配置（原核心翻译）
│   ├── 执行诊断/          ← 功能/执行诊断（原本地能力）
│   ├── 账号管理/          ← 功能/账号管理（原集成与会话）
│   ├── 请求历史/          ← 功能/请求历史
│   ├── 线程聚合/          ← 功能/线程聚合
│   ├── 插件管理/          ← 功能/插件管理（原插件系统）
│   ├── 使用帮助/          ← 功能/使用帮助（原帮助）
│   ├── DEX助手/           ← 功能/DEX助手
│   └── 个人中心/          ← 功能/个人中心
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

- `index.html` 通过 `loadNav()` 按文件名顺序加载 10 个片段
- 每个功能分支**只维护自己的片段文件**，合入 `deecodex-gui` 时零冲突

---

## 一、功能工作区

每个功能工作区均基于 `deecodex-gui` 分支创建，可独立开发、编译、测试，互不干扰。

### 1. 功能/协议配置

**原分支：** 功能/核心翻译

**覆盖模块：** `translate.rs` · `stream.rs` · `handlers.rs` · `sse.rs` · `types.rs` · `utils.rs`

**导航片段：** `gui/nav/02-协议配置.html`

**职责：** OpenAI Responses API ↔ Chat Completions API 双向协议翻译，HTTP 路由与 SSE 流处理。

**编译：**
```bash
cd 功能/协议配置
cargo build
cargo build --release
cargo test
cargo clippy -- -D warnings
```

**推送：**
```bash
cd 功能/协议配置
git add src/translate.rs src/stream.rs src/handlers.rs src/sse.rs src/types.rs src/utils.rs
git commit -m "<描述>"
git push deecodex-new 功能/协议配置
```

---

### 2. 功能/服务概览

**原分支：** 功能/桌面端

**覆盖模块：** `deecodex-gui/` 整个 Tauri 桌面应用 crate

**导航片段：** `gui/nav/01-服务概览.html`

**职责：** 系统托盘、控制台窗口、服务启停/状态/CDP、IPC 命令、插件生命周期管理。

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
git add deecodex-gui/
git commit -m "<描述>"
git push deecodex-new 功能/服务概览
```

---

### 3. 功能/插件管理

**原分支：** 功能/插件系统

**覆盖模块：** `deecodex-plugins/` 整个插件宿主 crate

**导航片段：** `gui/nav/07-插件管理.html`

**职责：** 插件安装/卸载/启停、子进程管理、JSON-RPC 通信、微信通道插件。

**编译：**
```bash
cd 功能/插件管理
cargo build -p deecodex-plugin-host
cargo build -p deecodex-plugin-host --release
cargo test -p deecodex-plugin-host
```

**推送：**
```bash
cd 功能/插件管理
git add deecodex-plugins/
git commit -m "<描述>"
git push deecodex-new 功能/插件管理
```

---

### 4. 功能/执行诊断

**原分支：** 功能/本地能力

**覆盖模块：** `files.rs` · `vector_stores.rs` · `prompts.rs` · `executor.rs`

**导航片段：** `gui/nav/03-执行诊断.html`

**职责：** 本地 Files API、Vector Stores API、Prompts 注册表、Computer Use / MCP 本地执行器诊断。

**编译：**
```bash
cd 功能/执行诊断
cargo build
cargo test
```

**推送：**
```bash
cd 功能/执行诊断
git add src/files.rs src/vector_stores.rs src/prompts.rs src/executor.rs
git commit -m "<描述>"
git push deecodex-new 功能/执行诊断
```

---

### 5. 功能/账号管理

**原分支：** 功能/集成与会话

**覆盖模块：** `accounts.rs` · `config.rs` · `validate.rs` · `codex_config.rs` · `cdp.rs` · `inject.rs` · `session.rs` · `cache.rs` · `backup_store.rs` · `ratelimit.rs` · `metrics.rs` · `token_anomaly.rs`

**导航片段：** `gui/nav/04-账号管理.html`

**职责：** 多账号管理、配置系统、Codex 配置注入/CDP 注入、会话存储、请求缓存、Token 异常检测、速率限制、Prometheus 指标。

**编译：**
```bash
cd 功能/账号管理
cargo build
cargo test
```

**推送：**
```bash
cd 功能/账号管理
git add src/
git commit -m "<描述>"
git push deecodex-new 功能/账号管理
```

---

### 6. 功能/请求历史

**覆盖模块：** `request_history.rs`

**导航片段：** `gui/nav/05-请求历史.html`

**职责：** 请求历史记录、月度统计、趋势图、自动清理。

**编译：**
```bash
cd 功能/请求历史
cargo build
cargo test
```

---

### 7. 功能/线程聚合

**覆盖模块：** `codex_threads.rs`

**导航片段：** `gui/nav/06-线程聚合.html`

**职责：** Codex 线程聚合、迁移、还原、校准。

**编译：**
```bash
cd 功能/线程聚合
cargo build
cargo test
```

---

### 8. 功能/使用帮助

**导航片段：** `gui/nav/08-使用帮助.html`

**职责：** 使用帮助文档、常见问题与故障排查。

**编译：**
```bash
cd 功能/使用帮助
cargo build
cargo test
```

---

### 9. 功能/DEX助手

**导航片段：** `gui/nav/09-DEX助手.html`

**职责：** 智能诊断与辅助工具。

**编译：**
```bash
cd 功能/DEX助手
cargo build
cargo test
```

---

### 10. 功能/个人中心

**导航片段：** `gui/nav/10-个人中心.html`

**职责：** 账户信息与偏好设置。

**编译：**
```bash
cd 功能/个人中心
cargo build
cargo test
```

---

## 二、编译工作区

三个编译工作区均基于 `deecodex-gui` 分支创建，各自独立 `target/` 目录，可**同时编译不同平台**互不冲突。

### 编译-mac

```bash
cd 编译二进制/编译-mac
cargo build --release          # 产物在 target-mac/release/
```

### 编译-win

```bash
cd 编译二进制/编译-win
cargo build --release          # 产物在 target-win/release/
```

### 编译-linux

```bash
cd 编译二进制/编译-linux
cargo build --release          # 产物在 target-linux/release/
```

### 发布二进制

编译完成后，发布到公开仓库 `origin`（`liguan-89/deecodex`）：

```bash
# 1. 切到对应的编译工作区
cd 编译二进制/编译-mac    # 以 mac 为例

# 2. 打发布 tag（英文）
git tag release/v1.9.6-beta

# 3. 推送到公开仓库
git push origin release/v1.9.6-beta

# 4. 在 GitHub Releases 页面上传编译产物
# https://github.com/liguan-89/deecodex/releases
```

---

## 三、开发工作流

### 新功能开发

```bash
# 在主工作区创建新功能 worktree
cd /Users/liguan/deecodex
git worktree add -b 功能/<新功能名> 功能/<新功能名> deecodex-gui
```

### 合入主分支

```bash
# 功能开发完成后
cd /Users/liguan/deecodex
git merge 功能/<功能名>
git push deecodex-new deecodex-gui
```

### 查看所有工作区

```bash
cd /Users/liguan/deecodex
git worktree list
```

### 删除已完成的工作区

```bash
cd /Users/liguan/deecodex
git worktree remove 功能/<功能名>
git branch -d 功能/<功能名>
```

---

## 四、已归档分支

以下旧分支已打 `archive/` 标签保留历史，暂未删除：

| 分支 | 归档标签 | 最后提交 |
|------|----------|----------|
| `build-v1.8.11-win` | `archive/build-v1.8.11-win` | Windows 兼容修复 |
| `download-page` | `archive/download-page` | README 下载页 |
| `deecodex-test` | `archive/deecodex-test` | CDP 注入 + 自动启动 |
| `deecodex-gui-pre-merge` | `archive/deecodex-gui-pre-merge` | CLAUDE.md + v1.4.1 |
| `deecodex-gui-rebuild` | `archive/deecodex-gui-rebuild` | .deecodex 排除 + v1.4.1 合并 |

以下旧功能分支已删除远程（本地保留重命名）：

| 旧分支 | 新分支 |
|--------|--------|
| `功能/核心翻译` | `功能/协议配置` |
| `功能/桌面端` | `功能/服务概览` |
| `功能/本地能力` | `功能/执行诊断` |
| `功能/集成与会话` | `功能/账号管理` |
| `功能/插件系统` | `功能/插件管理` |

```bash
# 查看归档标签
git tag -l 'archive/*'

# 查看归档分支的提交
git log archive/<分支名>
```
