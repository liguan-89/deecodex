use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tauri::State;

use crate::ServerManager;

#[cfg(target_os = "windows")]
use super::dex_cli::command_exists;
use super::dex_cli::{command_first_line, find_command_path, get_cli_version_flexible};
use super::dex_process::detect_process_instances;

#[cfg(windows)]
fn hide_window(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x08000000);
}

#[cfg(not(windows))]
fn hide_window(_command: &mut std::process::Command) {}

/// Codex 桌面在 macOS 上的统一识别/启动/退出依据。
///
/// 真值源在 `deecodex::codex_desktop`（同时被 GUI / daemon 共享），此处仅保留引用以避免破坏现有字面量与测试断言。
const CODEX_DESKTOP_BUNDLE_ID: &str = deecodex::codex_desktop::BUNDLE_ID;
const CODEX_DESKTOP_MAC_PATHS: &[&str] = deecodex::codex_desktop::MAC_PATHS;

#[derive(Clone, Copy)]
struct ClientAppSpec {
    kind: &'static str,
    label: &'static str,
    account_kind: &'static str,
    surface: &'static str,
    command: Option<&'static str>,
    launch_command: Option<&'static str>,
    launch_requires_cwd: bool,
    /// CLI 启动后需要帮用户打开的本机 Web 服务地址（如 `dsh web` 的 http://localhost:3080）。
    /// 设值后启动 CLI 会在后台等待服务端口就绪，再用系统浏览器打开。
    open_web_url: Option<&'static str>,
    desktop_app: Option<&'static str>,
    /// Windows 上的可执行文件名候选列表（如 `Codex.exe` / `ChatGPT.exe`）。
    /// 多个候选按顺序拼接路径探测，匹配第一个存在的；避免桌面应用改名后无法识别。
    desktop_exe: Option<&'static [&'static str]>,
    process_names: &'static [&'static str],
    mac_desktop_paths: &'static [&'static str],
    /// macOS 上的 Bundle ID，用于 `open -b` 启动 / `quit app id` 优雅退出。
    /// 设为 `None` 时退化为按 `desktop_app` 名称调用。
    /// macOS 上的 Bundle ID 候选列表，用于 `open -b` 启动 / `quit app id` 优雅退出。
    /// 支持同一客户端多版本（如 WorkBuddy 海外版与国内版），按顺序逐个尝试；
    /// 全部失败时退化为按 `desktop_app` / 应用名候选调用。
    mac_bundle_ids: &'static [&'static str],
    download_url: Option<&'static str>,
    mac_install_command: Option<&'static str>,
    windows_install_command: Option<&'static str>,
    mac_uninstall_command: Option<&'static str>,
    windows_uninstall_command: Option<&'static str>,
    mac_update_command: Option<&'static str>,
    windows_update_command: Option<&'static str>,
    /// 「清理配置文件」的专属路径（macOS / Windows 各一份，`~/` 前缀展开为家目录）。
    /// 每种客户端按真实配置位置配置，避免用通用命令误删其它客户端数据。
    mac_clean_config_paths: &'static [&'static str],
    windows_clean_config_paths: &'static [&'static str],
    /// 是否纳入 DEX AI 的账号/配置管理体系。
    /// `true`（默认）时生命周期在安装后要求先配置账号；`false`（如 dsh/Pi/OpenCode）
    /// 表示只做下载/安装/启动，安装完成后直接进入启动。
    managed_account: bool,
}

const CLIENT_APP_SPECS: &[ClientAppSpec] = &[
    ClientAppSpec {
        kind: "codex_cli",
        label: "Codex CLI",
        account_kind: "codex",
        surface: "cli",
        command: Some("codex"),
        launch_command: Some("codex"),
        launch_requires_cwd: true,
        open_web_url: None,
        desktop_app: None,
        desktop_exe: None,
        process_names: &["codex"],
        mac_desktop_paths: &[],
        mac_bundle_ids: &[],
        download_url: None,
        mac_install_command: Some("npm install -g @openai/codex"),
        windows_install_command: Some("npm install -g @openai/codex"),
        // macOS 上 codex 可能来自 npm（app 默认安装方式）或官方 standalone 安装器
        // （~/.codex/packages/standalone + ~/.local/bin/codex 软链），两者都处理：
        // npm 包不存在时静默跳过，standalone 目录/软链存在则移除。
        mac_uninstall_command: Some(
            "npm uninstall -g @openai/codex 2>/dev/null; rm -rf ~/.codex/packages/standalone; rm -f ~/.local/bin/codex ~/.local/bin/codex-code-mode-host",
        ),
        windows_uninstall_command: Some("npm uninstall -g @openai/codex"),
        mac_update_command: Some("npm install -g @openai/codex@latest"),
        windows_update_command: Some("npm install -g @openai/codex@latest"),
        mac_clean_config_paths: &["~/.codex"],
        windows_clean_config_paths: &["~/.codex"],
        managed_account: true,
    },
    ClientAppSpec {
        kind: "codex_desktop",
        label: "Codex 桌面",
        account_kind: "codex",
        surface: "desktop",
        command: None,
        launch_command: None,
        launch_requires_cwd: false,
        open_web_url: None,
        desktop_app: Some("Codex"),
        desktop_exe: Some(&["Codex.exe", "ChatGPT.exe"]),
        process_names: &["Codex", "codex"],
        mac_desktop_paths: CODEX_DESKTOP_MAC_PATHS,
        mac_bundle_ids: &[CODEX_DESKTOP_BUNDLE_ID],
        download_url: Some("https://developers.openai.com/codex/app"),
        mac_install_command: None,
        windows_install_command: None,
        mac_uninstall_command: None,
        windows_uninstall_command: None,
        mac_update_command: None,
        windows_update_command: None,
        // 新版 Codex 桌面改名为 ChatGPT.app，但数据目录仍是 ~/Library/Application Support/Codex
        // （见 src/codex_desktop.rs 的权威说明）。
        mac_clean_config_paths: &["~/Library/Application Support/Codex"],
        windows_clean_config_paths: &["~/AppData/Roaming/Codex"],
        managed_account: true,
    },
    ClientAppSpec {
        kind: "claude_cli",
        label: "Claude CLI",
        account_kind: "claude_code",
        surface: "cli",
        command: Some("claude"),
        launch_command: Some("claude"),
        launch_requires_cwd: true,
        open_web_url: None,
        desktop_app: None,
        desktop_exe: None,
        process_names: &["claude"],
        mac_desktop_paths: &[],
        mac_bundle_ids: &[],
        download_url: None,
        mac_install_command: Some("npm install -g @anthropic-ai/claude-code"),
        windows_install_command: Some(
            "winget install Anthropic.ClaudeCode || npm install -g @anthropic-ai/claude-code",
        ),
        mac_uninstall_command: Some("npm uninstall -g @anthropic-ai/claude-code"),
        windows_uninstall_command: Some(
            "winget uninstall Anthropic.ClaudeCode || npm uninstall -g @anthropic-ai/claude-code",
        ),
        mac_update_command: Some("npm install -g @anthropic-ai/claude-code@latest"),
        windows_update_command: Some(
            "winget upgrade Anthropic.ClaudeCode || npm install -g @anthropic-ai/claude-code@latest",
        ),
        // Claude Code 的本地配置在 ~/.claude（settings.json/projects 等），
        // 全局状态文件 ~/.claude.json 一并清理，避免残留登录态/开关。
        mac_clean_config_paths: &["~/.claude", "~/.claude.json"],
        windows_clean_config_paths: &["~/.claude", "~/.claude.json"],
        managed_account: true,
    },
    ClientAppSpec {
        kind: "claude_desktop",
        label: "Claude 桌面",
        account_kind: "claude_code",
        surface: "desktop",
        command: None,
        launch_command: None,
        launch_requires_cwd: false,
        open_web_url: None,
        desktop_app: Some("Claude"),
        desktop_exe: Some(&["Claude.exe"]),
        process_names: &["Claude", "claude"],
        mac_desktop_paths: &["/Applications/Claude.app", "~/Applications/Claude.app"],
        mac_bundle_ids: &[],
        download_url: Some("https://claude.ai/download"),
        mac_install_command: None,
        windows_install_command: None,
        mac_uninstall_command: None,
        windows_uninstall_command: None,
        mac_update_command: None,
        windows_update_command: None,
        // Claude 桌面 Electron 数据目录：Claude 与 Claude-3p 都存在时都要清
        // （与 client_integrations.rs 的 claude_desktop_user_data_dirs 保持一致）。
        mac_clean_config_paths: &[
            "~/Library/Application Support/Claude",
            "~/Library/Application Support/Claude-3p",
        ],
        windows_clean_config_paths: &["~/AppData/Roaming/Claude"],
        managed_account: true,
    },
    ClientAppSpec {
        kind: "hermes",
        label: "Hermes",
        account_kind: "hermes",
        surface: "cli",
        command: Some("hermes"),
        launch_command: Some("hermes"),
        launch_requires_cwd: false,
        open_web_url: None,
        desktop_app: None,
        desktop_exe: None,
        process_names: &["hermes"],
        mac_desktop_paths: &[],
        mac_bundle_ids: &[],
        download_url: Some("https://www.hermes-ai.net/docs/installation/"),
        mac_install_command: Some("curl -fsSL https://www.hermes-ai.net/install.sh | bash"),
        // Windows 装完还要把 pipx 的 bin 目录写入用户 PATH（ensurepath），
        // 否则 hermes.exe 装好后仍“检测不到/无法启动”。
        windows_install_command: Some(
            "python -m pip install --user pipx; python -m pipx install hermes-agent; python -m pipx ensurepath",
        ),
        // macOS：hermes 常以 launchd 网关服务（ai.hermes.gateway，KeepAlive）运行，
        // 只删目录会被服务拉活/残留。先 `hermes gateway stop`（bootout）再
        // `hermes gateway uninstall`（删 plist），随后清理安装目录
        // （~/.hermes、~/.local/state/hermes、旧安装 ~/hermes-agent）与启动器。
        mac_uninstall_command: Some(
            "hermes gateway stop 2>/dev/null; hermes gateway uninstall 2>/dev/null; rm -rf ~/.hermes ~/.local/state/hermes ~/hermes-agent; rm -f ~/.local/bin/hermes ~/Library/LaunchAgents/ai.hermes.gateway.plist",
        ),
        windows_uninstall_command: Some("python -m pipx uninstall hermes-agent"),
        mac_update_command: Some("curl -fsSL https://www.hermes-ai.net/install.sh | bash"),
        windows_update_command: Some("python -m pipx upgrade hermes-agent"),
        // Hermes CLI 配置：~/.hermes（config.yaml/.env/sessions）+ ~/.local/state/hermes。
        // 清理前会先走 dex_force_quit_client_impl 停止 launchd 网关，避免被 KeepAlive 拉活重建。
        mac_clean_config_paths: &["~/.hermes", "~/.local/state/hermes"],
        windows_clean_config_paths: &["~/.hermes"],
        managed_account: true,
    },
    ClientAppSpec {
        kind: "dsh",
        label: "dsh",
        account_kind: "dsh",
        surface: "cli",
        command: Some("dsh"),
        launch_command: Some("dsh web"),
        launch_requires_cwd: false,
        // dsh web 默认绑定 127.0.0.1:3080（实测：dsh --profile web 的默认端口）。
        open_web_url: Some("http://localhost:3080"),
        desktop_app: None,
        desktop_exe: None,
        process_names: &["dsh"],
        mac_desktop_paths: &[],
        mac_bundle_ids: &[],
        download_url: Some("https://github.com/deepseek-ai/deepseek-harness"),
        mac_install_command: Some("npm install -g @deepseek-ai/dsh"),
        windows_install_command: Some("npm install -g @deepseek-ai/dsh"),
        mac_uninstall_command: Some("npm uninstall -g @deepseek-ai/dsh"),
        windows_uninstall_command: Some("npm uninstall -g @deepseek-ai/dsh"),
        mac_update_command: Some("npm install -g @deepseek-ai/dsh@latest"),
        windows_update_command: Some("npm install -g @deepseek-ai/dsh@latest"),
        // dsh（DeepSeek Harness）配置目录 ~/.dsh：settings.yaml / .credentials.yaml / profiles / sessions。
        mac_clean_config_paths: &["~/.dsh"],
        windows_clean_config_paths: &["~/.dsh"],
        managed_account: false,
    },
    ClientAppSpec {
        kind: "pi",
        label: "Pi",
        account_kind: "pi",
        surface: "cli",
        command: Some("pi"),
        launch_command: Some("pi"),
        launch_requires_cwd: false,
        open_web_url: None,
        desktop_app: None,
        desktop_exe: None,
        process_names: &["pi"],
        mac_desktop_paths: &[],
        mac_bundle_ids: &[],
        download_url: Some("https://github.com/earendil-works/pi"),
        mac_install_command: Some("npm install -g @earendil-works/pi-coding-agent"),
        windows_install_command: Some("npm install -g @earendil-works/pi-coding-agent"),
        mac_uninstall_command: Some("npm uninstall -g @earendil-works/pi-coding-agent"),
        windows_uninstall_command: Some("npm uninstall -g @earendil-works/pi-coding-agent"),
        mac_update_command: Some("npm install -g @earendil-works/pi-coding-agent@latest"),
        windows_update_command: Some("npm install -g @earendil-works/pi-coding-agent@latest"),
        // Pi 配置目录 ~/.pi（agent/ 下含 auth.json / settings.json 等）。
        mac_clean_config_paths: &["~/.pi"],
        windows_clean_config_paths: &["~/.pi"],
        managed_account: false,
    },
    ClientAppSpec {
        kind: "opencode",
        label: "OpenCode",
        account_kind: "opencode",
        surface: "cli",
        command: Some("opencode"),
        launch_command: Some("opencode"),
        launch_requires_cwd: false,
        open_web_url: None,
        desktop_app: None,
        desktop_exe: None,
        process_names: &["opencode"],
        mac_desktop_paths: &[],
        mac_bundle_ids: &[],
        download_url: Some("https://github.com/sst/opencode"),
        mac_install_command: Some("curl -fsSL https://opencode.ai/install | bash"),
        windows_install_command: Some("npm i -g opencode-ai@latest"),
        // macOS 官方安装脚本落地到 ~/.opencode/bin/opencode；目录移除即完成卸载
        // （shell rc 里残留的 PATH 行指向空目录，不影响使用）。
        mac_uninstall_command: Some("rm -rf ~/.opencode"),
        windows_uninstall_command: Some("npm uninstall -g opencode-ai"),
        mac_update_command: Some("curl -fsSL https://opencode.ai/install | bash"),
        windows_update_command: Some("npm i -g opencode-ai@latest"),
        // OpenCode CLI 遵循 XDG：配置 ~/.config/opencode、数据 ~/.local/share/opencode、
        // 状态 ~/.local/state/opencode。注意 ~/.opencode 是安装目录（bin），不属于配置，不清理。
        mac_clean_config_paths: &[
            "~/.config/opencode",
            "~/.local/share/opencode",
            "~/.local/state/opencode",
        ],
        windows_clean_config_paths: &[
            "~/.config/opencode",
            "~/.local/share/opencode",
            "~/.local/state/opencode",
        ],
        managed_account: false,
    },
    ClientAppSpec {
        kind: "opencode_desktop",
        label: "OpenCode 桌面",
        account_kind: "opencode",
        surface: "desktop",
        command: None,
        launch_command: None,
        launch_requires_cwd: false,
        open_web_url: None,
        desktop_app: Some("OpenCode"),
        desktop_exe: Some(&["OpenCode.exe"]),
        process_names: &["OpenCode", "opencode"],
        mac_desktop_paths: &["/Applications/OpenCode.app", "~/Applications/OpenCode.app"],
        // 真机验证：/Applications/OpenCode.app 的 CFBundleIdentifier 为 ai.opencode.desktop。
        mac_bundle_ids: &["ai.opencode.desktop"],
        download_url: Some("https://opencode.ai/download"),
        mac_install_command: None,
        windows_install_command: None,
        mac_uninstall_command: None,
        windows_uninstall_command: None,
        mac_update_command: None,
        windows_update_command: None,
        // OpenCode 桌面 Electron 数据目录（与 bundle id ai.opencode.desktop 对应，本机实测）。
        mac_clean_config_paths: &["~/Library/Application Support/ai.opencode.desktop"],
        windows_clean_config_paths: &["~/AppData/Roaming/OpenCode"],
        managed_account: false,
    },
    ClientAppSpec {
        kind: "workbuddy",
        label: "WorkBuddy",
        account_kind: "workbuddy",
        surface: "desktop",
        command: None,
        launch_command: None,
        launch_requires_cwd: false,
        open_web_url: None,
        desktop_app: Some("WorkBuddy"),
        desktop_exe: Some(&["WorkBuddy.exe"]),
        process_names: &["WorkBuddy", "workbuddy"],
        mac_desktop_paths: &[
            "/Applications/WorkBuddy.app",
            "~/Applications/WorkBuddy.app",
            "/Applications/WorkBuddy AI.app",
            "~/Applications/WorkBuddy AI.app",
        ],
        // 真机验证（Mac mini）：官方安装的应用名带 "AI" 后缀，
        // /Applications/WorkBuddy AI.app 的 CFBundleIdentifier 为 com.workbuddy.workbuddy-ai，
        // 主进程为 Electron，Helper 为 "WorkBuddy AI Helper"。
        // 海外版 com.workbuddy.workbuddy-ai（WorkBuddy AI.app）+ 国内版 com.workbuddy.workbuddy（WorkBuddy.app）。
        mac_bundle_ids: &["com.workbuddy.workbuddy-ai", "com.workbuddy.workbuddy"],
        // 用户点击图标未安装时，打开国内版下载页（workbuddy.cn 是国内版，workbuddy.ai 是海外版）。
        download_url: Some("https://www.workbuddy.cn/"),
        mac_install_command: None,
        windows_install_command: None,
        mac_uninstall_command: None,
        windows_uninstall_command: None,
        mac_update_command: None,
        windows_update_command: None,
        // WorkBuddy 海外/国内两版 Electron 数据目录都清理（本机实测国内版为 com.workbuddy.workbuddy）。
        mac_clean_config_paths: &[
            "~/Library/Application Support/com.workbuddy.workbuddy",
            "~/Library/Application Support/com.workbuddy.workbuddy-ai",
        ],
        windows_clean_config_paths: &["~/AppData/Roaming/WorkBuddy"],
        managed_account: false,
    },
    ClientAppSpec {
        kind: "hermes_desktop",
        label: "Hermes 桌面",
        account_kind: "hermes",
        surface: "desktop",
        command: None,
        launch_command: None,
        launch_requires_cwd: false,
        open_web_url: None,
        desktop_app: Some("Hermes"),
        desktop_exe: Some(&["Hermes.exe"]),
        process_names: &["Hermes", "hermes"],
        mac_desktop_paths: &["/Applications/Hermes.app", "~/Applications/Hermes.app"],
        // 来源：Homebrew cask hermes-desktop 的 bundle id
        // （安装器 com.nousresearch.hermes.setup / 运行时 com.nousresearch.hermes，
        // 官方文档 `tccutil reset All com.nousresearch.hermes` 也指向运行时 id）。
        mac_bundle_ids: &["com.nousresearch.hermes", "com.nousresearch.hermes.setup"],
        // 官方桌面版下载页（hermes-agent.nousresearch.com）。
        download_url: Some("https://hermes-agent.nousresearch.com/"),
        mac_install_command: None,
        windows_install_command: None,
        mac_uninstall_command: None,
        windows_uninstall_command: None,
        mac_update_command: None,
        windows_update_command: None,
        // Hermes 桌面（Homebrew cask hermes-desktop）Electron 数据目录（本机实测）。
        mac_clean_config_paths: &["~/Library/Application Support/Hermes"],
        windows_clean_config_paths: &["~/AppData/Roaming/Hermes"],
        managed_account: false,
    },
];

