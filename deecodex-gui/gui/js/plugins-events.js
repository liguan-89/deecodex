// 插件事件、二维码弹窗和事件轮询
function pluginEventLabel(event) {
  const type = event && event.type;
  if (type === 'log') return event.level ? `日志 · ${event.level}` : '日志';
  if (type === 'status_changed') return '状态';
  if (type === 'qr_code') return '二维码';
  if (type === 'error') return '错误';
  if (type === 'asset_operation') return '资产';
  return type || '事件';
}

function pluginEventText(event) {
  if (!event) return '';
  if (event.type === 'log') return event.message || '';
  if (event.type === 'status_changed') {
    return `${event.account_id || 'default'} · ${ACCOUNT_STATUS_LABEL[event.status] || event.status || 'unknown'}`;
  }
  if (event.type === 'qr_code') return `${event.account_id || 'default'} · 已生成二维码`;
  if (event.type === 'error') return event.message || '';
  if (event.type === 'asset_operation') {
    const state = event.ok === false ? '失败' : '完成';
    const path = event.path ? ` · ${event.path}` : '';
    return `${event.scope || 'asset'} · ${event.action || 'operate'} · ${state}${path}`;
  }
  return JSON.stringify(event);
}

function pluginEventClass(event) {
  if (!event) return 'info';
  if (event.type === 'error' || event.level === 'error') return 'error';
  if (event.level === 'warn') return 'warn';
  if (event.type === 'status_changed' && event.status === 'connected') return 'ok';
  return 'info';
}

function pluginEventTime(ts) {
  if (!ts) return '-';
  try {
    const date = new Date(Number(ts) * 1000);
    if (Number.isNaN(date.getTime())) return String(ts);
    return date.toLocaleTimeString('zh-CN', { hour12: false });
  } catch (_) {
    return String(ts);
  }
}

function pluginEventStats(events) {
  return (events || []).reduce((stats, record) => {
    const event = record.event || {};
    const cls = pluginEventClass(event);
    stats.total += 1;
    if (cls === 'error') stats.error += 1;
    if (cls === 'warn') stats.warn += 1;
    if (event.type === 'status_changed') stats.status += 1;
    if (event.type === 'qr_code') stats.qr += 1;
    if (event.type === 'asset_operation') stats.asset += 1;
    return stats;
  }, { total: 0, error: 0, warn: 0, status: 0, qr: 0, asset: 0 });
}

function pluginLatestQrEvent(events) {
  const list = (events || []).slice().reverse();
  return list.find(record => {
    const event = record.event || {};
    const dataUrl = String(event.data_url || '');
    return event.type === 'qr_code' && dataUrl.startsWith('data:image/');
  }) || null;
}

function renderPluginEventSummary(events) {
  const stats = pluginEventStats(events);
  if (!stats.total) return '<span class="plugin-event-pill muted">0 条</span>';
  const pills = [`<span class="plugin-event-pill">${stats.total} 条</span>`];
  if (stats.error) pills.push(`<span class="plugin-event-pill error">${stats.error} 错误</span>`);
  if (stats.warn) pills.push(`<span class="plugin-event-pill warn">${stats.warn} 警告</span>`);
  if (stats.status) pills.push(`<span class="plugin-event-pill ok">${stats.status} 状态</span>`);
  if (stats.qr) pills.push(`<span class="plugin-event-pill">${stats.qr} 二维码</span>`);
  if (stats.asset) pills.push(`<span class="plugin-event-pill">${stats.asset} 资产</span>`);
  return pills.join('');
}

function renderPluginLatestQr(events) {
  const record = pluginLatestQrEvent(events);
  if (!record) return '';
  const event = record.event || {};
  return `<div class="plugin-event-qr">
    <img src="${escAttr(event.data_url || '')}" alt="QR">
    <div>
      <strong>${esc(event.account_id || 'default')}</strong>
      <span>${esc(pluginEventTime(record.ts))} 生成的最新二维码</span>
    </div>
  </div>`;
}

