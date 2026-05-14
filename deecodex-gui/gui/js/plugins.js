// 插件状态常量
let _pluginsData = [];
let _pluginsRefreshTimer = null;
let _pluginsRefreshMs = 0;
let _pluginsAutoRefresh = true;

var PLUGIN_STATE_LABEL = {
  installed: '已安装', starting: '启动中', running: '运行中',
  stopped: '已停止', error: '异常'
};
var ACCOUNT_STATUS_LABEL = {
  disconnected: '未连接', connecting: '连接中', connected: '已连接',
  login_expired: '登录过期', error: '异常'
};
var ACCOUNT_STATUS_COLOR = {
  disconnected: 'var(--text-muted)',
  connecting: 'var(--amber)',
  connected: 'var(--green)',
  login_expired: 'var(--red)',
  error: 'var(--red)'
};

var _pluginDetailId = null;

function renderPluginsPanel() {
  // detail view
  if (_pluginDetailId) return renderPluginDetail();
  // list view
  return `<div class="page-header">
    <h2>插件管理</h2>
    <p>安装和管理第三方通道插件</p>
  </div>
  <div class="plugin-install-bar">
    <input id="pluginZipPath" placeholder="插件包路径（.zip 文件）" style="flex:1;min-width:240px;padding:8px 12px;background:var(--bg-input);border:1px solid var(--border-default);border-radius:var(--radius-sm);color:var(--text-primary);font-size:13px;">
    <button class="btn btn-ghost" onclick="browsePluginZip()">浏览</button>
    <button class="btn btn-primary" onclick="installPluginFromPath()">安装</button>
  </div>
  <div class="plugin-controls">
    <label class="history-toggle" id="pluginAutoToggle" onclick="togglePluginAutoRefresh()">
      <div class="toggle-dot"></div> 自动刷新
    </label>
    <select class="history-select" id="pluginIntervalSel" onchange="setPluginRefreshInterval(this.value)" style="display:none;">
      <option value="5000">5s</option>
      <option value="10000" selected>10s</option>
      <option value="30000">30s</option>
    </select>
    <button class="btn btn-ghost" onclick="refreshPlugins()">⟳ 刷新</button>
  </div>
  <div id="pluginList"></div>`;
}

async function loadPluginsData() {
  if (!window.DeeCodexTauri?.hasTauri) {
    _pluginsData = [];
    renderPluginList();
    return;
  }
  try {
    _pluginsData = await invoke('list_plugins') || [];
    if (_pluginDetailId) {
      // detail view: re-render full panel to reflect updated data
      document.getElementById('mainContent').innerHTML = renderPluginsPanel();
    } else {
      renderPluginList();
    }
  } catch(e) {
    var el = document.getElementById('pluginList');
    if (el) el.innerHTML = '<div class="empty-state">加载失败: ' + esc(String(e)) + '</div>';
  }
}

function renderPluginList() {
  var el = document.getElementById('pluginList');
  if (!el) return;
  if (!_pluginsData.length) { el.innerHTML = '<div class="empty-state">暂无已安装插件</div>'; return; }
  el.innerHTML = _pluginsData.map(p => renderPluginCard(p)).join('');
}

// 列表视图卡片 — 状态灯 + 名称版本/简介 + 启停按钮
function renderPluginCard(p) {
  var running = p.state === 'running';
  var stateLabel = PLUGIN_STATE_LABEL[p.state] || p.state;
  var sc = running ? 'var(--green)' : 'var(--text-muted)';

  return `<div class="plugin-card${running ? ' running' : ''}" onclick="showPluginDetail('${escAttr(p.id)}')">
    <span class="plugin-card-status">
      <span class="plugin-status-dot${running ? ' live' : ''}" style="color:${sc};background:${sc}"></span>
    </span>
    <div class="plugin-card-body">
      <div class="plugin-card-row">
        <span class="plugin-card-name">${esc(p.name)}</span>
        <span class="plugin-card-version">v${esc(p.version)}</span>
        <span class="plugin-state-badge ${running ? 'on' : 'off'}"><span class="dot"></span>${stateLabel}</span>
      </div>
      ${p.description ? `<div class="plugin-card-desc">${esc(p.description)}</div>` : ''}
    </div>
    <div class="plugin-card-actions" onclick="event.stopPropagation()">
      ${running
        ? `<button class="btn-apply" onclick="stopPlugin('${escAttr(p.id)}')" style="background:var(--red-dim);color:var(--red);border-color:var(--red);">停止</button>`
        : `<button class="btn-apply" onclick="startPlugin('${escAttr(p.id)}')">启动</button>`}
    </div>
  </div>`;
}

// ── 插件详情页 ──
function showPluginDetail(id) {
  _pluginDetailId = id;
  switchPanel('plugins');
}

