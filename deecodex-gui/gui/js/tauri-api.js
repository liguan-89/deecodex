// Tauri IPC 边界 — 纯桌面 GUI，非 Tauri 环境显示阻断页
// ═══════════════════════════════════════════════════════════════
(function () {
  var invokeFn = window.__TAURI__?.core?.invoke;
  var listenFn = window.__TAURI__?.event?.listen;

  if (!invokeFn) {
    document.addEventListener('DOMContentLoaded', function () {
      document.body.innerHTML =
        '<div style="display:flex;align-items:center;justify-content:center;height:100vh;background:#060b14;color:#c4d0e4;font-family:-apple-system,sans-serif;text-align:center;">' +
        '<div><h1 style="color:#00c8e8;font-size:24px;">DEX AI</h1>' +
        '<p style="color:#6b7fa8;margin-top:12px;">此页面依赖 Tauri 桌面环境，不能在浏览器中运行。</p>' +
        '<p style="color:#3a4f72;font-size:13px;margin-top:8px;">请启动 DEX AI 桌面应用后使用。</p></div></div>';
    });
    window.DeeCodexTauri = {
      hasTauri: false,
      invoke: function () { return Promise.reject(new Error('非 Tauri 环境')); },
      listen: function () { return Promise.reject(new Error('非 Tauri 环境')); },
      startWindowDrag: function () { return Promise.reject(new Error('非 Tauri 环境')); },
      minimizeWindow: function () { return Promise.reject(new Error('非 Tauri 环境')); },
      toggleMaximizeWindow: function () { return Promise.reject(new Error('非 Tauri 环境')); },
      hideWindow: function () { return Promise.reject(new Error('非 Tauri 环境')); },
      isWindowMaximized: function () { return Promise.reject(new Error('非 Tauri 环境')); },
      onWindowResized: function () { return Promise.reject(new Error('非 Tauri 环境')); },
      convertFileSrc: function () { return ''; }
    };
    return;
  }

  var IPC_SECRET_KEY = /(password|token|secret|authorization|api[_-]?key|private[_-]?key|invitation[_-]?code|credential|auth[_-]?key)/i;

  function redactIpcValue(value, key, seen) {
    if (key && IPC_SECRET_KEY.test(String(key))) return '[REDACTED]';
    if (typeof value === 'string') {
      var trimmed = value.trim();
      if ((trimmed[0] === '{' && trimmed[trimmed.length - 1] === '}')
          || (trimmed[0] === '[' && trimmed[trimmed.length - 1] === ']')) {
        try {
          return JSON.stringify(redactIpcValue(JSON.parse(value), '', seen));
        } catch (_) {}
      }
      return value;
    }
    if (value === null || value === undefined || typeof value !== 'object') return value;
    if (seen.has(value)) return '[CIRCULAR]';
    seen.add(value);
    if (Array.isArray(value)) {
      return value.map(function (item) { return redactIpcValue(item, '', seen); });
    }
    var redacted = {};
    Object.keys(value).forEach(function (itemKey) {
      redacted[itemKey] = redactIpcValue(value[itemKey], itemKey, seen);
    });
    return redacted;
  }

  function redactIpcArgs(args) {
    return redactIpcValue(args || {}, '', new WeakSet());
  }

  // 统一 trace 包装 — 所有 IPC 调用自动记录，敏感参数必须先脱敏
  async function invoke(cmd, args) {
    console.debug('[ipc:start]', cmd, redactIpcArgs(args));
    try {
      var result = await invokeFn(cmd, args);
      console.debug('[ipc:ok]', cmd, typeof result === 'string' ? result.slice(0, 100) : '…');
      return result;
    } catch (error) {
      console.error('[ipc:error]', cmd, error);
      throw error;
    }
  }

  async function listen(eventName, handler) {
    if (typeof listenFn !== 'function') {
      throw new Error('Tauri 事件系统不可用');
    }
    console.debug('[ipc:listen]', eventName);
    return listenFn(eventName, handler);
  }

  async function minimizeWindow() {
    return await invokeFn('minimize_window');
  }

  async function toggleMaximizeWindow() {
    return await invokeFn('toggle_maximize_window');
  }

  async function hideWindow() {
    return await invokeFn('hide_window');
  }

  // 窗口状态查询/监听统一收敛在 IPC 边界内，业务脚本不直接接触 window.__TAURI__。
  async function isWindowMaximized() {
    var currentWindow = window.__TAURI__?.window?.getCurrentWindow?.();
    if (!currentWindow || typeof currentWindow.isMaximized !== 'function') return false;
    return currentWindow.isMaximized();
  }

  async function onWindowResized(handler) {
    var currentWindow = window.__TAURI__?.window?.getCurrentWindow?.();
    if (!currentWindow || typeof currentWindow.onResized !== 'function') return null;
    return currentWindow.onResized(handler);
  }

  async function startWindowDrag() {
    try {
      return await invokeFn('start_window_drag');
    } catch (error) {
      var currentWindow = window.__TAURI__?.window?.getCurrentWindow?.();
      if (currentWindow && typeof currentWindow.startDragging === 'function') {
        return currentWindow.startDragging();
      }
      throw error;
    }
  }

  document.documentElement.dataset.runtime = 'tauri';
  if (/Windows/i.test(navigator.userAgent)) {
    document.documentElement.dataset.platform = 'windows';
  }

  window.DeeCodexTauri = {
    hasTauri: true,
    invoke: invoke,
    listen: listen,
    startWindowDrag: startWindowDrag,
    minimizeWindow: minimizeWindow,
    toggleMaximizeWindow: toggleMaximizeWindow,
    hideWindow: hideWindow,
    isWindowMaximized: isWindowMaximized,
    onWindowResized: onWindowResized,
    convertFileSrc: function (path) {
      try {
        return window.__TAURI__.core.convertFileSrc(path);
      } catch (e) {
        return '';
      }
    }
  };
})();
