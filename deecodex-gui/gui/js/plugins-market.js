// 插件市场、列表筛选、安装预览和刷新
function renderPluginPreviewSummary(preview, manifest, risk) {
  const categories = pluginCategoryLabels(manifest);
  const featureCount = (manifest.features || []).length;
  const toolCount = (manifest.dex_tools || []).length;
  const permissionCount = (preview.permission_details || manifest.permissions || []).length;
  const items = [
    { label: '分类', value: categories.join(' / ') || '工具插件' },
    { label: '能力', value: `${featureCount} 项${toolCount ? ` · ${toolCount} 工具` : ''}` },
    { label: '权限', value: `${pluginRiskLabel(risk)} · ${permissionCount} 项`, cls: `risk-${PLUGIN_RISK_CLASS[risk] || 'low'}` },
    { label: '要求', value: manifest.min_deecodex_version ? `DEX AI ${manifest.min_deecodex_version}+` : '无最低版本' }
  ];
  return `<div class="plugin-preview-summary">
    ${items.map(item => `<div class="plugin-preview-summary-item">
      <span>${esc(item.label)}</span>
      <strong class="${escAttr(item.cls || '')}">${esc(item.value)}</strong>
    </div>`).join('')}
  </div>`;
}

function renderPluginCompatibilityChecks(item) {
  const compat = pluginCompatibility(item);
  const checks = Array.isArray(compat.checks) ? compat.checks : [];
  if (!checks.length) return '';
  const reasons = Array.isArray(compat.reasons) ? compat.reasons : [];
  return `<div class="plugin-preview-block plugin-compat-checks">
    <h4>兼容性</h4>
    <div class="plugin-compat-check-list">
      ${checks.map(check => `<div class="plugin-compat-check-row ${escAttr(check.tone || 'muted')}">
        <span>${esc(check.label || '-')}</span>
        <strong>${esc(check.value || '-')}</strong>
      </div>`).join('')}
    </div>
    ${reasons.length ? `<div class="plugin-compat-reasons">${reasons.map(reason => `<span>${esc(reason)}</span>`).join('')}</div>` : ''}
  </div>`;
}

function renderPluginPreviewFeatures(features) {
  if (!features.length) return '';
  return `<div class="plugin-preview-block">
    <h4>能力</h4>
    <div class="plugin-preview-feature-list">
      ${features.map(feature => {
        const methods = Object.keys(feature.methods || {});
        const methodText = methods.length ? methods.map(pluginActionLabel).join(' / ') : '无动作';
        return `<div class="plugin-preview-feature-row">
          <div>
            <strong>${esc(feature.label || feature.id || feature.kind || '能力')}</strong>
            ${feature.description ? `<span>${esc(feature.description)}</span>` : ''}
          </div>
          <em>${esc(pluginFeatureKindLabel(feature.kind))}</em>
          <code>${esc(methodText)}</code>
        </div>`;
      }).join('')}
    </div>
  </div>`;
}

function renderPluginPreviewDexTools(tools) {
  if (!tools.length) return '';
  return `<div class="plugin-preview-block">
    <h4>DEX 工具</h4>
    <div class="plugin-preview-tool-list">
      ${tools.map(tool => `<div class="plugin-preview-tool-row">
        <strong>${esc(tool.name || tool.method || 'tool')}</strong>
        <span>${esc(tool.description || '')}</span>
        <em>L${Number(tool.level || 0)}</em>
      </div>`).join('')}
    </div>
  </div>`;
}