fn client_app_spec(kind: &str) -> Option<&'static ClientAppSpec> {
    CLIENT_APP_SPECS.iter().find(|spec| spec.kind == kind)
}

fn client_app_spec_or_err(kind: &str) -> Result<&'static ClientAppSpec, String> {
    client_app_spec(kind).ok_or_else(|| format!("未知客户端: {kind}"))
}

fn client_account_kind_from_slug(kind: &str) -> Option<deecodex::accounts::AccountClientKind> {
    match kind {
        "codex" => Some(deecodex::accounts::AccountClientKind::Codex),
        "claude_code" => Some(deecodex::accounts::AccountClientKind::ClaudeCode),
        "hermes" => Some(deecodex::accounts::AccountClientKind::Hermes),
        "generic_client" => Some(deecodex::accounts::AccountClientKind::GenericClient),
        // dsh / Pi / OpenCode / WorkBuddy 等不属于 DEX AI 账号体系，
        // 不能映射到 Codex，否则会误把 Codex 账号当成它们的账号。
        _ => None,
    }
}

fn client_surface_from_slug(surface: &str) -> deecodex::accounts::AccountClientSurface {
    match surface {
        "desktop" => deecodex::accounts::AccountClientSurface::Desktop,
        _ => deecodex::accounts::AccountClientSurface::Cli,
    }
}

fn client_spec_is_cli(spec: &ClientAppSpec) -> bool {
    spec.surface == "cli"
}

fn client_cli_status(spec: &ClientAppSpec) -> Value {
    let Some(command) = spec.command else {
        return json!({"installed": false, "command": null, "version": null, "error": "该客户端不是 CLI"});
    };
    let path = find_command_path(command);
    let command_for_version = path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| command.to_string());
    // 命令存在但无法读取版本（残留/损坏的启动脚本、同名不同工具等）不算已安装，
    // 避免“终端已卸载但界面仍显示已安装”的假阳性（如 hermes 残留 pip 启动脚本）。
    let version = path.as_ref().and_then(|_| {
        get_cli_version_flexible(&command_for_version)
            .or_else(|| command_first_line(&command_for_version, &["--version"]))
    });
    let installed = path.is_some() && version.is_some();
    json!({
        "installed": installed,
        "command": command,
        "path": path.map(|p| p.to_string_lossy().to_string()),
        "version": version,
        "error": if installed { Value::Null } else { json!("未检测到可用命令") },
    })
}

fn expand_home_path(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = deecodex::config::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

fn mac_desktop_app_path(spec: &ClientAppSpec) -> Option<PathBuf> {
    spec.mac_desktop_paths
        .iter()
        .map(|path| expand_home_path(path))
        .find(|path| path.exists())
}

fn windows_command_output(cmd: &str, args: &[&str]) -> Option<String> {
    let mut command = std::process::Command::new(cmd);
    hide_window(&mut command);
    let output = command.args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn windows_desktop_app_candidates(spec: &ClientAppSpec) -> Vec<PathBuf> {
    let Some(exes) = spec.desktop_exe else {
        return Vec::new();
    };
    let app = spec.desktop_app.unwrap_or("").trim();
    let mut out = Vec::new();
    for exe in exes {
        for key in ["LOCALAPPDATA", "PROGRAMFILES", "PROGRAMFILES(X86)"] {
            if let Ok(base) = std::env::var(key) {
                let base = PathBuf::from(base);
                out.push(base.join("Programs").join(app).join(exe));
                out.push(base.join(app).join(exe));
                out.push(base.join("Anthropic").join(app).join(exe));
                out.push(base.join("OpenAI").join(app).join(exe));
            }
        }
    }
    out
}

/// 桌面客户端同家族的 CLI 命令（codex_desktop → codex、claude_desktop → claude、
/// hermes_desktop → hermes、opencode_desktop → opencode）。
fn family_cli_command(spec: &ClientAppSpec) -> Option<&'static str> {
    CLIENT_APP_SPECS
        .iter()
        .find(|candidate| candidate.surface == "cli" && candidate.account_kind == spec.account_kind)
        .and_then(|candidate| candidate.command)
}

fn same_file_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

/// Windows 文件名大小写不敏感：`where Codex.exe` 会命中同家族 CLI `codex.exe`
/// （例如 WinGet Links 里的 codex.exe），导致桌面版被误判为已安装。
/// 命中 CLI 入口的路径一律不视为桌面版。
fn path_is_family_cli(spec: &ClientAppSpec, path: &Path) -> bool {
    let Some(cli) = family_cli_command(spec) else {
        return false;
    };
    find_command_path(cli)
        .map(|cli_path| same_file_path(&cli_path, path))
        .unwrap_or(false)
}

/// 供启动/诊断复用：桌面版客户端在当前平台是否真实安装（与生命周期状态一致）。
pub(crate) fn desktop_client_installed(kind: &str) -> bool {
    let Some(spec) = client_app_spec(kind) else {
        return false;
    };
    #[cfg(target_os = "macos")]
    {
        return mac_desktop_app_path(spec).is_some();
    }
    #[cfg(target_os = "windows")]
    {
        return windows_desktop_app_path(spec).is_some()
            || windows_store_package_family(spec).is_some();
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = spec;
    }
    #[allow(unreachable_code)]
    false
}

fn windows_desktop_app_path(spec: &ClientAppSpec) -> Option<PathBuf> {
    for candidate in windows_desktop_app_candidates(spec) {
        if candidate.exists() && !path_is_family_cli(spec, &candidate) {
            return Some(candidate);
        }
    }
    if let Some(exes) = spec.desktop_exe {
        for exe in exes {
            if let Some(path) = windows_command_output("where", &[exe])
                .and_then(|text| text.lines().next().map(str::trim).map(PathBuf::from))
                .filter(|path| path.exists())
            {
                if !path_is_family_cli(spec, &path) {
                    return Some(path);
                }
            }
            let reg_key =
                format!(r"HKCU\Software\Microsoft\Windows\CurrentVersion\App Paths\{exe}");
            if let Some(text) = windows_command_output("reg", &["query", &reg_key, "/ve"]) {
                for line in text.lines() {
                    if let Some((_, value)) = line.split_once("REG_SZ") {
                        let path = PathBuf::from(value.trim().trim_matches('"'));
                        if path.exists() && !path_is_family_cli(spec, &path) {
                            return Some(path);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Windows MSIX Store 分发的桌面版包名（对应 `%LOCALAPPDATA%\Packages\<name>_*`）。
/// Codex/ChatGPT 桌面走 Microsoft Store 的 `OpenAI.Codex` 包；Claude 桌面为 `Claude` 包。
fn windows_store_package_name(spec: &ClientAppSpec) -> Option<&'static str> {
    match spec.kind {
        "codex_desktop" => Some("OpenAI.Codex"),
        "claude_desktop" => Some("Claude"),
        _ => None,
    }
}

/// 扫描 `%LOCALAPPDATA%\Packages`，返回已安装的 Store 包族名（PFN，如 `OpenAI.Codex_2p2nqsd0c76g0`）。
/// WindowsApps 目录非提权无法枚举，但包的用户数据目录（Packages/<PFN>）可读，
/// 是稳定且非提权的“已安装”标记。
fn windows_store_package_family(spec: &ClientAppSpec) -> Option<String> {
    let package = windows_store_package_name(spec)?;
    let local = std::env::var("LOCALAPPDATA").ok()?;
    let packages_dir = std::path::Path::new(&local).join("Packages");
    let prefix = format!("{package}_");
    std::fs::read_dir(&packages_dir)
        .ok()?
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(&prefix))
        .max()
}

/// 解析 Store 包的 AUMID（`<PFN>!<AppId>`），供 `shell:AppsFolder\<AUMID>` 启动。
fn windows_store_aumid(spec: &ClientAppSpec) -> Option<String> {
    let package = windows_store_package_name(spec)?;
    let script = format!(
        "try {{ $p = Get-AppxPackage -Name '{package}' -ErrorAction Stop; if (-not $p) {{ exit }}; $m = Get-AppxPackageManifest -Package $p; $a = $m.Package.Applications.Application | Select-Object -First 1; [Console]::Out.Write(('' + $p.PackageFamilyName + '!' + $a.Id)) }} catch {{ exit 1 }}"
    );
    let mut command = std::process::Command::new("powershell");
    hide_window(&mut command);
    let output = command
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.contains('!') {
        Some(text)
    } else {
        None
    }
}

/// 供启动复用：桌面版在当前平台的真实 exe 路径（不含 Store 包）。
pub(crate) fn desktop_client_exe_path(kind: &str) -> Option<std::path::PathBuf> {
    let spec = client_app_spec(kind)?;
    #[cfg(target_os = "macos")]
    {
        return mac_desktop_app_path(spec);
    }
    #[cfg(target_os = "windows")]
    {
        return windows_desktop_app_path(spec);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = spec;
    }
    #[allow(unreachable_code)]
    None
}

/// 供启动复用：Windows Store 版桌面应用的 AUMID（`shell:AppsFolder\<AUMID>`）。
pub(crate) fn desktop_client_store_aumid(kind: &str) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        let spec = client_app_spec(kind)?;
        return windows_store_aumid(spec);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = kind;
    }
    #[allow(unreachable_code)]
    None
}

fn client_desktop_status(spec: &ClientAppSpec) -> Value {
    let (installed, path) = if cfg!(target_os = "macos") {
        let path = mac_desktop_app_path(spec);
        (path.is_some(), path)
    } else if cfg!(target_os = "windows") {
        let path = windows_desktop_app_path(spec);
        (path.is_some(), path)
    } else {
        (false, None)
    };
    #[cfg(target_os = "windows")]
    let store_package = windows_store_package_family(spec);
    #[cfg(not(target_os = "windows"))]
    let store_package: Option<String> = None;
    let installed = installed || store_package.is_some();
    json!({
        "installed": installed,
        "app": spec.desktop_app,
        "path": path.map(|p| p.to_string_lossy().to_string()),
        "store_package": store_package,
        "download_url": spec.download_url,
        "error": if installed { Value::Null } else { json!("未检测到桌面版应用") },
    })
}

fn client_process_matches_spec(spec: &ClientAppSpec, command: &str) -> bool {
    // 新增 spec kind 时只需在这里加一个 arm。
    match spec.kind {
        "codex_desktop" => status_command_is_codex_desktop(command),
        "codex_cli" => status_command_is_codex_cli(command),
        "claude_desktop" => status_command_is_claude_desktop(command),
        "claude_cli" => status_command_is_claude_cli(command),
        // Hermes CLI 与桌面（Electron）可执行名大小写不同（`hermes` vs `Hermes`），
        // 但显式剔除桌面进程，避免桌面 Helper 等进程抢占 CLI 状态。
        "hermes" => {
            !status_command_is_hermes_desktop(command) && status_command_is_hermes_cli(command)
        }
        "hermes_desktop" => status_command_is_hermes_desktop(command),
        // dsh/Pi/OpenCode 用短命令名，必须按可执行名精确匹配，避免 pgrep 命中
        // `pip`/`dsh` 子串等无关进程（例如 Pi 的 `pi` 不能把 `pip` 算进来）。
        "dsh" => status_command_uses_executable(command, "dsh"),
        "pi" => status_command_uses_executable(command, "pi"),
        // OpenCode CLI 与桌面（Electron）共用 `opencode` 可执行名（桌面主进程大写 `OpenCode`，
        // 匹配时统一小写），必须先剔除桌面进程再按可执行名判断，避免 CLI 状态被桌面抢占。
        "opencode" => {
            !status_command_is_opencode_desktop(command)
                && status_command_uses_executable(command, "opencode")
        }
        "opencode_desktop" => status_command_is_opencode_desktop(command),
        // WorkBuddy 是 Electron 桌面工作台，主进程可执行名是 `Electron`（无专属 comm），
        // 只能靠应用路径 / Helper 名识别；独立安装的 CodeBuddy CLI（codebuddy）不在其列。
        "workbuddy" => status_command_is_workbuddy_desktop(command),
        // 无强识别规则的 spec 默认通过（不剔除）。
        _ => true,
    }
}

fn client_process_instances_for_spec(spec: &ClientAppSpec) -> Vec<Value> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    // Windows 上 Store 版 Codex/ChatGPT 桌面主进程名是 ChatGPT.exe，需额外扫描；
    // macOS 的 consumer ChatGPT.app 已由 status_command_is_codex_desktop 剔除。
    let extra_scan: &[&str] = if cfg!(target_os = "windows") && spec.kind == "codex_desktop" {
        &["ChatGPT"]
    } else {
        &[]
    };
    for name in spec.process_names.iter().copied().chain(extra_scan.iter().copied()) {
        for instance in detect_process_instances(name) {
            let pid = instance
                .get("pid")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if !pid.is_empty() && !seen.insert(pid) {
                continue;
            }
            let command = instance
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if !client_process_matches_spec(spec, &command) {
                continue;
            }
            out.push(instance);
        }
    }
    out
}

fn status_command_is_codex_desktop(command: &str) -> bool {
    let lower = command.to_ascii_lowercase().replace('\\', "/");
    let trimmed = command.trim();
    // 注意：这里进程名比对保持大小写敏感——CLI 的 `codex`（小写）必须不算桌面，
    // 否则 `status_command_is_codex_cli("codex")` 会被这里抢答。Electron 主进程名是大写的 `Codex`。
    // 旧版 Codex 桌面：`/Applications/Codex.app/` 路径特征，单凭路径即可识别。
    lower.contains("/codex.app/")
        || lower.contains("com.openai.codex")
        || lower.contains("codex helper")
        || lower.contains("application support/codex")
        || lower == "codex.exe"
        || trimmed == "Codex"
        // 新版 Codex 桌面 `ChatGPT.app` 必须配 Codex 独有特征：
        // - `Contents/Resources/codex`（内置 CLI 二进制，仅 Codex 桌面有）
        // - `Codex Helper.app`（Codex 桌面的 helper bundle 名，consumer ChatGPT 是 `ChatGPT Helper.app`）
        // 这样可以剔除 consumer 版 `ChatGPT.app`（`com.openai.chatgpt`，helper 是 `ChatGPT Helper`）。
        || (lower.contains("/chatgpt.app/") && lower.contains("/codex"))
        || (lower.contains("/chatgpt.app/") && lower.contains("codex helper"))
        // Windows：仅当可执行名匹配 Codex 桌面二进制（Codex.exe），剔除普通 ChatGPT.exe。
        || lower.contains("windowsapps")
            && (lower.contains("codex.exe") || lower.contains("openai.codex"))
        || lower.contains("/programs/codex/")
            && (lower.contains("codex.exe") || lower.contains("codex helper"))
        || lower.contains("/programs/openai/codex/")
            && (lower.contains("codex.exe") || lower.contains("codex helper"))
}

fn status_command_is_claude_desktop(command: &str) -> bool {
    command.contains("/Claude.app/")
        || command.contains("Claude Helper")
        || command.contains("Application Support/Claude")
        || command.trim() == "Claude"
}

fn status_command_is_opencode_desktop(command: &str) -> bool {
    let lower = command.to_ascii_lowercase().replace('\\', "/");
    let trimmed = command.trim();
    // macOS：主进程 `/Applications/OpenCode.app/Contents/MacOS/OpenCode`（可执行名大写 `OpenCode`），
    // Helper 进程名 `OpenCode Helper` / `OpenCode Helper (Renderer)`。
    // 保留大小写敏感比对：CLI 的 `opencode`（小写）不能算桌面。
    lower.contains("/opencode.app/")
        || lower.contains("opencode helper")
        || lower == "opencode.exe"
        || trimmed == "OpenCode"
        // Windows：Electron 桌面安装目录下的 OpenCode.exe / Helper。
        || (lower.contains("/programs/opencode/")
            && (lower.contains("opencode.exe") || lower.contains("opencode helper")))
}

fn status_command_is_workbuddy_desktop(command: &str) -> bool {
    let lower = command.to_ascii_lowercase().replace('\\', "/");
    let trimmed = command.trim();
    // WorkBuddy 主进程是 `/Applications/WorkBuddy AI.app/Contents/MacOS/Electron`（comm 为 `Electron`），
    // Helper 进程名 `WorkBuddy AI Helper` / `WorkBuddy AI Helper (Renderer)`；
    // 旧版安装名 `WorkBuddy.app` / `WorkBuddy Helper` 也保留兼容。
    // 统一用应用路径 + Helper 名识别，避免把独立 CodeBuddy CLI（codebuddy）算成桌面。
    lower.contains("/workbuddy.app/")
        || lower.contains("/workbuddy ai.app/")
        || lower.contains("workbuddy helper")
        || lower.contains("workbuddy ai helper")
        || lower == "workbuddy.exe"
        || trimmed == "WorkBuddy"
        || (lower.contains("/programs/workbuddy/")
            && (lower.contains("workbuddy.exe")
                || lower.contains("workbuddy helper")
                || lower.contains("workbuddy ai helper")))
}

fn status_command_is_hermes_desktop(command: &str) -> bool {
    let lower = command.to_ascii_lowercase().replace('\\', "/");
    let trimmed = command.trim();
    // Hermes 桌面是 Electron 应用：主进程 `/Applications/Hermes.app/Contents/MacOS/Hermes`，
    // Helper 进程名 `Hermes Helper` / `Hermes Helper (Renderer)`。
    // 保留大小写敏感比对：CLI 的 `hermes`（小写 Python 脚本）不能算桌面。
    lower.contains("/hermes.app/")
        || lower.contains("hermes helper")
        || lower == "hermes.exe"
        || trimmed == "Hermes"
        // Windows：Electron 桌面安装目录下的 Hermes.exe / Helper。
        || (lower.contains("/programs/hermes/")
            && (lower.contains("hermes.exe") || lower.contains("hermes helper")))
}

fn status_command_first_token(command: &str) -> Option<String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix('"') {
        let end = rest.find('"')?;
        return Some(rest[..end].to_string());
    }
    trimmed.split_whitespace().next().map(str::to_string)
}

fn status_command_executable_name(command: &str) -> Option<String> {
    let first = status_command_first_token(command)?;
    first
        .rsplit(['/', '\\'])
        .next()
        .map(|name| name.trim_matches('"').to_ascii_lowercase())
}

fn status_command_has_args(command: &str) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return false;
    }
    if let Some(rest) = trimmed.strip_prefix('"') {
        let Some(end) = rest.find('"') else {
            return false;
        };
        return !rest[end + 1..].trim().is_empty();
    }
    trimmed.split_whitespace().nth(1).is_some()
}

fn status_command_uses_executable(command: &str, name: &str) -> bool {
    let Some(exe) = status_command_executable_name(command) else {
        return false;
    };
    let exe_stem = exe
        .strip_suffix(".exe")
        .or_else(|| exe.strip_suffix(".cmd"))
        .or_else(|| exe.strip_suffix(".bat"))
        .unwrap_or(&exe);
    let first = status_command_first_token(command).unwrap_or_default();
    let first_lower = first.to_ascii_lowercase();
    if exe_stem == name && first_lower == name {
        return true;
    }
    if exe_stem == name
        && (first.contains('/') || first.contains('\\') || status_command_has_args(command))
    {
        return true;
    }
    if (exe == "node" || exe == "nodejs")
        && command.split_whitespace().any(|part| {
            part.trim_matches('"')
                .rsplit(['/', '\\'])
                .next()
                .map(|value| value.eq_ignore_ascii_case(name))
                .unwrap_or(false)
        })
    {
        return true;
    }
    false
}

fn status_command_is_codex_cli(command: &str) -> bool {
    !status_command_is_codex_desktop(command)
        && !command.to_ascii_lowercase().contains("deecodex")
        && status_command_uses_executable(command, "codex")
}

fn status_command_is_claude_cli(command: &str) -> bool {
    !status_command_is_claude_desktop(command) && status_command_uses_executable(command, "claude")
}

