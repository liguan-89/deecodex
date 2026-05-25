// 插件详情、配置、能力和连接操作
function pluginConfigFieldId(pluginId, key) {
  return 'pluginCfg_' + String(pluginId).replace(/[^a-zA-Z0-9_-]/g, '_') + '_' + String(key).replace(/[^a-zA-Z0-9_-]/g, '_');
}

function pluginConfigValue(p, key, schema) {
  if (p && p.config && Object.prototype.hasOwnProperty.call(p.config, key)) return p.config[key];
  if (schema && Object.prototype.hasOwnProperty.call(schema, 'default')) return schema.default;
  if (schema && schema.type === 'boolean') return false;
  if (schema && (schema.type === 'array' || schema.type === 'object')) return schema.type === 'array' ? [] : {};
  return '';
}

function renderPluginConfigField(p, key) {
  const schema = pluginSchemaProperties(p)[key] || {};
  const value = pluginConfigValue(p, key, schema);
  const fieldId = pluginConfigFieldId(p.id, key);
  const label = schema.title || key;
  const desc = schema.description ? `<span class="plugin-config-desc">${esc(schema.description)}</span>` : '';
  const type = schema.type || (Array.isArray(value) ? 'array' : typeof value);
  let control = '';

  if (Array.isArray(schema.enum)) {
    control = `<select id="${escAttr(fieldId)}" data-plugin-config-key="${escAttr(key)}" data-plugin-config-type="string" class="plugin-config-input">
      ${schema.enum.map(item => `<option value="${escAttr(item)}" ${String(value) === String(item) ? 'selected' : ''}>${esc(item)}</option>`).join('')}
    </select>`;
  } else if (type === 'boolean') {
    control = `<label class="plugin-config-switch">
      <input id="${escAttr(fieldId)}" data-plugin-config-key="${escAttr(key)}" data-plugin-config-type="boolean" type="checkbox" ${value ? 'checked' : ''}>
      <span>${value ? '开启' : '关闭'}</span>
    </label>`;
  } else if (type === 'integer' || type === 'number') {
    control = `<input id="${escAttr(fieldId)}" data-plugin-config-key="${escAttr(key)}" data-plugin-config-type="${escAttr(type)}" class="plugin-config-input" type="number" value="${escAttr(value ?? '')}">`;
  } else if (type === 'array') {
    const text = Array.isArray(value) ? value.join('\n') : String(value || '');
    control = `<textarea id="${escAttr(fieldId)}" data-plugin-config-key="${escAttr(key)}" data-plugin-config-type="array" class="plugin-config-input plugin-config-textarea">${esc(text)}</textarea>`;
  } else if (type === 'object') {
    const text = typeof value === 'object' ? JSON.stringify(value, null, 2) : String(value || '{}');
    control = `<textarea id="${escAttr(fieldId)}" data-plugin-config-key="${escAttr(key)}" data-plugin-config-type="object" class="plugin-config-input plugin-config-textarea">${esc(text)}</textarea>`;
  } else {
    control = `<input id="${escAttr(fieldId)}" data-plugin-config-key="${escAttr(key)}" data-plugin-config-type="string" class="plugin-config-input" value="${escAttr(value ?? '')}">`;
  }

  return `<label class="plugin-config-field">
    <span class="plugin-config-label">${esc(label)}</span>
    ${control}
    ${desc}
  </label>`;
}

function renderPluginConfigSection(p) {
  const keys = pluginConfigKeys(p);
  if (!keys.length) return '';
  return `<div class="plugin-detail-section plugin-config-section">
    <h3>配置</h3>
    <div class="plugin-config-grid">${keys.map(key => renderPluginConfigField(p, key)).join('')}</div>
    <div class="plugin-config-actions">
      <button class="btn-apply" onclick="savePluginConfig('${escAttr(p.id)}')">保存配置</button>
    </div>
  </div>`;
}

function renderPluginAccountsSection(p, accountsHtml) {
  const label = pluginAccountLabel(p);
  return `<div class="plugin-detail-section">
    <h3>${esc(label)}</h3>
    ${accountsHtml}
    <div class="pc-add-account">
      <input id="addAccountId_${escAttr(p.id)}" placeholder="新 ${escAttr(label)} ID">
      <button class="btn-apply" onclick="addPluginAccount('${escAttr(p.id)}')">添加${esc(label)}</button>
    </div>
    <div id="qrContainer_${escAttr(p.id)}" class="pc-qr"></div>
  </div>`;
}

