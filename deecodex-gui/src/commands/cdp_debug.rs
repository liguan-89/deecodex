//! CDP 调试快照：只读诊断 Codex 桌面版渲染进程的对话状态、模型选择、命令执行链路。
//!
//! 通过 `localhost:<cdp_port>/json` 拿到 page target 的 WebSocket URL，
//! 用 deecodex::cdp::CdpClient 装一次性 fetch hook、读取 DOM/Runtime 状态，
//! 输出 JSON 供手动排查 minimax 任务中断使用。

use std::time::Duration;

use deecodex::cdp::{find_codex_page, list_targets, CdpClient};
use serde_json::{json, Value};

use super::load_args;

const SNAPSHOT_INTERVAL_MS: u64 = 2000;

#[tauri::command]
/// 一次性抓取 Codex 渲染进程的快照：模型、对话气泡、fetch 钩子状态、Network 失败计数。
///
/// 触发方式：开发者从 GUI 开发者控制台调用 `invoke('cdp_debug_snapshot')`，
/// 或由 dev 环境从 CLI 调用 `cargo run --bin deecodex-gui -- --invoke cdp_debug_snapshot`。
/// 返回值会同时落到 `deecodex.log` 末尾，标记 `[CDP-DEBUG]`。
pub async fn cdp_debug_snapshot() -> Result<Value, String> {
    let args = load_args();
    let cdp_port = args.cdp_port;

    let targets = list_targets(cdp_port)
        .await
        .map_err(|e| format!("CDP 目标列表拉取失败 (端口 {cdp_port}): {e}"))?;
    let ws_url = find_codex_page(&targets)
        .ok_or_else(|| format!("未在端口 {cdp_port} 找到 Codex 页面目标"))?;

    let mut client = CdpClient::connect(&ws_url)
        .await
        .map_err(|e| format!("CDP WebSocket 连接失败 ({ws_url}): {e}"))?;

    // 装一次性 fetch 钩子 + Network 失败计数：抓 minimax / codex 后端请求。
    // 这里只挂 console 监听，失败事件由 Network 域另外捕获。
    client
        .evaluate(include_str!("./cdp_debug_hook.js"))
        .await
        .map_err(|e| format!("装 fetch 钩子失败: {e}"))?;

    let first = collect_snapshot(&mut client)
        .await
        .map_err(|e| format!("第一次快照失败: {e}"))?;

    tokio::time::sleep(Duration::from_millis(SNAPSHOT_INTERVAL_MS)).await;

    let second = collect_snapshot(&mut client)
        .await
        .map_err(|e| format!("第二次快照失败: {e}"))?;

    // diff：相同则不变化，差异则报告 streaming 状态、最后一条气泡、Network 失败次数差。
    let diff = diff_snapshots(&first, &second);

    let result = json!({
        "cdp_port": cdp_port,
        "ws_url": ws_url,
        "interval_ms": SNAPSHOT_INTERVAL_MS,
        "first": first,
        "second": second,
        "diff": diff,
    });

    let bubbles_first_log = first
        .get("bubble_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let bubbles_second_log = second
        .get("bubble_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let failed_delta_log = diff
        .get("network_failed_delta")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    tracing::info!(
        cdp_port,
        bubbles_first = bubbles_first_log,
        bubbles_second = bubbles_second_log,
        failed_delta = failed_delta_log,
        "CDP-DEBUG 快照完成"
    );

    Ok(result)
}

/// 在已建立的 CDP 客户端上抓一次快照。
async fn collect_snapshot(client: &mut CdpClient) -> Result<Value, String> {
    // 对话气泡：抓 [data-message-author-role] / [data-message-id] / [data-message-status] 三种常见挂载。
    let bubbles_expr = r#"
        (() => {
          const sels = [
            '[data-message-author-role]',
            '[data-message-id]',
            '[data-message-status]',
            '[data-testid*="message"]',
            '[data-testid*="Message"]',
          ];
          const seen = new Set();
          const out = [];
          for (const sel of sels) {
            for (const el of document.querySelectorAll(sel)) {
              if (seen.has(el)) continue;
              seen.add(el);
              out.push({
                role: el.getAttribute('data-message-author-role')
                  || el.getAttribute('data-author')
                  || el.getAttribute('data-testid')
                  || 'unknown',
                status: el.getAttribute('data-message-status')
                  || el.getAttribute('data-status')
                  || '',
                text: (el.innerText || '').slice(0, 240),
              });
              if (out.length >= 60) break;
            }
            if (out.length >= 60) break;
          }
          return out;
        })()
    "#;

    // 模型选择器：抓 React fiber 上 props.model，或可见的 combobox 文本。
    let model_expr = r#"
        (() => {
          // 1. 尝试 React fiber 摘 model 字段（codex 用 react-i18next + 自研 store，结构会变，仅作为兜底）
          const root = document.querySelector('#root') || document.body;
          const fiberKey = Object.keys(root).find(k => k.startsWith('__reactContainer$'));
          let fiberModel = null;
          if (fiberKey) {
            let node = root[fiberKey].stateNode.current;
            const visited = new Set();
            const stack = [node];
            let hops = 0;
            while (stack.length && hops < 5000) {
              const n = stack.pop();
              if (!n || visited.has(n)) continue;
              visited.add(n);
              hops++;
              const memo = n.memoizedProps || n.pendingProps;
              if (memo && typeof memo === 'object') {
                if (typeof memo.model === 'string') { fiberModel = memo.model; break; }
                if (memo.model && typeof memo.model === 'object' && typeof memo.model.id === 'string') {
                  fiberModel = memo.model.id; break;
                }
              }
              const st = n.memoizedState;
              if (st) {
                let s = st;
                while (s) {
                  if (s.memoizedState && typeof s.memoizedState === 'object') {
                    const v = s.memoizedState.model;
                    if (typeof v === 'string') { fiberModel = v; break; }
                  }
                  s = s.next;
                }
                if (fiberModel) break;
              }
              if (n.child) stack.push(n.child);
              if (n.sibling) stack.push(n.sibling);
            }
          }

          // 2. 抓 combobox / button 上可见的模型名
          const visibleModel = (() => {
            const candidates = document.querySelectorAll(
              'button[aria-haspopup], [role="combobox"], [data-testid*="model"], [data-testid*="Model"]'
            );
            for (const el of candidates) {
              const t = (el.innerText || el.textContent || '').trim();
              if (t && t.length <= 60) return t;
            }
            return null;
          })();

          return { fiber_model: fiberModel, visible_model: visibleModel };
        })()
    "#;

    // fetch 钩子状态：window.__cdp.fetch_hook_installed + minimax 请求计数
    let fetch_state_expr = r#"
        JSON.stringify({
          hook_installed: Boolean(window.__cdp && window.__cdp.fetch_hook_installed),
          minimax_requests: (window.__cdp && window.__cdp.minimax_requests) || [],
          minimax_failures: (window.__cdp && window.__cdp.minimax_failures) || [],
        })
    "#;

    // Performance.getMetrics 客户端版本不可用，改为 Network.loadingFailed / loadingFinished 计数
    let network_state_expr = r#"
        JSON.stringify({
          loading_finished: (window.__cdp && window.__cdp.loading_finished) || 0,
          loading_failed: (window.__cdp && window.__cdp.loading_failed) || 0,
        })
    "#;

    let bubbles_val = client
        .evaluate(bubbles_expr)
        .await
        .map_err(|e| format!("抓对话气泡失败: {e}"))?;
    let model_val = client
        .evaluate(model_expr)
        .await
        .map_err(|e| format!("抓模型字段失败: {e}"))?;
    let fetch_state_str = client
        .evaluate(fetch_state_expr)
        .await
        .map_err(|e| format!("抓 fetch 钩子状态失败: {e}"))?;
    let network_state_str = client
        .evaluate(network_state_expr)
        .await
        .map_err(|e| format!("抓 Network 计数失败: {e}"))?;

    // CDP Runtime.evaluate 响应：{ id, result: { result: { type, value } } }
    // 真正数据在 result.result.value；exceptionDetails 在 result.exceptionDetails。
    fn extract_value<'a>(v: &'a Value) -> Option<&'a Value> {
        v.get("result")
            .and_then(|r| r.get("result"))
            .and_then(|r| r.get("value"))
    }

    let bubbles: Vec<Value> = extract_value(&bubbles_val)
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let fetch_state: Value = extract_value(&fetch_state_str)
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| json!({}));
    let network_state: Value = extract_value(&network_state_str)
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| json!({}));

    Ok(json!({
        "bubble_count": bubbles.len(),
        "last_bubble": bubbles.last().cloned().unwrap_or(Value::Null),
        "bubbles_tail": bubbles.iter().rev().take(5).cloned().collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>(),
        "model": extract_value(&model_val).cloned().unwrap_or(Value::Null),
        "fetch": fetch_state,
        "network": network_state,
        "ts_ms": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0),
    }))
}

