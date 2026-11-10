//! 调试用：文件触发的 webview 快照。
//!
//! 用途：远程（SSH）视觉验证 DEX AI 界面。SSH 会话没有 macOS「屏幕录制」TCC
//! 权限，无法用 `screencapture` 截屏；这里走 WKWebView 原生快照（应用内渲染，
//! 不需要屏幕录制权限），由 GUI 进程自己完成并写 PNG 文件。
//!
//! 触发方式：创建 `/tmp/dex-debug-capture.trigger`，文件内容为输出 PNG 路径
//! （为空则写到 `/tmp/dex-debug-capture-<ts>.png`）。GUI 检测到后截图并删除
//! 触发文件。

use std::ffi::c_void;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use tauri::{AppHandle, Manager};

const TRIGGER_PATH: &str = "/tmp/dex-debug-capture.trigger";

#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSBitmapImageRepPropertyKey, NSImage,
};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSDictionary, NSError};
#[cfg(target_os = "macos")]
use objc2_web_kit::WKWebView;

/// 启动文件触发轮询（仅 macOS 生效；其它平台为 no-op）。
pub fn spawn_capture_watcher(app: AppHandle) {
    #[cfg(target_os = "macos")]
    {
        std::thread::spawn(move || loop {
            let trigger = Path::new(TRIGGER_PATH);
            if trigger.exists() {
                let out = std::fs::read_to_string(trigger)
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(default_output_path);
                let _ = std::fs::remove_file(trigger);
                match capture_webview(&app, &out) {
                    Ok(()) => tracing::info!(%out, "debug capture saved"),
                    Err(error) => tracing::warn!(%error, "debug capture failed"),
                }
            }
            std::thread::sleep(Duration::from_millis(300));
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
    }
}

#[cfg(target_os = "macos")]
fn default_output_path() -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("/tmp/dex-debug-capture-{ts}.png")
}

#[cfg(target_os = "macos")]
fn capture_webview(app: &AppHandle, out: &str) -> Result<(), String> {
    let app_for_closure = app.clone();
    let out = out.to_string();
    let (tx, rx) = mpsc::channel::<Result<Vec<u8>, String>>();
    // 只在主线程发起 takeSnapshot（WKWebView 必须主线程调用），不在这里等待，
    // 否则主线程 run loop 被阻塞，快照完成回调永远无法派发（死锁）。
    app.run_on_main_thread(move || {
        if let Err(error) = start_snapshot(&app_for_closure, tx.clone()) {
            let _ = tx.send(Err(error));
        }
    })
    .map_err(|error| format!("run_on_main_thread: {error}"))?;
    // 当前线程（watcher）等待快照完成，主线程保持空闲以派发回调。
    let png = rx
        .recv_timeout(Duration::from_secs(12))
        .map_err(|error| format!("快照超时: {error}"))??;
    std::fs::write(&out, &png).map_err(|error| format!("写文件 {out}: {error}"))
}

#[cfg(target_os = "macos")]
fn start_snapshot(
    app: &AppHandle,
    tx: mpsc::Sender<Result<Vec<u8>, String>>,
) -> Result<(), String> {
    use block2::RcBlock;

    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "找不到主窗口".to_string())?;
    window
        .with_webview(move |platform| {
            let inner = platform.inner() as usize;
            if inner == 0 {
                let _ = tx.send(Err("webview 句柄为空".to_string()));
                return;
            }
            let webview = unsafe { Retained::retain(inner as *mut WKWebView) };
            let Some(webview) = webview else {
                let _ = tx.send(Err("无法持有 WKWebView".to_string()));
                return;
            };
            let block = RcBlock::new(move |image: *mut NSImage, _error: *mut NSError| {
                let result = (|| -> Result<Vec<u8>, String> {
                    if image.is_null() {
                        return Err("快照返回空图".to_string());
                    }
                    let retained = unsafe { Retained::retain(image) }
                        .ok_or_else(|| "无法持有 NSImage".to_string())?;
                    let tiff = retained
                        .TIFFRepresentation()
                        .ok_or_else(|| "无 TIFF 表示".to_string())?;
                    let bitmap = NSBitmapImageRep::imageRepWithData(&tiff)
                        .ok_or_else(|| "无位图表示".to_string())?;
                    let props = NSDictionary::<
                        NSBitmapImageRepPropertyKey,
                        objc2::runtime::AnyObject,
                    >::dictionary();
                    let png = unsafe {
                        bitmap
                            .representationUsingType_properties(NSBitmapImageFileType::PNG, &props)
                    }
                    .ok_or_else(|| "PNG 编码失败".to_string())?;
                    let len = png.length();
                    let mut buf = vec![0u8; len];
                    if len > 0 {
                        let ptr = std::ptr::NonNull::new(buf.as_mut_ptr() as *mut c_void)
                            .ok_or_else(|| "缓冲区为空".to_string())?;
                        unsafe {
                            png.getBytes_length(ptr, len as _);
                        }
                    }
                    Ok(buf)
                })();
                let _ = tx.send(result);
            });
            unsafe {
                webview.takeSnapshotWithConfiguration_completionHandler(None, &block);
            }
        })
        .map_err(|error| format!("with_webview: {error}"))?;
    Ok(())
}