function showPluginInstallPreview(preview) {
  return new Promise(function (resolve) {
    var existing = document.getElementById('pluginPreviewModal');
    if (existing) existing.remove();

    const manifest = preview.manifest || {};
    const features = manifest.features || [];
    const permissions = preview.permission_details || [];
    const permissionChanges = preview.permission_changes || [];
    const risk = preview.permission_risk || 'low';
    const isInstalled = Boolean(preview.already_installed);
    const isUpdate = Boolean(preview.update_available);
    const statusLabel = isUpdate ? '可更新' : (isInstalled ? '已安装' : '可安装');
    const categories = pluginCategoryLabels(manifest);
    const compatibility = pluginCompatibility(preview);
    const canInstall = compatibility.compatible !== false && (!isInstalled || isUpdate);
    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';
    overlay.id = 'pluginPreviewModal';
    overlay.innerHTML = `<div class="modal-box plugin-preview-modal">
      <div class="modal-header"><h3>插件详情</h3></div>
      <div class="modal-body plugin-preview-body">
        <div class="plugin-preview-hero">
          <div class="plugin-preview-title">
            <strong>${esc(manifest.name || manifest.id || '未知插件')}</strong>
            <span>v${esc(manifest.version || '-')}</span>
            <span class="plugin-preview-status ${isUpdate ? 'installed' : (isInstalled ? 'installed' : 'available')}">${esc(statusLabel)}</span>
            ${renderPluginCompatibilityPill(preview)}
            <span class="plugin-risk-badge ${escAttr(PLUGIN_RISK_CLASS[risk] || 'low')}">风险 ${esc(pluginRiskLabel(risk))}</span>
          </div>
          ${manifest.description ? `<p>${esc(manifest.description)}</p>` : ''}
          ${categories.length ? `<div class="plugin-perm-tags">${categories.map(label => `<span class="plugin-perm-tag">${esc(label)}</span>`).join('')}</div>` : ''}
        </div>
        ${renderPluginPreviewSummary(preview, manifest, risk)}
        ${renderPluginCompatibilityChecks(preview)}
        <div class="plugin-source-grid">
          <span>ID</span><code>${esc(manifest.id || '-')}</code>
          <span>类型</span><code>${esc(pluginKindLabel(manifest))}</code>
          ${manifest.author ? `<span>作者</span><code>${esc(manifest.author)}</code>` : ''}
          ${manifest.min_deecodex_version ? `<span>最低版本</span><code>${esc(manifest.min_deecodex_version)}</code>` : ''}
          ${preview.existing_version ? `<span>当前版本</span><code>v${esc(preview.existing_version)}</code>` : ''}
          ${preview.previous_source_hash ? `<span>当前 SHA</span><code>${esc(preview.previous_source_hash)}</code>` : ''}
          <span>来源</span><code>${esc(preview.source_path || '-')}</code>
          <span>SHA-256</span><code>${esc(preview.source_hash || '-')}</code>
          <span>安装目录</span><code>${esc(preview.install_dir || '-')}</code>
          <span>资产目录</span><code>${esc(preview.asset_dir || '-')}</code>
        </div>
        ${isUpdate ? `<div class="plugin-preview-warning">该插件已安装，将更新插件文件并保留配置、启用状态和连接资产。</div>` : ''}
        ${renderPluginPreviewFeatures(features)}
        ${renderPluginPreviewDexTools(manifest.dex_tools || [])}
        ${permissionChanges.length ? `<div class="plugin-preview-block">
          <h4>权限变化</h4>
          <div class="plugin-permission-list">${permissionChanges.map(item => `<div class="plugin-permission-row">
            <span class="plugin-permission-name">${esc(item.permission)}</span>
            <span class="plugin-risk-badge ${escAttr(PLUGIN_RISK_CLASS[item.risk] || 'low')}">${esc(item.change === 'added' ? '新增' : item.change === 'removed' ? '移除' : '保留')}</span>
            <span class="plugin-permission-desc">${esc(item.description || '')}</span>
          </div>`).join('')}</div>
        </div>` : ''}
        ${permissions.length ? `<div class="plugin-preview-block">
          <h4>权限</h4>
          <div class="plugin-permission-list">${permissions.map(item => `<div class="plugin-permission-row">
            <span class="plugin-permission-name">${esc(item.permission)}</span>
            <span class="plugin-risk-badge ${escAttr(PLUGIN_RISK_CLASS[item.risk] || 'low')}">${esc(pluginRiskLabel(item.risk))}</span>
            <span class="plugin-permission-desc">${esc(item.description || '')}</span>
          </div>`).join('')}</div>
        </div>` : ''}
      </div>
      <div class="plugin-preview-actions">
        <button class="btn btn-primary" id="pluginPreviewOk" type="button" ${canInstall ? '' : 'disabled'}>${canInstall ? (isUpdate ? '更新' : '安装') : (isInstalled ? '已安装' : '不可安装')}</button>
        <button class="btn btn-ghost" id="pluginPreviewCancel" type="button">取消</button>
      </div>
    </div>`;
    document.body.appendChild(overlay);

    function cleanup(value) { overlay.remove(); resolve(value); }
    const ok = document.getElementById('pluginPreviewOk');
    const cancel = document.getElementById('pluginPreviewCancel');
    if (ok && canInstall) ok.onclick = function () { cleanup(isUpdate ? 'update' : 'install'); };
    if (cancel) cancel.onclick = function () { cleanup(false); };
    overlay.addEventListener('click', function (e) { if (e.target === overlay) cleanup(false); });
  });
}

