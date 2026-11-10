//! 共享的 Codex 桌面客户端 macOS 兼容层。
//!
//! 新版 Codex 桌面版在 macOS 上的 `.app` 文件名已从 `Codex.app` 改为 `ChatGPT.app`，
//! 但 Bundle ID 仍是 `com.openai.codex`，内置 CLI 仍在 `<App>/Contents/Resources/codex`，
//! 数据目录仍是 `~/Library/Application Support/Codex`。
//!
//! 本模块集中管理：
//! - 常量（`BUNDLE_ID` / `MAC_PATHS`）：供 GUI 与 daemon 共享同一份真值源，避免 5 处字面量散落。
//! - 启动/退出 helper：bundle id 优先 + app 名多候选 fallback，供 GUI 状态页、Dock、CDP 启停共用。

/// Codex 桌面在 macOS 上的 Bundle ID（LaunchServices 注册名）。
pub const BUNDLE_ID: &str = "com.openai.codex";

/// Codex 桌面在 macOS 上的常见安装路径（按 first-existing 顺序探测）。
/// 同时覆盖旧版 `Codex.app` 与新版 `ChatGPT.app` 两个 .app 文件名。
pub const MAC_PATHS: &[&str] = &[
    "/Applications/Codex.app",
    "/Applications/ChatGPT.app",
    "~/Applications/Codex.app",
    "~/Applications/ChatGPT.app",
];

/// 判断一条进程命令是否属于 Codex 桌面运行时。
///
/// 新版主应用改名为 `ChatGPT.app`，但仍会启动内置 `codex app-server`
/// 和明确命名的 Codex Service/Renderer helper。不能仅凭数据目录参数判断，
/// 否则残留 crashpad 或读取 `Application Support/Codex` 的脚本会产生误报。
#[cfg(target_os = "macos")]
pub fn macos_process_command_is_codex_desktop(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let codex_main = lower.contains("/applications/codex.app/contents/macos/codex");
    let chatgpt_main = lower.contains("/applications/chatgpt.app/contents/macos/chatgpt");
    let chatgpt_app_server = lower.contains("/applications/chatgpt.app/contents/resources/codex")
        && lower.contains(" app-server");
    let chatgpt_helper = lower
        .contains("/applications/chatgpt.app/contents/frameworks/codex framework.framework/")
        && (lower.contains("/helpers/codex (service).app/contents/macos/codex (service)")
            || lower.contains("/helpers/codex (renderer).app/contents/macos/codex (renderer)"));
    codex_main || chatgpt_main || chatgpt_app_server || chatgpt_helper
}

/// 检测 Codex/ChatGPT 桌面运行时是否正在运行。
#[cfg(target_os = "macos")]
pub fn is_running() -> bool {
    let Ok(output) = std::process::Command::new("ps")
        .args(["ax", "-o", "command="])
        .output()
    else {
        return false;
    };
    output.status.success()
        && String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(macos_process_command_is_codex_desktop)
}

#[cfg(not(target_os = "macos"))]
pub fn is_running() -> bool {
    false
}

