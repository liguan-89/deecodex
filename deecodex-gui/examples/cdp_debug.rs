//! 独立 CLI 工具：从 Codex 桌面版渲染进程抓一次 CDP 调试快照。
//!
//! 用法：`cargo run -p deecodex-gui --example cdp_debug -- [端口]`
//! 默认端口取自 `load_args()` 的 cdp_port（通常 9235）。
//! 输出完整 JSON 到 stdout，可管道给 jq 过滤。

use deecodex_gui_lib::commands::cdp_debug::cdp_debug_snapshot;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let result = cdp_debug_snapshot().await;
    match result {
        Ok(v) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&v)
                    .unwrap_or_else(|e| { format!("序列化失败: {e}") })
            );
        }
        Err(e) => {
            eprintln!("CDP 调试快照失败: {e}");
            std::process::exit(1);
        }
    }
}