function renderPluginToolsSection(dexTools) {
  if (!dexTools.length) return '';
  return `<div class="plugin-detail-section">
    <h3>DEX 工具</h3>
    <div class="plugin-perm-tags">${dexTools.map(t => `<span class="plugin-perm-tag" title="${escAttr(t.description || '')}">${esc(t.name)} · L${Number(t.level || 0)}</span>`).join('')}</div>
  </div>`;
}

function renderPluginPermissionsSection(p) {
  const details = p.permission_details || [];
  const perms = p.permissions || [];
  if (!details.length && !perms.length) return '';
  const rows = details.length
    ? details.map(item => `<div class="plugin-permission-row">
        <span class="plugin-permission-name">${esc(item.permission)}</span>
        <span class="plugin-risk-badge ${escAttr(PLUGIN_RISK_CLASS[item.risk] || 'low')}">${esc(pluginRiskLabel(item.risk))}</span>
        <span class="plugin-permission-desc">${esc(item.description || '')}</span>
      </div>`).join('')
    : `<div class="plugin-perm-tags">${perms.map(perm => `<span class="plugin-perm-tag">${esc(perm)}</span>`).join('')}</div>`;
  return `<div class="plugin-detail-section">
    <h3>权限</h3>
    <div class="plugin-permission-list">${rows}</div>
  </div>`;
}

function renderPluginSourceSection(p) {
  if (!p.source_hash && !p.source_path) return '';
  return `<div class="plugin-detail-section">
    <h3>来源</h3>
    <div class="plugin-source-grid">
      ${p.source_path ? `<span>路径</span><code>${esc(p.source_path)}</code>` : ''}
      ${p.source_hash ? `<span>SHA-256</span><code>${esc(p.source_hash)}</code>` : ''}
    </div>
  </div>`;
}

function pluginBytes(bytes) {
  const value = Number(bytes || 0);
  if (!Number.isFinite(value) || value <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB'];
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit++;
  }
  return `${size >= 10 || unit === 0 ? Math.round(size) : size.toFixed(1)} ${units[unit]}`;
}

function renderPluginAssetsSection(p) {
  const assets = p && p.assets;
  if (!assets || !assets.paths) return '';
  const paths = assets.paths || {};
  return `<div class="plugin-detail-section plugin-assets-section">
    <div class="plugin-section-head">
      <h3>资产</h3>
      <button class="btn-apply" onclick="clearPluginCache('${escAttr(p.id)}')">清理缓存</button>
    </div>
    <div class="plugin-asset-metrics">
      <div><strong>${esc(pluginBytes(assets.total_bytes))}</strong><span>总占用</span></div>
      <div><strong>${esc(pluginBytes(assets.data_bytes))}</strong><span>数据</span></div>
      <div><strong>${esc(pluginBytes(assets.cache_bytes))}</strong><span>缓存</span></div>
      <div><strong>${Number(assets.secret_count || 0)}</strong><span>密钥</span></div>
      <div><strong>${Number(assets.account_count || 0)}</strong><span>连接资产</span></div>
    </div>
    <div class="plugin-source-grid plugin-asset-paths">
      <span>数据</span><code>${esc(paths.data_dir || '-')}</code>
      <span>缓存</span><code>${esc(paths.cache_dir || '-')}</code>
      <span>密钥</span><code>${esc(paths.secrets_dir || '-')}</code>
    </div>
    <p class="plugin-detail-text plugin-asset-note">更新插件会保留资产；卸载插件会清理这些目录。</p>
  </div>`;
}

function pluginFeatureActionOrder(feature) {
  const methods = feature.methods || {};
  const keys = Object.keys(methods);
  const preferred = PLUGIN_WORKFLOW_ACTIONS[feature.kind] || [];
  const ordered = preferred.filter(action => keys.includes(action));
  keys.forEach(action => {
    if (!ordered.includes(action)) ordered.push(action);
  });
  return ordered;
}

function pluginFeatureResultKey(pluginId, featureId, action) {
  return [pluginId, featureId, action].map(value => String(value || '')).join('::');
}

function renderPluginFeatureResult(pluginId, featureId) {
  const prefix = pluginFeatureResultKey(pluginId, featureId, '');
  const entryKey = Object.keys(_pluginFeatureResults).reverse().find(key => key.startsWith(prefix));
  if (!entryKey) return '';
  const entry = _pluginFeatureResults[entryKey];
  const text = JSON.stringify(entry.result == null ? null : entry.result, null, 2);
  return `<div class="plugin-workflow-result">
    <div class="plugin-workflow-result-head">
      <span>${esc(pluginActionLabel(entry.action))}</span>
      <button class="btn-apply" onclick="showPluginFeatureResult(_pluginFeatureResults['${escAttr(entryKey)}'].result)">展开</button>
    </div>
    <pre>${esc(text)}</pre>
  </div>`;
}

