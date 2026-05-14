// Tauri IPC 边界 — 纯桌面 GUI，非 Tauri 环境显示阻断页
// ═══════════════════════════════════════════════════════════════
(function () {
  var invokeFn = window.__TAURI__?.core?.invoke;

  if (!invokeFn) {
    document.addEventListener('DOMContentLoaded', function () {
      document.body.innerHTML =
        '<div style="display:flex;align-items:center;justify-content:center;height:100vh;background:#060b14;color:#c4d0e4;font-family:-apple-system,sans-serif;text-align:center;">' +
        '<div><h1 style="color:#00c8e8;font-size:24px;">deecodex 控制台</h1>' +
        '<p style="color:#6b7fa8;margin-top:12px;">此页面依赖 Tauri 桌面环境，不能在浏览器中运行。</p>' +
        '<p style="color:#3a4f72;font-size:13px;margin-top:8px;">请启动 deecodex 桌面 GUI 后使用。</p></div></div>';
    });
    window.DeeCodexTauri = { hasTauri: false, invoke: function () { return Promise.reject(new Error('非 Tauri 环境')); } };
    return;
  }

  async function invoke(cmd, args) {
    return invokeFn(cmd, args);
  }

  document.documentElement.dataset.runtime = 'tauri';

  window.DeeCodexTauri = { hasTauri: true, invoke: invoke };
})();
