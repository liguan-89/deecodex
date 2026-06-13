use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tauri::State;

use crate::ServerManager;

#[cfg(target_os = "windows")]
use super::dex_cli::command_exists;
use super::dex_cli::{command_first_line, find_command_path, get_cli_version_flexible};
use super::dex_process::detect_process_instances;

#[derive(Clone, Copy)]
struct ClientAppSpec {
    kind: &'static str,
    label: &'static str,
    account_kind: &'static str,
    surface: &'static str,
    command: Option<&'static str>,
    launch_command: Option<&'static str>,
    launch_requires_cwd: bool,
    desktop_app: Option<&'static str>,
    desktop_exe: Option<&'static str>,
    process_names: &'static [&'static str],
    mac_desktop_paths: &'static [&'static str],
    download_url: Option<&'static str>,
    mac_install_command: Option<&'static str>,
    windows_install_command: Option<&'static str>,
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
        desktop_app: None,
        desktop_exe: None,
        process_names: &["codex"],
        mac_desktop_paths: &[],
        download_url: None,
        mac_install_command: Some("npm install -g @openai/codex"),
        windows_install_command: Some("npm install -g @openai/codex"),
    },
    ClientAppSpec {
        kind: "codex_desktop",
        label: "Codex 桌面",
        account_kind: "codex",
        surface: "desktop",
        command: None,
        launch_command: None,
        launch_requires_cwd: false,
        desktop_app: Some("Codex"),
        desktop_exe: Some("Codex.exe"),
        process_names: &["Codex", "codex"],
        mac_desktop_paths: &["/Applications/Codex.app", "~/Applications/Codex.app"],
        download_url: Some("https://developers.openai.com/codex/app"),
        mac_install_command: None,
        windows_install_command: None,
    },
    ClientAppSpec {
        kind: "claude_cli",
        label: "Claude CLI",
        account_kind: "claude_code",
        surface: "cli",
        command: Some("claude"),
        launch_command: Some("claude"),
        launch_requires_cwd: true,
        desktop_app: None,
        desktop_exe: None,
        process_names: &["claude"],
        mac_desktop_paths: &[],
        download_url: None,
        mac_install_command: Some("npm install -g @anthropic-ai/claude-code"),
        windows_install_command: Some(
            "winget install Anthropic.ClaudeCode || npm install -g @anthropic-ai/claude-code",
        ),
    },
    ClientAppSpec {
        kind: "claude_desktop",
        label: "Claude 桌面",
        account_kind: "claude_code",
        surface: "desktop",
        command: None,
        launch_command: None,
        launch_requires_cwd: false,
        desktop_app: Some("Claude"),
        desktop_exe: Some("Claude.exe"),
        process_names: &["Claude", "claude"],
        mac_desktop_paths: &["/Applications/Claude.app", "~/Applications/Claude.app"],
        download_url: Some("https://claude.ai/download"),
        mac_install_command: None,
        windows_install_command: None,
    },
    ClientAppSpec {
        kind: "openclaw",
        label: "OpenClaw",
        account_kind: "openclaw",
        surface: "cli",
        command: Some("openclaw"),
        launch_command: Some("openclaw tui"),
        launch_requires_cwd: false,
        desktop_app: None,
        desktop_exe: None,
        process_names: &["openclaw"],
        mac_desktop_paths: &[],
        download_url: Some("https://docs.openclaw.ai/install"),
        mac_install_command: Some(
            "curl -fsSL https://openclaw.ai/install.sh | bash -s -- --no-onboard",
        ),
        windows_install_command: Some(
            "& ([scriptblock]::Create((iwr -useb https://openclaw.ai/install.ps1))) -NoOnboard",
        ),
    },
    ClientAppSpec {
        kind: "hermes",
        label: "Hermes",
        account_kind: "hermes",
        surface: "cli",
        command: Some("hermes"),
        launch_command: Some("hermes"),
        launch_requires_cwd: false,
        desktop_app: None,
        desktop_exe: None,
        process_names: &["hermes"],
        mac_desktop_paths: &[],
        download_url: Some("https://www.hermes-ai.net/docs/installation/"),
        mac_install_command: Some("curl -fsSL https://www.hermes-ai.net/install.sh | bash"),
        windows_install_command: Some(
            "python -m pip install --user pipx; python -m pipx install hermes-agent",
        ),
    },
];