fn status_command_is_hermes_cli(command: &str) -> bool {
    let text = command.trim();
    if text.is_empty() {
        return false;
    }
    // Hermes CLI 的常驻形态可能是 launchd 网关服务（`python -m hermes_cli.main gateway run`，
    // KeepAlive 自动拉起）。网关属于 Hermes CLI 基础设施，计入运行态；否则「强制关闭」
    // 找不到进程、卸载后又被 launchd 拉活，表现为“又显示运行 / 强制关闭失败”。
    if text.contains("hermes_cli.main") {
        return true;
    }
    status_command_uses_executable(text, "hermes")
        || text
            .split_whitespace()
            .any(|part| Path::new(part).file_name().and_then(|v| v.to_str()) == Some("hermes"))
}

fn install_command_for_current_os(spec: &ClientAppSpec) -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        spec.mac_install_command
    } else if cfg!(target_os = "windows") {
        spec.windows_install_command
    } else {
        None
    }
}

fn uninstall_command_for_current_os(spec: &ClientAppSpec) -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        spec.mac_uninstall_command
    } else if cfg!(target_os = "windows") {
        spec.windows_uninstall_command
    } else {
        None
    }
}

fn update_command_for_current_os(spec: &ClientAppSpec) -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        spec.mac_update_command
    } else if cfg!(target_os = "windows") {
        spec.windows_update_command
    } else {
        None
    }
}

fn install_command_for_status(spec: &ClientAppSpec) -> Option<&'static str> {
    install_command_for_current_os(spec)
        .or(spec.mac_install_command)
        .or(spec.windows_install_command)
}

fn client_primary_account(
    store: &deecodex::accounts::AccountStore,
    spec: &ClientAppSpec,
) -> Option<deecodex::accounts::Account> {
    // 非受管客户端也可能复用同家族账号（如 hermes_desktop 复用 hermes CLI 的配置），
    // 这里仍按 account_kind 查找；没有对应账号体系的客户端返回 None。
    let kind = client_account_kind_from_slug(spec.account_kind)?;
    let surface = client_surface_from_slug(spec.surface);
    let mut matches: Vec<_> = store
        .accounts
        .iter()
        .filter(|account| account.client_kind == kind)
        .filter(|account| {
            if !kind.supports_desktop_surface() {
                return true;
            }
            account.client_surface == surface
        })
        .cloned()
        .collect();
    if matches.is_empty() {
        return None;
    }
    if kind.is_codex() {
        let active_id = store
            .active_account_id
            .as_ref()
            .or(store.active_id.as_ref());
        if let Some(account) = active_id
            .and_then(|id| matches.iter().find(|account| &account.id == id))
            .cloned()
        {
            return Some(account);
        }
        return matches.into_iter().next();
    }
    matches.sort_by_key(|account| account.last_applied_at.unwrap_or(0));
    matches.pop()
}

fn lifecycle_next_action(
    installed: bool,
    account_exists: bool,
    process_running: bool,
    managed_account: bool,
) -> &'static str {
    if !installed {
        "install"
    } else if managed_account && !account_exists {
        "configure"
    } else if process_running {
        "running"
    } else {
        "launch"
    }
}

/// 原生配置客户端（pi/dsh/opencode/workbuddy）已安装但尚未写入原生配置时，
/// 快捷动作应为「配置」而不是「启动」，与受管客户端「未配置 → configure」保持一致。
fn lifecycle_next_action_with_native(
    installed: bool,
    native_detected: bool,
    native_configured: bool,
    account_exists: bool,
    process_running: bool,
    managed_account: bool,
) -> &'static str {
    if native_detected && installed && !native_configured {
        return "configure";
    }
    lifecycle_next_action(installed, account_exists, process_running, managed_account)
}

/// 非受管客户端（pi/dsh）没有 DEX 账号，状态以客户端自身配置为准：
/// 只读取原生配置文件判断是否已配置，不写、不创建 DEX 账号。
pub(crate) fn native_client_config_detected(kind: &str) -> bool {
    // opencode 桌面版与 CLI 共用 ~/.config/opencode；workbuddy 写 ~/.workbuddy/models.json。
    matches!(
        kind,
        "pi" | "dsh" | "opencode" | "opencode_desktop" | "workbuddy"
    )
}

fn read_json_file(path: &Path) -> Option<Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
}