function backToPluginList() {
  _pluginDetailId = null;
  switchPanel('plugins');
}

function renderPluginDetail() {
  var p = _pluginsData.find(p => p.id === _pluginDetailId);
  if (!p) {
    _pluginDetailId = null;
    return renderPluginsPanel();
  }
  var running = p.state === 'running';
  var stateLabel = PLUGIN_STATE_LABEL[p.state] || p.state;
  var sc = running ? 'var(--green)' : 'var(--text-muted)';
  var perms = p.permissions || [];
  var accounts = p.accounts || [];

  var accountsHtml = '';
  if (accounts.length) {
    accountsHtml = accounts.map(a => {
      var asc = ACCOUNT_STATUS_COLOR[a.status] || 'var(--text-muted)';
      var isConnected = a.status === 'connected';
      return `<div class="pc-account-row">
        <span class="pc-account-name">${esc(a.name || a.account_id)}</span>
        <span class="pc-account-status" style="color:${asc}">
          <span class="plugin-status-dot" style="color:${asc};background:${asc};box-shadow:0 0 5px ${asc}"></span>
          ${ACCOUNT_STATUS_LABEL[a.status] || a.status}
        </span>
        <span class="pc-account-actions">
          ${isConnected
            ? `<button class="btn-apply" onclick="stopPluginAccount('${escAttr(p.id)}','${escAttr(a.account_id)}')" style="background:var(--red-dim);color:var(--red);border-color:var(--red);">停止网关</button>`
            : `<button class="btn-apply" onclick="scanAndStart('${escAttr(p.id)}','${escAttr(a.account_id)}')">启动网关</button>`}
          <button class="btn-apply" onclick="scanPluginQr('${escAttr(p.id)}','${escAttr(a.account_id)}')" style="background:var(--accent-dim);color:var(--accent);border-color:var(--accent);">扫码登录</button>
          <button class="btn-apply" onclick="deletePluginAccount('${escAttr(p.id)}','${escAttr(a.account_id)}')" style="background:var(--red-dim);color:var(--red);border-color:var(--red);">删除</button>
        </span>
      </div>`;
    }).join('');
  } else {
    accountsHtml = '<div style="color:var(--text-muted);font-size:12px;padding:8px 0;">暂无账号</div>';
  }

  return `<button class="plugin-detail-back" onclick="backToPluginList()">← 插件管理</button>

  <div class="plugin-detail-header">
    <div class="plugin-detail-title">
      <h2>
        <span class="plugin-status-dot" style="color:${sc};background:${sc};box-shadow:0 0 8px ${sc};vertical-align:middle;margin-right:8px;"></span>
        ${esc(p.name)}
        <span class="plugin-state-badge ${running ? 'on' : 'off'}" style="vertical-align:middle;margin-left:10px;"><span class="dot"></span>${stateLabel}</span>
      </h2>
      <div class="meta">
        <span>版本 v${esc(p.version)}</span>
        ${p.author ? `<span>作者 ${esc(p.author)}</span>` : ''}
      </div>
    </div>
    ${running
      ? `<button class="btn-apply" onclick="stopPlugin('${escAttr(p.id)}')" style="background:var(--red-dim);color:var(--red);border-color:var(--red);flex-shrink:0;">停止插件</button>`
      : `<button class="btn-apply" onclick="startPlugin('${escAttr(p.id)}')" style="flex-shrink:0;">启动插件</button>`}
  </div>

  ${p.description ? `<div class="plugin-detail-section">
    <h3>简介</h3>
    <p style="color:var(--text-secondary);font-size:13px;line-height:1.6;">${esc(p.description)}</p>
  </div>` : ''}

  ${perms.length ? `<div class="plugin-detail-section">
    <h3>权限</h3>
    <div class="plugin-perm-tags">${perms.map(perm => `<span class="plugin-perm-tag">${esc(perm)}</span>`).join('')}</div>
  </div>` : ''}

  <div class="plugin-detail-section">
    <h3>账号</h3>
    ${accountsHtml}
    <div class="pc-add-account">
      <input id="addAccountId_${escAttr(p.id)}" placeholder="新 Account ID">
      <button class="btn-apply" onclick="addPluginAccount('${escAttr(p.id)}')">添加账号</button>
    </div>
    <div id="qrContainer_${escAttr(p.id)}" class="pc-qr"></div>
  </div>

  <div class="plugin-danger-zone">
    <h3>卸载插件</h3>
    <p style="color:var(--text-muted);font-size:12px;margin-bottom:10px;">卸载后将删除插件所有文件，此操作不可恢复。</p>
    <button class="btn-apply" onclick="uninstallPlugin('${escAttr(p.id)}')" style="background:var(--red-dim);color:var(--red);border-color:var(--red);">卸载插件</button>
  </div>`;
}

