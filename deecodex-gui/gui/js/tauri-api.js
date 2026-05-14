(function () {
  const hasTauri = Boolean(window.__TAURI__?.core?.invoke);

  async function invoke(command, args) {
    if (!hasTauri) {
      console.warn('Tauri API 不可用，命令未执行:', command, args);
      throw new Error('当前不是桌面 GUI 环境，无法调用本地命令');
    }
    return window.__TAURI__.core.invoke(command, args);
  }

  function requireTauri(featureName) {
    if (hasTauri) return true;
    const message = `当前不是桌面 GUI，无法${featureName}`;
    if (typeof window.showToast === 'function') {
      window.showToast(message, 'error');
    } else {
      console.warn(message);
    }
    return false;
  }

  document.documentElement.dataset.runtime = hasTauri ? 'tauri' : 'browser-preview';

  window.DeeCodexTauri = {
    hasTauri,
    invoke,
    requireTauri,
  };
})();