/// 去除 JSONC 注释（`//` 行注释与 `/* */` 块注释），字符串字面量内容原样保留。
fn strip_jsonc_comments(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            // 字符串字面量：保留到闭合引号（含转义），注释标记在字符串内不生效。
            out.push(c);
            i += 1;
            while i < chars.len() {
                let sc = chars[i];
                out.push(sc);
                i += 1;
                if sc == '\\' && i < chars.len() {
                    out.push(chars[i]);
                    i += 1;
                } else if sc == '"' {
                    break;
                }
            }
            continue;
        }
        if c == '/' && i + 1 < chars.len() {
            let next = chars[i + 1];
            if next == '/' {
                // 行注释：跳过到行尾
                i += 2;
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            if next == '*' {
                // 块注释：跳过到 `*/`
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i = (i + 2).min(chars.len());
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// 读取 opencode 配置：`.jsonc` 先剥离注释再按 JSON 解析，`.json` 直接解析。
fn read_opencode_config(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    let stripped = strip_jsonc_comments(&text);
    serde_json::from_str(&stripped).ok()
}

/// 写前备份已存在的配置文件（防止重写丢失用户配置），备份失败只告警不阻断。
fn backup_existing_file(path: &Path) {
    if !path.exists() {
        return;
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let Some(file_name) = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
    else {
        return;
    };
    let backup = path.with_file_name(format!("{file_name}.bak-{stamp}"));
    if let Err(error) = std::fs::copy(path, &backup) {
        tracing::warn!("备份 {} 失败: {error}", path.display());
    }
}

fn write_file_private(path: &Path, content: &str) -> Result<(), String> {
    use std::io::Write;
    let mut file =
        std::fs::File::create(path).map_err(|e| format!("创建 {} 失败: {e}", path.display()))?;
    file.write_all(content.as_bytes())
        .map_err(|e| format!("写入 {} 失败: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// GUI 以 root 运行时，把写出的客户端配置文件属主改为主目录属主，
/// 否则用户身份的客户端进程（dsh/pi）读不到 / 监视不到 root 属主的文件。
fn chown_to_home_owner(path: &Path, home: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::chown;
        use std::os::unix::fs::MetadataExt;
        if let Ok(meta) = std::fs::metadata(home) {
            let _ = chown(path, Some(meta.uid()), Some(meta.gid()));
        }
    }
}

/// pi/dsh 等客户端默认模型名可能带上下文长度后缀（如 `MiniMax-M3[1m]`），
/// 但客户端自身配置通常只认纯模型名，写入前去掉 `[...]` 后缀。
fn strip_model_suffix(model: &str) -> String {
    model.split('[').next().unwrap_or(model).trim().to_string()
}

/// dsh（dsh-llm-pi-ai）与 pi 都支持 3 种端点格式，按供应商/上游推导 wire API：
/// - upstream 含 `/anthropic` → Anthropic Messages
/// - openai / mimo（Responses 直连） → OpenAI Responses
/// - 其余 OpenAI 兼容 → OpenAI Chat Completions
fn dsh_pi_ai_api(provider: &str, upstream: &str) -> &'static str {
    if upstream.contains("/anthropic") {
        "anthropic-messages"
    } else if matches!(provider, "openai" | "mimo") {
        "openai-responses"
    } else {
        "openai-completions"
    }
}

/// 原生配置客户端（pi/dsh）的快速配置：直接写客户端自身配置文件，不创建 DEX 账号。
pub(crate) fn write_native_client_config(
    kind: &str,
    account: &deecodex::accounts::Account,
) -> Result<Value, String> {
    let Some(home) = deecodex::config::home_dir() else {
        return Err("无法定位用户主目录".into());
    };
    match kind {
        "pi" => write_pi_native_config(&home, account),
        "dsh" => write_dsh_native_config(&home, account),
        "opencode" | "opencode_desktop" => write_opencode_native_config(&home, account),
        "workbuddy" => write_workbuddy_native_config(&home, account),
        _ => Err(format!("暂不支持 {kind} 的原生配置写入")),
    }
}

/// opencode：写入 `~/.config/opencode/opencode.json(.c)` 的 provider 路由
/// （@ai-sdk/openai-compatible + baseURL + apiKey + models），与 pi/dsh 走同一套快速配置。
fn write_opencode_native_config(
    home: &Path,
    account: &deecodex::accounts::Account,
) -> Result<Value, String> {
    let config_dir = opencode_config_dir_preferred(home);
    std::fs::create_dir_all(&config_dir)
        .map_err(|e| format!("创建 {} 失败: {e}", config_dir.display()))?;
    let jsonc_path = config_dir.join("opencode.jsonc");
    let json_path = config_dir.join("opencode.json");
    let (path, is_jsonc) = if jsonc_path.is_file() {
        (jsonc_path, true)
    } else {
        (json_path, false)
    };

    // 已存在但解析失败：报错并中止，绝不静默降级成 {} 覆盖用户配置。
    let config_exists = path.exists();
    let mut config = if config_exists {
        read_opencode_config(&path).ok_or_else(|| {
            format!(
                "无法解析 {}（JSON/JSONC），已停止写入以避免覆盖，请修复配置格式后重试",
                path.display()
            )
        })?
    } else {
        json!({})
    };
    if !config.is_object() {
        return Err("opencode 配置文件不是 JSON 对象".into());
    }
    if config.get("provider").is_none() {
        config["provider"] = json!({});
    }
    let providers = config
        .get_mut("provider")
        .and_then(|value| value.as_object_mut())
        .ok_or_else(|| "opencode 配置缺少 provider 字段".to_string())?;
    let provider_id = account.provider.as_str();
    providers.insert(
        provider_id.to_string(),
        json!({
            "name": account.provider.to_uppercase(),
            "npm": "@ai-sdk/openai-compatible",
            "options": {
                "baseURL": account.upstream,
                "apiKey": account.api_key,
            },
            "models": {
                strip_model_suffix(&account.default_model): {
                    "name": strip_model_suffix(&account.default_model),
                }
            },
        }),
    );

    let text = if is_jsonc {
        // jsonc 允许注释，直接以 JSON 序列化（opencode 也能读）
        serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?
    } else {
        serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?
    };
    backup_existing_file(&path);
    write_file_private(&path, &text)?;
    chown_to_home_owner(&path, home);

    Ok(json!({
        "config_path": path.to_string_lossy(),
        "provider": provider_id,
    }))
}

/// opencode 配置目录（写入优先）：Unix 用 `~/.config/opencode`，
/// Windows 用 `%APPDATA%\opencode`（opencode 在 Windows 读平台配置目录）。
#[cfg(not(target_os = "windows"))]
fn opencode_config_dir_preferred(home: &Path) -> PathBuf {
    home.join(".config").join("opencode")
}

#[cfg(target_os = "windows")]
fn opencode_config_dir_preferred(home: &Path) -> PathBuf {
    std::env::var("APPDATA")
        .map(PathBuf::from)
        .map(|dir| dir.join("opencode"))
        .unwrap_or_else(|_| home.join(".config").join("opencode"))
}

/// opencode 配置目录候选（检测用，全部检查）：Unix `~/.config/opencode` + Windows `%APPDATA%`。
fn opencode_config_candidate_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![home.join(".config").join("opencode")];
    #[cfg(target_os = "windows")]
    if let Ok(appdata) = std::env::var("APPDATA") {
        dirs.push(PathBuf::from(&appdata).join("opencode"));
    }
    dirs
}

/// opencode 数据目录候选（auth.json 所在）：Unix `~/.local/share/opencode` +
/// Windows `%APPDATA%` / `%LOCALAPPDATA%`。
fn opencode_share_candidate_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![home.join(".local").join("share").join("opencode")];
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            dirs.push(PathBuf::from(&appdata).join("opencode"));
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            dirs.push(PathBuf::from(&local).join("opencode"));
        }
    }
    dirs
}

/// WorkBuddy：`~/.workbuddy/models.json` 是自定义模型 JSON 数组，
/// 每项含 id/name/vendor/url/apiKey/supportsToolCall/supportsImages/supportsReasoning。
fn write_workbuddy_native_config(
    home: &Path,
    account: &deecodex::accounts::Account,
) -> Result<Value, String> {
    let config_dir = home.join(".workbuddy");
    std::fs::create_dir_all(&config_dir).map_err(|e| format!("创建 ~/.workbuddy 失败: {e}"))?;
    let models_path = config_dir.join("models.json");
    let mut models: Vec<Value> = read_json_file(&models_path)
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();

    let model = strip_model_suffix(&account.default_model);
    let upstream = account.upstream.trim_end_matches('/');
    let url = if upstream.ends_with("/chat/completions") {
        upstream.to_string()
    } else {
        format!("{upstream}/chat/completions")
    };
    let entry = json!({
        "id": model,
        "name": model,
        "vendor": workbuddy_vendor_label(&account.provider),
        "url": url,
        "apiKey": account.api_key,
        "supportsToolCall": true,
        "supportsImages": workbuddy_provider_supports_images(&account.provider),
        "supportsReasoning": false,
    });
    if let Some(existing) = models
        .iter_mut()
        .find(|item| item.get("id") == entry.get("id"))
    {
        *existing = entry;
    } else {
        models.push(entry);
    }

    let text = serde_json::to_string_pretty(&models).map_err(|e| e.to_string())?;
    write_file_private(&models_path, &text)?;
    chown_to_home_owner(&models_path, home);

    Ok(json!({
        "config_path": models_path.to_string_lossy(),
        "model": model,
        "provider": account.provider,
    }))
}

fn workbuddy_vendor_label(provider: &str) -> String {
    match provider {
        "minimax" => "MiniMax".into(),
        "deepseek" => "DeepSeek".into(),
        "kimi" => "Kimi".into(),
        "qwen" => "Qwen".into(),
        "glm" => "GLM".into(),
        "mimo" => "MiMo".into(),
        "openai" => "OpenAI".into(),
        "opencode" => "OpenCode".into(),
        "longcat" => "LongCat".into(),
        other => other.to_string(),
    }
}

fn workbuddy_provider_supports_images(provider: &str) -> bool {
    matches!(
        provider,
        "minimax" | "openai" | "qwen" | "glm" | "kimi" | "mimo"
    )
}

/// WorkBuddy：`~/.workbuddy/models.json` 存在且任一模型有 apiKey → 已配置。
fn workbuddy_native_config_configured(home: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(home.join(".workbuddy").join("models.json")) else {
        return false;
    };
    let Ok(models) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    models
        .as_array()
        .map(|list| {
            list.iter().any(|item| {
                item.get("apiKey")
                    .and_then(serde_json::Value::as_str)
                    .map(|key| !key.trim().is_empty())
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// pi：`~/.pi/agent/auth.json`（provider → api_key）+ `~/.pi/agent/settings.json`（defaultProvider/defaultModel）。
/// DEX 的 minimax 对应 pi 的 minimax-cn（国内端点），其余供应商沿用 pi 自身 provider id。
fn write_pi_native_config(
    home: &Path,
    account: &deecodex::accounts::Account,
) -> Result<Value, String> {
    let agent = home.join(".pi").join("agent");
    std::fs::create_dir_all(&agent).map_err(|e| format!("创建 ~/.pi/agent 失败: {e}"))?;
    let provider = if account.provider == "minimax" {
        "minimax-cn"
    } else {
        account.provider.as_str()
    };

    let auth_path = agent.join("auth.json");
    let mut auth = read_json_file(&auth_path).unwrap_or_else(|| json!({}));
    auth[provider] = json!({ "type": "api_key", "key": account.api_key });
    let auth_text = serde_json::to_string_pretty(&auth).map_err(|e| e.to_string())?;
    write_file_private(&auth_path, &auth_text)?;
    chown_to_home_owner(&auth_path, home);

    let settings_path = agent.join("settings.json");
    let mut settings = read_json_file(&settings_path).unwrap_or_else(|| json!({}));
    settings["defaultProvider"] = json!(provider);
    settings["defaultModel"] = json!(strip_model_suffix(&account.default_model));
    let settings_text = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    write_file_private(&settings_path, &settings_text)?;
    chown_to_home_owner(&settings_path, home);

    Ok(json!({
        "config_path": auth_path.to_string_lossy(),
        "provider": provider,
    }))
}

/// dsh：`~/.dsh/settings.yaml`（agent-default-model.provider/model）+ `~/.dsh/.credentials.yaml`（DEEPSEEK_API_KEY）。
/// dsh 的 LLM 适配器只有 DeepSeek / Pi-AI，暂只支持 deepseek。
fn write_dsh_native_config(
    home: &Path,
    account: &deecodex::accounts::Account,
) -> Result<Value, String> {
    // dsh 走 pi-ai 适配器（dsh-llm-pi-ai），支持全部 OpenAI 兼容模型格式。
    // 内置路由：deepseek → deepseek-official（dsh 自身适配器）；minimax → minimax-cn（pi-ai 目录路由）。
    // 其它供应商：以供应商 slug 为路由，完整声明 api/baseURL/models。
    let provider = account.provider.as_str();
    let api = dsh_pi_ai_api(provider, &account.upstream);
    let (route, cred_env, catalog) = match provider {
        "deepseek" => (
            "deepseek-official".to_string(),
            "DEEPSEEK_API_KEY".to_string(),
            false,
        ),
        "minimax" => (
            "minimax-cn".to_string(),
            "MINIMAX_CN_API_KEY".to_string(),
            true,
        ),
        other => (
            other.to_string(),
            format!("{}_API_KEY", other.to_uppercase()),
            false,
        ),
    };
    let dsh_dir = home.join(".dsh");
    std::fs::create_dir_all(&dsh_dir).map_err(|e| format!("创建 ~/.dsh 失败: {e}"))?;

    let settings_path = dsh_dir.join("settings.yaml");
    let text = std::fs::read_to_string(&settings_path).unwrap_or_default();
    let mut yaml: serde_yaml::Mapping = if text.trim().is_empty() {
        serde_yaml::Mapping::new()
    } else {
        serde_yaml::from_str(&text).unwrap_or_else(|_| serde_yaml::Mapping::new())
    };
    // 除 deepseek 外都走 llm-pi-ai 适配器：声明 provider 路由
    // （目录路由只引用凭据；非目录路由补全 api/baseURL/models）。
    if provider != "deepseek" {
        let mut pi_ai: serde_yaml::Mapping = yaml
            .get(serde_yaml::Value::String("llm-pi-ai".into()))
            .and_then(|value| value.as_mapping().cloned())
            .unwrap_or_default();
        let mut providers: serde_yaml::Mapping = pi_ai
            .get(serde_yaml::Value::String("providers".into()))
            .and_then(|value| value.as_mapping().cloned())
            .unwrap_or_default();
        let mut route_cfg: serde_yaml::Mapping = providers
            .get(serde_yaml::Value::String(route.clone()))
            .and_then(|value| value.as_mapping().cloned())
            .unwrap_or_default();
        route_cfg.insert(
            serde_yaml::Value::String("apiKeyEnv".into()),
            serde_yaml::Value::String(cred_env.clone()),
        );
        route_cfg.insert(
            serde_yaml::Value::String("api".into()),
            serde_yaml::Value::String(api.into()),
        );
        if !catalog && !account.upstream.trim().is_empty() {
            route_cfg.insert(
                serde_yaml::Value::String("baseURL".into()),
                serde_yaml::Value::String(account.upstream.clone()),
            );
        }
        if !catalog && !account.default_model.trim().is_empty() {
            route_cfg.insert(
                serde_yaml::Value::String("models".into()),
                serde_yaml::Value::Sequence(vec![serde_yaml::Mapping::from_iter([(
                    serde_yaml::Value::String("id".into()),
                    serde_yaml::Value::String(strip_model_suffix(&account.default_model)),
                )])
                .into()]),
            );
        }
        providers.insert(
            serde_yaml::Value::String(route.clone()),
            serde_yaml::Value::Mapping(route_cfg),
        );
        pi_ai.insert(
            serde_yaml::Value::String("providers".into()),
            serde_yaml::Value::Mapping(providers),
        );
        yaml.insert(
            serde_yaml::Value::String("llm-pi-ai".into()),
            serde_yaml::Value::Mapping(pi_ai),
        );
    }
    let mut model: serde_yaml::Mapping = yaml
        .get(serde_yaml::Value::String("agent-default-model".into()))
        .and_then(|value| value.as_mapping().cloned())
        .unwrap_or_default();
    model.insert(
        serde_yaml::Value::String("provider".into()),
        serde_yaml::Value::String(route.clone()),
    );
    model.insert(
        serde_yaml::Value::String("model".into()),
        serde_yaml::Value::String(strip_model_suffix(&account.default_model)),
    );
    yaml.insert(
        serde_yaml::Value::String("agent-default-model".into()),
        serde_yaml::Value::Mapping(model),
    );
    let yaml_text = serde_yaml::to_string(&yaml).map_err(|e| e.to_string())?;
    write_file_private(&settings_path, &yaml_text)?;
    chown_to_home_owner(&settings_path, home);

    let creds_path = dsh_dir.join(".credentials.yaml");
    let ctext = std::fs::read_to_string(&creds_path).unwrap_or_default();
    let mut creds: serde_yaml::Mapping = if ctext.trim().is_empty() {
        serde_yaml::Mapping::new()
    } else {
        serde_yaml::from_str(&ctext).unwrap_or_else(|_| serde_yaml::Mapping::new())
    };
    creds.insert(
        serde_yaml::Value::String(cred_env.clone()),
        serde_yaml::Value::String(account.api_key.clone()),
    );
    let creds_text = serde_yaml::to_string(&creds).map_err(|e| e.to_string())?;
    write_file_private(&creds_path, &creds_text)?;
    chown_to_home_owner(&creds_path, home);

    Ok(json!({
        "config_path": settings_path.to_string_lossy(),
        "provider": route,
        "credential_env": cred_env,
    }))
}

fn native_client_config_configured(kind: &str) -> bool {
    let Some(home) = deecodex::config::home_dir() else {
        return false;
    };
    match kind {
        "pi" => pi_native_config_configured(&home),
        "dsh" => dsh_native_config_configured(&home),
        "opencode" => opencode_native_config_configured(&home),
        "opencode_desktop" => opencode_native_config_configured(&home),
        "workbuddy" => workbuddy_native_config_configured(&home),
        _ => false,
    }
}

/// opencode：任一来源有凭据/端点即视为已配置：
/// 1) `~/.local/share/opencode/auth.json` 任一 provider 有 key；
/// 2) `~/.config/opencode/opencode.json(.c)` 有 provider 且 options.apiKey/baseURL 非空（快速配置写入位置）。
fn opencode_native_config_configured(home: &std::path::Path) -> bool {
    // 1) auth.json 有 provider key
    for share_dir in opencode_share_candidate_dirs(home) {
        if let Ok(text) = std::fs::read_to_string(share_dir.join("auth.json")) {
            if let Ok(auth) = serde_json::from_str::<serde_json::Value>(&text) {
                if auth
                    .as_object()
                    .map(|map| {
                        map.values().any(|value| {
                            value
                                .get("key")
                                .and_then(serde_json::Value::as_str)
                                .map(|key| !key.trim().is_empty())
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false)
                {
                    return true;
                }
            }
        }
    }
    // 2) opencode.json(.c) 有 provider 且 options.apiKey/baseURL 非空
    for config_dir in opencode_config_candidate_dirs(home) {
        for name in ["opencode.json", "opencode.jsonc"] {
            let Some(config) = read_opencode_config(&config_dir.join(name)) else {
                continue;
            };
            let Some(providers) = config
                .get("provider")
                .and_then(serde_json::Value::as_object)
            else {
                continue;
            };
            if providers.values().any(|provider| {
                provider
                    .get("options")
                    .and_then(serde_json::Value::as_object)
                    .map(|options| {
                        let has_key = options
                            .get("apiKey")
                            .and_then(serde_json::Value::as_str)
                            .map(|value| !value.trim().is_empty())
                            .unwrap_or(false);
                        let has_url = options
                            .get("baseURL")
                            .and_then(serde_json::Value::as_str)
                            .map(|value| !value.trim().is_empty())
                            .unwrap_or(false);
                        has_key || has_url
                    })
                    .unwrap_or(false)
            }) {
                return true;
            }
        }
    }
    false
}

/// pi：`~/.pi/agent/auth.json` 任一 provider 有 API Key，且
/// `~/.pi/agent/settings.json` 配了 defaultProvider / defaultModel 才视为已配置。
fn pi_native_config_configured(home: &std::path::Path) -> bool {
    let agent = home.join(".pi").join("agent");
    let Ok(auth_text) = std::fs::read_to_string(agent.join("auth.json")) else {
        return false;
    };
    let Ok(auth) = serde_json::from_str::<serde_json::Value>(&auth_text) else {
        return false;
    };
    let has_key = auth
        .as_object()
        .map(|map| {
            map.values().any(|value| {
                value.get("type").and_then(serde_json::Value::as_str) == Some("api_key")
                    && value
                        .get("key")
                        .and_then(serde_json::Value::as_str)
                        .map(|key| !key.trim().is_empty())
                        .unwrap_or(false)
            })
        })
        .unwrap_or(false);
    if !has_key {
        return false;
    }
    let Ok(settings_text) = std::fs::read_to_string(agent.join("settings.json")) else {
        return false;
    };
    let Ok(settings) = serde_json::from_str::<serde_json::Value>(&settings_text) else {
        return false;
    };
    let has_provider = settings
        .get("defaultProvider")
        .and_then(serde_json::Value::as_str)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    let has_model = settings
        .get("defaultModel")
        .and_then(serde_json::Value::as_str)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    has_provider && has_model
}

/// dsh：`~/.dsh/settings.yaml` 配了 `agent-default-model.provider` / `.model`，
/// 且 `~/.dsh/.credentials.yaml` 存在（保存了密钥）才视为已配置。
fn dsh_native_config_configured(home: &std::path::Path) -> bool {
    let dsh_dir = home.join(".dsh");
    if !dsh_dir.join(".credentials.yaml").is_file() {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(dsh_dir.join("settings.yaml")) else {
        return false;
    };
    let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(&text) else {
        return false;
    };
    let model_default = yaml.get("agent-default-model");
    let has_provider = model_default
        .and_then(|value| value.get("provider"))
        .and_then(serde_yaml::Value::as_str)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    let has_model = model_default
        .and_then(|value| value.get("model"))
        .and_then(serde_yaml::Value::as_str)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    has_provider && has_model
}

/// 状态页客户端 Dock：读取单个客户端的一键接入生命周期状态。
pub async fn dex_client_lifecycle_status_impl(
    manager: State<'_, ServerManager>,
    kind: String,
) -> Result<Value, String> {
    let spec = client_app_spec_or_err(&kind)?;
    let install = if client_spec_is_cli(spec) {
        client_cli_status(spec)
    } else {
        client_desktop_status(spec)
    };
    let installed = install
        .get("installed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let process_instances = client_process_instances_for_spec(spec);
    let process_running = !process_instances.is_empty();

    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);
    let account = client_primary_account(&store, spec);
    let native_detected = native_client_config_detected(spec.kind);
    let native_configured = native_detected && native_client_config_configured(spec.kind);
    let status_report = account
        .as_ref()
        .map(deecodex::client_integrations::status)
        .and_then(|report| serde_json::to_value(report).ok());
    // 非受管客户端（如 hermes_desktop）复用同家族账号时，只要账号存在且已写入就算已配置；
    // 完全无账号体系的客户端（dsh/Pi/OpenCode/WorkBuddy）标记为“无需账号”。
    let account_exists = account.is_some();
    let requires_account = spec.managed_account || account_exists;
    let account_configured = if requires_account {
        account.as_ref().is_some_and(|account| {
            account.client_kind.is_codex() || account.last_applied_at.is_some()
        })
    } else {
        false
    };
    let config_ok = if spec.managed_account {
        status_report
            .as_ref()
            .and_then(|report| report.get("ok"))
            .and_then(Value::as_bool)
            .unwrap_or(account_exists)
    } else {
        true
    };
    let next_action = lifecycle_next_action_with_native(
        installed,
        native_detected,
        native_configured,
        account_exists,
        process_running,
        spec.managed_account,
    );

    Ok(json!({
        "kind": spec.kind,
        "label": spec.label,
        "account_kind": spec.account_kind,
        "surface": spec.surface,
        "installed": installed,
        "install": {
            "command": install_command_for_status(spec),
            "download_url": spec.download_url,
            "mode": if client_spec_is_cli(spec) { "terminal" } else { "download_page" },
        },
        "runtime": {
            "running": process_running,
            "instances": process_instances,
        },
        "launch": {
            "mode": if client_spec_is_cli(spec) { "terminal" } else { "desktop" },
            "requires_cwd": spec.launch_requires_cwd,
        },
        "account": account.as_ref().map(|account| json!({
            "id": account.id,
            "name": account.name,
            "provider": account.provider,
            "client_kind": account.client_kind,
            "client_surface": account.client_surface,
            "last_applied_at": account.last_applied_at,
        })),
        "account_exists": account_exists,
        "account_configured": account_configured,
        "requires_account": requires_account,
        "native_detected": native_detected,
        "native_configured": native_configured,
        "config_ok": config_ok,
        "status_report": status_report,
        "next_action": next_action,
        "cli": client_spec_is_cli(spec),
        "command": spec.command,
        "launch_command": spec.launch_command,
        "open_web_url": spec.open_web_url,
        "desktop_app": spec.desktop_app,
        "installed_detail": install,
    }))
}

/// 状态页客户端 Dock：安装或打开官方下载页。
pub fn dex_install_client_impl(kind: String) -> Result<Value, String> {
    let spec = client_app_spec_or_err(&kind)?;
    if client_spec_is_cli(spec) {
        let command = install_command_for_current_os(spec)
            .ok_or_else(|| format!("当前平台暂不支持自动安装 {}", spec.label))?;
        spawn_terminal_command(command, None)?;
        return Ok(json!({
            "ok": true,
            "kind": spec.kind,
            "label": spec.label,
            "mode": "terminal",
            "command": command,
        }));
    }

    let url = spec
        .download_url
        .ok_or_else(|| format!("{} 没有配置下载地址", spec.label))?;
    open_url_with_system(url)?;
    Ok(json!({
        "ok": true,
        "kind": spec.kind,
        "label": spec.label,
        "mode": "download_page",
        "url": url,
    }))
}

/// 状态页客户端 Dock：卸载客户端。
///
/// - CLI：有平台卸载命令时在终端打开执行（与安装流程一致），否则明确提示暂不支持自动卸载。
/// - 桌面版：先强制退出进程；macOS 直接移除已检测到的 .app，Windows 打开系统「应用和功能」让用户手动卸载。
pub fn dex_uninstall_client_impl(kind: String) -> Result<Value, String> {
    let spec = client_app_spec_or_err(&kind)?;

    // 卸载前先尽力退出正在运行的客户端进程，避免文件占用或残留。
    let _ = dex_force_quit_client_impl(kind.clone());

    if client_spec_is_cli(spec) {
        let command = uninstall_command_for_current_os(spec)
            .ok_or_else(|| format!("暂不支持自动卸载 {}，请参考官方文档手动卸载", spec.label))?;
        spawn_terminal_command(command, None)?;
        return Ok(json!({
            "ok": true,
            "kind": spec.kind,
            "label": spec.label,
            "mode": "terminal",
            "command": command,
        }));
    }

    #[cfg(target_os = "macos")]
    {
        let app_path = mac_desktop_app_path(spec)
            .ok_or_else(|| format!("未检测到 {} 的安装，无法卸载", spec.label))?;
        let app_str = app_path.to_string_lossy().to_string();
        let status = std::process::Command::new("rm")
            .arg("-rf")
            .arg(&app_path)
            .status()
            .map_err(|e| format!("移除 {} 失败: {e}", app_str))?;
        if !status.success() {
            return Err(format!(
                "移除 {} 失败（退出码 {:?}）",
                app_str,
                status.code()
            ));
        }
        Ok(json!({
            "ok": true,
            "kind": spec.kind,
            "label": spec.label,
            "mode": "remove_app",
            "path": app_str,
        }))
    }

    #[cfg(target_os = "windows")]
    {
        let mut command = std::process::Command::new("cmd");
        hide_window(&mut command);
        let _ = command
            .args(["/C", "start", "", "ms-settings:appsfeatures"])
            .spawn();
        Ok(json!({
            "ok": true,
            "kind": spec.kind,
            "label": spec.label,
            "mode": "open_settings",
        }))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Err("桌面客户端卸载暂只支持 macOS 与 Windows".to_string())
    }
}

/// 状态页客户端 Dock：卸载前的目标确认（只检测、不执行），供前端二次确认弹窗展示
/// 实际将删除的路径 / 将执行的命令。检测逻辑与 `dex_uninstall_client_impl` 完全一致，
/// 避免“确认文案”与“真实操作目标”不一致（例如 Codex 桌面实际指向 ChatGPT.app）。
pub fn dex_uninstall_target_impl(kind: String) -> Result<Value, String> {
    let spec = client_app_spec_or_err(&kind)?;

    if client_spec_is_cli(spec) {
        let command = uninstall_command_for_current_os(spec);
        return Ok(match command {
            Some(command) => json!({
                "ok": true,
                "kind": spec.kind,
                "label": spec.label,
                "mode": "terminal",
                "command": command,
            }),
            None => json!({
                "ok": true,
                "kind": spec.kind,
                "label": spec.label,
                "mode": "unsupported",
                "reason": format!("暂不支持自动卸载 {}，请参考官方文档手动卸载", spec.label),
            }),
        });
    }

    #[cfg(target_os = "macos")]
    {
        match mac_desktop_app_path(spec) {
            Some(path) => Ok(json!({
                "ok": true,
                "kind": spec.kind,
                "label": spec.label,
                "mode": "remove_app",
                "path": path.to_string_lossy(),
            })),
            None => Ok(json!({
                "ok": true,
                "kind": spec.kind,
                "label": spec.label,
                "mode": "not_installed",
            })),
        }
    }

    #[cfg(target_os = "windows")]
    {
        Ok(json!({
            "ok": true,
            "kind": spec.kind,
            "label": spec.label,
            "mode": "open_settings",
        }))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Err("桌面客户端卸载暂只支持 macOS 与 Windows".to_string())
    }
}

/// 读取桌面版已安装版本。
/// macOS 从 .app 的 Info.plist 读取 `CFBundleShortVersionString`；Windows 暂不读取 exe 资源版本。
fn desktop_installed_version(spec: &ClientAppSpec) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let app_path = mac_desktop_app_path(spec)?;
        let plist = app_path.join("Contents").join("Info.plist");
        if plist.is_file() {
            let output = std::process::Command::new("plutil")
                .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
                .arg(&plist)
                .output()
                .ok()?;
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !text.is_empty() {
                    return Some(text);
                }
            }
        }
        None
    }
    #[cfg(target_os = "windows")]
    {
        let _ = spec;
        None
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = spec;
        None
    }
}

/// 状态页客户端 Dock：更新客户端。
///
/// - CLI：在终端打开平台更新命令（与安装/卸载流程一致），无更新命令时明确提示。
/// - 桌面版：执行「检测」——读取已安装版本（macOS 从 Info.plist），并打开官方下载/更新页供用户确认。
pub fn dex_update_client_impl(kind: String) -> Result<Value, String> {
    let spec = client_app_spec_or_err(&kind)?;

    if client_spec_is_cli(spec) {
        let command = update_command_for_current_os(spec)
            .ok_or_else(|| format!("暂不支持自动更新 {}，请参考官方文档手动更新", spec.label))?;
        spawn_terminal_command(command, None)?;
        return Ok(json!({
            "ok": true,
            "kind": spec.kind,
            "label": spec.label,
            "mode": "terminal",
            "command": command,
        }));
    }

    // 桌面版：检测已安装版本，并打开官方下载/更新入口。
    let installed_version = desktop_installed_version(spec);
    if let Some(url) = spec.download_url {
        let _ = open_url_with_system(url);
    }
    Ok(json!({
        "ok": true,
        "kind": spec.kind,
        "label": spec.label,
        "mode": "check_desktop",
        "installed_version": installed_version,
        "download_url": spec.download_url,
    }))
}

/// 当前平台下该客户端「清理配置文件」的目标路径列表（`~/` 前缀展开为家目录）。
fn clean_config_paths_for_current_os(spec: &ClientAppSpec) -> &'static [&'static str] {
    // 与 install/uninstall/update 的取平台命令一致用 cfg!()：两分支都编译，
    // 避免 macOS 构建时 windows_* 字段被判定为死代码。
    if cfg!(target_os = "windows") {
        spec.windows_clean_config_paths
    } else {
        spec.mac_clean_config_paths
    }
}

/// 清理配置文件的路径信息（纯路径、不读账号库），供目标确认与单测复用。
fn clean_config_paths_target(spec: &ClientAppSpec) -> Value {
    let all: Vec<String> = clean_config_paths_for_current_os(spec)
        .iter()
        .map(|p| expand_home_path(p).to_string_lossy().into_owned())
        .collect();
    if all.is_empty() {
        return json!({
            "ok": true,
            "kind": spec.kind,
            "label": spec.label,
            "mode": "unsupported",
            "reason": format!("暂未配置 {} 的配置文件清理路径", spec.label),
        });
    }
    let existing: Vec<String> = clean_config_paths_for_current_os(spec)
        .iter()
        .map(|p| expand_home_path(p))
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    if existing.is_empty() {
        json!({
            "ok": true,
            "kind": spec.kind,
            "label": spec.label,
            "mode": "none",
            "paths": all,
        })
    } else {
        json!({
            "ok": true,
            "kind": spec.kind,
            "label": spec.label,
            "mode": "paths",
            "paths": existing,
            "all": all,
        })
    }
}

/// 该客户端在 DEX AI 账号库中「可一并移除」的账号（只读）。
/// dsh/Pi/OpenCode/WorkBuddy 不属于 DEX AI 账号体系，恒返回空。
fn client_removable_accounts<'a>(
    store: &'a deecodex::accounts::AccountStore,
    spec: &ClientAppSpec,
) -> Vec<&'a deecodex::accounts::Account> {
    let Some(kind) = client_account_kind_from_slug(spec.account_kind) else {
        return Vec::new();
    };
    let surface = client_surface_from_slug(spec.surface);
    store
        .accounts
        .iter()
        .filter(|account| account.client_kind == kind)
        .filter(|account| {
            if !kind.supports_desktop_surface() {
                return true;
            }
            account.client_surface == surface
        })
        .collect()
}

/// 移除该客户端家族在 DEX AI 账号库中的全部账号（与 `delete_account` 同一套保护与
/// active 修正逻辑：不能删到 Codex 代理账号为 0，删除 active 后切到剩余 Codex 账号）。
/// 返回被移除的账号摘要。只操作账号库，不碰客户端本地配置文件。
fn remove_client_family_accounts(
    data_dir: &Path,
    spec: &ClientAppSpec,
) -> Result<Vec<Value>, String> {
    let Some(kind) = client_account_kind_from_slug(spec.account_kind) else {
        return Ok(Vec::new());
    };
    let surface = client_surface_from_slug(spec.surface);
    let (_store, removed) =
        super::mutate_account_store(data_dir, "移除 DEX AI 账号失败", |store| {
            let targets: Vec<_> = store
                .accounts
                .iter()
                .filter(|account| account.client_kind == kind)
                .filter(|account| {
                    !kind.supports_desktop_surface() || account.client_surface == surface
                })
                .cloned()
                .collect();
            if targets.is_empty() {
                return Ok(Vec::new());
            }
            // 保护：不能删到 Codex 代理账号为 0（代理服务/当前会话依赖）。
            if kind.is_codex() {
                let remaining_codex = store
                    .accounts
                    .iter()
                    .filter(|account| account.client_kind.is_codex())
                    .count()
                    .saturating_sub(targets.len());
                if remaining_codex == 0 {
                    return Err(
                        "不能删除最后一个 Codex 代理账号，否则 DEX 助手和代理服务没有可用活跃账号"
                            .to_string(),
                    );
                }
            }
            let target_ids: HashSet<String> =
                targets.iter().map(|account| account.id.clone()).collect();
            let was_global_active = store
                .active_id
                .as_deref()
                .is_some_and(|id| target_ids.contains(id))
                || store
                    .active_account_id
                    .as_deref()
                    .is_some_and(|id| target_ids.contains(id));
            let was_surface_active = store
                .active_selection_for_surface(&kind, &surface)
                .and_then(|selection| selection.account_id.as_deref())
                .is_some_and(|id| target_ids.contains(id));

            store.accounts.retain(|account| !target_ids.contains(&account.id));
            for account in &mut store.accounts {
                if account
                    .capability_account_id
                    .as_deref()
                    .is_some_and(|id| target_ids.contains(id))
                {
                    account.capability_enabled = false;
                    account.capability_account_id = None;
                }
            }
            if was_global_active {
                let next_id = store
                    .accounts
                    .iter()
                    .find(|account| {
                        account.client_kind.is_codex()
                            && account.client_surface
                                == deecodex::accounts::AccountClientSurface::Cli
                    })
                    .or_else(|| {
                        store
                            .accounts
                            .iter()
                            .find(|account| account.client_kind.is_codex())
                    })
                    .map(|account| account.id.clone());
                store.active_id = next_id.clone();
                store.active_account_id = next_id.clone();
                store.active_endpoint_id = next_id.as_ref().and_then(|id| {
                    store
                        .accounts
                        .iter()
                        .find(|account| &account.id == id)
                        .and_then(|account| account.endpoints.first())
                        .map(|endpoint| endpoint.id.clone())
                });
            } else if was_surface_active {
                if let Some(next_surface) = store
                    .accounts
                    .iter()
                    .find(|account| {
                        account.client_kind == kind && account.client_surface == surface
                    })
                    .cloned()
                {
                    super::activate_client_surface_account(
                        store,
                        &next_surface,
                        next_surface.endpoints.first().map(|endpoint| endpoint.id.clone()),
                        false,
                    );
                }
            }
            deecodex::accounts::validate_capability_links(store).map_err(|e| e.to_string())?;
            deecodex::accounts::validate_dev_pipeline_links(store).map_err(|e| e.to_string())?;
            Ok(targets)
        })?;
    Ok(removed
        .into_iter()
        .map(|account| {
            json!({
                "id": account.id,
                "name": account.name,
                "provider": account.provider,
                "client_kind": account.client_kind,
                "client_surface": account.client_surface,
            })
        })
        .collect())
}

/// 状态页客户端 Dock：清理配置文件前的目标确认（只检测、不执行），供前端二次确认弹窗
/// 展示实际将删除的路径与可一并移除的 DEX AI 账号。
pub async fn dex_clean_config_target_impl(
    manager: State<'_, ServerManager>,
    kind: String,
) -> Result<Value, String> {
    let spec = client_app_spec_or_err(&kind)?;
    let mut value = clean_config_paths_target(spec);
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);
    let removable: Vec<Value> = client_removable_accounts(&store, spec)
        .iter()
        .map(|account| {
            json!({
                "id": account.id,
                "name": account.name,
                "provider": account.provider,
                "client_kind": account.client_kind,
                "client_surface": account.client_surface,
            })
        })
        .collect();
    value["removable_accounts"] = json!(removable);
    value["removable_count"] = json!(removable.len());
    // 保护：Codex 账号删后归零时不允许移除（与 delete_account 一致），前端据此隐藏勾选。
    let blocked = client_account_kind_from_slug(spec.account_kind)
        .map(|kind| {
            kind.is_codex()
                && !removable.is_empty()
                && store
                    .accounts
                    .iter()
                    .filter(|account| account.client_kind.is_codex())
                    .count()
                    <= removable.len()
        })
        .unwrap_or(false);
    value["remove_account_blocked"] = json!(blocked);
    Ok(value)
}

/// 状态页客户端 Dock：清理客户端配置文件。
///
/// 每种客户端使用其专属的配置路径（`ClientAppSpec.clean_config_paths`）直接删除，
/// 不用通用命令。执行前先退出正在运行的客户端进程，避免运行时改写/重建配置
/// （Hermes 的 launchd 网关会先被 bootout）。
/// `remove_account=true` 时，额外移除该客户端家族在 DEX AI 账号库中的账号（受保护）。
pub async fn dex_clean_config_impl(
    manager: State<'_, ServerManager>,
    kind: String,
    remove_account: bool,
) -> Result<Value, String> {
    let spec = client_app_spec_or_err(&kind)?;
    tracing::info!(client = spec.kind, remove_account, "清理配置文件开始");
    let paths: Vec<PathBuf> = clean_config_paths_for_current_os(spec)
        .iter()
        .map(|p| expand_home_path(p))
        .collect();
    if paths.is_empty() {
        return Err(format!(
            "暂未配置 {} 的配置文件清理路径",
            spec.label
        ));
    }

    // 先尽力退出正在运行的客户端进程，避免退出时把内存中的配置/登录态写回磁盘。
    let quit_result = dex_force_quit_client_impl(kind.clone());
    tracing::debug!(client = spec.kind, quit_ok = quit_result.is_ok(), "清理前强制退出结果");

    let mut removed: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    for path in paths {
        if !path.exists() {
            continue;
        }
        // 符号链接用 remove_file（只删链接本身），目录用 remove_dir_all，文件用 remove_file。
        let meta = std::fs::symlink_metadata(&path);
        let is_symlink = meta.as_ref().map(|m| m.file_type().is_symlink()).unwrap_or(false);
        let result = if is_symlink || path.is_file() {
            std::fs::remove_file(&path)
        } else {
            std::fs::remove_dir_all(&path)
        };
        match result {
            Ok(()) => removed.push(path.to_string_lossy().into_owned()),
            Err(err) => failed.push(format!("{}: {err}", path.display())),
        }
    }

    let mut removed_accounts: Vec<Value> = Vec::new();
    let mut account_failed: Option<String> = None;
    if remove_account {
        let data_dir = manager.data_dir.lock().await.clone();
        match remove_client_family_accounts(&data_dir, spec) {
            Ok(accounts) => {
                removed_accounts = accounts;
                tracing::info!(client = spec.kind, count = removed_accounts.len(), "已移除 DEX AI 账号");
            }
            Err(err) => {
                account_failed = Some(err.clone());
                tracing::error!(client = spec.kind, error = %err, "移除 DEX AI 账号失败");
            }
        }
    }
    tracing::info!(
        client = spec.kind,
        removed = removed.len(),
        failed = failed.len(),
        removed_accounts = removed_accounts.len(),
        "清理配置文件结束"
    );

    Ok(json!({
        "ok": true,
        "kind": spec.kind,
        "label": spec.label,
        "removed": removed,
        "failed": failed,
        "removed_accounts": removed_accounts,
        "removed_account_count": removed_accounts.len(),
        "account_failed": account_failed,
    }))
}

/// 状态页客户端 Dock：启动桌面版或在终端中启动 CLI。
pub fn dex_launch_client_impl(kind: String, cwd: Option<String>) -> Result<Value, String> {
    let spec = client_app_spec_or_err(&kind)?;
    if client_spec_is_cli(spec) {
        // web 型 CLI（如 dsh web）：服务已在运行时，直接帮用户打开 Web 界面，
        // 避免重复拉起服务进程 / 弹出多个终端。
        if let Some(web_url) = spec.open_web_url {
            if !client_process_instances_for_spec(spec).is_empty() {
                open_web_ui_after_ready(web_url);
                return Ok(json!({
                    "ok": true,
                    "kind": spec.kind,
                    "label": spec.label,
                    "mode": "web_open",
                    "web_url": web_url,
                }));
            }
        }
        let command = spec
            .launch_command
            .or(spec.command)
            .ok_or_else(|| format!("{} 没有 CLI 启动命令", spec.label))?;
        let cwd = if spec.launch_requires_cwd {
            let cwd = cwd
                .map(PathBuf::from)
                .ok_or_else(|| format!("{} 需要先选择启动目录", spec.label))?;
            if !cwd.exists() || !cwd.is_dir() {
                return Err(format!("启动目录不存在或不是目录: {}", cwd.display()));
            }
            Some(cwd)
        } else {
            None
        };
        spawn_terminal_command(command, cwd.as_deref())?;
        if let Some(web_url) = spec.open_web_url {
            open_web_ui_after_ready(web_url);
        }
        return Ok(json!({
            "ok": true,
            "kind": spec.kind,
            "label": spec.label,
            "mode": "terminal",
            "command": command,
            "cwd": cwd.as_ref().map(|path| path.to_string_lossy().to_string()),
            "requires_cwd": spec.launch_requires_cwd,
            "web_url": spec.open_web_url,
        }));
    }

    launch_desktop_app(spec)?;
    Ok(json!({
        "ok": true,
        "kind": spec.kind,
        "label": spec.label,
        "mode": "desktop",
        "app": spec.desktop_app,
    }))
}

/// 状态页客户端 Dock：选择 CLI 启动目录。
pub async fn dex_pick_client_launch_dir_impl() -> Result<Option<String>, String> {
    let path = rfd::AsyncFileDialog::new()
        .set_title("选择客户端启动目录")
        .pick_folder()
        .await
        .map(|folder| folder.path().to_string_lossy().to_string());
    Ok(path)
}

fn open_url_with_system(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("打开下载页失败: {e}"))?;
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        let mut command = std::process::Command::new("cmd");
        hide_window(&mut command);
        command
            .args(["/C", "start", "", url])
            .spawn()
            .map_err(|e| format!("打开下载页失败: {e}"))?;
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("打开下载页失败: {e}"))?;
        Ok(())
    }
}

