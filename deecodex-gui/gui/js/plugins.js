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

// 插件页面其余视图拆分到 plugins-market/detail/dev/events.js。