function syncPluginAutoRefreshUi() {
  const toggle = document.getElementById('pluginAutoToggle');
  const intervalSel = document.getElementById('pluginIntervalSel');
  if (toggle) toggle.classList.toggle('on', Boolean(_pluginsRefreshTimer));
  if (intervalSel) {
    intervalSel.style.display = _pluginsRefreshTimer ? '' : 'none';
    if (_pluginsRefreshMs) intervalSel.value = String(_pluginsRefreshMs);
  }
}

function pluginPanelData() {
  return _pluginPanelMode === 'installed' ? (_pluginsData || []) : (_pluginMarketplaceData || []);
}

function pluginModeLabel(mode) {
  return mode === 'installed' ? '已安装' : '插件市场';
}

function renderPluginModeTabs() {
  const marketCount = (_pluginMarketplaceData || []).length;
  const installedCount = (_pluginsData || []).length;
  return `<div class="plugin-mode-tabs" role="tablist" aria-label="插件视图">
    <button type="button" class="${_pluginPanelMode === 'market' ? 'active' : ''}" onclick="setPluginPanelMode('market')" role="tab" aria-selected="${_pluginPanelMode === 'market' ? 'true' : 'false'}">
      <span>插件市场</span><em>${marketCount}</em>
    </button>
    <button type="button" class="${_pluginPanelMode === 'installed' ? 'active' : ''}" onclick="setPluginPanelMode('installed')" role="tab" aria-selected="${_pluginPanelMode === 'installed' ? 'true' : 'false'}">
      <span>已安装</span><em>${installedCount}</em>
    </button>
  </div>`;
}

function renderPluginCategoryTabs() {
  const data = pluginPanelData();
  const counts = { all: data.length };
  data.forEach(plugin => {
    pluginCategoryKeys(plugin).forEach(key => {
      counts[key] = (counts[key] || 0) + 1;
    });
  });
  const visibleCategories = PLUGIN_CATEGORY_DEFS.filter(item => item.key === 'all' || (counts[item.key] || 0) > 0);
  if (_pluginCategoryFilter !== 'all' && !visibleCategories.some(item => item.key === _pluginCategoryFilter)) {
    _pluginCategoryFilter = 'all';
  }
  return `<div class="plugin-category-tabs" role="tablist" aria-label="插件分类">
    ${visibleCategories.map(item => {
      const count = counts[item.key] || 0;
      return `<button type="button" class="${_pluginCategoryFilter === item.key ? 'active' : ''}" onclick="setPluginCategoryFilter('${escAttr(item.key)}')" role="tab" aria-selected="${_pluginCategoryFilter === item.key ? 'true' : 'false'}">
        <span>${esc(item.label)}</span><em>${count}</em>
      </button>`;
    }).join('')}
  </div>`;
}