/// 一次性探针：装 cdp_threads_probe.js 钩子，读 window.__cdp_threads。
///
/// 抓取 Codex 桌面版渲染进程里置顶 / 项目 / 线程 / 侧边栏 / 顶层标题的
/// DOM 元数据，用于和 deecodex 自己的线程管理逻辑做对照（pinned / project / thread）。
/// 文档位置：deecodex-gui/src/commands/cdp_threads_probe.js
#[tauri::command]
pub async fn cdp_threads_probe() -> Result<Value, String> {
    let args = load_args();
    let cdp_port = args.cdp_port;

    let targets = list_targets(cdp_port)
        .await
        .map_err(|e| format!("CDP 目标列表拉取失败 (端口 {cdp_port}): {e}"))?;
    let ws_url = find_codex_page(&targets)
        .ok_or_else(|| format!("未在端口 {cdp_port} 找到 Codex 页面目标"))?;

    let mut client = CdpClient::connect(&ws_url)
        .await
        .map_err(|e| format!("CDP WebSocket 连接失败 ({ws_url}): {e}"))?;

    // 装探针钩子。
    let install_resp = client
        .evaluate(include_str!("./cdp_threads_probe.js"))
        .await
        .map_err(|e| format!("装 threads 探针失败: {e}"))?;

    // 读取结果。Runtime.evaluate 响应：{ id, result: { result: { type, value } } }
    fn extract_value<'a>(v: &'a Value) -> Option<&'a Value> {
        v.get("result")
            .and_then(|r| r.get("result"))
            .and_then(|r| r.get("value"))
    }
    let install_value = extract_value(&install_resp)
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let dump_expr = "JSON.stringify(window.__cdp_threads || {})";
    let dump_val = client
        .evaluate(dump_expr)
        .await
        .map_err(|e| format!("读 __cdp_threads 失败: {e}"))?;
    let dump_str = extract_value(&dump_val)
        .and_then(|v| v.as_str())
        .unwrap_or("{}");
    let dump: Value = serde_json::from_str(dump_str)
        .unwrap_or_else(|e| json!({"parse_error": e.to_string(), "raw": dump_str}));

    let pinned_count = dump
        .get("pinned")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let projects_count = dump
        .get("projects")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let threads_count = dump
        .get("threads")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let nav_count = dump
        .get("nav")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    tracing::info!(
        cdp_port,
        install_value = install_value,
        pinned = pinned_count,
        projects = projects_count,
        threads = threads_count,
        nav = nav_count,
        "CDP-DEBUG 线程结构探针完成"
    );

    Ok(json!({
        "cdp_port": cdp_port,
        "ws_url": ws_url,
        "install_value": install_value,
        "pinned_count": pinned_count,
        "projects_count": projects_count,
        "threads_count": threads_count,
        "nav_count": nav_count,
        "data": dump,
    }))
}