function renderPluginFeaturesSection(p) {
  const features = pluginFeatures(p);
  if (!features.length) return '';
  const enabled = pluginIsEnabled(p);
  return `<div class="plugin-detail-section">
    <h3>能力</h3>
    <div class="plugin-workflow-grid">${features.map(feature => {
      const methodKeys = pluginFeatureActionOrder(feature);
      const kind = String(feature.kind || 'integration');
      return `<div class="plugin-feature-card plugin-feature-card-${escAttr(kind)}">
        <div class="plugin-feature-main">
          <div class="plugin-feature-title-row">
            <span class="plugin-feature-label">${esc(feature.label || pluginFeatureKindLabel(feature.kind))}</span>
            <span class="plugin-feature-kind">${esc(pluginFeatureKindLabel(feature.kind))}</span>
          </div>
          ${feature.description ? `<span class="plugin-feature-desc">${esc(feature.description)}</span>` : ''}
        </div>
        ${methodKeys.length ? `<div class="plugin-feature-actions">${methodKeys.map(action => `<button class="btn-apply" ${enabled ? `onclick="event.stopPropagation(); executePluginFeatureAction('${escAttr(p.id)}','${escAttr(feature.id)}','${escAttr(action)}')"` : 'disabled title="插件已停用"'}>${esc(pluginActionLabel(action))}</button>`).join('')}</div>` : '<div class="plugin-empty-line">暂无可执行动作</div>'}
        ${renderPluginFeatureResult(p.id, feature.id)}
      </div>`;
    }).join('')}</div>
  </div>`;
}

function pluginFeatureActionSchema(feature, action) {
  const schemas = (feature && feature.params_schema) || {};
  const schema = schemas[action] || null;
  if (!schema || schema.type !== 'object' || !schema.properties) return null;
  return schema;
}

function renderPluginActionParamField(key, schema, requiredKeys) {
  const type = schema.type || 'string';
  const value = Object.prototype.hasOwnProperty.call(schema, 'default') ? schema.default : '';
  const label = schema.title || key;
  const required = requiredKeys.includes(key);
  const desc = schema.description ? `<span class="plugin-config-desc">${esc(schema.description)}</span>` : '';
  const attrs = `data-plugin-action-key="${escAttr(key)}" data-plugin-action-type="${escAttr(type)}"`;
  let control = '';
  if (Array.isArray(schema.enum)) {
    control = `<select ${attrs} class="plugin-config-input">
      ${schema.enum.map(item => `<option value="${escAttr(item)}" ${String(value) === String(item) ? 'selected' : ''}>${esc(item)}</option>`).join('')}
    </select>`;
  } else if (type === 'boolean') {
    control = `<label class="plugin-config-switch">
      <input ${attrs} type="checkbox" ${value ? 'checked' : ''}>
      <span>${value ? '开启' : '关闭'}</span>
    </label>`;
  } else if (type === 'integer' || type === 'number') {
    control = `<input ${attrs} class="plugin-config-input" type="number" value="${escAttr(value ?? '')}">`;
  } else if (type === 'array') {
    const text = Array.isArray(value) ? value.join('\n') : String(value || '');
    control = `<textarea ${attrs} class="plugin-config-input plugin-config-textarea">${esc(text)}</textarea>`;
  } else if (type === 'object') {
    const text = typeof value === 'object' ? JSON.stringify(value, null, 2) : String(value || '{}');
    control = `<textarea ${attrs} class="plugin-config-input plugin-config-textarea">${esc(text)}</textarea>`;
  } else {
    control = `<input ${attrs} class="plugin-config-input" value="${escAttr(value ?? '')}">`;
  }
  return `<label class="plugin-config-field">
    <span class="plugin-config-label">${esc(label)}${required ? ' *' : ''}</span>
    ${control}
    ${desc}
  </label>`;
}

function parsePluginTypedInput(el, typeAttr) {
  const type = el.getAttribute(typeAttr) || 'string';
  if (type === 'boolean') return Boolean(el.checked);
  if (type === 'integer') {
    const value = parseInt(el.value, 10);
    return Number.isFinite(value) ? value : 0;
  }
  if (type === 'number') {
    const value = parseFloat(el.value);
    return Number.isFinite(value) ? value : 0;
  }
  if (type === 'array') {
    return String(el.value || '').split('\n').map(item => item.trim()).filter(Boolean);
  }
  if (type === 'object') {
    const text = String(el.value || '').trim();
    return text ? JSON.parse(text) : {};
  }
  return el.value || '';
}

function collectPluginActionParams(schema) {
  if (!schema) return null;
  const params = {};
  const fields = Array.from(document.querySelectorAll('[data-plugin-action-key]'));
  fields.forEach(el => {
    const key = el.getAttribute('data-plugin-action-key');
    if (!key) return;
    params[key] = parsePluginTypedInput(el, 'data-plugin-action-type');
  });
  return params;
}