function renderPluginsPanel() {
  // detail view
  if (_pluginDetailId) return renderPluginDetail();
  // list view
  return `<div class="page-header">
    <h2>插件市场</h2>
  </div>
  <div class="plugin-console">
    ${renderPluginModeTabs()}
    ${renderPluginCategoryTabs()}
    <div class="plugin-install-bar">
      <input id="pluginZipPath" placeholder="插件包路径（.zip 或插件目录）">
      <button class="btn btn-ghost" onclick="browsePluginZip()">选择包</button>
      <button class="btn btn-ghost" onclick="browsePluginDir()">选择目录</button>
      <button class="btn btn-primary" onclick="installPluginFromPath()">安装</button>
    </div>
    ${renderPluginDevBar()}
    ${renderPluginFilterBar()}
    <div class="plugin-controls">
      <label class="history-toggle${_pluginsRefreshTimer ? ' on' : ''}" id="pluginAutoToggle" onclick="togglePluginAutoRefresh()">
        <div class="toggle-dot"></div> 自动刷新
      </label>
      <select class="history-select" id="pluginIntervalSel" onchange="setPluginRefreshInterval(this.value)" style="${_pluginsRefreshTimer ? '' : 'display:none;'}">
        <option value="5000" ${_pluginsRefreshMs === 5000 ? 'selected' : ''}>5s</option>
        <option value="10000" ${_pluginsRefreshMs === 10000 || !_pluginsRefreshMs ? 'selected' : ''}>10s</option>
        <option value="30000" ${_pluginsRefreshMs === 30000 ? 'selected' : ''}>30s</option>
      </select>
      <button class="btn btn-ghost" onclick="refreshPlugins()">刷新</button>
    </div>
  </div>
  <div id="pluginList" class="plugin-list"></div>`;
}

function renderPluginFilterBar() {
  const kindOptions = pluginFilterOptions('kind');
  const featureOptions = pluginFilterOptions('feature');
  return `<div class="plugin-filter-bar">
    <input id="pluginSearchInput" value="${escAttr(_pluginSearch)}" placeholder="搜索插件" oninput="setPluginSearch(this.value)">
    <select class="history-select" onchange="setPluginKindFilter(this.value)">
      <option value="all">全部类型</option>
      ${kindOptions.map(item => `<option value="${escAttr(item.value)}" ${_pluginKindFilter === item.value ? 'selected' : ''}>${esc(item.label)}</option>`).join('')}
    </select>
    <select class="history-select" onchange="setPluginFeatureFilter(this.value)">
      <option value="all">全部能力</option>
      ${featureOptions.map(item => `<option value="${escAttr(item.value)}" ${_pluginFeatureFilter === item.value ? 'selected' : ''}>${esc(item.label)}</option>`).join('')}
    </select>
  </div>`;
}

function pluginFilterOptions(type) {
  const map = new Map();
  pluginPanelData().forEach(plugin => {
    if (type === 'kind') {
      const kind = String(plugin.kind || 'tool');
      map.set(kind, pluginKindLabel(plugin));
    } else {
      pluginFeatures(plugin).forEach(feature => {
        const kind = String(feature.kind || 'integration');
        map.set(kind, pluginFeatureKindLabel(kind));
      });
    }
  });
  return Array.from(map.entries())
    .map(([value, label]) => ({ value, label }))
    .sort((a, b) => a.label.localeCompare(b.label, 'zh-Hans-CN'));
}

function pluginMatchesFilters(plugin) {
  if (_pluginCategoryFilter !== 'all' && !pluginCategoryKeys(plugin).includes(_pluginCategoryFilter)) {
    return false;
  }
  if (_pluginKindFilter !== 'all' && String(plugin.kind || 'tool') !== _pluginKindFilter) {
    return false;
  }
  if (_pluginFeatureFilter !== 'all') {
    const hasFeature = pluginFeatures(plugin).some(feature => String(feature.kind || '') === _pluginFeatureFilter);
    if (!hasFeature) return false;
  }
  const search = String(_pluginSearch || '').trim().toLowerCase();
  if (!search) return true;
  const haystack = [
    plugin.id,
    plugin.name,
    plugin.description,
    plugin.author,
    plugin.kind,
    ...pluginCategoryLabels(plugin),
    ...(plugin.tags || []),
    ...pluginFeatures(plugin).map(feature => feature.label + ' ' + feature.kind)
  ].join(' ').toLowerCase();
  return haystack.includes(search);
}

