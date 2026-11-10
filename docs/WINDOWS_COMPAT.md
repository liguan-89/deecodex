# Windows 兼容性加固清单（DEX AI / deecodex）

> 用途：对 Windows 分支做兼容性审计/修复时，按此清单逐类排查；新代码也按「通用原则」规避。
> 对应提交：`c6365aa4`（终端）/ `50ef1d36`（弹窗）/ `affaf218`（路径）/ `3488280d`（换行/BOM）/
> `bb043f7e`（编码）/ `dd2d7f23`（命令/进程）/ `d532d6f0`（动态库/WebView2/文件锁）/
> `fca2631b`（环境变量/配置目录）/ `4281d51d`（权限/原子写）/ `06528d3d`（GUI/线程模型）

## 通用原则（跨平台编码红线）

- 用户主目录：一律 `config::home_dir()`（Windows 走 `USERPROFILE`，回退 `HOME`）；**不要直接 `env::var("HOME")`**。
- PATH：用 `std::env::split_paths` / `join_paths`（Windows 分隔符 `;`），**不要 `split(':')`**；查可执行文件要补 `.exe/.cmd/.bat` 后缀。
- 临时目录：`std::env::temp_dir()`。
- 行切分：`.lines()`（自动剥 `\r\n`）；**不要 `split('\n')`** 残留 `\r`。
- 平台专属依赖：放 `[target.'cfg(windows)'.dependencies]` / `[target.'cfg(target_os = "macos")'.dependencies]`。
- 平台专属命令：`osascript/open/plutil/scutil/pgrep/lsof/ps/df/tail/sh` 等必须 `#[cfg(...)]` 门控，或提供 Windows 等价（powershell/tasklist/netstat/cmd）。
- 路径匹配：涉及 `.codex` / `.deecodex` / worktree 等目录时先 `replace('\\', "/")` 归一化再 `contains/ends_with`。
- 不依赖英文本地化文本（如 tasklist 的 `No tasks`），按结构（CSV 第 2 列 = PID）判断。

## 11 类清单

### 1. 终端/CLI 交互（cmd CRLF）
- 现象：Windows 内置终端回车不执行、输入无回显。
- 原因：`cmd.exe` 管道模式只在收到 `\n` 才处理一行，xterm 回车只发 `\r`；无 PTY 无逐字符回显。
- 修复：转发层把单独的 `\r` 规范化为 `\r\n`（`data.replace(/\r(?!\n)/g, '\r\n')`），仅 Windows cmd 输入路径。
- 已修：`c6365aa4`（dexai-debug `index.js` shell.open 输入）。

### 2. 弹窗/控制台窗口隐藏（CREATE_NO_WINDOW）
- 现象：spawn powershell/cmd/tasklist/curl/opencode 等时弹黑色控制台窗口。
- 修复：
  - `std::process::Command`：`use std::os::windows::process::CommandExt; cmd.creation_flags(0x08000000);`
  - `tokio::process::Command` 同样支持该 trait。
- 排查：`grep -rn "Command::new"` 逐个确认 spawn 点是否加 `creation_flags` / `hide_window`。
- 已修：`50ef1d36`（20 处 / 11 文件）。

### 3. 路径
- PATH 分隔符（`;` vs `:`）、`HOME`/`USERPROFILE`、`~/.config` vs `%APPDATA%`（见第 8 类）。
- 会话/临时工作区/Codex worktree 路径识别先归一化反斜杠；`Path::join` 在 Windows 接受 `/`。
- 已修：`affaf218`。

### 4. 换行符 / BOM
- 写回 Codex rollout/`global-state.json` 时保留原 CRLF（先检测 `\r\n`/`\n` 再按原样写）。
- 读文件剥行尾 `\r`；生成内嵌进 JS 的文件要剥 `\r`（build.rs nav fragments）。
- UTF-8 BOM / UTF-16 解码统一用 `codex_config::read_config_file`。
- 插件包哈希按原始字节：Windows 检出需 `core.autocrlf=false|input`，否则与 mac/Linux 哈希不一致会 panic。
- 已修：`3488280d`。

### 5. 字符编码
- Windows PowerShell 5.1 重定向 stdout 默认 ANSI/OEM 代码页（中文系统 GBK）→ 脚本前缀
  `[Console]::OutputEncoding=[Text.Encoding]::UTF8;` 后再 `from_utf8_lossy`。
- Node 子进程输出：先 UTF-8（`TextDecoder('utf-8',{fatal:true})`），失败回退 `new TextDecoder('gbk').decode(buf)`。
- 不依赖本地化文本（如 tasklist "No tasks"）→ 按 CSV 结构判断。
- 已修：`bb043f7e`。