/// 从 `MAC_PATHS` 派生 macOS 应用名候选列表（剥 `.app` 后缀并去重）。
/// Codex 桌面 spec 的 `MAC_PATHS` 同时含 `Codex.app` 与 `ChatGPT.app`，
/// 派生出的候选为 `["Codex", "ChatGPT"]`，从而自动获得两层 fallback。
pub fn macos_app_name_candidates() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for path in MAC_PATHS {
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

/// macOS 上启动 Codex 桌面。
/// - 优先 `open -b <BUNDLE_ID> [extra_args]`，失败按 `macos_app_name_candidates()` 顺序回退到 `open -a <App> [extra_args]`。
/// - `extra_args` 透传给 `open --args`，CDP 端口场景典型传 `["--remote-debugging-port=9222"]`。
#[cfg(target_os = "macos")]
pub fn launch(extra_args: &[&str]) -> Result<(), String> {
    // 第一层：bundle id 启动。
    let first_err: Option<String> = match std::process::Command::new("open")
        .arg("-b")
        .arg(BUNDLE_ID)
        .arg("--args")
        .args(extra_args)
        .spawn()
    {
        Ok(_) => return Ok(()),
        Err(err) => {
            tracing::warn!(
                bundle_id = BUNDLE_ID,
                error = %err,
                "按 Bundle ID 启动 Codex 桌面失败，回退到按应用名启动"
            );
            Some(err.to_string())
        }
    };
    // 第二层：按 app 名逐个尝试。
    let mut last_err = first_err;
    for app_name in macos_app_name_candidates() {
        match std::process::Command::new("open")
            .arg("-a")
            .arg(app_name)
            .arg("--args")
            .args(extra_args)
            .spawn()
        {
            Ok(_) => return Ok(()),
            Err(err) => {
                tracing::warn!(
                    app = app_name,
                    error = %err,
                    "按应用名启动 Codex 桌面失败，尝试下一候选"
                );
                last_err = Some(err.to_string());
            }
        }
    }
    Err(format!(
        "启动 Codex 桌面失败: {}",
        last_err.unwrap_or_else(|| "无可用应用名候选".to_string())
    ))
}

/// macOS 上优雅退出 Codex 桌面。
/// - 优先 `osascript -e 'quit app id "<BUNDLE_ID>"'`，失败按 `macos_app_name_candidates()` 顺序回退到 `quit app "<App>"`。
/// - 调用 `.status()` 检退出码，非 0 视为本次尝试失败并触发下一候选。
#[cfg(target_os = "macos")]
pub fn quit() -> Result<(), String> {
    // 第一层：bundle id 优雅退出。
    let first_err: Option<String> = match std::process::Command::new("osascript")
        .arg("-e")
        .arg(format!("quit app id \"{BUNDLE_ID}\""))
        .status()
    {
        Ok(status) if status.success() => return Ok(()),
        Ok(status) => {
            tracing::warn!(
                bundle_id = BUNDLE_ID,
                exit_code = ?status.code(),
                "AppleScript quit app id 退出码非 0，回退到按应用名退出"
            );
            Some(format!("exit_code={:?}", status.code()))
        }
        Err(err) => {
            tracing::warn!(
                bundle_id = BUNDLE_ID,
                error = %err,
                "派发 AppleScript quit app id 失败，回退到按应用名退出"
            );
            Some(err.to_string())
        }
    };
    // 第二层：按 app 名逐个尝试。
    let mut last_err = first_err;
    for app_name in macos_app_name_candidates() {
        match std::process::Command::new("osascript")
            .arg("-e")
            .arg(format!("quit app \"{app_name}\""))
            .status()
        {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => {
                tracing::warn!(
                    app = app_name,
                    exit_code = ?status.code(),
                    "AppleScript quit app 退出码非 0，尝试下一候选"
                );
                last_err = Some(format!("exit_code={:?}", status.code()));
            }
            Err(err) => {
                tracing::warn!(
                    app = app_name,
                    error = %err,
                    "派发 AppleScript quit app 失败，尝试下一候选"
                );
                last_err = Some(err.to_string());
            }
        }
    }
    Err(format!(
        "退出 Codex 桌面失败: {}",
        last_err.unwrap_or_else(|| "无可用应用名候选".to_string())
    ))
}

/// Windows 上停止 Codex 桌面进程。
/// 用 `taskkill /im <exe>` 强杀；同时尝试 `Codex.exe` 与 `ChatGPT.exe`（新版桌面可能的可执行名）。
#[cfg(target_os = "windows")]
pub fn quit() -> Result<(), String> {
    let mut last_err: Option<String> = None;
    for exe in ["Codex.exe", "ChatGPT.exe"] {
        let mut command = std::process::Command::new("taskkill");
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        match command
            .arg("/f")
            .arg("/im")
            .arg(exe)
            .spawn()
        {
            Ok(_) => return Ok(()),
            Err(err) => {
                tracing::warn!(
                    exe = exe,
                    error = %err,
                    "taskkill 失败，尝试下一候选"
                );
                last_err = Some(err.to_string());
            }
        }
    }
    Err(format!(
        "停止 Codex 桌面失败: {}",
        last_err.unwrap_or_else(|| "无可用可执行名候选".to_string())
    ))
}

/// Windows 上启动 Codex 桌面进程（用于 CDP 等场景）。
/// 用 `taskkill` 探测 exe 是否存在。
#[cfg(target_os = "windows")]
pub fn launch(extra_args: &[&str]) -> Result<(), String> {
    for exe in ["Codex.exe", "ChatGPT.exe"] {
        let result = std::process::Command::new(exe).args(extra_args).spawn();
        if result.is_ok() {
            return Ok(());
        }
    }
    Err("启动 Codex 桌面失败: 未找到 Codex.exe / ChatGPT.exe".to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn launch(_extra_args: &[&str]) -> Result<(), String> {
    Err("Codex 桌面启动暂只支持 macOS 与 Windows".to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn quit() -> Result<(), String> {
    Err("Codex 桌面退出暂只支持 macOS 与 Windows".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_app_name_candidates_covers_codex_and_chatgpt() {
        let names = macos_app_name_candidates();
        assert!(names.contains(&"Codex"));
        assert!(names.contains(&"ChatGPT"));
        // 去重
        assert_eq!(
            names.iter().filter(|n| **n == "Codex").count(),
            1,
            "Codex 应该只出现一次"
        );
    }

    #[test]
    fn bundle_id_is_openai_codex() {
        assert_eq!(BUNDLE_ID, "com.openai.codex");
    }

    #[test]
    fn mac_paths_contain_both_app_layouts() {
        assert!(MAC_PATHS.contains(&"/Applications/Codex.app"));
        assert!(MAC_PATHS.contains(&"/Applications/ChatGPT.app"));
        assert!(MAC_PATHS.contains(&"~/Applications/Codex.app"));
        assert!(MAC_PATHS.contains(&"~/Applications/ChatGPT.app"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_process_detection_covers_chatgpt_runtime_layout() {
        assert!(macos_process_command_is_codex_desktop(
            "/Applications/ChatGPT.app/Contents/Resources/codex app-server --analytics-default-enabled"
        ));
        assert!(macos_process_command_is_codex_desktop(
            "/Applications/ChatGPT.app/Contents/Frameworks/Codex Framework.framework/Versions/150/Helpers/Codex (Renderer).app/Contents/MacOS/Codex (Renderer)"
        ));
        assert!(macos_process_command_is_codex_desktop(
            "/Applications/Codex.app/Contents/MacOS/Codex"
        ));
        assert!(macos_process_command_is_codex_desktop(
            "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT"
        ));
        assert!(!macos_process_command_is_codex_desktop(
            "/Applications/ChatGPT.app/Contents/Frameworks/Codex Framework.framework/Versions/150/Helpers/browser_crashpad_handler --database=/Users/test/Library/Application Support/Codex/Crashpad"
        ));
        assert!(!macos_process_command_is_codex_desktop(
            "/bin/cat /Users/test/Library/Application Support/Codex/state.json"
        ));
    }
}
