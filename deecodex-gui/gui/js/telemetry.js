// 匿名使用统计 / 广告位埋点封装
// 规则：前端只传业务枚举和非敏感状态；后端二次脱敏、落本地队列并上报。
(function () {
  const invoke = (...args) => window.invoke ? window.invoke(...args) : window.DeeCodexTauri?.invoke?.(...args);
  const ACTIVE_INTERVAL_MS = 5 * 60 * 1000;
  let ready = false;
  let preferences = null;
  let lastActiveAt = 0;
  const pending = [];

  function safeProps(props) {
    const input = props && typeof props === 'object' ? props : {};
    const output = {};
    Object.keys(input).slice(0, 64).forEach((key) => {
      const lower = String(key).toLowerCase();
      if (/key|token|secret|password|authorization|cookie|prompt|message|content|email|path|file|dir/.test(lower)) {
        output[key] = '[REDACTED]';
        return;
      }
      const value = input[key];
      if (value == null || typeof value === 'boolean' || typeof value === 'number') {
        output[key] = value;
      } else if (typeof value === 'string') {
        output[key] = value.length > 256 ? value.slice(0, 256) : value;
      } else if (Array.isArray(value)) {
        output[key] = value.slice(0, 16).map(item => typeof item === 'string' ? item.slice(0, 128) : item);
      } else if (typeof value === 'object') {
        output[key] = safeProps(value);
      }
    });
    return output;
  }

  function contextProps(extra) {
    const cfg = window.currentConfig || (typeof currentConfig !== 'undefined' ? currentConfig : {}) || {};
    return safeProps({
      page: typeof currentPanel !== 'undefined' ? currentPanel : '',
      router_mode: cfg.codex_router_mode || '',
      selected_client: typeof selectedClientKind !== 'undefined' ? selectedClientKind : '',
      selected_config_client: typeof selectedConfigClientKind !== 'undefined' ? selectedConfigClientKind : '',
      ...(extra || {})
    });
  }

  async function loadPreferences() {
    if (!invoke) return null;
    try {
      preferences = await invoke('get_telemetry_preferences');
      ready = true;
      while (pending.length) {
        const item = pending.shift();
        track(item.event, item.props);
      }
      return preferences;
    } catch (error) {
      console.warn('[telemetry] 获取统计设置失败:', error);
      ready = false;
      return null;
    }
  }

  function isEnabled() {
    return preferences ? preferences.enabled !== false : true;
  }

  async function setEnabled(enabled) {
    if (!invoke) return null;
    preferences = await invoke('set_telemetry_preferences', { enabled: !!enabled });
    return preferences;
  }

  function track(event, props) {
    if (!event || !invoke) return Promise.resolve(null);
    if (!ready) {
      if (pending.length < 50) pending.push({ event, props });
      return Promise.resolve(null);
    }
    if (!isEnabled()) return Promise.resolve(null);
    return invoke('track_telemetry_event', { event, props: contextProps(props) }).catch((error) => {
      console.warn('[telemetry] 事件上报失败:', event, error);
      return null;
    });
  }

  function trackPageView(page, extra) {
    return track('page_view', { page, ...(extra || {}) });
  }

  function trackAdSlotView(slot, extra) {
    return track('ad_slot_view', { ad_slot: slot, ...(extra || {}) });
  }

  function trackAdClick(slot, extra) {
    return track('ad_click', { ad_slot: slot, ...(extra || {}) });
  }

  function trackActive(reason) {
    const now = Date.now();
    if (now - lastActiveAt < ACTIVE_INTERVAL_MS) return;
    lastActiveAt = now;
    track('app_active', { reason: reason || 'timer' });
  }

  function bindActivityEvents() {
    document.addEventListener('visibilitychange', () => {
      if (document.visibilityState === 'visible') trackActive('visible');
    });
    window.addEventListener('focus', () => trackActive('focus'));
    window.setInterval(() => trackActive('interval'), ACTIVE_INTERVAL_MS);
  }

  async function initTelemetry() {
    await loadPreferences();
    track('app_start', { source: 'gui' });
    trackActive('init');
    bindActivityEvents();
  }

  window.DexTelemetry = {
    init: initTelemetry,
    loadPreferences,
    getPreferences: () => preferences,
    setEnabled,
    track,
    trackPageView,
    trackAdSlotView,
    trackAdClick,
    isEnabled
  };
})();
