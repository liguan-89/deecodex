// CDP 调试钩子（一次性）：只读诊断 Codex 渲染进程的网络与对话状态。
// - 装 window.fetch 包装，记录包含 minimax / codex 后端的请求 URL、方法、起止时间、状态/错误
// - 装 PerformanceObserver 监听 resource timing，统计 Network.loadingFailed / loadingFinished
// - 所有数据写到 window.__cdp 上，方便 Rust 端 Runtime.evaluate 读取
// 注意：钩子幂等，重复执行不会重复挂载。

(function () {
  if (window.__cdp && window.__cdp.fetch_hook_installed) {
    return 'already_installed';
  }
  window.__cdp = window.__cdp || {
    fetch_hook_installed: false,
    minimax_requests: [],
    minimax_failures: [],
    loading_finished: 0,
    loading_failed: 0,
  };

  // ----- fetch 包装 -----
  var origFetch = window.fetch && window.fetch.bind(window);
  if (origFetch) {
    window.fetch = function (input, init) {
      var url = typeof input === 'string' ? input : (input && input.url) || '';
      var method = (init && init.method) || (input && input.method) || 'GET';
      var isMinimax = /minimax/i.test(url) || /minimax/i.test(JSON.stringify(init && init.body) || '');
      var isCodexBackend = /(chatgpt|openai|codex|api2)/i.test(url);
      if (!isMinimax && !isCodexBackend) {
        return origFetch(input, init);
      }
      var start = performance.now();
      var record = {
        url: url.slice(0, 300),
        method: method,
        start_ms: Math.round(start),
        kind: isMinimax ? 'minimax' : 'codex_backend',
      };
      window.__cdp.minimax_requests.push(record);
      // 防止数组无限增长
      if (window.__cdp.minimax_requests.length > 200) {
        window.__cdp.minimax_requests.splice(0, window.__cdp.minimax_requests.length - 200);
      }
      return origFetch(input, init).then(function (resp) {
        record.status = resp.status;
        record.duration_ms = Math.round(performance.now() - start);
        if (!resp.ok) {
          record.error = 'http_status_' + resp.status;
          window.__cdp.minimax_failures.push(record);
        }
        return resp;
      }).catch(function (err) {
        record.duration_ms = Math.round(performance.now() - start);
        record.error = (err && (err.name + ': ' + err.message)) || 'fetch_failed';
        window.__cdp.minimax_failures.push(record);
        if (window.__cdp.minimax_failures.length > 50) {
          window.__cdp.minimax_failures.splice(0, window.__cdp.minimax_failures.length - 50);
        }
        throw err;
      });
    };
  }

  // ----- PerformanceObserver 监听 resource timing（与 Chromium Network.loadingFailed 对应） -----
  if (typeof PerformanceObserver !== 'undefined') {
    try {
      var obs = new PerformanceObserver(function (list) {
        for (var i = 0; i < list.getEntries().length; i++) {
          var entry = list.getEntries()[i];
          // transferSize === 0 且 duration 极短通常表示本地命中；responseStart === 0 表示中断
          if (entry.responseStart === 0 && entry.duration < 50) {
            window.__cdp.loading_failed += 1;
          } else {
            window.__cdp.loading_finished += 1;
          }
        }
      });
      obs.observe({ type: 'resource', buffered: true });
      window.__cdp._perf_observer = obs;
    } catch (e) {
      // 旧 Chromium 不支持 resource buffered，吞掉
    }
  }

  // ----- navigator.sendBeacon 也包一下（codex 偶发用 beacon 报埋点） -----
  if (navigator.sendBeacon) {
    var origBeacon = navigator.sendBeacon.bind(navigator);
    navigator.sendBeacon = function (url, data) {
      if (/minimax/i.test(url)) {
        window.__cdp.minimax_failures.push({
          url: String(url).slice(0, 300),
          method: 'BEACON',
          error: 'sendBeacon',
          start_ms: Math.round(performance.now()),
        });
      }
      return origBeacon(url, data);
    };
  }

  window.__cdp.fetch_hook_installed = true;
  return 'installed';
})();