function showPluginFeatureActionModal(p, feature, action) {
  return new Promise(function (resolve) {
    var existing = document.getElementById('pluginActionModal');
    if (existing) existing.remove();

    const schema = pluginFeatureActionSchema(feature, action);
    const requiredKeys = Array.isArray(schema && schema.required) ? schema.required : [];
    const paramFields = schema
      ? Object.keys(schema.properties || {}).map(key => renderPluginActionParamField(key, schema.properties[key] || {}, requiredKeys)).join('')
      : '';
    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';
    overlay.id = 'pluginActionModal';
    overlay.innerHTML = `<div class="modal-box plugin-action-modal">
      <div class="modal-header"><h3>执行插件动作</h3></div>
      <div class="modal-body plugin-action-body">
        <div class="plugin-preview-title">
          <strong>${esc(feature.label || feature.id || '插件能力')}</strong>
          <span>${esc(action)}</span>
          <span>${esc(p.name || p.id)}</span>
        </div>
        ${schema
          ? `<div class="plugin-action-form">${paramFields || '<div class="plugin-empty-line">该动作不需要参数</div>'}</div>`
          : '<textarea id="pluginActionParamsJson" class="plugin-action-json" spellcheck="false">{}</textarea>'}
        <pre id="pluginActionResult" class="plugin-action-result" style="display:none;"></pre>
      </div>
      <div class="plugin-preview-actions">
        <button class="btn btn-primary" id="pluginActionRun" type="button">执行</button>
        <button class="btn btn-ghost" id="pluginActionCancel" type="button">取消</button>
      </div>
    </div>`;
    document.body.appendChild(overlay);

    function cleanup(value) { overlay.remove(); resolve(value); }
    const run = document.getElementById('pluginActionRun');
    const cancel = document.getElementById('pluginActionCancel');
    const textarea = document.getElementById('pluginActionParamsJson');
    const firstField = document.querySelector('[data-plugin-action-key]') || textarea;
    if (firstField) firstField.focus();
    if (run) run.onclick = function () {
      try {
        if (schema) {
          cleanup(collectPluginActionParams(schema) || {});
        } else {
          const text = String((textarea && textarea.value) || '').trim();
          cleanup(text ? JSON.parse(text) : {});
        }
      } catch(e) {
        showToast('参数 JSON 格式错误: ' + esc(String(e)), 'error');
      }
    };
    if (cancel) cancel.onclick = function () { cleanup(null); };
    overlay.addEventListener('click', function (e) { if (e.target === overlay) cleanup(null); });
  });
}

function showPluginFeatureResult(result) {
  var existing = document.getElementById('pluginActionResultModal');
  if (existing) existing.remove();
  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay';
  overlay.id = 'pluginActionResultModal';
  const text = JSON.stringify(result == null ? null : result, null, 2);
  overlay.innerHTML = `<div class="modal-box plugin-action-modal">
    <div class="modal-header"><h3>执行结果</h3></div>
    <div class="modal-body plugin-action-body">
      <pre class="plugin-action-result">${esc(text)}</pre>
    </div>
    <div class="plugin-preview-actions">
      <button class="btn btn-primary" id="pluginActionResultClose" type="button">完成</button>
    </div>
  </div>`;
  document.body.appendChild(overlay);
  const close = document.getElementById('pluginActionResultClose');
  const cleanup = function () { overlay.remove(); };
  if (close) close.onclick = cleanup;
  overlay.addEventListener('click', function (e) { if (e.target === overlay) cleanup(); });
}

function renderPluginCard(p) {
  var state = normalizePluginState(p.state);
  var enabled = pluginIsEnabled(p);
  var running = enabled && pluginIsRunning(p);
  var stateLabel = enabled ? (PLUGIN_STATE_LABEL[state] || PLUGIN_STATE_LABEL.stopped) : '已停用';
  var sc = running ? 'var(--green)' : 'var(--text-muted)';
  var dexTools = p.dex_tools || [];
  var features = pluginFeatures(p);
  var meta = pluginCategoryLabels(p).concat(features.slice(0, 2).map(feature => feature.label));
  if (features.length > 2) meta.push('+' + (features.length - 2));

  return `<div class="plugin-card${running ? ' running' : ''}${enabled ? '' : ' disabled'}" onclick="showPluginDetail('${escAttr(p.id)}')">
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
      <div class="plugin-card-meta">${esc(meta.join(' · '))}</div>
    </div>
    <div class="plugin-card-actions" onclick="event.stopPropagation()">
      ${!enabled
        ? `<button class="btn-apply" onclick="setPluginEnabled('${escAttr(p.id)}', true)">启用</button>`
        : running
          ? `<button class="btn-apply btn-danger" onclick="stopPlugin('${escAttr(p.id)}')">停止</button>`
          : `<button class="btn-apply" onclick="startPlugin('${escAttr(p.id)}')">启动</button>`}
    </div>
  </div>`;
}

