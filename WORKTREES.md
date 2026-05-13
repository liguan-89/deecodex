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
| 源码功能分支 / 工作区 | 中文 | `功能/核心翻译`、`功能/桌面端` |
| 编译 / 二进制分支 | 英文 | `编译-mac`、`编译-win`、`编译-linux` |
| 发布页 | 英文（对应二进制） | `release/v1.9.5` |

---

## 项目结构

```
/Users/liguan/deecodex/                       ← 主工作区（deecodex-gui 分支）
│
├── 功能/
│   ├── 核心翻译/          ← 方案B-1
│   ├── 桌面端/            ← 方案B-2
│   ├── 插件系统/          ← 方案B-3
│   ├── 本地能力/          ← 方案B-4
│   └── 集成与会话/        ← 方案B-5
│
└── 编译二进制/
    ├── 编译-mac/          ← macOS 编译
    ├── 编译-win/          ← Windows 编译
    └── 编译-linux/        ← Linux 编译
```

---

## 一、功能工作区

每个功能工作区均基于 `deecodex-gui` 分支创建，可独立开发、编译、测试，互不干扰。

### 1. 功能/核心翻译

**覆盖模块：** `translate.rs` · `stream.rs` · `handlers.rs` · `sse.rs` · `types.rs` · `utils.rs`

**职责：** OpenAI Responses API ↔ Chat Completions API 双向协议翻译，HTTP 路由与 SSE 流处理。

**编译：**
```bash
cd 功能/核心翻译
cargo build
cargo build --release
cargo test
cargo clippy -- -D warnings
```

**推送：**
```bash
cd 功能/核心翻译
git add src/translate.rs src/stream.rs src/handlers.rs src/sse.rs src/types.rs src/utils.rs
git commit -m "<描述>"
git push deecodex-new 功能/核心翻译
```

---

### 2. 功能/桌面端

**覆盖模块：** `deecodex-gui/` 整个 Tauri 桌面应用 crate

**职责：** 系统托盘、控制台窗口、IPC 命令、插件生命周期管理。

**编译：**
```bash
cd 功能/桌面端
cargo build -p deecodex-gui
cargo build -p deecodex-gui --release
cargo test -p deecodex-gui
```

**推送：**
```bash
cd 功能/桌面端
git add deecodex-gui/
git commit -m "<描述>"
git push deecodex-new 功能/桌面端
```

---

### 3. 功能/插件系统

**覆盖模块：** `deecodex-plugins/` 整个插件宿主 crate

**职责：** 插件安装/卸载/启停、子进程管理、JSON-RPC 通信、微信通道插件。

**编译：**
```bash
cd 功能/插件系统
cargo build -p deecodex-plugin-host
cargo build -p deecodex-plugin-host --release
cargo test -p deecodex-plugin-host
```

**推送：**
```bash
cd 功能/插件系统
git add deecodex-plugins/
git commit -m "<描述>"
git push deecodex-new 功能/插件系统
```

---

### 4. 功能/本地能力

**覆盖模块：** `files.rs` · `vector_stores.rs` · `prompts.rs` · `executor.rs`

**职责：** 本地 Files API、Vector Stores API、Prompts 注册表、Computer Use / MCP 本地执行器。

**编译：**
```bash
cd 功能/本地能力
cargo build
cargo test
```

**推送：**
```bash
cd 功能/本地能力
git add src/files.rs src/vector_stores.rs src/prompts.rs src/executor.rs
git commit -m "<描述>"
git push deecodex-new 功能/本地能力
```

---

### 5. 功能/集成与会话

**覆盖模块：** `config.rs` · `validate.rs` · `codex_config.rs` · `cdp.rs` · `inject.rs` · `codex_threads.rs` · `accounts.rs` · `backup_store.rs` · `session.rs` · `cache.rs` · `request_history.rs` · `token_anomaly.rs` · `ratelimit.rs` · `metrics.rs`

**职责：** 配置系统、Codex 配置注入/CDP 注入、线程聚合/迁移、多账号管理、会话存储、请求缓存与历史、Token 异常检测、速率限制、Prometheus 指标。

**编译：**
```bash
cd 功能/集成与会话
cargo build
cargo test
```

**推送：**
```bash
cd 功能/集成与会话
git add src/
git commit -m "<描述>"
git push deecodex-new 功能/集成与会话
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

```bash
# 查看归档标签
git tag -l 'archive/*'

# 查看归档分支的提交
git log archive/<分支名>
```