function renderPluginEventsBody(pluginId) {
  const events = _pluginEventsById[pluginId] || [];
  if (!events.length) return '<div class="plugin-empty-line">暂无事件</div>';
  return events.slice(-16).reverse().map(record => {
    const event = record.event || {};
    const cls = pluginEventClass(event);
    return `<div class="plugin-event-row ${escAttr(cls)}">
      <span class="plugin-event-time">${esc(pluginEventTime(record.ts))}</span>
      <span class="plugin-event-type">${esc(pluginEventLabel(event))}</span>
      <span class="plugin-event-message" title="${escAttr(pluginEventText(event))}">${esc(pluginEventText(event))}</span>
    </div>`;
  }).join('');
}

function renderPluginEventsSection(p) {
  const events = _pluginEventsById[p.id] || [];
  return `<div class="plugin-detail-section plugin-events-section">
    <div class="plugin-section-head">
      <h3>运行事件</h3>
      <div class="plugin-event-actions">
        <span id="pluginEventSummary_${escAttr(p.id)}" class="plugin-event-summary">${renderPluginEventSummary(events)}</span>
        <button class="btn-apply" onclick="loadPluginEvents('${escAttr(p.id)}')">刷新</button>
      </div>
    </div>
    <div id="pluginEventQr_${escAttr(p.id)}">${renderPluginLatestQr(events)}</div>
    <div id="pluginEvents_${escAttr(p.id)}" class="plugin-event-list">${renderPluginEventsBody(p.id)}</div>
  </div>`;
}

async function loadPluginEvents(pluginId, silent) {
  if (!pluginId) return;
  try {
    const events = await invoke('list_plugin_events', { pluginId: pluginId, limit: 80 }) || [];
    _pluginEventsById[pluginId] = events;
    const el = document.getElementById('pluginEvents_' + pluginId);
    if (el) el.innerHTML = renderPluginEventsBody(pluginId);
    const summary = document.getElementById('pluginEventSummary_' + pluginId);
    if (summary) summary.innerHTML = renderPluginEventSummary(events);
    const qr = document.getElementById('pluginEventQr_' + pluginId);
    if (qr) qr.innerHTML = renderPluginLatestQr(events);
    if (!silent) showToast('插件事件已刷新', 'success');
  } catch(e) {
    if (!silent) showToast('事件加载失败: ' + esc(String(e)), 'error');
  }
}

function clearPluginQrPolling() {
  if (_pluginQrPollTimer) {
    clearInterval(_pluginQrPollTimer);
    _pluginQrPollTimer = null;
  }
}

function showQrOverlay() {
  const overlay = document.getElementById('qrOverlay');
  if (!overlay) {
    showToast('二维码弹窗未初始化，请重启 GUI', 'error');
    return false;
  }
  overlay.classList.add('show');
  overlay.setAttribute('aria-hidden', 'false');
  return true;
}
function closeQrOverlay() {
  clearPluginQrPolling();
  const overlay = document.getElementById('qrOverlay');
  if (!overlay) return;
  overlay.classList.remove('show');
  overlay.setAttribute('aria-hidden', 'true');
}

function startPluginEventRefresh(pluginId) {
  if (!pluginId) return;
  if (_pluginEventRefreshTimer && _pluginEventRefreshId === pluginId) return;
  stopPluginEventRefresh();
  _pluginEventRefreshId = pluginId;
  _pluginEventRefreshTimer = setInterval(() => {
    if (_pluginDetailId === pluginId) loadPluginEvents(pluginId, true);
  }, 4000);
}

function stopPluginEventRefresh() {
  if (_pluginEventRefreshTimer) {
    clearInterval(_pluginEventRefreshTimer);
    _pluginEventRefreshTimer = null;
  }
  _pluginEventRefreshId = null;
}