function renderPluginOverviewSection(p) {
  const features = pluginFeatures(p);
  const risk = pluginPermissionRisk(p);
  const categories = pluginCategoryLabels(p).join(' / ');
  const accountText = pluginHasAccountFeature(p) ? pluginAccountLabel(p) : '无需连接';
  const toolCount = (p.dex_tools || []).length;
  const permissionList = p.permission_details && p.permission_details.length ? p.permission_details : (p.permissions || []);
  const permissionCount = permissionList.length;
  const items = [
    { label: '分类', value: categories || '工具插件' },
    { label: '能力', value: `${features.length || 0} 项${toolCount ? ` · ${toolCount} 工具` : ''}` },
    { label: '权限', value: `${pluginRiskLabel(risk)} · ${permissionCount} 项`, cls: `risk-${PLUGIN_RISK_CLASS[risk] || 'low'}` },
    { label: '连接', value: accountText }
  ];
  return `<div class="plugin-detail-section plugin-overview-section">
    <div class="plugin-overview-grid">
      ${items.map(item => `<div class="plugin-overview-item">
        <span>${esc(item.label)}</span>
        <strong class="${escAttr(item.cls || '')}">${esc(item.value)}</strong>
      </div>`).join('')}
    </div>
  </div>`;
}

// ── 插件详情页 ──
function showPluginDetail(id) {
  stopPluginAutoRefresh();
  _pluginDetailId = id;
  switchPanel('plugins');
}

function backToPluginList() {
  stopPluginEventRefresh();
  _pluginDetailId = null;
  switchPanel('plugins');
}

function renderPluginDetail() {
  var p = _pluginsData.find(p => p.id === _pluginDetailId);
  if (!p) {
    _pluginDetailId = null;
    return renderPluginsPanel();
  }
  var state = normalizePluginState(p.state);
  var enabled = pluginIsEnabled(p);
  var running = enabled && pluginIsRunning(p);
  var stateLabel = enabled ? (PLUGIN_STATE_LABEL[state] || PLUGIN_STATE_LABEL.stopped) : '已停用';
  var sc = running ? 'var(--green)' : 'var(--text-muted)';
  var perms = p.permissions || [];
  var accounts = p.accounts || [];
  var dexTools = p.dex_tools || [];
  var tags = p.tags || [];
  var hasAccountFeature = pluginHasAccountFeature(p);

  var accountsHtml = '';
  if (accounts.length) {
    accountsHtml = accounts.map(a => {
      var asc = ACCOUNT_STATUS_COLOR[a.status] || 'var(--text-muted)';
      var isConnected = a.status === 'connected';
      return `<div class="pc-account-row">
        <span class="pc-account-name">${esc(a.name || a.account_id)}</span>
        <span class="pc-account-status" style="color:${asc}">
          <span class="plugin-status-dot" style="color:${asc};background:${asc};"></span>
          ${ACCOUNT_STATUS_LABEL[a.status] || a.status}
        </span>
        <span class="pc-account-actions">
          ${isConnected
            ? `<button class="btn-apply btn-danger" onclick="stopPluginAccount('${escAttr(p.id)}','${escAttr(a.account_id)}')">停止连接</button>`
            : `<button class="btn-apply" onclick="scanAndStart('${escAttr(p.id)}','${escAttr(a.account_id)}')">启动连接</button>`}
          <button class="btn-apply" onclick="scanPluginQr('${escAttr(p.id)}','${escAttr(a.account_id)}')" style="background:var(--accent-dim);color:var(--accent);border-color:var(--accent);">认证</button>
          <button class="btn-apply btn-danger" onclick="deletePluginAccount('${escAttr(p.id)}','${escAttr(a.account_id)}')">删除</button>
        </span>
      </div>`;
    }).join('');
  } else {
    accountsHtml = '<div class="plugin-empty-line">暂无连接</div>';
  }
  var infoTags = [
    ...pluginCategoryLabels(p).map(label => `<span class="plugin-perm-tag">${esc(label)}</span>`),
    `<span class="plugin-perm-tag">类型 ${esc(pluginKindLabel(p))}</span>`,
    ...(tags || []).map(tag => `<span class="plugin-perm-tag">${esc(tag)}</span>`)
  ].join('');

  return `<button class="page-back-button plugin-detail-back" onclick="backToPluginList()" aria-label="返回插件市场"><span class="line-action-icon line-action-icon-back" aria-hidden="true"></span></button>

  <div class="plugin-detail-shell">
  <div class="plugin-detail-header">
    <div class="plugin-detail-title">
      <h2>
        <span class="plugin-status-dot" style="color:${sc};background:${sc};"></span>
        ${esc(p.name)}
      </h2>
      <div class="meta">
        <span>${esc(pluginKindLabel(p))}</span>
        <span>版本 v${esc(p.version)}</span>
        ${p.author ? `<span>作者 ${esc(p.author)}</span>` : ''}
        <span class="plugin-state-badge ${running ? 'on' : 'off'}"><span class="dot"></span>${stateLabel}</span>
      </div>
    </div>
    <div class="plugin-detail-actions">
      ${enabled
        ? `<button class="btn-apply" onclick="setPluginEnabled('${escAttr(p.id)}', false)" style="background:rgba(107,127,168,0.08);color:var(--text-muted);border-color:rgba(107,127,168,0.18);">停用插件</button>`
        : `<button class="btn-apply" onclick="setPluginEnabled('${escAttr(p.id)}', true)">启用插件</button>`}
      ${enabled
        ? (running
          ? `<button class="btn-apply btn-danger" onclick="stopPlugin('${escAttr(p.id)}')">停止插件</button>`
          : `<button class="btn-apply" onclick="startPlugin('${escAttr(p.id)}')">启动插件</button>`)
        : ''}
    </div>
  </div>

  ${p.description ? `<div class="plugin-detail-section">
    <h3>简介</h3>
    <p class="plugin-detail-text">${esc(p.description)}</p>
    ${infoTags ? `<div class="plugin-perm-tags">${infoTags}</div>` : ''}
  </div>` : ''}

  ${renderPluginOverviewSection(p)}

  ${renderPluginFeaturesSection(p)}

  ${renderPluginToolsSection(dexTools)}

  ${renderPluginConfigSection(p)}

	  ${renderPluginPermissionsSection(p)}

	  ${renderPluginSourceSection(p)}

	  ${renderPluginAssetsSection(p)}

	  ${hasAccountFeature ? renderPluginAccountsSection(p, accountsHtml) : ''}

  ${renderPluginEventsSection(p)}

  <div class="plugin-danger-zone">
    <h3>卸载插件</h3>
    <p class="plugin-detail-text">卸载后将删除插件所有文件，此操作不可恢复。</p>
    <button class="btn-apply btn-danger" onclick="uninstallPlugin('${escAttr(p.id)}')">卸载插件</button>
  </div>
  </div>`;
}

