//! 独立 CLI：跑一次 cdp_threads_probe 抓取 Codex 桌面版的置顶 / 项目 / 线程 / 侧边栏 DOM。
//!
//! 用法：`cargo run -p deecodex-gui --example cdp_threads_probe -- [端口]`
//! 默认端口取自 `load_args().cdp_port`（通常 9235）。

use deecodex_gui_lib::commands::cdp_debug::cdp_threads_probe;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let result = cdp_threads_probe().await;
    match result {
        Ok(v) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&v).unwrap_or_else(|e| format!("序列化失败: {e}"))
            );
        }
        Err(e) => {
            eprintln!("CDP 线程探针失败: {e}");
            std::process::exit(1);
        }
    }
}