function setPluginCategoryFilter(value) {
  _pluginCategoryFilter = value || 'all';
  const categoryTabs = document.querySelector('.plugin-category-tabs');
  if (categoryTabs) categoryTabs.outerHTML = renderPluginCategoryTabs();
  renderPluginList();
}

function setPluginKindFilter(value) {
  _pluginKindFilter = value || 'all';
  renderPluginList();
}

function setPluginFeatureFilter(value) {
  _pluginFeatureFilter = value || 'all';
  renderPluginList();
}

function setPluginSearch(value) {
  _pluginSearch = value || '';
  renderPluginList();
}

function setPluginPanelMode(mode) {
  _pluginPanelMode = mode === 'installed' ? 'installed' : 'market';
  capturePluginDevDraft();
  const consoleEl = document.querySelector('.plugin-console');
  if (consoleEl) {
    const tabs = consoleEl.querySelector('.plugin-mode-tabs');
    if (tabs) tabs.outerHTML = renderPluginModeTabs();
    const categoryTabs = consoleEl.querySelector('.plugin-category-tabs');
    if (categoryTabs) categoryTabs.outerHTML = renderPluginCategoryTabs();
    const devBar = consoleEl.querySelector('.plugin-dev-entry');
    if (devBar) devBar.outerHTML = renderPluginDevBar();
    const filterBar = consoleEl.querySelector('.plugin-filter-bar');
    if (filterBar) filterBar.outerHTML = renderPluginFilterBar();
  }
  renderPluginList();
}