async function installPluginFromPath() {
  const path = document.getElementById('pluginZipPath').value.trim();
  if (!path) { showToast('请输入插件包路径'); return; }
  try {
    await invoke('install_plugin', { archivePath: path });
    showToast('插件已安装', 'success');
    document.getElementById('pluginZipPath').value = '';
    await loadPluginsData();
  } catch(e) { showToast('安装失败: ' + esc(String(e)), 'error'); }
}

async function browsePluginZip() {
  try {
    const path = await invoke('browse_file');
    if (path) {
      document.getElementById('pluginZipPath').value = path;
    }
  } catch(e) {
    showToast('文件选择失败: ' + esc(String(e)), 'error');
  }
}

async function startPlugin(id) {
  try {
    await invoke('start_plugin', { pluginId: id });
    showToast('插件已启动', 'success');
    await loadPluginsData();
  } catch(e) { showToast('启动失败: ' + esc(String(e)), 'error'); }
}

async function stopPlugin(id) {
  try {
    await invoke('stop_plugin', { pluginId: id });
    showToast('插件已停止', 'success');
    await loadPluginsData();
  } catch(e) { showToast('停止失败: ' + esc(String(e)), 'error'); }
}

async function uninstallPlugin(id) {
  if (!confirm('确定要卸载该插件吗？此操作不可恢复。')) return;
  try {
    await invoke('uninstall_plugin', { pluginId: id });
    showToast('插件已卸载', 'success');
    await loadPluginsData();
  } catch(e) { showToast('卸载失败: ' + esc(String(e)), 'error'); }
}

async function startPluginAccount(pluginId, accountId) {
  try {
    await invoke('start_plugin_account', { pluginId: pluginId, accountId: accountId });
    showToast('网关启动指令已发送', 'success');
    await loadPluginsData();
  } catch(e) { showToast('启动网关失败: ' + esc(String(e)), 'error'); }
}

async function stopPluginAccount(pluginId, accountId) {
  try {
    await invoke('stop_plugin_account', { pluginId: pluginId, accountId: accountId });
    showToast('网关已停止', 'success');
    await loadPluginsData();
  } catch(e) { showToast('停止网关失败: ' + esc(String(e)), 'error'); }
}

async function scanAndStart(pluginId, accountId) {
  // 先尝试直接启动网关
  try {
    const r = await invoke('start_plugin_account', { pluginId: pluginId, accountId: accountId });
    showToast('网关已启动', 'success');
    await loadPluginsData();
    return;
  } catch(e) {
    // 如果未登录，自动触发扫码流程
    if (String(e).includes('bot_token') || String(e).includes('扫码') || String(e).includes('登录')) {
      showToast('需要先登录，正在获取二维码...');
    } else {
      showToast('启动失败: ' + esc(String(e)), 'error');
      return;
    }
  }
  // 扫码登录
  var oc = document.getElementById('qrOverlayContent');
  var oa = document.getElementById('qrOverlayAccount');
  oc.innerHTML = '<span style="color:var(--text-muted);">获取二维码中...</span>';
  oa.textContent = '账号：' + accountId + '（扫码后自动启动网关）';
  showQrOverlay();
  try {
    var result2 = await invoke('get_plugin_qrcode', { pluginId: pluginId, accountId: accountId });
    var url2 = result2.qrcode_data_url || result2.data_url || '';
    if (url2) {
      oc.innerHTML = `<img src="${esc(url2)}" alt="QR"><p class="qr-hint" style="color:var(--amber);">请使用微信扫码，扫码后网关将自动启动</p>`;
      // 轮询状态，连接成功后自动启动网关
      var pollCount = 0;
      var pollTimer = setInterval(async () => {
        pollCount++;
        try {
          var status = await invoke('query_plugin_status', { pluginId: pluginId, accountId: accountId });
          if (status && status.status === 'connected') {
            clearInterval(pollTimer);
            oc.innerHTML = '<span style="color:var(--green);">登录成功，正在启动网关...</span>';
            try {
              await invoke('start_plugin_account', { pluginId: pluginId, accountId: accountId });
              showToast('网关已启动', 'success');
              closeQrOverlay();
            } catch(e2) {
              oc.innerHTML = '<span style="color:var(--red);">网关启动失败: ' + esc(String(e2)) + '</span>';
            }
            await loadPluginsData();
          }
        } catch(ee) {}
        if (pollCount > 60) { clearInterval(pollTimer); oc.innerHTML = '<span style="color:var(--red);">登录超时，请重试</span>'; }
      }, 2000);
    } else {
      oc.innerHTML = '<span style="color:var(--red);">' + esc(JSON.stringify(result2)) + '</span>';
    }
  } catch(e2) {
    oc.innerHTML = '<span style="color:var(--red);">获取二维码失败: ' + esc(String(e2)) + '</span>';
  }
}