function parsePluginConfigInput(el) {
  const type = el.getAttribute('data-plugin-config-type') || 'string';
  if (type === 'boolean') return Boolean(el.checked);
  if (type === 'integer') {
    const value = parseInt(el.value, 10);
    return Number.isFinite(value) ? value : 0;
  }
  if (type === 'number') {
    const value = parseFloat(el.value);
    return Number.isFinite(value) ? value : 0;
  }
  if (type === 'array') {
    return String(el.value || '').split('\n').map(item => item.trim()).filter(Boolean);
  }
  if (type === 'object') {
    const text = String(el.value || '').trim();
    return text ? JSON.parse(text) : {};
  }
  return el.value || '';
}

async function savePluginConfig(pluginId) {
  const p = _pluginsData.find(p => p.id === pluginId);
  if (!p) return;
  const nextConfig = Object.assign({}, p.config || {});
  const fields = Array.from(document.querySelectorAll('[data-plugin-config-key]'));
  try {
    fields.forEach(el => {
      const key = el.getAttribute('data-plugin-config-key');
      if (!key) return;
      nextConfig[key] = parsePluginConfigInput(el);
    });
  } catch(e) {
    showToast('配置格式错误: ' + esc(String(e)), 'error');
    return;
  }
  try {
    await invoke('update_plugin_config', { pluginId: pluginId, config: nextConfig });
    showToast('配置已保存', 'success');
    await loadPluginsData();
  } catch(e) { showToast('保存失败: ' + esc(String(e)), 'error'); }
}

async function clearPluginCache(pluginId) {
  const ok = await showConfirm('清理该插件缓存？长期数据、密钥和连接资产不会被删除。');
  if (!ok) return;
  try {
    await invoke('clear_plugin_cache', { pluginId: pluginId });
    showToast('缓存已清理', 'success');
    await loadPluginsData();
    await loadPluginEvents(pluginId, true);
  } catch(e) {
    showToast('清理失败: ' + esc(String(e)), 'error');
  }
}