fn diff_snapshots(first: &Value, second: &Value) -> Value {
    let bubble_count_first = first
        .get("bubble_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let bubble_count_second = second
        .get("bubble_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let last_first = first.get("last_bubble");
    let last_second = second.get("last_bubble");

    let streaming_first = last_first
        .and_then(|v| v.get("status"))
        .and_then(|v| v.as_str())
        .map(|s| s.contains("streaming") || s.contains("pending") || s.contains("in_progress"))
        .unwrap_or(false);
    let streaming_second = last_second
        .and_then(|v| v.get("status"))
        .and_then(|v| v.as_str())
        .map(|s| s.contains("streaming") || s.contains("pending") || s.contains("in_progress"))
        .unwrap_or(false);

    let network_failed_first = first
        .get("network")
        .and_then(|n| n.get("loading_failed"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let network_failed_second = second
        .get("network")
        .and_then(|n| n.get("loading_failed"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let loading_finished_first = first
        .get("network")
        .and_then(|n| n.get("loading_finished"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let loading_finished_second = second
        .get("network")
        .and_then(|n| n.get("loading_finished"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let model_first = first.get("model");
    let model_second = second.get("model");

    let minimax_failures_second = second
        .get("fetch")
        .and_then(|f| f.get("minimax_failures"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    json!({
        "bubble_count_delta": bubble_count_second as i64 - bubble_count_first as i64,
        "bubble_last_unchanged": last_first == last_second,
        "streaming_first": streaming_first,
        "streaming_second": streaming_second,
        "stuck_in_streaming": streaming_second && bubble_count_second == bubble_count_first && last_first == last_second,
        "network_failed_delta": network_failed_second as i64 - network_failed_first as i64,
        "loading_finished_delta": loading_finished_second as i64 - loading_finished_first as i64,
        "model_unchanged": model_first == model_second,
        "model": model_second,
        "minimax_failures": minimax_failures_second,
    })
}