/// 从 `http://host:port/...` 提取 `host:port`，用于 TCP 就绪探测。
fn url_connect_addr(url: &str) -> Option<String> {
    let (default_port, rest) = if let Some(rest) = url.strip_prefix("http://") {
        (80, rest)
    } else if let Some(rest) = url.strip_prefix("https://") {
        (443, rest)
    } else {
        return None;
    };
    let host_port = rest.split('/').next()?;
    if host_port.is_empty() {
        return None;
    }
    if host_port.contains(':') {
        Some(host_port.to_string())
    } else {
        Some(format!("{host_port}:{default_port}"))
    }
}

/// CLI 启动后帮用户打开本机 Web 服务：后台轮询 TCP 端口，等服务就绪再调用系统浏览器。
/// 避免服务还没绑定端口就打开页面导致连接失败。
fn open_web_ui_after_ready(url: &str) {
    let Some(addr) = url_connect_addr(url) else {
        tracing::warn!(url, "无法解析 web 服务地址，跳过自动打开浏览器");
        return;
    };
    let url = url.to_string();
    std::thread::spawn(move || {
        for _ in 0..40 {
            if std::net::TcpStream::connect(addr.as_str()).is_ok() {
                match open_url_with_system(&url) {
                    Ok(_) => tracing::info!(url, "已打开客户端 Web 界面"),
                    Err(err) => tracing::warn!(url, error = %err, "打开客户端 Web 界面失败"),
                }
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        tracing::warn!(url, addr, "等待客户端 Web 服务就绪超时，未打开浏览器");
    });
}

/// 从 `spec.mac_desktop_paths` 派生 macOS 应用名候选列表（如 `Codex.app` → `Codex`）。
/// 同时把 `spec.desktop_app` 作为首选插入队首。
/// Codex 桌面 spec 的 `mac_desktop_paths` 包含 `Codex.app` 与新版 `ChatGPT.app`，
/// 派生出的候选列表为 `["Codex", "ChatGPT"]`，无需为每个 spec 单独声明别名。
fn macos_app_name_candidates(spec: &ClientAppSpec) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    if let Some(name) = spec.desktop_app {
        out.push(name);
    }
    for path in spec.mac_desktop_paths {
        let Some(stripped) = path.strip_suffix(".app") else {
            continue;
        };
        let Some(name) = stripped.rsplit('/').next() else {
            continue;
        };
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// macOS 上按 Bundle ID 启动桌面应用，失败回退到按应用名列表逐个尝试。
/// 抽这个 helper 让 `launch_desktop_app` / `dex_toggle_desktop_client_impl` / CDP 启停共用同一段逻辑。
#[cfg(target_os = "macos")]
fn macos_open_app(spec: &ClientAppSpec) -> Result<(), String> {
    let candidates = macos_app_name_candidates(spec);
    for bundle_id in spec.mac_bundle_ids {
        // 注意：`open` 进程能 spawn 成功 ≠ 打开成功（bundle id 不存在时 open 退出码非 0）。
        // 必须用 .status() 检退出码，失败继续尝试下一个 bundle id / 应用名候选。
        match std::process::Command::new("open")
            .arg("-b")
            .arg(bundle_id)
            .status()
        {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => {
                tracing::warn!(
                    client = spec.label,
                    bundle_id,
                    exit_code = ?status.code(),
                    "按 Bundle ID 启动失败，尝试下一候选",
                );
            }
            Err(err) => {
                tracing::warn!(
                    client = spec.label,
                    bundle_id,
                    error = %err,
                    "按 Bundle ID 启动失败，回退到按应用名启动",
                );
            }
        }
    }
    let mut last_err: Option<String> = None;
    for app_name in candidates {
        match std::process::Command::new("open")
            .arg("-a")
            .arg(app_name)
            .status()
        {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => {
                tracing::warn!(
                    client = spec.label,
                    app = app_name,
                    exit_code = ?status.code(),
                    "按应用名启动失败，尝试下一候选",
                );
                last_err = Some(format!("exit_code={:?}", status.code()));
            }
            Err(err) => {
                tracing::warn!(
                    client = spec.label,
                    app = app_name,
                    error = %err,
                    "按应用名启动失败，尝试下一候选",
                );
                last_err = Some(err.to_string());
            }
        }
    }
    Err(format!(
        "打开 {} 失败: {}",
        spec.label,
        last_err.unwrap_or_else(|| "无可用应用名候选".to_string())
    ))
}

/// macOS 上按 Bundle ID 优雅退出，失败回退到按应用名列表逐个 `osascript quit app`。
#[cfg(target_os = "macos")]
fn macos_quit_app(spec: &ClientAppSpec) -> Result<(), String> {
    let candidates = macos_app_name_candidates(spec);
    let mut last_err: Option<String> = None;
    for bundle_id in spec.mac_bundle_ids {
        match std::process::Command::new("osascript")
            .arg("-e")
            .arg(format!("quit app id \"{bundle_id}\""))
            .status()
        {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => {
                tracing::warn!(
                    client = spec.label,
                    bundle_id,
                    exit_code = ?status.code(),
                    "AppleScript quit app id 退出码非 0，回退到按应用名退出",
                );
                last_err = Some(format!("exit_code={:?}", status.code()));
            }
            Err(err) => {
                tracing::warn!(
                    client = spec.label,
                    bundle_id,
                    error = %err,
                    "派发 AppleScript quit app id 失败，回退到按应用名退出",
                );
                last_err = Some(err.to_string());
            }
        }
    }
    for app_name in candidates {
        match std::process::Command::new("osascript")
            .arg("-e")
            .arg(format!("quit app \"{app_name}\""))
            .status()
        {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => {
                tracing::warn!(
                    client = spec.label,
                    app = app_name,
                    exit_code = ?status.code(),
                    "AppleScript quit app 退出码非 0，尝试下一候选",
                );
                last_err = Some(format!("exit_code={:?}", status.code()));
            }
            Err(err) => {
                tracing::warn!(
                    client = spec.label,
                    app = app_name,
                    error = %err,
                    "派发 AppleScript quit app 失败，尝试下一候选",
                );
                last_err = Some(err.to_string());
            }
        }
    }
    Err(format!(
        "退出 {} 失败: {}",
        spec.label,
        last_err.unwrap_or_else(|| "无可用应用名候选".to_string())
    ))
}

fn launch_desktop_app(spec: &ClientAppSpec) -> Result<(), String> {
    if spec.desktop_app.is_none() && spec.mac_desktop_paths.is_empty() {
        return Err(format!("{} 不是桌面版客户端", spec.label));
    }
    #[cfg(target_os = "macos")]
    {
        macos_open_app(spec)
    }
    #[cfg(target_os = "windows")]
    {
        let app_name = spec.desktop_app.unwrap_or("");
        if let Some(path) = windows_desktop_app_path(spec) {
            let mut command = std::process::Command::new(path);
            hide_window(&mut command);
            command
                .spawn()
                .map_err(|e| format!("打开 {app_name} 失败: {e}"))?;
            return Ok(());
        }
        // MSIX Store 版（OpenAI.Codex/ChatGPT、Claude 桌面）：explorer shell:AppsFolder\<AUMID>
        if let Some(aumid) = windows_store_aumid(spec) {
            std::process::Command::new("explorer.exe")
                .arg(format!("shell:AppsFolder\\{aumid}"))
                .spawn()
                .map_err(|e| format!("打开 {app_name}（Store 版）失败: {e}"))?;
            return Ok(());
        }
        let hint = spec
            .download_url
            .map(|url| format!("，请先安装：{url}"))
            .unwrap_or_default();
        Err(format!("未检测到已安装的 {app_name} 桌面版{hint}"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Err("桌面客户端启动暂只支持 macOS 与 Windows".to_string())
    }
}

fn shell_quote_posix(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn escape_applescript_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(any(target_os = "windows", test))]
fn powershell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn mac_terminal_script(command: &str, cwd: Option<&Path>) -> String {
    let shell = if let Some(cwd) = cwd {
        format!(
            "cd {} && {command}",
            shell_quote_posix(&cwd.to_string_lossy())
        )
    } else {
        command.to_string()
    };
    format!(
        "tell application \"Terminal\"\nactivate\ndo script \"{}\"\nend tell",
        escape_applescript_string(&shell)
    )
}

#[cfg(any(target_os = "windows", test))]
fn windows_terminal_script(command: &str, cwd: Option<&Path>) -> String {
    if let Some(cwd) = cwd {
        format!(
            "Set-Location -LiteralPath {}; {}",
            powershell_single_quote(&cwd.to_string_lossy()),
            command
        )
    } else {
        command.to_string()
    }
}

fn spawn_terminal_command(command: &str, cwd: Option<&Path>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let script = mac_terminal_script(command, cwd);
        std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .spawn()
            .map_err(|e| format!("打开终端失败: {e}"))?;
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        let script = windows_terminal_script(command, cwd);
        let result = if command_exists("wt") {
            std::process::Command::new("wt")
                .args(["powershell.exe", "-NoExit", "-Command", &script])
                .spawn()
        } else {
            std::process::Command::new("powershell.exe")
                .args(["-NoExit", "-Command", &script])
                .spawn()
        };
        result.map_err(|e| format!("打开终端失败: {e}"))?;
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (command, cwd);
        Err("CLI 终端启动暂只支持 macOS 与 Windows".to_string())
    }
}

/// 状态页客户端 Dock：打开或退出桌面版客户端。
pub fn dex_toggle_desktop_client_impl(kind: String, running: bool) -> Result<Value, String> {
    let spec = client_app_spec_or_err(&kind)?;
    let action = if running { "quit" } else { "open" };

    #[cfg(target_os = "macos")]
    {
        if running {
            macos_quit_app(spec)?;
        } else {
            macos_open_app(spec)?;
        }
        Ok(json!({
            "ok": true,
            "kind": spec.kind,
            "app": spec.desktop_app,
            "action": action,
        }))
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (spec, action);
        Err("桌面客户端开关暂只支持 macOS".to_string())
    }
}

const FORCE_QUIT_MAX_ROUNDS: usize = 4;
const FORCE_QUIT_ROUND_DELAY_MS: u64 = 350;
/// 优雅退出等待窗口（毫秒）：先发 `quit app id`，分轮 poll 进程退出，超出此上限才进入 SIGKILL 兜底。
/// 该值需大于 Electron 应用处理 `applicationWillTerminate` 的常见耗时（百毫秒级）。
const FORCE_QUIT_GRACEFUL_WAIT_MS: u64 = 800;
/// 优雅退出 poll 间隔（毫秒）。
const FORCE_QUIT_GRACEFUL_POLL_MS: u64 = 100;

fn force_kill_pid(pid: &str) -> Result<(), String> {
    let pid = pid.trim();
    if pid.is_empty() {
        return Err("PID 为空".into());
    }
    #[cfg(target_os = "windows")]
    {
        let mut command = std::process::Command::new("taskkill");
        hide_window(&mut command);
        let status = command
            .args(["/PID", pid, "/F"])
            .status()
            .map_err(|e| format!("执行 taskkill 失败: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("taskkill 退出码: {:?}", status.code()))
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let status = std::process::Command::new("kill")
            .args(["-9", pid])
            .status()
            .map_err(|e| format!("执行 kill 失败: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("kill 退出码: {:?}", status.code()))
        }
    }
}

fn force_quit_client_processes<F, K, S>(
    spec: &ClientAppSpec,
    mut collect_instances: F,
    mut kill_pid: K,
    mut sleep_after_round: S,
) -> Value
where
    F: FnMut() -> Vec<Value>,
    K: FnMut(&str) -> Result<(), String>,
    S: FnMut(),
{
    let mut killed = Vec::new();
    let mut killed_pids = HashSet::new();
    let mut errors = Vec::new();
    let mut saw_process = false;
    let mut rounds = 0usize;

    for round in 1..=FORCE_QUIT_MAX_ROUNDS {
        let instances = collect_instances();
        if instances.is_empty() {
            if !saw_process {
                return json!({
                    "ok": false,
                    "kind": spec.kind,
                    "label": spec.label,
                    "killed": [],
                    "errors": ["未检测到运行中的客户端进程"],
                    "remaining": [],
                    "rounds": 0,
                });
            }
            break;
        }

        saw_process = true;
        rounds = round;
        let mut attempted = 0usize;
        for instance in instances {
            let pid = instance
                .get("pid")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            let command = instance.get("command").cloned().unwrap_or(Value::Null);
            if pid.is_empty() {
                errors.push(json!({
                    "pid": null,
                    "round": round,
                    "error": "进程 PID 为空",
                    "command": command,
                }));
                continue;
            }

            attempted += 1;
            match kill_pid(&pid) {
                Ok(()) => {
                    if killed_pids.insert(pid.clone()) {
                        killed.push(json!({
                            "pid": pid,
                            "round": round,
                            "command": command,
                        }));
                    }
                }
                Err(err) => errors.push(json!({
                    "pid": pid,
                    "round": round,
                    "error": err,
                    "command": command,
                })),
            }
        }

        if attempted > 0 {
            sleep_after_round();
        }
    }

    let remaining = collect_instances();
    if !remaining.is_empty() {
        errors.push(json!({
            "error": format!("仍检测到 {} 个客户端进程", remaining.len()),
            "remaining": remaining.clone(),
        }));
    }

    json!({
        "ok": !killed.is_empty() && remaining.is_empty() && errors.is_empty(),
        "kind": spec.kind,
        "label": spec.label,
        "killed": killed,
        "errors": errors,
        "remaining": remaining,
        "rounds": rounds,
    })
}

/// 状态页客户端 Dock：按已检测到的进程 PID 强制退出客户端，分多轮确认残留进程。
pub fn dex_force_quit_client_impl(kind: String) -> Result<Value, String> {
    let spec = client_app_spec_or_err(&kind)?;
    // Hermes CLI 的常驻形态是 launchd 网关服务（ai.hermes.gateway，KeepAlive 自动拉起）：
    // 通用 SIGKILL 会被 launchd 立即重启，必须先 `hermes gateway stop`
    // （内部 `launchctl bootout` 卸载服务定义，KeepAlive 不再拉起）。
    #[cfg(target_os = "macos")]
    let mut hermes_gateway_stopped = false;
    #[cfg(target_os = "macos")]
    if spec.kind == "hermes" {
        // GUI 发布版进程 PATH 只有系统目录（不含 ~/.local/bin），直接用绝对路径
        // 调 hermes 启动器（hermes 官方安装脚本写入 ~/.local/bin/hermes），
        // 找不到时回退到 PATH 上的 `hermes`。
        let hermes_bin = std::env::var("HOME")
            .ok()
            .map(|home| Path::new(&home).join(".local/bin/hermes"))
            .filter(|p| p.exists())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "hermes".to_string());
        match std::process::Command::new(&hermes_bin)
            .args(["gateway", "stop"])
            .status()
        {
            Ok(status) if status.success() => {
                hermes_gateway_stopped = true;
                tracing::info!(client = spec.label, "hermes 网关已停止（launchd bootout）");
            }
            Ok(status) => tracing::warn!(
                client = spec.label,
                exit_code = ?status.code(),
                "hermes gateway stop 退出码非 0，进入通用强杀兜底"
            ),
            Err(err) => tracing::warn!(
                client = spec.label,
                error = %err,
                "hermes gateway stop 执行失败，进入通用强杀兜底"
            ),
        }
    }
    // macOS 桌面客户端：先发 `quit app id <bundle>` 触发应用层 graceful shutdown，
    // 分轮 poll 进程退出，避免对 Electron 主进程 SIGKILL 触发 Crashpad 弹窗。
    // 注意：osascript 派发 AppleEvent 是异步的，spawn 成功 ≠ quit 实际生效，
    // 必须用 .status() 检退出码并轮询实例列表确认进程已退。
    let mut graceful_quit_dispatched = false;
    #[cfg(target_os = "macos")]
    for bundle_id in spec.mac_bundle_ids {
        match std::process::Command::new("osascript")
            .arg("-e")
            .arg(format!("quit app id \"{bundle_id}\""))
            .status()
        {
            Ok(status) if !status.success() => tracing::warn!(
                client = spec.label,
                bundle_id,
                exit_code = ?status.code(),
                "AppleScript quit app id 退出码非 0，可能未生效，进入 SIGKILL 兜底",
            ),
            Err(err) => tracing::warn!(
                client = spec.label,
                bundle_id,
                error = %err,
                "派发 AppleScript quit app id 失败，将直接进入 SIGKILL 兜底",
            ),
            Ok(_) => {
                graceful_quit_dispatched = true;
                // 轮询等待应用进程退出，间隔 100ms，超出 FORCE_QUIT_GRACEFUL_WAIT_MS 上限才进入 SIGKILL。
                let mut waited_ms = 0u64;
                while waited_ms < FORCE_QUIT_GRACEFUL_WAIT_MS {
                    if client_process_instances_for_spec(spec).is_empty() {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(
                        FORCE_QUIT_GRACEFUL_POLL_MS,
                    ));
                    waited_ms += FORCE_QUIT_GRACEFUL_POLL_MS;
                }
            }
        }
    }
    // 优雅退出已派发且进程已全部退出：这是“成功退出”，不是“未检测到进程”，
    // 直接返回成功，避免强杀阶段把空实例误报为失败（前端会提示“强制退出失败”）。
    #[cfg(target_os = "macos")]
    if graceful_quit_dispatched && client_process_instances_for_spec(spec).is_empty() {
        return Ok(json!({
            "ok": true,
            "kind": spec.kind,
            "label": spec.label,
            "mode": "graceful",
            "killed": [],
            "remaining": [],
            "rounds": 0,
        }));
    }
    // Hermes 网关已通过 bootout 停止且无残留进程：视为成功退出，避免通用强杀阶段
    // 把“无进程”误报为失败（否则前端提示“强制退出失败”）。
    #[cfg(target_os = "macos")]
    if hermes_gateway_stopped && client_process_instances_for_spec(spec).is_empty() {
        return Ok(json!({
            "ok": true,
            "kind": spec.kind,
            "label": spec.label,
            "mode": "gateway_stopped",
            "killed": [],
            "remaining": [],
            "rounds": 0,
        }));
    }
    Ok(force_quit_client_processes(
        spec,
        || client_process_instances_for_spec(spec),
        force_kill_pid,
        || {
            std::thread::sleep(std::time::Duration::from_millis(FORCE_QUIT_ROUND_DELAY_MS));
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn fake_process(pid: &str, command: &str) -> Value {
        json!({
            "pid": pid,
            "command": command,
        })
    }

    #[test]
    fn client_lifecycle_specs_cover_status_dock_entries() {
        let kinds: Vec<_> = CLIENT_APP_SPECS.iter().map(|spec| spec.kind).collect();
        assert_eq!(
            kinds,
            vec![
                "codex_cli",
                "codex_desktop",
                "claude_cli",
                "claude_desktop",
                "hermes",
                "dsh",
                "pi",
                "opencode",
                "opencode_desktop",
                "workbuddy",
                "hermes_desktop"
            ]
        );
        assert!(client_app_spec_or_err("codex_cli").unwrap().command == Some("codex"));
        assert_eq!(
            client_app_spec_or_err("dsh").unwrap().launch_command,
            Some("dsh web")
        );
        // dsh 是 web 型 CLI：启动后必须自动帮用户打开本地 Web 界面（默认 127.0.0.1:3080）。
        assert_eq!(
            client_app_spec_or_err("dsh").unwrap().open_web_url,
            Some("http://localhost:3080")
        );
        assert_eq!(
            url_connect_addr("http://localhost:3080"),
            Some("localhost:3080".to_string())
        );
        assert_eq!(
            url_connect_addr("http://127.0.0.1:8080/"),
            Some("127.0.0.1:8080".to_string())
        );
        assert_eq!(
            url_connect_addr("https://example.com/path"),
            Some("example.com:443".to_string())
        );
        assert_eq!(url_connect_addr("not a url"), None);
        assert_eq!(client_app_spec_or_err("pi").unwrap().command, Some("pi"));
        assert_eq!(
            client_app_spec_or_err("opencode").unwrap().command,
            Some("opencode")
        );
        assert_eq!(
            client_app_spec_or_err("opencode_desktop")
                .unwrap()
                .desktop_app,
            Some("OpenCode")
        );
        assert_eq!(
            client_app_spec_or_err("workbuddy").unwrap().desktop_app,
            Some("WorkBuddy")
        );
        assert_eq!(
            client_app_spec_or_err("hermes_desktop")
                .unwrap()
                .desktop_app,
            Some("Hermes")
        );
        // 点击图标未安装时，必须打开国内版下载页（workbuddy.cn），而非海外版（workbuddy.ai）。
        assert_eq!(
            client_app_spec_or_err("workbuddy").unwrap().download_url,
            Some("https://www.workbuddy.cn/")
        );
        assert_eq!(
            client_app_spec_or_err("hermes_desktop")
                .unwrap()
                .download_url,
            Some("https://hermes-agent.nousresearch.com/")
        );
        assert!(
            client_app_spec_or_err("codex_cli")
                .unwrap()
                .launch_requires_cwd
        );
        assert!(
            client_app_spec_or_err("claude_cli")
                .unwrap()
                .launch_requires_cwd
        );
        assert!(
            !client_app_spec_or_err("hermes")
                .unwrap()
                .launch_requires_cwd
        );
        // dsh/Pi/OpenCode 只做下载/安装/启动，不要求账号配置。
        assert!(!client_app_spec_or_err("dsh").unwrap().managed_account);
        assert!(!client_app_spec_or_err("pi").unwrap().managed_account);
        assert!(!client_app_spec_or_err("opencode").unwrap().managed_account);
        assert!(
            !client_app_spec_or_err("opencode_desktop")
                .unwrap()
                .managed_account
        );
        assert!(!client_app_spec_or_err("workbuddy").unwrap().managed_account);
        assert!(
            !client_app_spec_or_err("hermes_desktop")
                .unwrap()
                .managed_account
        );
        assert!(client_app_spec_or_err("codex_cli").unwrap().managed_account);
        assert!(client_app_spec("missing").is_none());
    }

    #[test]
    fn client_lifecycle_install_commands_are_whitelisted() {
        let commands: Vec<_> = CLIENT_APP_SPECS
            .iter()
            .filter_map(install_command_for_status)
            .collect();
        assert!(commands.iter().any(|cmd| cmd.contains("@openai/codex")));
        assert!(commands
            .iter()
            .any(|cmd| cmd.contains("@anthropic-ai/claude-code")));
        assert!(commands.iter().any(|cmd| cmd.contains("hermes")));
        assert!(commands.iter().any(|cmd| cmd.contains("@deepseek-ai/dsh")));
        assert!(commands
            .iter()
            .any(|cmd| cmd.contains("@earendil-works/pi-coding-agent")));
        assert!(commands.iter().any(|cmd| cmd.contains("opencode")));
    }

    #[test]
    fn client_lifecycle_uninstall_commands_are_whitelisted() {
        let commands: Vec<_> = CLIENT_APP_SPECS
            .iter()
            .filter_map(uninstall_command_for_current_os)
            .collect();
        assert!(commands
            .iter()
            .any(|cmd| cmd.contains("npm uninstall -g @openai/codex")));
        assert!(commands
            .iter()
            .any(|cmd| cmd.contains("npm uninstall -g @anthropic-ai/claude-code")));
        assert!(commands
            .iter()
            .any(|cmd| cmd.contains("npm uninstall -g @deepseek-ai/dsh")));
        assert!(commands
            .iter()
            .any(|cmd| cmd.contains("npm uninstall -g @earendil-works/pi-coding-agent")));
        // curl 脚本安装的 CLI（hermes/opencode）已按官方落地路径配置 macOS 卸载命令。
        let hermes = client_app_spec("hermes").unwrap();
        let opencode = client_app_spec("opencode").unwrap();
        let codex = client_app_spec("codex_cli").unwrap();
        assert!(hermes
            .mac_uninstall_command
            .unwrap_or("")
            .contains("~/.hermes"));
        assert!(opencode
            .mac_uninstall_command
            .unwrap_or("")
            .contains("~/.opencode"));
        // codex 卸载命令必须同时覆盖 npm 与 standalone 两种安装方式。
        assert!(codex
            .mac_uninstall_command
            .unwrap_or("")
            .contains("~/.codex/packages/standalone"));
    }

    #[test]
    fn client_lifecycle_uninstall_target_matches_uninstall() {
        // 卸载前的目标确认必须与真实卸载走同一检测逻辑：
        // CLI 返回同一命令（或无命令时标记 unsupported），桌面版返回检测到的应用路径。
        for spec in CLIENT_APP_SPECS {
            let target = dex_uninstall_target_impl(spec.kind.to_string()).unwrap();
            let mode = target
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if client_spec_is_cli(spec) {
                match uninstall_command_for_current_os(spec) {
                    Some(cmd) => {
                        assert_eq!(mode, "terminal", "CLI {} 应返回 terminal 模式", spec.kind);
                        assert_eq!(
                            target.get("command").and_then(Value::as_str),
                            Some(cmd),
                            "CLI {} 目标命令与卸载命令不一致",
                            spec.kind
                        );
                    }
                    None => {
                        assert_eq!(
                            mode, "unsupported",
                            "CLI {} 无卸载命令时应标记 unsupported",
                            spec.kind
                        );
                    }
                }
            } else if cfg!(target_os = "macos") {
                assert!(
                    matches!(mode, "remove_app" | "not_installed"),
                    "桌面版 {} 在 macOS 上应返回 remove_app 或 not_installed，实际 mode={mode}",
                    spec.kind
                );
            } else if cfg!(target_os = "windows") {
                assert_eq!(
                    mode, "open_settings",
                    "桌面版 {} 在 Windows 上应打开系统设置",
                    spec.kind
                );
            }
        }
    }

    #[test]
    fn client_lifecycle_update_commands_are_whitelisted() {
        let commands: Vec<_> = CLIENT_APP_SPECS
            .iter()
            .filter_map(update_command_for_current_os)
            .collect();
        assert!(commands
            .iter()
            .any(|cmd| cmd.contains("@openai/codex@latest")));
        assert!(commands
            .iter()
            .any(|cmd| cmd.contains("@anthropic-ai/claude-code@latest")));
        assert!(commands
            .iter()
            .any(|cmd| cmd.contains("@deepseek-ai/dsh@latest")));
        assert!(commands
            .iter()
            .any(|cmd| cmd.contains("@earendil-works/pi-coding-agent@latest")));
        // 桌面版不提供 CLI 更新命令，走「检测 + 打开官方页面」。
        for kind in [
            "codex_desktop",
            "claude_desktop",
            "opencode_desktop",
            "workbuddy",
            "hermes_desktop",
        ] {
            let spec = client_app_spec(kind).unwrap();
            assert_eq!(spec.mac_update_command, None);
            assert_eq!(spec.windows_update_command, None);
        }
    }

    #[test]
    fn client_lifecycle_clean_config_paths_are_whitelisted() {
        // 每个客户端都必须配置专属清理路径（不允许空，否则右键「清理配置文件」无法执行）。
        for spec in CLIENT_APP_SPECS {
            assert!(
                !spec.mac_clean_config_paths.is_empty(),
                "{} 未配置 macOS 清理路径",
                spec.kind
            );
            assert!(
                !spec.windows_clean_config_paths.is_empty(),
                "{} 未配置 Windows 清理路径",
                spec.kind
            );
        }
        // 各 CLI 客户端必须指向自己的配置目录，不能共用/漏配。
        let codex = client_app_spec("codex_cli").unwrap();
        assert!(codex.mac_clean_config_paths.contains(&"~/.codex"));
        let claude = client_app_spec("claude_cli").unwrap();
        assert!(claude.mac_clean_config_paths.contains(&"~/.claude"));
        assert!(claude.mac_clean_config_paths.contains(&"~/.claude.json"));
        let hermes = client_app_spec("hermes").unwrap();
        assert!(hermes.mac_clean_config_paths.contains(&"~/.hermes"));
        let dsh = client_app_spec("dsh").unwrap();
        assert!(dsh.mac_clean_config_paths.contains(&"~/.dsh"));
        let pi = client_app_spec("pi").unwrap();
        assert!(pi.mac_clean_config_paths.contains(&"~/.pi"));
        let opencode = client_app_spec("opencode").unwrap();
        assert!(opencode
            .mac_clean_config_paths
            .contains(&"~/.config/opencode"));
        assert!(opencode
            .mac_clean_config_paths
            .contains(&"~/.local/share/opencode"));
        // 桌面版必须指向 Electron 数据目录而非安装 .app（清理配置 ≠ 卸载）。
        for kind in [
            "codex_desktop",
            "claude_desktop",
            "opencode_desktop",
            "workbuddy",
            "hermes_desktop",
        ] {
            let spec = client_app_spec(kind).unwrap();
            assert!(
                spec.mac_clean_config_paths[0].contains("Application Support"),
                "{kind} 清理路径应指向 Application Support 数据目录"
            );
        }
    }

    #[test]
    fn dex_clean_config_target_lists_all_configured_paths() {
        // 清理目标确认必须与配置路径一一对应（只读，不删除）。
        for spec in CLIENT_APP_SPECS {
            let target = clean_config_paths_target(spec);
            let mode = target
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or_default();
            assert!(
                matches!(mode, "paths" | "none" | "unsupported"),
                "{} 清理目标模式异常: {mode}",
                spec.kind
            );
            if mode != "unsupported" {
                let all = target
                    .get("all")
                    .or_else(|| target.get("paths"))
                    .and_then(Value::as_array)
                    .unwrap_or_else(|| panic!("{} 清理目标应返回路径列表", spec.kind));
                assert_eq!(
                    all.len(),
                    clean_config_paths_for_current_os(spec).len(),
                    "{} 清理目标路径数与配置不一致",
                    spec.kind
                );
            }
        }
    }

    #[test]
    fn remove_client_family_accounts_guards_last_codex_account() {
        // 用临时账号库验证：清理时移除账号必须遵守「不能删到 Codex 代理账号为 0」。
        let dir = std::env::temp_dir().join(format!(
            "deecodex-clean-config-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let codex_cli = client_app_spec("codex_cli").unwrap();

        // 只有一个 codex 账号时，移除被保护拒绝。
        let store = serde_json::json!({
            "active_account_id": "cx1",
            "accounts": [{
                "id": "cx1",
                "name": "Codex 主账号",
                "client_kind": "codex",
                "client_surface": "cli",
                "provider": "deepseek",
                "upstream": "https://api.deepseek.com/v1",
                "api_key": "",
            }]
        });
        std::fs::write(dir.join("accounts.json"), store.to_string()).unwrap();
        let result = remove_client_family_accounts(&dir, codex_cli);
        assert!(result.is_err(), "最后一个 Codex 账号应被保护，不允许移除");
        assert!(
            result.unwrap_err().contains("最后一个 Codex"),
            "保护提示应说明不能删除最后一个 Codex 代理账号"
        );

        // 两个 codex 账号时，移除 cli 家族只删 cli，保留 desktop，active 切到剩余 codex。
        let store = serde_json::json!({
            "active_account_id": "cx1",
            "active_id": "cx1",
            "accounts": [
                {
                    "id": "cx1",
                    "name": "Codex CLI",
                    "client_kind": "codex",
                    "client_surface": "cli",
                    "provider": "deepseek",
                    "upstream": "https://api.deepseek.com/v1",
                    "api_key": "",
                },
                {
                    "id": "cx2",
                    "name": "Codex 桌面",
                    "client_kind": "codex",
                    "client_surface": "desktop",
                    "provider": "minimax",
                    "upstream": "https://api.minimaxi.com/v1",
                    "api_key": "",
                }
            ]
        });
        std::fs::write(dir.join("accounts.json"), store.to_string()).unwrap();
        let removed = remove_client_family_accounts(&dir, codex_cli).unwrap();
        assert_eq!(removed.len(), 1, "只应移除 codex cli 家族账号");
        assert_eq!(removed[0]["id"], "cx1");
        let after = deecodex::accounts::load_accounts(&dir);
        assert_eq!(after.accounts.len(), 1, "应只剩 codex desktop 账号");
        assert_eq!(after.accounts[0].id, "cx2");
        assert_eq!(
            after.active_account_id.as_deref(),
            Some("cx2"),
            "删除 active 后应切到剩余 Codex 账号"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unmanaged_clients_skip_configure_step() {
        // 未安装 → install；已安装且不要求账号 → 直接 launch，不再卡在 configure。
        assert_eq!(lifecycle_next_action(false, false, false, false), "install");
        assert_eq!(lifecycle_next_action(true, false, false, false), "launch");
        assert_eq!(lifecycle_next_action(true, false, true, false), "running");
        // 受管客户端仍保留 configure 步骤。
        assert_eq!(lifecycle_next_action(true, false, false, true), "configure");
    }

    #[test]
    fn native_unconfigured_clients_route_to_configure() {
        // 原生客户端（pi/dsh/opencode/workbuddy）：已安装但未写入原生配置 → configure。
        assert_eq!(
            lifecycle_next_action_with_native(true, true, false, false, false, false),
            "configure"
        );
        // 未安装仍优先 install。
        assert_eq!(
            lifecycle_next_action_with_native(false, true, false, false, false, false),
            "install"
        );
        // 已配置的原生客户端 → launch / running。
        assert_eq!(
            lifecycle_next_action_with_native(true, true, true, false, false, false),
            "launch"
        );
        assert_eq!(
            lifecycle_next_action_with_native(true, true, true, false, true, false),
            "running"
        );
        // 非原生客户端不受影响（保持原行为）。
        assert_eq!(
            lifecycle_next_action_with_native(true, false, false, false, false, false),
            "launch"
        );
    }

    #[test]
    fn client_primary_account_shares_family_account_and_ignores_unmanaged_kinds() {
        fn store(json: serde_json::Value) -> deecodex::accounts::AccountStore {
            serde_json::from_value(json).unwrap()
        }

        let hermes_store = store(json!({
            "version": 1,
            "accounts": [{
                "id": "h1",
                "name": "Hermes · DeepSeek",
                "provider": "deepseek",
                "client_kind": "hermes",
                "client_surface": "cli",
                "upstream": "https://api.deepseek.com/v1",
                "api_key": "sk-test",
                "last_applied_at": 1700000000
            }]
        }));
        // hermes_desktop 复用 hermes CLI 账号：即使账号是 cli 形态也应命中。
        assert_eq!(
            client_primary_account(&hermes_store, client_app_spec("hermes_desktop").unwrap())
                .map(|a| a.id),
            Some("h1".to_string())
        );
        assert_eq!(
            client_primary_account(&hermes_store, client_app_spec("hermes").unwrap()).map(|a| a.id),
            Some("h1".to_string())
        );

        let codex_cli_only = store(json!({
            "version": 1,
            "accounts": [{
                "id": "cx1",
                "name": "Codex · OpenCode",
                "provider": "opencode",
                "client_kind": "codex",
                "client_surface": "cli",
                "upstream": "https://opencode.ai/zen/go/v1",
                "api_key": "sk-test",
                "last_applied_at": 1700000000
            }]
        }));
        // dsh 不属于 DEX AI 账号体系：即使存在 Codex 账号也不能误匹配成它。
        assert!(client_primary_account(&codex_cli_only, client_app_spec("dsh").unwrap()).is_none());
        assert!(
            client_primary_account(&codex_cli_only, client_app_spec("opencode").unwrap()).is_none()
        );
        // codex_desktop 需要 desktop 形态账号；仅有 cli 账号时不应命中。
        assert!(
            client_primary_account(&codex_cli_only, client_app_spec("codex_desktop").unwrap())
                .is_none()
        );
        assert!(
            client_primary_account(&codex_cli_only, client_app_spec("codex_cli").unwrap())
                .is_some()
        );
    }

    #[test]
    fn quick_configure_deepseek_applies_all_client_configs() {
        use deecodex::accounts::{Account, AccountClientKind, AccountClientSurface};

        let temp = tempfile::tempdir().expect("创建临时目录失败");
        let root = temp.path();

        // 与前端 QuickConfig 默认 DeepSeek 模板保持一致：
        // claude_code / hermes 走官方 Anthropic 端点（DeepSeek 文档 api-docs.deepseek.com/guides/anthropic_api/），
        // 其余走 OpenAI Chat 兼容 v1。
        let cases: Vec<(&str, AccountClientKind, serde_json::Value, &str)> = vec![
            (
                "claude_code",
                AccountClientKind::ClaudeCode,
                serde_json::json!({
                    "config_path": root.join("claude-settings.json"),
                }),
                "https://api.deepseek.com/anthropic",
            ),
            (
                "hermes",
                AccountClientKind::Hermes,
                serde_json::json!({
                    "config_path": root.join("hermes-config.yaml"),
                    "env_path": root.join("hermes-env"),
                }),
                "https://api.deepseek.com/anthropic",
            ),
            (
                "generic_client",
                AccountClientKind::GenericClient,
                serde_json::json!({
                    "config_path": root.join("generic-client-env"),
                }),
                "https://api.deepseek.com/v1",
            ),
        ];

        for (label, kind, client_options, upstream) in cases {
            let mut account: Account = serde_json::from_value(serde_json::json!({
                "id": "test-deepseek",
                "name": format!("{label} · DeepSeek"),
                "provider": "deepseek",
                "client_kind": kind,
                "client_surface": AccountClientSurface::Cli,
                "upstream": upstream,
                "api_key": "sk-test-deepseek",
                "default_model": "deepseek-v4-pro[1m]",
                "client_options": client_options,
            }))
            .expect("构造 DeepSeek 测试账号失败");

            let report = deecodex::client_integrations::apply(&mut account, false)
                .expect(&format!("{label} 写入配置不应报错"));
            assert!(
                report.ok,
                "{label} DeepSeek 模板配置应成功: {:?}",
                report.diagnostics
            );
            assert!(
                account.last_applied_at.is_some(),
                "{label} 应用成功应记录 last_applied_at"
            );
        }

        // 校验三类配置确实写入了 DeepSeek 上游与密钥。
        let claude_settings = std::fs::read_to_string(root.join("claude-settings.json")).unwrap();
        assert!(claude_settings.contains("https://api.deepseek.com/anthropic"));
        assert!(claude_settings.contains("sk-test-deepseek"));

        let hermes_yaml = std::fs::read_to_string(root.join("hermes-config.yaml")).unwrap();
        assert!(hermes_yaml.contains("https://api.deepseek.com/anthropic"));
        assert!(hermes_yaml.contains("deepseek-v4-pro[1m]"));
        let hermes_env = std::fs::read_to_string(root.join("hermes-env")).unwrap();
        assert!(hermes_env.contains("sk-test-deepseek"));

        let generic_env = std::fs::read_to_string(root.join("generic-client-env")).unwrap();
        assert!(generic_env.contains("https://api.deepseek.com/v1"));
        assert!(generic_env.contains("sk-test-deepseek"));
    }

    #[test]
    fn native_config_detection_reads_pi_and_dsh() {
        let temp = tempfile::tempdir().expect("创建临时目录失败");
        let home = temp.path();

        // pi 未配置：auth.json 空 + settings 无 provider/model
        let pi_agent = home.join(".pi").join("agent");
        std::fs::create_dir_all(&pi_agent).unwrap();
        std::fs::write(pi_agent.join("auth.json"), "{}").unwrap();
        std::fs::write(pi_agent.join("settings.json"), r#"{"theme":"dark"}"#).unwrap();
        assert!(!pi_native_config_configured(home));

        // pi 已配置：auth.json 有 api_key + settings 有 defaultProvider/defaultModel
        std::fs::write(
            pi_agent.join("auth.json"),
            r#"{"minimax-cn":{"type":"api_key","key":"sk-test"}}"#,
        )
        .unwrap();
        std::fs::write(
            pi_agent.join("settings.json"),
            r#"{"defaultProvider":"minimax-cn","defaultModel":"MiniMax-M3"}"#,
        )
        .unwrap();
        assert!(pi_native_config_configured(home));

        // dsh 未配置：无 credentials
        let dsh_dir = home.join(".dsh");
        std::fs::create_dir_all(&dsh_dir).unwrap();
        std::fs::write(
            dsh_dir.join("settings.yaml"),
            "ui-onboarding:\n  welcomeNoticeVersion: 'x'\n",
        )
        .unwrap();
        assert!(!dsh_native_config_configured(home));

        // dsh 已配置：settings 有 agent-default-model + credentials 存在
        std::fs::write(dsh_dir.join(".credentials.yaml"), "deepseek: sk-test\n").unwrap();
        std::fs::write(
            dsh_dir.join("settings.yaml"),
            "ui-onboarding:\n  welcomeNoticeVersion: 'x'\nagent-default-model:\n  provider: deepseek-official\n  model: deepseek-v4-flash\n",
        )
        .unwrap();
        assert!(dsh_native_config_configured(home));

        // opencode 未配置：无 auth.json
        let opencode_share = home.join(".local").join("share").join("opencode");
        assert!(!opencode_native_config_configured(home));
        // opencode 已配置：auth.json 有 provider key
        std::fs::create_dir_all(&opencode_share).unwrap();
        std::fs::write(
            opencode_share.join("auth.json"),
            r#"{"opencode-go":{"type":"api","key":"sk-test"}}"#,
        )
        .unwrap();
        assert!(opencode_native_config_configured(home));
        // opencode 已配置（快速配置路径）：无 auth.json，但 opencode.json 有 provider options.apiKey
        let opencode_config_dir = home.join(".config").join("opencode");
        std::fs::create_dir_all(&opencode_config_dir).unwrap();
        std::fs::write(
            opencode_config_dir.join("opencode.json"),
            r#"{"provider":{"minimax":{"options":{"baseURL":"https://api.minimaxi.com/v1","apiKey":"sk-minimax"}}}}"#,
        )
        .unwrap();
        // 先删 auth.json，模拟快速配置只写了 opencode.json 的场景
        std::fs::remove_file(opencode_share.join("auth.json")).unwrap();
        assert!(opencode_native_config_configured(home));

        // 只对 pi/dsh/opencode 启用原生检测
        assert!(native_client_config_detected("pi"));
        assert!(native_client_config_detected("dsh"));
        assert!(native_client_config_detected("opencode"));
        assert!(native_client_config_detected("opencode_desktop"));
        assert!(native_client_config_detected("workbuddy"));

        // workbuddy 未配置：无 models.json
        assert!(!workbuddy_native_config_configured(home));
        // workbuddy 已配置：models.json 有 apiKey
        let wb_dir = home.join(".workbuddy");
        std::fs::create_dir_all(&wb_dir).unwrap();
        std::fs::write(
            wb_dir.join("models.json"),
            r#"[{"id":"MiniMax-M3","name":"MiniMax-M3","vendor":"MiniMax","url":"https://api.minimaxi.com/v1/chat/completions","apiKey":"sk-test"}]"#,
        )
        .unwrap();
        assert!(workbuddy_native_config_configured(home));
    }

    #[test]
    fn opencode_jsonc_comments_preserved_and_parse_failure_aborts() {
        use deecodex::accounts::{Account, AccountClientKind, AccountClientSurface};

        let temp = tempfile::tempdir().expect("创建临时目录失败");
        let home = temp.path();
        let config_dir = home.join(".config").join("opencode");
        std::fs::create_dir_all(&config_dir).unwrap();

        // jsonc 含行注释与块注释、以及既有 provider：应解析成功并保留既有配置。
        let jsonc_path = config_dir.join("opencode.jsonc");
        std::fs::write(
            &jsonc_path,
            r#"{
  // 已有主题设置
  "theme": "dark",
  "provider": {
    /* 块注释：既有 provider */
    "existing": { "options": { "baseURL": "https://old.example/v1", "apiKey": "sk-old" } }
  }
}
"#,
        )
        .unwrap();
        // 检测：带注释的配置也应识别为已配置（有 key/baseURL）。
        assert!(opencode_native_config_configured(home));

        let account: Account = serde_json::from_value(json!({
            "id": "oc-test",
            "name": "OpenCode",
            "provider": "deepseek",
            "client_kind": AccountClientKind::GenericClient,
            "client_surface": AccountClientSurface::Cli,
            "upstream": "https://api.deepseek.com/v1",
            "api_key": "sk-deepseek",
            "default_model": "deepseek-v4-pro",
        }))
        .unwrap();
        let report =
            write_opencode_native_config(home, &account).expect("含注释 jsonc 写入不应失败");
        assert!(report["config_path"]
            .as_str()
            .unwrap()
            .ends_with("opencode.jsonc"));

        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&jsonc_path).unwrap()).unwrap();
        // 既有 provider 与主题保留，新 provider 追加。
        assert_eq!(written["theme"], "dark");
        assert!(written["provider"]["existing"].is_object());
        assert_eq!(
            written["provider"]["deepseek"]["options"]["apiKey"],
            "sk-deepseek"
        );
        // 覆盖前应生成备份。
        let backups: Vec<_> = std::fs::read_dir(&config_dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("opencode.jsonc.bak-"))
            .collect();
        assert_eq!(backups.len(), 1, "覆盖前应生成备份");

        // 损坏的 jsonc：应报错且不覆盖原文件。
        std::fs::write(&jsonc_path, "{  // 未闭合").unwrap();
        let before = std::fs::read_to_string(&jsonc_path).unwrap();
        let error = write_opencode_native_config(home, &account).unwrap_err();
        assert!(error.contains("无法解析"));
        assert_eq!(
            std::fs::read_to_string(&jsonc_path).unwrap(),
            before,
            "解析失败不得覆盖原文件"
        );
    }

    #[test]
    fn quick_configure_writes_pi_and_dsh_native_config() {
        use deecodex::accounts::{Account, AccountClientKind, AccountClientSurface};

        let temp = tempfile::tempdir().expect("创建临时目录失败");
        let home = temp.path();

        fn account(provider: &str, api_key: &str, default_model: &str) -> Account {
            serde_json::from_value(json!({
                "id": "native-test",
                "name": "Native Test",
                "provider": provider,
                "client_kind": AccountClientKind::GenericClient,
                "client_surface": AccountClientSurface::Cli,
                "upstream": "https://example.com/v1",
                "api_key": api_key,
                "default_model": default_model,
            }))
            .unwrap()
        }

        // pi + minimax → minimax-cn，模型后缀去掉 [1m]
        let pi = account("minimax", "sk-minimax", "MiniMax-M3[1m]");
        let report = write_pi_native_config(home, &pi).expect("pi 写入不应失败");
        assert_eq!(report["provider"], "minimax-cn");
        let auth: Value = serde_json::from_str(
            &std::fs::read_to_string(home.join(".pi").join("agent").join("auth.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(auth["minimax-cn"]["type"], "api_key");
        assert_eq!(auth["minimax-cn"]["key"], "sk-minimax");
        let settings: Value = serde_json::from_str(
            &std::fs::read_to_string(home.join(".pi").join("agent").join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(settings["defaultProvider"], "minimax-cn");
        assert_eq!(settings["defaultModel"], "MiniMax-M3");

        // dsh + deepseek → deepseek-official，写 settings.yaml + .credentials.yaml
        let dsh = account("deepseek", "sk-deepseek", "deepseek-v4-flash");
        write_dsh_native_config(home, &dsh).expect("dsh 写入不应失败");
        let settings_text =
            std::fs::read_to_string(home.join(".dsh").join("settings.yaml")).unwrap();
        assert!(settings_text.contains("deepseek-official"));
        assert!(settings_text.contains("deepseek-v4-flash"));
        let creds_text =
            std::fs::read_to_string(home.join(".dsh").join(".credentials.yaml")).unwrap();
        assert!(creds_text.contains("DEEPSEEK_API_KEY: sk-deepseek"));

        // dsh + minimax → 走 llm-pi-ai 路由 minimax-cn + MINIMAX_CN_API_KEY
        let dsh_minimax = account("minimax", "sk-minimax", "MiniMax-M3");
        let report = write_dsh_native_config(home, &dsh_minimax).expect("dsh minimax 写入不应失败");
        assert_eq!(report["provider"], "minimax-cn");
        assert_eq!(report["credential_env"], "MINIMAX_CN_API_KEY");
        let settings_text =
            std::fs::read_to_string(home.join(".dsh").join("settings.yaml")).unwrap();
        assert!(settings_text.contains("llm-pi-ai"));
        assert!(settings_text.contains("minimax-cn"));
        assert!(settings_text.contains("apiKeyEnv: MINIMAX_CN_API_KEY"));
        let creds_text =
            std::fs::read_to_string(home.join(".dsh").join(".credentials.yaml")).unwrap();
        assert!(creds_text.contains("MINIMAX_CN_API_KEY: sk-minimax"));

        // dsh + openai（非内置路由）→ 走 llm-pi-ai 完整声明：api=openai-completions + baseURL + models
        let dsh_openai = account("openai", "sk-openai", "gpt-5");
        let report = write_dsh_native_config(home, &dsh_openai).expect("dsh openai 写入不应失败");
        assert_eq!(report["provider"], "openai");
        assert_eq!(report["credential_env"], "OPENAI_API_KEY");
        let settings_text =
            std::fs::read_to_string(home.join(".dsh").join("settings.yaml")).unwrap();
        assert!(settings_text.contains("api: openai-completions"));
        assert!(settings_text.contains("baseURL:"));
        assert!(settings_text.contains("id: gpt-5"));
        let creds_text =
            std::fs::read_to_string(home.join(".dsh").join(".credentials.yaml")).unwrap();
        assert!(creds_text.contains("OPENAI_API_KEY: sk-openai"));

        // 3 种端点格式推导：mimo → openai-responses；/anthropic 上游 → anthropic-messages
        let dsh_mimo = account("mimo", "sk-mimo", "mimo-v2.5-pro");
        write_dsh_native_config(home, &dsh_mimo).unwrap();
        let settings_text =
            std::fs::read_to_string(home.join(".dsh").join("settings.yaml")).unwrap();
        assert!(settings_text.contains("api: openai-responses"));
        // 带 /anthropic 上游但 provider=deepseek 时走内置路由，此处直接验证 api 推导函数。
        assert_eq!(
            dsh_pi_ai_api("deepseek", "https://api.deepseek.com/anthropic"),
            "anthropic-messages"
        );
        assert_eq!(
            dsh_pi_ai_api("mimo", "https://token-plan-cn.xiaomimimo.com/v1"),
            "openai-responses"
        );
        assert_eq!(
            dsh_pi_ai_api("kimi", "https://api.moonshot.cn/v1"),
            "openai-completions"
        );

        // opencode + deepseek → 写入 ~/.config/opencode/opencode.json 的 provider 路由
        let opencode = account("deepseek", "sk-opencode-deepseek", "deepseek-v4-pro[1m]");
        write_opencode_native_config(home, &opencode).expect("opencode 写入不应失败");
        let cfg: Value = serde_json::from_str(
            &std::fs::read_to_string(home.join(".config").join("opencode").join("opencode.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            cfg["provider"]["deepseek"]["options"]["apiKey"],
            "sk-opencode-deepseek"
        );
        assert_eq!(
            cfg["provider"]["deepseek"]["options"]["baseURL"],
            "https://example.com/v1"
        );
        // 模型名去掉 [1m] 后缀
        assert!(cfg["provider"]["deepseek"]["models"]["deepseek-v4-pro"].is_object());

        // workbuddy + minimax → 写入 ~/.workbuddy/models.json 的自定义模型项
        let wb = account("minimax", "sk-wb-minimax", "MiniMax-M3[1m]");
        let report = write_workbuddy_native_config(home, &wb).expect("workbuddy 写入不应失败");
        assert_eq!(report["model"], "MiniMax-M3");
        let models: Value = serde_json::from_str(
            &std::fs::read_to_string(home.join(".workbuddy").join("models.json")).unwrap(),
        )
        .unwrap();
        assert!(models.is_array());
        let entry = models[0].as_object().unwrap();
        assert_eq!(entry["id"], "MiniMax-M3");
        assert_eq!(entry["vendor"], "MiniMax");
        assert_eq!(entry["url"], "https://example.com/v1/chat/completions");
        assert_eq!(entry["apiKey"], "sk-wb-minimax");
        assert_eq!(entry["supportsToolCall"], true);
    }

    #[test]
    fn desktop_process_names_do_not_count_as_cli() {
        assert!(status_command_is_codex_desktop("Codex"));
        assert!(status_command_is_claude_desktop("Claude"));
        assert!(status_command_is_codex_desktop(
            "/Applications/Codex.app/Contents/Resources/codex app-server --listen stdio://"
        ));
        // 新版 Codex 桌面：`ChatGPT.app` 但内置 CLI 仍是 `codex`。
        assert!(status_command_is_codex_desktop(
            "/Applications/ChatGPT.app/Contents/Resources/codex app-server --listen stdio://"
        ));
        // 新版 Codex 桌面的 helper bundle 仍叫 `Codex Helper`（位于 `Codex Helper.app`）。
        assert!(status_command_is_codex_desktop(
            "/Applications/ChatGPT.app/Contents/Frameworks/Codex Helper.app/Contents/MacOS/Codex Helper (Renderer) --type=renderer"
        ));
        // 反向断言：consumer 版 `ChatGPT.app`（bundle id com.openai.chatgpt）的进程不能被误识别为 Codex 桌面。
        // 主进程 `ChatGPT` 在新版 Codex 桌面与 consumer 共用同一可执行文件名，单凭命令行无法区分；为安全计收紧为不识别。
        assert!(!status_command_is_codex_desktop("ChatGPT"));
        assert!(!status_command_is_codex_desktop(
            "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT --type=renderer"
        ));
        // consumer 的 helper bundle 是 `ChatGPT Helper.app`，路径不含 `/codex`，应被剔除。
        assert!(!status_command_is_codex_desktop("ChatGPT Helper"));
        assert!(!status_command_is_codex_desktop(
            "/Applications/ChatGPT.app/Contents/Frameworks/ChatGPT Helper.app/Contents/MacOS/ChatGPT Helper (Renderer) --type=renderer"
        ));
        assert!(!status_command_is_codex_cli(
            "/Applications/Codex.app/Contents/Resources/codex app-server --listen stdio://"
        ));
        assert!(!status_command_is_codex_cli(
            "/Applications/ChatGPT.app/Contents/Resources/codex app-server --listen stdio://"
        ));
        assert!(!status_command_is_codex_cli(
            "/Users/me/project/target/debug/deecodex-gui"
        ));
        assert!(!status_command_is_codex_cli(
            "/Users/me/.codex/plugins/example/index.js"
        ));
        assert!(status_command_is_codex_cli("codex"));
        assert!(status_command_is_codex_cli("/usr/local/bin/codex"));
        assert!(status_command_is_codex_cli("codex --model gpt-5"));
        assert!(status_command_is_codex_cli(
            r#""C:\Users\me\AppData\Roaming\npm\codex.cmd" --model gpt-5"#
        ));
        assert!(status_command_is_codex_desktop(
            r#"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0_x64__abc\app\Codex.exe"#
        ));
        assert!(status_command_is_codex_desktop(
            r#"C:\Users\me\AppData\Local\Programs\Codex\Codex.exe --type=renderer"#
        ));
        assert!(!status_command_is_codex_cli(
            r#"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0_x64__abc\app\Codex.exe"#
        ));
        assert!(!status_command_is_claude_cli("Claude"));
        assert!(status_command_is_claude_cli("claude"));
        assert!(status_command_is_claude_cli("/usr/local/bin/claude"));
    }

    #[test]
    fn opencode_desktop_and_workbuddy_process_matching() {
        // OpenCode 桌面：主进程 / Helper 算桌面，不算 CLI。
        assert!(status_command_is_opencode_desktop(
            "/Applications/OpenCode.app/Contents/MacOS/OpenCode"
        ));
        assert!(status_command_is_opencode_desktop("OpenCode"));
        assert!(status_command_is_opencode_desktop(
            "/Applications/OpenCode.app/Contents/Frameworks/OpenCode Helper.app/Contents/MacOS/OpenCode Helper --type=utility"
        ));
        assert!(status_command_is_opencode_desktop(
            "/Applications/OpenCode.app/Contents/Frameworks/OpenCode Helper (Renderer).app/Contents/MacOS/OpenCode Helper (Renderer) --type=renderer"
        ));
        assert!(status_command_is_opencode_desktop(
            r#"C:\Users\me\AppData\Local\Programs\OpenCode\OpenCode.exe --type=renderer"#
        ));
        // OpenCode CLI：独立安装的 `opencode` 可执行文件不算桌面。
        assert!(!status_command_is_opencode_desktop(
            "/Users/me/.local/opencode/1.18.18/opencode serve --port 41376"
        ));
        assert!(!status_command_is_opencode_desktop("opencode"));
        assert!(!status_command_is_opencode_desktop(
            "/usr/local/bin/opencode --version"
        ));
        assert!(!status_command_is_claude_desktop(
            "/Applications/OpenCode.app/Contents/MacOS/OpenCode"
        ));

        // OpenCode CLI 匹配必须排除桌面进程（桌面主进程可执行名大写 `OpenCode`）。
        let cli_spec = client_app_spec_or_err("opencode").unwrap();
        let desktop_spec = client_app_spec_or_err("opencode_desktop").unwrap();
        assert!(client_process_matches_spec(
            cli_spec,
            "/Users/me/.local/opencode/1.18.18/opencode serve --port 41376"
        ));
        assert!(!client_process_matches_spec(
            cli_spec,
            "/Applications/OpenCode.app/Contents/MacOS/OpenCode"
        ));
        assert!(!client_process_matches_spec(
            cli_spec,
            "/Applications/OpenCode.app/Contents/Frameworks/OpenCode Helper.app/Contents/MacOS/OpenCode Helper --type=utility"
        ));
        assert!(client_process_matches_spec(
            desktop_spec,
            "/Applications/OpenCode.app/Contents/MacOS/OpenCode"
        ));
        assert!(client_process_matches_spec(
            desktop_spec,
            "/Applications/OpenCode.app/Contents/Frameworks/OpenCode Helper (Renderer).app/Contents/MacOS/OpenCode Helper (Renderer) --type=renderer"
        ));
        assert!(!client_process_matches_spec(
            desktop_spec,
            "/Users/me/.local/opencode/1.18.18/opencode serve --port 41376"
        ));

        // WorkBuddy 桌面：主进程（Electron）/ Helper 都算桌面。
        let wb_spec = client_app_spec_or_err("workbuddy").unwrap();
        // 真实安装名（Mac mini 实测）：/Applications/WorkBuddy AI.app
        assert!(status_command_is_workbuddy_desktop(
            "/Applications/WorkBuddy AI.app/Contents/MacOS/Electron"
        ));
        assert!(status_command_is_workbuddy_desktop(
            "/Applications/WorkBuddy AI.app/Contents/Frameworks/WorkBuddy AI Helper.app/Contents/MacOS/WorkBuddy AI Helper --type=utility"
        ));
        assert!(status_command_is_workbuddy_desktop(
            "/Applications/WorkBuddy AI.app/Contents/Frameworks/WorkBuddy AI Helper (Renderer).app/Contents/MacOS/WorkBuddy AI Helper (Renderer) --type=renderer"
        ));
        assert!(client_process_matches_spec(
            wb_spec,
            "/Applications/WorkBuddy AI.app/Contents/MacOS/Electron"
        ));
        assert!(client_process_matches_spec(
            wb_spec,
            "/Applications/WorkBuddy AI.app/Contents/Frameworks/WorkBuddy AI Helper (Renderer).app/Contents/MacOS/WorkBuddy AI Helper (Renderer) --type=renderer"
        ));
        // 旧安装名（WorkBuddy.app）继续兼容。
        assert!(status_command_is_workbuddy_desktop(
            "/Applications/WorkBuddy.app/Contents/MacOS/Electron"
        ));
        assert!(status_command_is_workbuddy_desktop("WorkBuddy"));
        assert!(status_command_is_workbuddy_desktop(
            "/Applications/WorkBuddy.app/Contents/Frameworks/WorkBuddy Helper.app/Contents/MacOS/WorkBuddy Helper --type=utility"
        ));
        assert!(status_command_is_workbuddy_desktop(
            "/Applications/WorkBuddy.app/Contents/Frameworks/WorkBuddy Helper (Renderer).app/Contents/MacOS/WorkBuddy Helper (Renderer) --type=renderer"
        ));
        assert!(status_command_is_workbuddy_desktop(
            "/Applications/WorkBuddy.app/Contents/Resources/app.asar/cli/bin/codebuddy --serve --port 65243"
        ));
        assert!(client_process_matches_spec(
            wb_spec,
            "/Applications/WorkBuddy.app/Contents/MacOS/Electron"
        ));
        assert!(client_process_matches_spec(
            wb_spec,
            "/Applications/WorkBuddy.app/Contents/Frameworks/WorkBuddy Helper (Renderer).app/Contents/MacOS/WorkBuddy Helper (Renderer) --type=renderer"
        ));
        // 独立安装的 CodeBuddy CLI（不在 WorkBuddy.app 内）不应算作 WorkBuddy 桌面。
        assert!(!status_command_is_workbuddy_desktop(
            "/Users/me/.codebuddy/bin/codebuddy --serve"
        ));
        assert!(!status_command_is_workbuddy_desktop(
            "/usr/local/bin/codebuddy"
        ));
        assert!(!client_process_matches_spec(
            wb_spec,
            "/Users/me/.codebuddy/bin/codebuddy --serve"
        ));
        // 其它 Electron 应用（如 ChatGPT/Claude 桌面）不能被误判为 WorkBuddy。
        assert!(!status_command_is_workbuddy_desktop(
            "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT"
        ));
    }

    #[test]
    fn codex_desktop_spec_covers_chatgpt_app() {
        let spec = client_app_spec_or_err("codex_desktop").unwrap();
        // 安装识别同时覆盖 `Codex.app` 与新版 `ChatGPT.app`。
        assert!(spec.mac_desktop_paths.contains(&"/Applications/Codex.app"));
        assert!(spec
            .mac_desktop_paths
            .contains(&"/Applications/ChatGPT.app"));
        assert!(spec.mac_desktop_paths.contains(&"~/Applications/Codex.app"));
        assert!(spec
            .mac_desktop_paths
            .contains(&"~/Applications/ChatGPT.app"));
        // 进程识别只走 Codex 桌面独有的可执行名特征，避免与 consumer ChatGPT.app 的主进程 `ChatGPT` 冲突。
        assert!(spec.process_names.contains(&"Codex"));
        assert!(spec.process_names.contains(&"codex"));
        assert!(!spec.process_names.contains(&"ChatGPT"));
        // 启动/退出按 Bundle ID 走，避免 `open -a Codex` 找不到 `ChatGPT.app`。
        assert_eq!(spec.mac_bundle_ids, &[CODEX_DESKTOP_BUNDLE_ID]);
    }

    #[test]
    fn hermes_gateway_counts_as_cli_runtime() {
        assert!(status_command_is_hermes_cli("/Users/me/.local/bin/hermes"));
        assert!(status_command_is_hermes_cli(
            "/Library/Frameworks/Python.framework/Versions/3.11/Resources/Python.app/Contents/MacOS/Python /Users/me/.local/bin/hermes"
        ));
        // hermes 网关（launchd KeepAlive 服务）是 Hermes CLI 的常驻形态，计入运行态，
        // 保证「强制关闭」能发现并停止网关（hermes gateway stop → launchctl bootout）。
        assert!(status_command_is_hermes_cli(
            "/Library/Frameworks/Python.framework/Versions/3.11/Resources/Python.app/Contents/MacOS/Python -m hermes_cli.main gateway run --replace"
        ));
    }

    #[test]
    fn hermes_desktop_process_matching() {
        let spec = client_app_spec_or_err("hermes_desktop").unwrap();
        // Hermes 桌面：主进程 / Helper 都算桌面。
        assert!(status_command_is_hermes_desktop(
            "/Applications/Hermes.app/Contents/MacOS/Hermes"
        ));
        assert!(status_command_is_hermes_desktop("Hermes"));
        assert!(status_command_is_hermes_desktop(
            "/Applications/Hermes.app/Contents/Frameworks/Hermes Helper.app/Contents/MacOS/Hermes Helper --type=utility"
        ));
        assert!(status_command_is_hermes_desktop(
            "/Applications/Hermes.app/Contents/Frameworks/Hermes Helper (Renderer).app/Contents/MacOS/Hermes Helper (Renderer) --type=renderer"
        ));
        assert!(status_command_is_hermes_desktop(
            r#"C:\Users\me\AppData\Local\Programs\Hermes\Hermes.exe --type=renderer"#
        ));
        // CLI（小写 hermes，Python 脚本）不算桌面。
        assert!(!status_command_is_hermes_desktop(
            "/Users/me/.local/bin/hermes"
        ));
        assert!(!status_command_is_hermes_desktop("hermes"));
        assert!(!status_command_is_hermes_desktop(
            "/Users/me/.local/bin/hermes --version"
        ));
        // 进程匹配：桌面 spec 命中桌面进程、剔除 CLI；CLI spec 命中 CLI、剔除桌面进程。
        let cli_spec = client_app_spec_or_err("hermes").unwrap();
        assert!(client_process_matches_spec(
            cli_spec,
            "/Users/me/.local/bin/hermes"
        ));
        assert!(!client_process_matches_spec(
            cli_spec,
            "/Applications/Hermes.app/Contents/MacOS/Hermes"
        ));
        assert!(!client_process_matches_spec(
            cli_spec,
            "/Applications/Hermes.app/Contents/Frameworks/Hermes Helper (Renderer).app/Contents/MacOS/Hermes Helper (Renderer) --type=renderer"
        ));
        assert!(client_process_matches_spec(
            spec,
            "/Applications/Hermes.app/Contents/MacOS/Hermes"
        ));
        assert!(client_process_matches_spec(
            spec,
            "/Applications/Hermes.app/Contents/Frameworks/Hermes Helper (Renderer).app/Contents/MacOS/Hermes Helper (Renderer) --type=renderer"
        ));
        assert!(!client_process_matches_spec(
            spec,
            "/Users/me/.local/bin/hermes"
        ));
    }

    #[test]
    fn force_quit_rechecks_until_late_codex_helpers_are_gone() {
        let spec = client_app_spec_or_err("codex_desktop").unwrap();
        let first_round = (1..=7)
            .map(|idx| fake_process(&format!("10{idx}"), "Codex Helper"))
            .collect::<Vec<_>>();
        let second_round = (1..=3)
            .map(|idx| {
                fake_process(
                    &format!("20{idx}"),
                    "/Applications/Codex.app/Contents/Resources/codex app-server --listen stdio://",
                )
            })
            .collect::<Vec<_>>();
        let mut batches = VecDeque::from([first_round, second_round, Vec::new(), Vec::new()]);
        let mut killed_pids = Vec::new();

        let result = force_quit_client_processes(
            spec,
            || batches.pop_front().unwrap_or_default(),
            |pid| {
                killed_pids.push(pid.to_string());
                Ok(())
            },
            || {},
        );

        assert_eq!(result["ok"], json!(true));
        assert_eq!(result["rounds"], json!(2));
        assert_eq!(result["remaining"].as_array().unwrap().len(), 0);
        assert_eq!(result["killed"].as_array().unwrap().len(), 10);
        assert_eq!(killed_pids.len(), 10);
    }

    #[test]
    fn force_quit_reports_remaining_after_max_rounds() {
        let spec = client_app_spec_or_err("codex_desktop").unwrap();
        let mut killed_pids = Vec::new();

        let result = force_quit_client_processes(
            spec,
            || vec![fake_process("301", "Codex Helper")],
            |pid| {
                killed_pids.push(pid.to_string());
                Ok(())
            },
            || {},
        );

        assert_eq!(result["ok"], json!(false));
        assert_eq!(result["rounds"], json!(FORCE_QUIT_MAX_ROUNDS));
        assert_eq!(result["remaining"].as_array().unwrap().len(), 1);
        assert_eq!(result["killed"].as_array().unwrap().len(), 1);
        assert_eq!(killed_pids.len(), FORCE_QUIT_MAX_ROUNDS);
    }

    #[test]
    fn terminal_scripts_quote_launch_directories() {
        let cwd = Path::new("/tmp/dee codex/it's ok");
        let mac = mac_terminal_script("codex", Some(cwd));
        assert!(mac.contains("cd '/tmp/dee codex/it'\\\\''s ok' && codex"));

        let win = windows_terminal_script("claude", Some(Path::new(r"C:\Users\A B\repo")));
        assert!(win.contains("Set-Location -LiteralPath 'C:\\Users\\A B\\repo'; claude"));
    }

    #[test]
    fn terminal_scripts_without_launch_directory_do_not_cd() {
        assert_eq!(windows_terminal_script("hermes", None), "hermes");
    }
}