fn client_app_spec(kind: &str) -> Option<&'static ClientAppSpec> {
    CLIENT_APP_SPECS.iter().find(|spec| spec.kind == kind)
}

fn client_app_spec_or_err(kind: &str) -> Result<&'static ClientAppSpec, String> {
    client_app_spec(kind).ok_or_else(|| format!("未知客户端: {kind}"))
}

fn client_account_kind_from_slug(kind: &str) -> deecodex::accounts::AccountClientKind {
    match kind {
        "claude_code" => deecodex::accounts::AccountClientKind::ClaudeCode,
        "openclaw" => deecodex::accounts::AccountClientKind::Openclaw,
        "hermes" => deecodex::accounts::AccountClientKind::Hermes,
        "generic_client" => deecodex::accounts::AccountClientKind::GenericClient,
        _ => deecodex::accounts::AccountClientKind::Codex,
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
    let installed = path.is_some();
    let command_for_version = path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| command.to_string());
    let version = if installed {
        get_cli_version_flexible(&command_for_version)
            .or_else(|| command_first_line(&command_for_version, &["--version"]))
            .or_else(|| Some("已安装".into()))
    } else {
        None
    };
    json!({
        "installed": installed,
        "command": command,
        "path": path.map(|p| p.to_string_lossy().to_string()),
        "version": version,
        "error": if installed { Value::Null } else { json!("未检测到命令") },
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
    let output = std::process::Command::new(cmd).args(args).output().ok()?;
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
    let Some(exe) = spec.desktop_exe else {
        return Vec::new();
    };
    let app = spec.desktop_app.unwrap_or("").trim();
    let mut out = Vec::new();
    for key in ["LOCALAPPDATA", "PROGRAMFILES", "PROGRAMFILES(X86)"] {
        if let Ok(base) = std::env::var(key) {
            let base = PathBuf::from(base);
            out.push(base.join("Programs").join(app).join(exe));
            out.push(base.join(app).join(exe));
            out.push(base.join("Anthropic").join(app).join(exe));
            out.push(base.join("OpenAI").join(app).join(exe));
        }
    }
    out
}

fn windows_desktop_app_path(spec: &ClientAppSpec) -> Option<PathBuf> {
    for candidate in windows_desktop_app_candidates(spec) {
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if let Some(exe) = spec.desktop_exe {
        if let Some(path) = windows_command_output("where", &[exe])
            .and_then(|text| text.lines().next().map(str::trim).map(PathBuf::from))
            .filter(|path| path.exists())
        {
            return Some(path);
        }
        let reg_key = format!(r"HKCU\Software\Microsoft\Windows\CurrentVersion\App Paths\{exe}");
        if let Some(text) = windows_command_output("reg", &["query", &reg_key, "/ve"]) {
            for line in text.lines() {
                if let Some((_, value)) = line.split_once("REG_SZ") {
                    let path = PathBuf::from(value.trim().trim_matches('"'));
                    if path.exists() {
                        return Some(path);
                    }
                }
            }
        }
    }
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
    json!({
        "installed": installed,
        "app": spec.desktop_app,
        "path": path.map(|p| p.to_string_lossy().to_string()),
        "download_url": spec.download_url,
        "error": if installed { Value::Null } else { json!("未检测到桌面版应用") },
    })
}

fn client_process_instances_for_spec(spec: &ClientAppSpec) -> Vec<Value> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for name in spec.process_names {
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
            if spec.kind == "codex_desktop" && !status_command_is_codex_desktop(&command) {
                continue;
            }
            if spec.kind == "codex_cli" && !status_command_is_codex_cli(&command) {
                continue;
            }
            if spec.kind == "claude_desktop" && !status_command_is_claude_desktop(&command) {
                continue;
            }
            if spec.kind == "claude_cli" && !status_command_is_claude_cli(&command) {
                continue;
            }
            if spec.kind == "hermes" && !status_command_is_hermes_cli(&command) {
                continue;
            }
            out.push(instance);
        }
    }
    out
}

fn status_command_is_codex_desktop(command: &str) -> bool {
    command.contains("/Codex.app/")
        || command.contains("com.openai.codex")
        || command.contains("Codex Helper")
        || command.contains("Application Support/Codex")
        || command.trim() == "Codex"
}

fn status_command_is_claude_desktop(command: &str) -> bool {
    command.contains("/Claude.app/")
        || command.contains("Claude Helper")
        || command.contains("Application Support/Claude")
        || command.trim() == "Claude"
}

fn status_command_executable_name(command: &str) -> Option<&str> {
    let first = command.split_whitespace().next()?;
    Path::new(first).file_name()?.to_str()
}

fn status_command_has_args(command: &str) -> bool {
    command.split_whitespace().nth(1).is_some()
}

fn status_command_uses_executable(command: &str, name: &str) -> bool {
    let Some(exe) = status_command_executable_name(command) else {
        return false;
    };
    let first = command.split_whitespace().next().unwrap_or("");
    if exe == name && first == name {
        return true;
    }
    if exe == name
        && (first.contains(std::path::MAIN_SEPARATOR) || status_command_has_args(command))
    {
        return true;
    }
    if (exe == "node" || exe == "nodejs")
        && command
            .split_whitespace()
            .any(|part| Path::new(part).file_name().and_then(|v| v.to_str()) == Some(name))
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
    if text.is_empty() || text.contains("hermes_cli.main gateway") {
        return false;
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

fn install_command_for_status(spec: &ClientAppSpec) -> Option<&'static str> {
    install_command_for_current_os(spec)
        .or(spec.mac_install_command)
        .or(spec.windows_install_command)
}

fn client_primary_account(
    store: &deecodex::accounts::AccountStore,
    spec: &ClientAppSpec,
) -> Option<deecodex::accounts::Account> {
    let kind = client_account_kind_from_slug(spec.account_kind);
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
) -> &'static str {
    if !installed {
        "install"
    } else if !account_exists {
        "configure"
    } else if process_running {
        "running"
    } else {
        "launch"
    }
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
    let status_report = account
        .as_ref()
        .map(deecodex::client_integrations::status)
        .and_then(|report| serde_json::to_value(report).ok());
    let account_exists = account.is_some();
    let account_configured = account
        .as_ref()
        .is_some_and(|account| account.client_kind.is_codex() || account.last_applied_at.is_some());
    let config_ok = status_report
        .as_ref()
        .and_then(|report| report.get("ok"))
        .and_then(Value::as_bool)
        .unwrap_or(account_exists);
    let next_action = lifecycle_next_action(installed, account_exists, process_running);

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
        "config_ok": config_ok,
        "status_report": status_report,
        "next_action": next_action,
        "cli": client_spec_is_cli(spec),
        "command": spec.command,
        "launch_command": spec.launch_command,
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

/// 状态页客户端 Dock：启动桌面版或在终端中启动 CLI。
pub fn dex_launch_client_impl(kind: String, cwd: Option<String>) -> Result<Value, String> {
    let spec = client_app_spec_or_err(&kind)?;
    if client_spec_is_cli(spec) {
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
        return Ok(json!({
            "ok": true,
            "kind": spec.kind,
            "label": spec.label,
            "mode": "terminal",
            "command": command,
            "cwd": cwd.as_ref().map(|path| path.to_string_lossy().to_string()),
            "requires_cwd": spec.launch_requires_cwd,
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
        std::process::Command::new("cmd")
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

fn launch_desktop_app(spec: &ClientAppSpec) -> Result<(), String> {
    let app_name = spec
        .desktop_app
        .ok_or_else(|| format!("{} 不是桌面版客户端", spec.label))?;
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-a")
            .arg(app_name)
            .spawn()
            .map_err(|e| format!("打开 {app_name} 失败: {e}"))?;
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(path) = windows_desktop_app_path(spec) {
            std::process::Command::new(path)
                .spawn()
                .map_err(|e| format!("打开 {app_name} 失败: {e}"))?;
            return Ok(());
        }
        std::process::Command::new("cmd")
            .args(["/C", "start", "", app_name])
            .spawn()
            .map_err(|e| format!("打开 {app_name} 失败: {e}"))?;
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = app_name;
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
    let app_name = match kind.as_str() {
        "codex_desktop" => "Codex",
        "claude_desktop" => "Claude",
        _ => return Err(format!("不支持的桌面客户端: {kind}")),
    };
    let action = if running { "quit" } else { "open" };

    #[cfg(target_os = "macos")]
    {
        let result = if running {
            std::process::Command::new("osascript")
                .arg("-e")
                .arg(format!("quit app \"{app_name}\""))
                .spawn()
        } else {
            std::process::Command::new("open")
                .arg("-a")
                .arg(app_name)
                .spawn()
        };
        result.map_err(|e| {
            format!(
                "{} {} 失败: {e}",
                if running { "退出" } else { "打开" },
                app_name
            )
        })?;
        Ok(json!({
            "ok": true,
            "kind": kind,
            "app": app_name,
            "action": action,
        }))
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app_name, action);
        Err("桌面客户端开关暂只支持 macOS".to_string())
    }
}

/// 状态页客户端 Dock：强制退出识别到的客户端进程。
pub fn dex_force_quit_client_impl(kind: String) -> Result<Value, String> {
    let spec = client_app_spec_or_err(&kind)?;
    let instances = client_process_instances_for_spec(spec);
    if instances.is_empty() {
        return Ok(json!({
            "ok": true,
            "kind": spec.kind,
            "label": spec.label,
            "killed": 0,
            "message": "未检测到运行中的客户端进程",
        }));
    }

    let mut killed = 0usize;
    let mut errors = Vec::new();
    for instance in instances {
        let Some(pid) = instance.get("pid").and_then(Value::as_str) else {
            continue;
        };
        if pid.trim().is_empty() {
            continue;
        }
        match force_kill_pid(pid) {
            Ok(()) => killed += 1,
            Err(err) => errors.push(format!("{pid}: {err}")),
        }
    }

    if killed == 0 && !errors.is_empty() {
        return Err(format!(
            "强制退出 {} 失败: {}",
            spec.label,
            errors.join("; ")
        ));
    }
    Ok(json!({
        "ok": errors.is_empty(),
        "kind": spec.kind,
        "label": spec.label,
        "killed": killed,
        "errors": errors,
    }))
}

fn force_kill_pid(pid: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let status = std::process::Command::new("taskkill")
            .args(["/PID", pid, "/F"])
            .status()
            .map_err(|e| format!("taskkill 启动失败: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("taskkill 退出码: {status}"))
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let status = std::process::Command::new("kill")
            .args(["-9", pid])
            .status()
            .map_err(|e| format!("kill 启动失败: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("kill 退出码: {status}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                "openclaw",
                "hermes"
            ]
        );
        assert!(client_app_spec_or_err("codex_cli").unwrap().command == Some("codex"));
        assert_eq!(
            client_app_spec_or_err("openclaw").unwrap().launch_command,
            Some("openclaw tui")
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
            !client_app_spec_or_err("openclaw")
                .unwrap()
                .launch_requires_cwd
        );
        assert!(
            !client_app_spec_or_err("hermes")
                .unwrap()
                .launch_requires_cwd
        );
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
        assert!(commands.iter().any(|cmd| cmd.contains("openclaw.ai")));
        assert!(commands.iter().any(|cmd| cmd.contains("hermes")));
    }

    #[test]
    fn desktop_process_names_do_not_count_as_cli() {
        assert!(status_command_is_codex_desktop("Codex"));
        assert!(status_command_is_claude_desktop("Claude"));
        assert!(status_command_is_codex_desktop(
            "/Applications/Codex.app/Contents/Resources/codex app-server --listen stdio://"
        ));
        assert!(!status_command_is_codex_cli(
            "/Applications/Codex.app/Contents/Resources/codex app-server --listen stdio://"
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
        assert!(!status_command_is_claude_cli("Claude"));
        assert!(status_command_is_claude_cli("claude"));
        assert!(status_command_is_claude_cli("/usr/local/bin/claude"));
    }

    #[test]
    fn hermes_gateway_does_not_count_as_cli_runtime() {
        assert!(status_command_is_hermes_cli("/Users/me/.local/bin/hermes"));
        assert!(status_command_is_hermes_cli(
            "/Library/Frameworks/Python.framework/Versions/3.11/Resources/Python.app/Contents/MacOS/Python /Users/me/.local/bin/hermes"
        ));
        assert!(!status_command_is_hermes_cli(
            "/Library/Frameworks/Python.framework/Versions/3.11/Resources/Python.app/Contents/MacOS/Python -m hermes_cli.main gateway run --replace"
        ));
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
        assert_eq!(
            mac_terminal_script("openclaw", None),
            "tell application \"Terminal\"\nactivate\ndo script \"openclaw\"\nend tell"
        );
        assert_eq!(windows_terminal_script("hermes", None), "hermes");
    }
}
