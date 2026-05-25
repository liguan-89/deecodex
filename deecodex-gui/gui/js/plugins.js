// 插件状态常量
let _pluginsData = [];
let _pluginMarketplaceData = [];
let _pluginsRefreshTimer = null;
let _pluginsRefreshMs = 0;
let _pluginsAutoRefresh = true;
let _pluginQrPollTimer = null;
let _pluginPanelMode = 'market';
let _pluginCategoryFilter = 'all';
let _pluginKindFilter = 'all';
let _pluginFeatureFilter = 'all';
let _pluginSearch = '';
let _pluginDevOpen = Boolean(window.deeStorage && window.deeStorage.getItem('deecodex.pluginDevOpen') === '1');
let _pluginDevDraft = {
  templateId: '',
  pluginId: '',
  name: '',
  root: '',
  path: ''
};
let _pluginEventsById = {};
let _pluginEventRefreshTimer = null;
let _pluginEventRefreshId = null;

var PLUGIN_STATE_LABEL = {
  installed: '已停止', starting: '启动中', running: '运行中',
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
var PLUGIN_KIND_LABEL = {
  tool: '工具',
  channel: '通道',
  provider: '供应商',
  datasource: '数据源',
  automation: '自动化',
  integration: '集成'
};
var PLUGIN_FEATURE_KIND_LABEL = {
  dex_tool: 'DEX 工具',
  model_provider: '模型供应商',
  datasource: '数据源',
  automation: '自动化',
  workspace: '工作区',
  channel: '通道',
  ui_panel: '界面扩展',
  connection: '连接',
  config: '配置'
};
var PLUGIN_RISK_LABEL = { low: '低', medium: '中', high: '高' };
var PLUGIN_RISK_CLASS = { low: 'low', medium: 'medium', high: 'high' };
var PLUGIN_ACTION_LABEL = {
  search: '搜索',
  read: '读取',
  status: '状态',
  run: '运行',
  start: '启动',
  stop: '停止',
  sync: '同步',
  list: '列表',
  models: '模型',
  test: '测试',
  login: '认证'
};
var PLUGIN_WORKFLOW_ACTIONS = {
  datasource: ['search', 'read', 'status'],
  automation: ['run', 'status', 'stop'],
  workspace: ['list', 'read', 'write', 'status'],
  model_provider: ['models', 'status', 'test'],
  provider: ['models', 'status', 'test'],
  channel: ['start', 'status', 'stop', 'login'],
  connection: ['login', 'status', 'start', 'stop'],
  dex_tool: ['status', 'run']
};
var PLUGIN_CATEGORY_DEFS = [
  { key: 'all', label: '全部' },
  { key: 'tool', label: '工具插件' },
  { key: 'datasource', label: '数据源' },
  { key: 'automation', label: '自动化' },
  { key: 'account', label: '账号连接' },
  { key: 'model', label: '模型供应商' },
  { key: 'channel', label: '通讯通道' },
  { key: 'ui', label: 'UI 扩展' }
];
var PLUGIN_CATEGORY_LABEL = PLUGIN_CATEGORY_DEFS.reduce((map, item) => {
  map[item.key] = item.label;
  return map;
}, {});
var PLUGIN_CATEGORY_PRIORITY = ['datasource', 'automation', 'account', 'model', 'channel', 'ui', 'tool'];

var _pluginDetailId = null;
var _pluginFeatureResults = {};

function normalizePluginState(state) {
  if (state === 'starting' || state === 'running' || state === 'error') return state;
  return 'stopped';
}

function pluginIsRunning(p) {
  return normalizePluginState(p && p.state) === 'running';
}

function pluginIsEnabled(p) {
  return !p || p.enabled !== false;
}

function pluginKindLabel(p) {
  const kind = String((p && p.kind) || 'tool').trim() || 'tool';
  return PLUGIN_KIND_LABEL[kind] || kind;
}

function pluginFeatureKindLabel(kind) {
  const value = String(kind || '').trim();
  return PLUGIN_FEATURE_KIND_LABEL[value] || PLUGIN_KIND_LABEL[value] || value || '能力';
}

function pluginRiskLabel(risk) {
  return PLUGIN_RISK_LABEL[String(risk || 'low')] || risk || '低';
}

function pluginActionLabel(action) {
  const value = String(action || '').trim();
  return PLUGIN_ACTION_LABEL[value] || value || '执行';
}

function pluginSchemaProperties(p) {
  return (p && p.config_schema && p.config_schema.properties) || {};
}

function pluginAccountLabel(p) {
  const account = p && p.account;
  const label = account && typeof account.label === 'string' ? account.label.trim() : '';
  return label || '连接';
}

function pluginHasAccountFeature(p) {
  if (!p) return false;
  if (p.account && p.account.enabled !== false) return true;
  const props = pluginSchemaProperties(p);
  if (props.accounts) return true;
  if ((p.accounts || []).length) return true;
  const configAccounts = p.config && p.config.accounts;
  if (configAccounts && typeof configAccounts === 'object' && Object.keys(configAccounts).length) return true;
  const perms = p.permissions || [];
  return perms.some(perm => {
    const value = String(perm || '').toLowerCase();
    return value === 'account' || value === 'accounts' || value.startsWith('account.');
  });
}

function pluginConfigKeys(p) {
  const props = pluginSchemaProperties(p);
  return Object.keys(props).filter(key => key !== 'accounts');
}

function pluginFeatures(p) {
  const features = (p && Array.isArray(p.features) ? p.features : []).map(feature => ({
    id: feature.id || feature.label || feature.kind,
    kind: feature.kind || 'integration',
    label: feature.label || pluginFeatureKindLabel(feature.kind),
    description: feature.description || '',
    methods: feature.methods || {},
    params_schema: feature.params_schema || {}
  }));
  const seen = new Set(features.map(feature => String(feature.id)));
  const addInferred = (id, kind, label, description) => {
    if (seen.has(id)) return;
    seen.add(id);
    features.push({ id, kind, label, description, methods: {} });
  };
  if ((p.dex_tools || []).length) {
    addInferred('dex-tools', 'dex_tool', 'DEX 工具', '向 DEX 助手暴露可调用工具');
  }
  if (pluginConfigKeys(p).length) {
    addInferred('config', 'config', '配置表单', '根据插件 schema 生成配置项');
  }
  if (pluginHasAccountFeature(p)) {
    addInferred('connection', 'connection', pluginAccountLabel(p), '需要维护连接或认证状态');
  }
  return features;
}

function pluginCategoryLabel(key) {
  return PLUGIN_CATEGORY_LABEL[String(key || '')] || key || '工具插件';
}

function pluginCategoryKeys(p) {
  const keys = new Set();
  const kind = String((p && p.kind) || 'tool').trim() || 'tool';
  const features = pluginFeatures(p);
  const featureKinds = features.map(feature => String(feature.kind || '').trim()).filter(Boolean);
  const has = value => kind === value || featureKinds.includes(value);

  if (has('datasource')) keys.add('datasource');
  if (has('automation') || has('workspace')) keys.add('automation');
  if (pluginHasAccountFeature(p) || has('connection')) keys.add('account');
  if (kind === 'provider' || has('model_provider')) keys.add('model');
  if (has('channel')) keys.add('channel');
  if (has('ui_panel')) keys.add('ui');
  if ((p && (p.dex_tools || []).length) || kind === 'tool' || has('dex_tool') || !keys.size) {
    keys.add('tool');
  }
  return Array.from(keys);
}

function pluginPrimaryCategory(p) {
  const keys = pluginCategoryKeys(p);
  return PLUGIN_CATEGORY_PRIORITY.find(key => keys.includes(key)) || keys[0] || 'tool';
}

function pluginCategoryLabels(p) {
  return pluginCategoryKeys(p).map(pluginCategoryLabel);
}

function pluginPermissionRisk(p) {
  const direct = String((p && p.permission_risk) || '').trim();
  if (direct) return direct;
  const details = (p && p.permission_details) || [];
  if (details.some(item => item && item.risk === 'high')) return 'high';
  if (details.some(item => item && item.risk === 'medium')) return 'medium';
  return 'low';
}

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
    const isUpdate = Boolean(preview.already_installed);
    const statusLabel = isUpdate ? '已安装' : '可安装';
    const categories = pluginCategoryLabels(manifest);
    const compatibility = pluginCompatibility(preview);
    const canInstall = compatibility.compatible !== false;
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
            <span class="plugin-preview-status ${isUpdate ? 'installed' : 'available'}">${esc(statusLabel)}</span>
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
        <button class="btn btn-primary" id="pluginPreviewOk" type="button" ${canInstall ? '' : 'disabled'}>${canInstall ? (isUpdate ? '更新' : '安装') : '不可安装'}</button>
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

function pluginTemplateItems() {
  return (_pluginMarketplaceData || []).filter(item => item.template);
}

function renderPluginDevBar() {
  const templates = pluginTemplateItems();
  const options = templates.length
    ? templates.map(item => `<option value="${escAttr(item.id)}" ${_pluginDevDraft.templateId === item.id ? 'selected' : ''}>${esc(item.name || item.id)}</option>`).join('')
    : '<option value="">暂无模板</option>';
  return `<div class="plugin-dev-entry" id="pluginDevEntry">
    <div class="plugin-dev-head">
      <button class="plugin-dev-toggle ${_pluginDevOpen ? 'active' : ''}" type="button" onclick="togglePluginDevBar()" aria-expanded="${_pluginDevOpen ? 'true' : 'false'}">
        <strong>开发入口</strong>
        <span>模板创建 / 校验 / 打包</span>
        <em>${_pluginDevOpen ? '收起' : '展开'}</em>
      </button>
      <button class="btn btn-ghost" type="button" onclick="openPluginMarketplaceRoot()">市场目录</button>
    </div>
    ${_pluginDevOpen ? `<div class="plugin-dev-bar">
      <div class="plugin-dev-group plugin-dev-create">
        <span class="plugin-dev-label">创建</span>
        <select class="history-select" id="pluginTemplateSelect" ${templates.length ? '' : 'disabled'}>
          ${options}
        </select>
        <input id="pluginDevId" placeholder="插件 ID" value="${escAttr(_pluginDevDraft.pluginId)}">
        <input id="pluginDevName" placeholder="插件名称" value="${escAttr(_pluginDevDraft.name)}">
        <input id="pluginDevRoot" placeholder="创建到目录" value="${escAttr(_pluginDevDraft.root)}">
        <button class="btn btn-ghost" onclick="browsePluginDevRoot()">目录</button>
        <button class="btn btn-primary" onclick="createPluginFromTemplate()">创建</button>
      </div>
      <div class="plugin-dev-group plugin-dev-tools">
        <span class="plugin-dev-label">检查</span>
        <input id="pluginDevPath" placeholder="插件目录或包路径" value="${escAttr(_pluginDevDraft.path)}">
        <button class="btn btn-ghost" onclick="browsePluginDevPath()">目录</button>
        <button class="btn btn-ghost" onclick="browsePluginDevPackage()">包</button>
        <button class="btn btn-ghost" onclick="validatePluginDevPath()">校验</button>
        <button class="btn btn-ghost" onclick="packagePluginDevPath()">打包</button>
        <button class="btn btn-ghost" onclick="openPluginDevPath()">打开</button>
      </div>
    </div>` : ''}
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
  return `<div class="plugin-market-card ${escAttr(status.cls)}" onclick="previewMarketplacePlugin('${escAttr(item.id)}')">
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
          ? `<button class="btn-apply" onclick="stopPlugin('${escAttr(p.id)}')" style="background:var(--red-dim);color:var(--red);border-color:var(--red);">停止</button>`
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
            ? `<button class="btn-apply" onclick="stopPluginAccount('${escAttr(p.id)}','${escAttr(a.account_id)}')" style="background:var(--red-dim);color:var(--red);border-color:var(--red);">停止连接</button>`
            : `<button class="btn-apply" onclick="scanAndStart('${escAttr(p.id)}','${escAttr(a.account_id)}')">启动连接</button>`}
          <button class="btn-apply" onclick="scanPluginQr('${escAttr(p.id)}','${escAttr(a.account_id)}')" style="background:var(--accent-dim);color:var(--accent);border-color:var(--accent);">认证</button>
          <button class="btn-apply" onclick="deletePluginAccount('${escAttr(p.id)}','${escAttr(a.account_id)}')" style="background:var(--red-dim);color:var(--red);border-color:var(--red);">删除</button>
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
          ? `<button class="btn-apply" onclick="stopPlugin('${escAttr(p.id)}')" style="background:var(--red-dim);color:var(--red);border-color:var(--red);">停止插件</button>`
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
    <button class="btn-apply" onclick="uninstallPlugin('${escAttr(p.id)}')" style="background:var(--red-dim);color:var(--red);border-color:var(--red);">卸载插件</button>
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

function pluginDevInputValue(id) {
  const el = document.getElementById(id);
  return el ? String(el.value || '').trim() : '';
}

function pluginDevMaybeValue(id, fallback) {
  const el = document.getElementById(id);
  return el ? String(el.value || '').trim() : (fallback || '');
}

function capturePluginDevDraft() {
  _pluginDevDraft = {
    templateId: pluginDevMaybeValue('pluginTemplateSelect', _pluginDevDraft.templateId),
    pluginId: pluginDevMaybeValue('pluginDevId', _pluginDevDraft.pluginId),
    name: pluginDevMaybeValue('pluginDevName', _pluginDevDraft.name),
    root: pluginDevMaybeValue('pluginDevRoot', _pluginDevDraft.root),
    path: pluginDevMaybeValue('pluginDevPath', _pluginDevDraft.path)
  };
}

function rememberPluginDevValue(id, value) {
  const next = value || '';
  if (id === 'pluginTemplateSelect') _pluginDevDraft.templateId = next;
  if (id === 'pluginDevId') _pluginDevDraft.pluginId = next;
  if (id === 'pluginDevName') _pluginDevDraft.name = next;
  if (id === 'pluginDevRoot') _pluginDevDraft.root = next;
  if (id === 'pluginDevPath') _pluginDevDraft.path = next;
}

function setPluginDevInputValue(id, value) {
  const el = document.getElementById(id);
  if (el) el.value = value || '';
  rememberPluginDevValue(id, value);
}

function showPluginDevResult(title, result) {
  var existing = document.getElementById('pluginDevResultModal');
  if (existing) existing.remove();
  const manifest = result && (result.manifest || (result.preview && result.preview.manifest));
  const compat = result && (result.compatibility || (result.preview && result.preview.compatibility));
  const rows = result && result.ok
    ? [
        ['状态', compat && compat.label ? compat.label : '通过'],
        ['插件', manifest ? `${manifest.name || manifest.id} v${manifest.version || '-'}` : '-'],
        ['ID', manifest ? manifest.id || '-' : '-'],
        ['类型', manifest ? pluginKindLabel(manifest) : '-'],
        ['路径', result.path || (result.preview && result.preview.source_path) || '-']
      ]
    : [
        ['状态', '失败'],
        ['原因', result && result.error ? result.error : '未知错误'],
        ['路径', result && result.path ? result.path : '-']
      ];
  const checks = compat && Array.isArray(compat.checks) ? compat.checks : [];
  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay';
  overlay.id = 'pluginDevResultModal';
  overlay.innerHTML = `<div class="modal-box plugin-action-modal">
    <div class="modal-header"><h3>${esc(title || '插件开发')}</h3></div>
    <div class="modal-body plugin-action-body">
      <div class="plugin-source-grid">${rows.map(row => `<span>${esc(row[0])}</span><code>${esc(row[1])}</code>`).join('')}</div>
      ${checks.length ? `<div class="plugin-compat-check-list plugin-dev-checks">${checks.map(check => `<div class="plugin-compat-check-row ${escAttr(check.tone || 'muted')}">
        <span>${esc(check.label || '-')}</span>
        <strong>${esc(check.value || '-')}</strong>
      </div>`).join('')}</div>` : ''}
    </div>
    <div class="plugin-preview-actions">
      <button class="btn btn-primary" id="pluginDevResultClose" type="button">完成</button>
    </div>
  </div>`;
  document.body.appendChild(overlay);
  const close = document.getElementById('pluginDevResultClose');
  const cleanup = function () { overlay.remove(); };
  if (close) close.onclick = cleanup;
  overlay.addEventListener('click', function (e) { if (e.target === overlay) cleanup(); });
}

async function browsePluginDevRoot() {
  try {
    const path = await invoke('browse_plugin_directory');
    if (path) setPluginDevInputValue('pluginDevRoot', path);
  } catch(e) {
    showToast('目录选择失败: ' + esc(String(e)), 'error');
  }
}

async function browsePluginDevPath() {
  try {
    const path = await invoke('browse_plugin_directory');
    if (path) setPluginDevInputValue('pluginDevPath', path);
  } catch(e) {
    showToast('插件目录选择失败: ' + esc(String(e)), 'error');
  }
}

async function browsePluginDevPackage() {
  try {
    const path = await invoke('browse_plugin_package');
    if (path) setPluginDevInputValue('pluginDevPath', path);
  } catch(e) {
    showToast('插件包选择失败: ' + esc(String(e)), 'error');
  }
}

function togglePluginDevBar() {
  capturePluginDevDraft();
  _pluginDevOpen = !_pluginDevOpen;
  if (window.deeStorage) {
    window.deeStorage.setItem('deecodex.pluginDevOpen', _pluginDevOpen ? '1' : '0');
  }
  const entry = document.getElementById('pluginDevEntry');
  if (entry) entry.outerHTML = renderPluginDevBar();
}

async function createPluginFromTemplate() {
  const templateId = pluginDevInputValue('pluginTemplateSelect');
  const pluginId = pluginDevInputValue('pluginDevId');
  const name = pluginDevInputValue('pluginDevName');
  const destinationDir = pluginDevInputValue('pluginDevRoot');
  if (!templateId || !pluginId || !name || !destinationDir) {
    showToast('请补全模板、插件 ID、名称和目录', 'error');
    return;
  }
  try {
    const result = await invoke('create_plugin_from_template', {
      templateId,
      pluginId,
      name,
      destinationDir
    });
    setPluginDevInputValue('pluginDevPath', result.path || '');
    showToast('插件草稿已创建', 'success');
    showPluginDevResult('插件草稿', result || {});
    await loadPluginsData();
  } catch(e) {
    showToast('创建失败: ' + esc(String(e)), 'error');
  }
}

async function validatePluginDevPath() {
  const path = pluginDevInputValue('pluginDevPath') || pluginDevInputValue('pluginZipPath');
  if (!path) {
    showToast('请选择插件目录或插件包', 'error');
    return;
  }
  try {
    const result = await invoke('validate_plugin_path', { path });
    showPluginDevResult('插件校验', result || {});
    showToast(result && result.ok ? '插件校验通过' : '插件校验失败', result && result.ok ? 'success' : 'error');
  } catch(e) {
    showToast('校验失败: ' + esc(String(e)), 'error');
  }
}

async function packagePluginDevPath() {
  const path = pluginDevInputValue('pluginDevPath');
  if (!path) {
    showToast('请选择插件目录', 'error');
    return;
  }
  try {
    const result = await invoke('package_plugin_directory', { path });
    if (result && result.path) setPluginDevInputValue('pluginZipPath', result.path);
    showToast('插件包已生成', 'success');
    showPluginDevResult('插件打包', result || {});
  } catch(e) {
    showToast('打包失败: ' + esc(String(e)), 'error');
  }
}

async function openPluginDevPath() {
  const path = pluginDevInputValue('pluginDevPath') || pluginDevInputValue('pluginZipPath');
  if (!path) {
    showToast('请选择要打开的路径', 'error');
    return;
  }
  try {
    await invoke('open_plugin_directory', { path });
  } catch(e) {
    showToast('打开失败: ' + esc(String(e)), 'error');
  }
}

async function openPluginMarketplaceRoot() {
  try {
    const result = await invoke('open_plugin_marketplace_directory');
    if (result && result.path) {
      showToast('已打开个人插件市场目录', 'success');
    }
  } catch(e) {
    showToast('打开市场目录失败: ' + esc(String(e)), 'error');
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

function clearPluginQrPolling() {
  if (_pluginQrPollTimer) {
    clearInterval(_pluginQrPollTimer);
    _pluginQrPollTimer = null;
  }
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

function setPluginRefreshInterval(val) {
  _pluginsRefreshMs = parseInt(val);
  if (_pluginsRefreshTimer) { clearInterval(_pluginsRefreshTimer); _pluginsRefreshTimer = setInterval(loadPluginsData, _pluginsRefreshMs); }
  syncPluginAutoRefreshUi();
}

async function refreshPlugins() {
  await loadPluginsData();
  showToast('已刷新');
}

window.stopPluginAutoRefresh = stopPluginAutoRefresh;
window.stopPluginEventRefresh = stopPluginEventRefresh;
window.clearPluginQrPolling = clearPluginQrPolling;

// ═══════════════════════════════════════════════════════════════