async function loadPluginsData() {
  try {
    capturePluginDevDraft();
    _pluginsData = await invoke('list_plugins') || [];
    try {
      _pluginMarketplaceData = await invoke('list_plugin_marketplace') || [];
    } catch(e) {
      console.warn('插件市场加载失败', e);
      _pluginMarketplaceData = [];
    }
    if (_pluginDetailId) {
      const input = document.activeElement;
      const focusedId = input && input.id ? input.id : null;
      const focusedValue = input && 'value' in input ? input.value : null;
      const main = document.getElementById('mainContent');
      const scrollTop = main ? main.scrollTop : 0;
      const html = renderPluginsPanel();
      document.getElementById('mainContent').innerHTML = typeof wrapPrimaryPanel === 'function' ? wrapPrimaryPanel('plugins', html) : html;
      const restored = focusedId ? document.getElementById(focusedId) : null;
      if (restored && focusedValue !== null && 'value' in restored) {
        restored.value = focusedValue;
        restored.focus();
      }
      const nextMain = document.getElementById('mainContent');
      if (nextMain) nextMain.scrollTop = scrollTop;
      loadPluginEvents(_pluginDetailId, true);
      startPluginEventRefresh(_pluginDetailId);
    } else {
      stopPluginEventRefresh();
      const modeTabs = document.querySelector('.plugin-mode-tabs');
      if (modeTabs) modeTabs.outerHTML = renderPluginModeTabs();
      const categoryTabs = document.querySelector('.plugin-category-tabs');
      if (categoryTabs) categoryTabs.outerHTML = renderPluginCategoryTabs();
      const devBar = document.getElementById('pluginDevEntry');
      if (devBar) devBar.outerHTML = renderPluginDevBar();
      const filterBar = document.querySelector('.plugin-filter-bar');
      if (filterBar) filterBar.outerHTML = renderPluginFilterBar();
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
  const data = pluginPanelData();
  el.className = _pluginPanelMode === 'market' ? 'plugin-market-grid' : 'plugin-list';
  if (!data.length) {
    el.innerHTML = `<div class="empty-state">${_pluginPanelMode === 'market' ? '暂无市场插件' : '暂无已安装插件'}</div>`;
    return;
  }
  const filtered = data.filter(pluginMatchesFilters);
  if (!filtered.length) { el.innerHTML = `<div class="empty-state">没有匹配的${esc(pluginModeLabel(_pluginPanelMode))}插件</div>`; return; }
  el.innerHTML = filtered.map(p => _pluginPanelMode === 'market' ? renderPluginMarketCard(p) : renderPluginCard(p)).join('');
}

function pluginMarketStatus(item) {
  if (item.update_available) return { label: '可更新', cls: 'update' };
  if (item.installed) return { label: '已安装', cls: 'installed' };
  return { label: '可安装', cls: 'available' };
}

function pluginCompatibility(item) {
  return (item && item.compatibility) || {};
}

function pluginCompatibilityTone(item) {
  const tone = String(pluginCompatibility(item).tone || '');
  if (tone === 'block' || tone === 'warn' || tone === 'ok') return tone;
  return pluginCompatibility(item).compatible === false ? 'block' : 'ok';
}

function pluginCompatibilityLabel(item) {
  const compat = pluginCompatibility(item);
  return compat.label || (pluginCompatibilityTone(item) === 'block' ? '不可安装' : '兼容');
}

function pluginCompatibilityReasonText(item) {
  const reasons = pluginCompatibility(item).reasons || [];
  return reasons.length ? reasons.join('；') : pluginCompatibilityLabel(item);
}

function renderPluginCompatibilityPill(item) {
  const tone = pluginCompatibilityTone(item);
  return `<span class="plugin-compat-pill ${escAttr(tone)}" title="${escAttr(pluginCompatibilityReasonText(item))}">${esc(pluginCompatibilityLabel(item))}</span>`;
}

function renderPluginMarketCard(item) {
  const status = pluginMarketStatus(item);
  const compatibility = pluginCompatibility(item);
  const canInstall = compatibility.compatible !== false;
  const features = pluginFeatures(item);
  const risk = pluginPermissionRisk(item);
  const featureText = features.length
    ? features.slice(0, 3).map(feature => feature.label || pluginFeatureKindLabel(feature.kind)).join(' · ')
    : pluginKindLabel(item);
  const tags = [
    pluginCategoryLabel(pluginPrimaryCategory(item)),
    item.source_label || '本地',
    ...(item.template ? ['模板'] : []),
  ];
  const action = item.update_available
    ? `<button class="btn-apply" onclick="event.stopPropagation(); installMarketplacePlugin('${escAttr(item.id)}')" ${canInstall ? '' : 'disabled'}>更新</button>`
    : item.installed
      ? `<button class="btn-apply" onclick="event.stopPropagation(); openInstalledPluginFromMarket('${escAttr(item.id)}')">管理</button>`
      : `<button class="btn-apply" onclick="event.stopPropagation(); installMarketplacePlugin('${escAttr(item.id)}')" ${canInstall ? '' : 'disabled'}>安装</button>`;
  const installed = item.installed_version ? `<span class="plugin-market-installed">当前 v${esc(item.installed_version)}</span>` : '';
  return `<div class="plugin-market-card ${escAttr(status.cls)}" onclick="openMarketplaceCard('${escAttr(item.id)}')">
    <div class="plugin-market-head">
      <div class="plugin-market-title">
        <span>${esc(item.name || item.id)}</span>
        <em>v${esc(item.version || '-')}</em>
      </div>
      <span class="plugin-market-status ${escAttr(status.cls)}">${esc(status.label)}</span>
    </div>
    ${item.description ? `<p class="plugin-market-desc">${esc(item.description)}</p>` : ''}
    <div class="plugin-market-tags">${tags.map(tag => `<span>${esc(tag)}</span>`).join('')}</div>
    <div class="plugin-market-foot">
      <span title="${escAttr(featureText)}">${esc(featureText || '基础插件')}</span>
      ${renderPluginCompatibilityPill(item)}
      <span class="plugin-risk-badge ${escAttr(PLUGIN_RISK_CLASS[risk] || 'low')}">${esc(pluginRiskLabel(risk))}</span>
      ${installed}
      ${action}
    </div>
  </div>`;
}

// 列表视图卡片 — 状态灯 + 名称版本/简介 + 启停按钮

function marketplaceItemById(id) {
  return (_pluginMarketplaceData || []).find(item => String(item.id) === String(id));
}

async function installPluginPathWithPreview(path) {
  try {
    const preview = await invoke('preview_plugin_install', { path: path });
    const ok = await showPluginInstallPreview(preview || {});
    if (!ok) return false;
    let installedManifest = null;
    if (ok === 'update') {
      installedManifest = await invoke('update_plugin', { path: path });
      showToast('插件已更新', 'success');
    } else {
      installedManifest = await invoke('install_plugin', { path: path });
      showToast('插件已安装', 'success');
    }
    const nextId = (installedManifest && installedManifest.id) || (preview && preview.manifest && preview.manifest.id);
    if (nextId) {
      _pluginPanelMode = 'installed';
      _pluginCategoryFilter = 'all';
      _pluginKindFilter = 'all';
      _pluginFeatureFilter = 'all';
      _pluginDetailId = nextId;
    }
    await loadPluginsData();
    return true;
  } catch(e) {
    showToast('安装失败: ' + esc(String(e)), 'error');
    return false;
  }
}

async function previewMarketplacePlugin(id) {
  const item = marketplaceItemById(id);
  if (!item || !item.path) return;
  await installPluginPathWithPreview(item.path);
}

async function openMarketplaceCard(id) {
  const item = marketplaceItemById(id);
  if (!item) return;
  if (item.installed && !item.update_available) {
    openInstalledPluginFromMarket(id);
    return;
  }
  if (!item.path) return;
  await installPluginPathWithPreview(item.path);
}

async function installMarketplacePlugin(id) {
  const item = marketplaceItemById(id);
  if (!item || !item.path) {
    showToast('市场条目不可用: ' + esc(id), 'error');
    return;
  }
  await installPluginPathWithPreview(item.path);
}

function openInstalledPluginFromMarket(id) {
  const installed = (_pluginsData || []).find(plugin => String(plugin.id) === String(id));
  if (!installed) {
    showToast('插件还未安装', 'error');
    return;
  }
  _pluginPanelMode = 'installed';
  showPluginDetail(id);
}

async function installPluginFromPath() {
  const path = document.getElementById('pluginZipPath').value.trim();
  if (!path) { showToast('请输入插件包路径'); return; }
  const changed = await installPluginPathWithPreview(path);
  if (changed) {
    const input = document.getElementById('pluginZipPath');
    if (input) input.value = '';
  }
}

async function browsePluginZip() {
  try {
    const path = await invoke('browse_plugin_package');
    if (path) {
      document.getElementById('pluginZipPath').value = path;
    }
  } catch(e) {
    showToast('文件选择失败: ' + esc(String(e)), 'error');
  }
}

async function browsePluginDir() {
  try {
    const path = await invoke('browse_plugin_directory');
    if (path) {
      document.getElementById('pluginZipPath').value = path;
    }
  } catch(e) {
    showToast('目录选择失败: ' + esc(String(e)), 'error');
  }
}

function togglePluginAutoRefresh() {
  if (_pluginsRefreshTimer) {
    stopPluginAutoRefresh();
  } else {
    _pluginsRefreshMs = parseInt(document.getElementById('pluginIntervalSel').value) || 10000;
    _pluginsRefreshTimer = setInterval(loadPluginsData, _pluginsRefreshMs);
    syncPluginAutoRefreshUi();
  }
}

function stopPluginAutoRefresh() {
  if (_pluginsRefreshTimer) {
    clearInterval(_pluginsRefreshTimer);
    _pluginsRefreshTimer = null;
  }
  _pluginsRefreshMs = 0;
  syncPluginAutoRefreshUi();
}

function setPluginRefreshInterval(val) {
  _pluginsRefreshMs = parseInt(val);
  if (_pluginsRefreshTimer) { clearInterval(_pluginsRefreshTimer); _pluginsRefreshTimer = setInterval(loadPluginsData, _pluginsRefreshMs); }
  syncPluginAutoRefreshUi();
}

async function refreshPlugins() {
  await loadPluginsData();
  showToast('已刷新');
}