async function executePluginFeatureAction(pluginId, featureId, action) {
  const p = _pluginsData.find(p => p.id === pluginId);
  if (!p) {
    showToast('插件不存在: ' + esc(pluginId), 'error');
    return;
  }
  if (!pluginIsEnabled(p)) {
    showToast('插件已停用，请先启用', 'error');
    return;
  }
  const feature = pluginFeatures(p).find(feature => String(feature.id) === String(featureId)) || { id: featureId, label: featureId };
  const params = await showPluginFeatureActionModal(p, feature, action);
  if (params === null) return;
  let confirmed = false;
  if (p.permission_risk === 'high') {
    confirmed = await showConfirm('该插件包含高风险权限，确定执行这个能力动作吗？');
    if (!confirmed) return;
  }
  try {
    const result = await invoke('execute_plugin_feature', {
      pluginId: pluginId,
      featureId: featureId,
      action: action,
      params: params,
      confirmed: confirmed
    });
    showToast('插件动作已执行', 'success');
    _pluginFeatureResults[pluginFeatureResultKey(pluginId, featureId, action)] = {
      action: action,
      result: result,
      ts: Date.now()
    };
    showPluginFeatureResult(result);
    await loadPluginsData();
  } catch(e) {
    showToast('执行失败: ' + esc(String(e)), 'error');
    await loadPluginEvents(pluginId, true);
  }
}

async function setPluginEnabled(id, enabled) {
  try {
    await invoke('set_plugin_enabled', { pluginId: id, enabled: Boolean(enabled) });
    showToast(enabled ? '插件已启用' : '插件已停用', 'success');
    await loadPluginsData();
  } catch(e) {
    showToast('更新失败: ' + esc(String(e)), 'error');
    await loadPluginEvents(id, true);
  }
}

async function startPlugin(id) {
  try {
    await invoke('start_plugin', { pluginId: id });
    showToast('插件已启动', 'success');
    await loadPluginsData();
  } catch(e) {
    showToast('启动失败: ' + esc(String(e)), 'error');
    await loadPluginEvents(id, true);
  }
}

async function stopPlugin(id) {
  try {
    await invoke('stop_plugin', { pluginId: id });
    showToast('插件已停止', 'success');
    await loadPluginsData();
  } catch(e) {
    showToast('停止失败: ' + esc(String(e)), 'error');
    await loadPluginEvents(id, true);
  }
}

async function uninstallPlugin(id) {
  var ok = await showConfirm('确定要卸载该插件吗？此操作不可恢复。');
        if (!ok) return;
  try {
    await invoke('uninstall_plugin', { pluginId: id });
    showToast('插件已卸载', 'success');
    await loadPluginsData();
  } catch(e) { showToast('卸载失败: ' + esc(String(e)), 'error'); }
}

async function startPluginAccount(pluginId, accountId) {
  try {
    await invoke('start_plugin_account', { pluginId: pluginId, accountId: accountId });
    showToast('连接启动指令已发送', 'success');
    await loadPluginsData();
  } catch(e) { showToast('启动连接失败: ' + esc(String(e)), 'error'); }
}

async function stopPluginAccount(pluginId, accountId) {
  try {
    await invoke('stop_plugin_account', { pluginId: pluginId, accountId: accountId });
    showToast('连接已停止', 'success');
    await loadPluginsData();
  } catch(e) { showToast('停止连接失败: ' + esc(String(e)), 'error'); }
}