### 6. 系统命令 / 进程
- Unix-only 命令给 Windows 等价：
  - `kill` → `taskkill /PID`
  - `lsof` → `netstat -ano` + `tasklist`
  - `pgrep -f` → `powershell Get-CimInstance ... CommandLine -like '*...*'`
  - `tail -f` → `Get-Content -Wait`
  - `sh -c` → `cmd /C`
- `ping` 参数按平台（mac `-c/-t`、win `-n/-w`、linux `-c/-W`）。
- daemon 后台进程派生要 `CREATE_NO_WINDOW`。
- 已修：`dd2d7f23`。

### 7. 动态库 / 共享库
- 主 exe 只依赖 Windows 系统 DLL（UCRT api-sets）+ rustls（ring 静态），无 vcruntime140 → **无需 VC++ 运行库**。
- `deecodex-netd.exe`（Go）只依赖 kernel32.dll；插件为纯 JS，无原生 Node 模块。
- WebView2 Runtime：`downloadBootstrapper` 需联网；离线环境用 `offlineInstaller`
  （NSIS 内嵌 ~200MB，安装包约 90MB → 292MB）。
- 已修：`d532d6f0`（含 WebView2 离线 + 文件锁）。

### 8. 环境变量与配置目录
- 用户目录统一 `home_dir()`；`%APPDATA%`/`%LOCALAPPDATA%` 仅在 `cfg(windows)` 使用。
- opencode 原生配置：Windows 在 `%APPDATA%\opencode`（写入优先）；检测候选含
  `~/.config/opencode` + `%APPDATA%` + `%LOCALAPPDATA%`（保留 Unix 路径兜底）。
- `dsh`/`pi`/`workbuddy` 用 home 点目录（`~/.dsh` 等），Windows 天然兼容。
- 已修：`fca2631b`。

### 9. 文件权限与文件锁
- `chmod/chown/PermissionsExt` 仅 Unix 生效（Windows no-op）；Windows 用隐藏属性（`SetFileAttributesW`
  `FILE_ATTRIBUTE_HIDDEN`）对齐点目录/点文件语义。
- 文件锁：`flock` → Windows 用 `LockFileEx`（整文件排他、阻塞语义、句柄关闭自动释放）。
- 原子写 `tmp + rename`：Windows 目标只读/被占用时 `rename` 失败 → 先清只读属性重试，
  仍失败回退直接写/复制（保证落盘）。
- 已修：`4281d51d`。

### 10. 图形界面
- `titleBarStyle: "Overlay"` / `hiddenTitle` 仅 macOS；Windows 用 `decorations: false` + 前端自绘标题栏
  （配合 `start_window_drag` 命令）。
- 托盘/菜单事件处理器用 `async_runtime::spawn`，不阻塞主线程。
- 状态：配置作用域正确，无需改动。

### 11. 线程模型
- **setup 里不要用 `block_on` 做长任务**（插件安装/校验/HTTP）：先同步发布 manager 引用，
  再用 `async_runtime::spawn` 后台执行。
- 同步 Tauri 命令在 tokio worker 上运行；长阻塞（curl 10s / 诊断 / 强杀轮询）用 `async + spawn_blocking`。
- 命令内避免 `block_on`（改 `async fn` 直接 `.await`）。
- 已修：`06528d3d`。

## 验证命令

```bash
# 主机（macOS）
cargo check -p deecodex -p deecodex-gui -p deecodex-plugin-host
# Windows 目标（需 rustup target add x86_64-pc-windows-msvc + brew install llvm）
export PATH="$(brew --prefix llvm)/bin:$PATH"
cargo xwin check --target x86_64-pc-windows-msvc -p deecodex -p deecodex-gui -p deecodex-plugin-host
# 插件 JS 语法
node --check deecodex-plugins/plugins/deecodex-debug/index.js
```

> 普通 `cargo check --target x86_64-pc-windows-msvc` 会因 ring 缺 MSVC 头失败，必须走 `cargo xwin check`。

## 新代码审查清单（Windows 分支）

- [ ] 用户目录用 `home_dir()` 而非 `env HOME`
- [ ] PATH 用 `split_paths`；查可执行文件补 `.exe/.cmd/.bat`
- [ ] 每个 spawn 点都有 `CREATE_NO_WINDOW`（除非要显示终端）
- [ ] Unix-only 命令已 cfg 门控或有 Windows 等价
- [ ] 写回 Codex/文本文件保留 CRLF；读行用 `.lines()`
- [ ] PowerShell 输出强制 UTF-8；Node 输出 GBK 回退
- [ ] 路径匹配先归一化反斜杠；`%APPDATA%` 等平台目录差异化处理
- [ ] 文件锁 Windows 用 `LockFileEx`；原子写有 rename 回退
- [ ] 长阻塞操作不进主线程/setup；命令用 `async + spawn_blocking`
- [ ] 双端编译：`cargo check` + `cargo xwin check`