async function deletePluginAccount(pluginId, accountId) {
  showToast('正在删除账号 ' + esc(accountId) + '...');
  try {
    var p = _pluginsData.find(p => p.id === pluginId);
    var accounts = (p && p.config && p.config.accounts) || {};
    delete accounts[accountId];
    await invoke('update_plugin_config', { pluginId: pluginId, config: { accounts: accounts } });
    showToast('账号已删除', 'success');
    await loadPluginsData();
  } catch(e) { showToast('删除失败: ' + esc(String(e)), 'error'); }
}

async function addPluginAccount(pluginId) {
  const input = document.getElementById('addAccountId_' + pluginId);
  const accountId = (input && input.value || '').trim();
  if (!accountId) { showToast('请输入 Account ID'); return; }
  try {
    const p = _pluginsData.find(p => p.id === pluginId);
    const existingAccounts = (p && p.config && p.config.accounts) || {};
    existingAccounts[accountId] = { name: accountId, enabled: true };
    await invoke('update_plugin_config', { pluginId: pluginId, config: { accounts: existingAccounts } });
    showToast('账号已添加', 'success');
    if (input) input.value = '';
    await loadPluginsData();
  } catch(e) { showToast('添加失败: ' + esc(String(e)), 'error'); }
}

function showQrOverlay() { document.getElementById('qrOverlay').classList.add('show'); }
function closeQrOverlay() { document.getElementById('qrOverlay').classList.remove('show'); }

async function scanPluginQr(pluginId, accountId) {
  if (!accountId) { showToast('请先添加账号'); return; }
  var oc = document.getElementById('qrOverlayContent');
  var oa = document.getElementById('qrOverlayAccount');
  oc.innerHTML = '<span style="color:var(--text-muted);">获取二维码中...</span>';
  oa.textContent = '账号：' + accountId + '（扫码后自动启动网关）';
  showQrOverlay();
  try {
    var result = await invoke('get_plugin_qrcode', { pluginId: pluginId, accountId: accountId });
    var url = result.qrcode_data_url || result.data_url || '';
    if (url) {
      oc.innerHTML = `<img src="${esc(url)}" alt="QR"><p class="qr-hint" style="color:var(--amber);">请使用微信扫码，扫码后网关将自动启动</p>`;
      // 轮询登录状态，确认后自动启动网关
      var pollCount = 0;
      var pollTimer = setInterval(async () => {
        pollCount++;
        try {
          var status = await invoke('query_plugin_status', { pluginId: pluginId, accountId: accountId });
          if (status && status.status === 'connected') {
            clearInterval(pollTimer);
            oc.innerHTML = '<span style="color:var(--green);">登录成功，正在启动网关...</span>';
            try {
              await invoke('start_plugin_account', { pluginId: pluginId, accountId: accountId });
              showToast('网关已启动', 'success');
              closeQrOverlay();
            } catch(e2) {
              oc.innerHTML = '<span style="color:var(--red);">网关启动失败: ' + esc(String(e2)) + '</span>';
            }
            await loadPluginsData();
          }
        } catch(ee) {}
        if (pollCount > 60) { clearInterval(pollTimer); oc.innerHTML = '<span style="color:var(--red);">登录超时，请重试</span>'; }
      }, 2000);
    } else {
      oc.innerHTML = '<span style="color:var(--red);">' + esc(JSON.stringify(result)) + '</span>';
    }
  } catch(e) { oc.innerHTML = '<span style="color:var(--red);">获取失败: ' + esc(String(e)) + '</span>'; }
}

function togglePluginAutoRefresh() {
  if (_pluginsRefreshTimer) {
    clearInterval(_pluginsRefreshTimer);
    _pluginsRefreshTimer = null;
    _pluginsRefreshMs = 0;
    document.getElementById('pluginAutoToggle').classList.remove('on');
    document.getElementById('pluginIntervalSel').style.display = 'none';
  } else {
    _pluginsRefreshMs = parseInt(document.getElementById('pluginIntervalSel').value) || 10000;
    _pluginsRefreshTimer = setInterval(loadPluginsData, _pluginsRefreshMs);
    document.getElementById('pluginAutoToggle').classList.add('on');
    document.getElementById('pluginIntervalSel').style.display = '';
  }
}

function setPluginRefreshInterval(val) {
  _pluginsRefreshMs = parseInt(val);
  if (_pluginsRefreshTimer) { clearInterval(_pluginsRefreshTimer); _pluginsRefreshTimer = setInterval(loadPluginsData, _pluginsRefreshMs); }
}

async function refreshPlugins() {
  await loadPluginsData();
  showToast('已刷新');
}

// ═══════════════════════════════════════════════════════════════