async function scanAndStart(pluginId, accountId) {
  // 先尝试直接启动连接
  try {
    const r = await invoke('start_plugin_account', { pluginId: pluginId, accountId: accountId });
    showToast('连接已启动', 'success');
    await loadPluginsData();
    return;
  } catch(e) {
    // 如果未登录，自动触发认证流程
    if (String(e).includes('bot_token') || String(e).includes('扫码') || String(e).includes('登录')) {
      showToast('需要先认证，正在获取二维码...');
    } else {
      showToast('启动失败: ' + esc(String(e)), 'error');
      return;
    }
  }
  // 扫码登录
  var oc = document.getElementById('qrOverlayContent');
  var oa = document.getElementById('qrOverlayAccount');
  if (!oc || !oa) { showToast('二维码弹窗未初始化，请重启 GUI', 'error'); return; }
  oc.innerHTML = '<span style="color:var(--text-muted);">获取二维码中...</span>';
  oa.textContent = '连接：' + accountId + '（认证后自动启动）';
  if (!showQrOverlay()) return;
  try {
    var result2 = await invoke('get_plugin_qrcode', { pluginId: pluginId, accountId: accountId });
    var url2 = result2.qrcode_data_url || result2.data_url || '';
    if (url2) {
      oc.innerHTML = `<img src="${esc(url2)}" alt="QR"><p class="qr-hint" style="color:var(--amber);">请扫码完成认证，认证后连接将自动启动</p>`;
      // 轮询状态，连接成功后自动启动
      var pollCount = 0;
      clearPluginQrPolling();
      _pluginQrPollTimer = setInterval(async () => {
        pollCount++;
        try {
          var status = await invoke('query_plugin_status', { pluginId: pluginId, accountId: accountId });
          if (status && status.status === 'connected') {
            clearPluginQrPolling();
            oc.innerHTML = '<span style="color:var(--green);">认证成功，正在启动连接...</span>';
            try {
              await invoke('start_plugin_account', { pluginId: pluginId, accountId: accountId });
              showToast('连接已启动', 'success');
              closeQrOverlay();
            } catch(e2) {
              oc.innerHTML = '<span style="color:var(--red);">连接启动失败: ' + esc(String(e2)) + '</span>';
            }
            await loadPluginsData();
          }
        } catch(ee) {}
        if (pollCount > 60) { clearPluginQrPolling(); oc.innerHTML = '<span style="color:var(--red);">登录超时，请重试</span>'; }
      }, 2000);
    } else {
      oc.innerHTML = '<span style="color:var(--red);">' + esc(JSON.stringify(result2)) + '</span>';
    }
  } catch(e2) {
    oc.innerHTML = '<span style="color:var(--red);">获取二维码失败: ' + esc(String(e2)) + '</span>';
  }
}

async function deletePluginAccount(pluginId, accountId) {
  showToast('正在删除连接 ' + esc(accountId) + '...');
  try {
    await invoke('remove_plugin_account_asset', { pluginId: pluginId, accountId: accountId });
    showToast('连接已删除', 'success');
    await loadPluginsData();
  } catch(e) { showToast('删除失败: ' + esc(String(e)), 'error'); }
}

async function addPluginAccount(pluginId) {
  const input = document.getElementById('addAccountId_' + pluginId);
  const accountId = (input && input.value || '').trim();
  if (!accountId) { showToast('请输入连接 ID'); return; }
  try {
    await invoke('upsert_plugin_account_asset', {
      pluginId: pluginId,
      accountId: accountId,
      asset: { name: accountId, enabled: true }
    });
    showToast('连接已添加', 'success');
    if (input) input.value = '';
    await loadPluginsData();
  } catch(e) { showToast('添加失败: ' + esc(String(e)), 'error'); }
}

async function scanPluginQr(pluginId, accountId) {
  if (!accountId) { showToast('请先添加连接'); return; }
  var oc = document.getElementById('qrOverlayContent');
  var oa = document.getElementById('qrOverlayAccount');
  if (!oc || !oa) { showToast('二维码弹窗未初始化，请重启 GUI', 'error'); return; }
  oc.innerHTML = '<span style="color:var(--text-muted);">获取二维码中...</span>';
  oa.textContent = '连接：' + accountId + '（认证后自动启动）';
  if (!showQrOverlay()) return;
  try {
    var result = await invoke('get_plugin_qrcode', { pluginId: pluginId, accountId: accountId });
    var url = result.qrcode_data_url || result.data_url || '';
    if (url) {
      oc.innerHTML = `<img src="${esc(url)}" alt="QR"><p class="qr-hint" style="color:var(--amber);">请扫码完成认证，认证后连接将自动启动</p>`;
      // 轮询认证状态，确认后自动启动连接
      var pollCount = 0;
      clearPluginQrPolling();
      _pluginQrPollTimer = setInterval(async () => {
        pollCount++;
        try {
          var status = await invoke('query_plugin_status', { pluginId: pluginId, accountId: accountId });
          if (status && status.status === 'connected') {
            clearPluginQrPolling();
            oc.innerHTML = '<span style="color:var(--green);">认证成功，正在启动连接...</span>';
            try {
              await invoke('start_plugin_account', { pluginId: pluginId, accountId: accountId });
              showToast('连接已启动', 'success');
              closeQrOverlay();
            } catch(e2) {
              oc.innerHTML = '<span style="color:var(--red);">连接启动失败: ' + esc(String(e2)) + '</span>';
            }
            await loadPluginsData();
          }
        } catch(ee) {}
        if (pollCount > 60) { clearPluginQrPolling(); oc.innerHTML = '<span style="color:var(--red);">登录超时，请重试</span>'; }
      }, 2000);
    } else {
      oc.innerHTML = '<span style="color:var(--red);">' + esc(JSON.stringify(result)) + '</span>';
    }
  } catch(e) { oc.innerHTML = '<span style="color:var(--red);">获取失败: ' + esc(String(e)) + '</span>'; }
}
