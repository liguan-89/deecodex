const assert = require('assert');
const fs = require('fs');
const path = require('path');
const vm = require('vm');

const context = {
  console,
  window: { DeeCodexTauri: { invoke: async () => ({}) } },
  document: {
    documentElement: { setAttribute() {} },
    getElementById() { return null; },
    querySelectorAll() { return []; },
  },
  deeStorage: {
    data: {},
    getItem(key) { return this.data[key] || ''; },
    setItem(key, value) { this.data[key] = String(value); },
  },
  accountsData: {
    client_counts: { codex: 1, claude_code: 1, openclaw: 1, hermes: 1, generic_client: 1 },
    accounts: [
      { id: 'cx1', name: 'Codex OpenRouter', client_kind: 'codex', provider: 'openrouter', upstream: 'https://openrouter.ai/api/v1', client_options: {} },
      { id: 'cc1', name: 'Claude DeepSeek', client_kind: 'claude_code', provider: 'anthropic', upstream: 'https://api.deepseek.com/anthropic', default_model: 'deepseek-v4-pro', client_options: { auth_env: 'ANTHROPIC_AUTH_TOKEN', model_map: { default: 'deepseek-v4-pro', sonnet: 'deepseek-v4-flash' }, env: { ENABLE_TOOL_SEARCH: 'true' }, claude_custom_filter_enabled: true, claude_custom_filter_rules: ['x-custom-cache-noise:'] }, last_check: { ok: true, message: 'Claude Code 配置已准备' } },
      { id: 'oc1', name: 'OpenClaw OpenRouter', client_kind: 'openclaw', provider: 'openrouter', upstream: 'https://openrouter.ai/api/v1', default_model: 'anthropic/claude-sonnet-4.5', client_options: { api_key_env: 'OPENROUTER_API_KEY', model_map: { default: 'anthropic/claude-sonnet-4.5', image: 'openai/gpt-4o-mini' } } },
      { id: 'hm1', name: 'Hermes MiniMax', client_kind: 'hermes', provider: 'minimax', upstream: 'https://api.minimaxi.com/v1', default_model: 'MiniMax-M2.7', client_options: { config_path: '~/.hermes/config.yaml', env_path: '~/.hermes/.env', api_key_env: 'MINIMAX_API_KEY', model_map: { default: 'MiniMax-M2.7', vision: 'MiniMax-M2.7' } }, last_check: { ok: false, message: 'Hermes 密钥为空' } },
      { id: 'gc1', name: '通用 Env', client_kind: 'generic_client', provider: 'custom', upstream: 'https://api.example.com/v1', default_model: 'gpt-5', client_options: { config_path: '~/.deecodex/client-env', api_key_env: 'OPENAI_API_KEY', model_map: { default: 'gpt-5', fast: 'gpt-5.4-mini' } } },
    ],
  },
  clientProfiles: [
    { slug: 'codex', label: 'Codex', description: 'Codex 代理配置', config_path_hint: '~/.codex/config.toml', model_slots: [] },
    { slug: 'claude_code', label: 'Claude Code', description: 'Claude 本地配置', config_path_hint: '~/.claude/settings.json', model_slots: [
      { key: 'default', label: '主模型', target: 'ANTHROPIC_MODEL', required: true },
      { key: 'sonnet', label: 'Sonnet 模型', target: 'ANTHROPIC_DEFAULT_SONNET_MODEL' },
    ] },
    { slug: 'openclaw', label: 'OpenClaw', description: 'OpenClaw 配置', config_path_hint: '~/.openclaw/openclaw.json', model_slots: [
      { key: 'default', label: '默认 Agent 模型', target: 'agents.defaults.model', required: true },
      { key: 'image', label: '图片理解模型', target: 'agents.defaults.imageModel' },
    ] },
    { slug: 'hermes', label: 'Hermes', description: 'Hermes 配置', config_path_hint: '~/.hermes/config.yaml', model_slots: [
      { key: 'default', label: '主模型', target: 'model.default', required: true },
      { key: 'vision', label: '视觉辅助模型', target: 'auxiliary.vision.model' },
    ] },
    { slug: 'generic_client', label: '通用客户端', description: 'OpenAI 兼容 Env', config_path_hint: '~/.deecodex/client-env', model_slots: [
      { key: 'default', label: '默认模型', target: 'OPENAI_MODEL', required: true },
      { key: 'fast', label: '快速模型', target: 'OPENAI_FAST_MODEL' },
    ] },
  ],
  esc: value => String(value ?? '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;'),
  escAttr: value => String(value ?? '').replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;'),
  renderPanel() {},
  showToast() {},
  loadAccountsData: async () => {},
  invoke: async () => ({}),
};

context.window.window = context.window;
context.window.document = context.document;
context.window.deeStorage = context.deeStorage;

vm.createContext(context);
vm.runInContext(`
  var currentPanel = 'config';
  var currentConfig = {
    host: '127.0.0.1',
    port: 4446,
    max_body_mb: 64,
    chinese_thinking: true,
    data_dir: '.deecodex',
    prompts_dir: 'prompts',
    token_anomaly_prompt_max: 200000,
    token_anomaly_spike_ratio: 5,
    token_anomaly_burn_window: 120,
    token_anomaly_burn_rate: 500000,
    allowed_mcp_servers: '',
    allowed_computer_displays: '',
    computer_executor: 'disabled',
    computer_executor_timeout_secs: 30,
    mcp_executor_config: '{}',
    mcp_executor_timeout_secs: 30,
    playwright_state_dir: '',
    browser_use_bridge_url: '',
    browser_use_bridge_command: '',
    codex_auto_inject: true,
    codex_persistent_inject: false,
    codex_launch_with_cdp: false,
    cdp_port: 9222,
  };
  var selectedConfigClientKind = 'global';
  var selectedClientKind = 'codex';
  var accountsView = 'list';
  var CONFIG_CLIENT_STORAGE_KEY = 'deecodex.configClientKind';
  function normalizeClientKind(kind) {
    const value = String(kind || 'codex');
    if (value === 'ClaudeCode') return 'claude_code';
    if (value === 'Openclaw') return 'openclaw';
    if (value === 'GenericClient') return 'generic_client';
    if (value === 'Hermes') return 'hermes';
    return ['codex', 'claude_code', 'openclaw', 'hermes', 'generic_client'].includes(value) ? value : 'codex';
  }
  function accountClientKind(a) { return normalizeClientKind(a && (a.client_kind || a.target)); }
  function clientAccountHasIssue(a) { return Boolean((a && a.last_check && a.last_check.ok === false)); }
  function clientIcon(kind) { return '<span class="client-logo-box">' + kind + '</span>'; }
  function defaultApiKeyEnvForClient(account) {
    const kind = normalizeClientKind(account && account.client_kind);
    if (kind === 'claude_code') return 'ANTHROPIC_API_KEY';
    if (kind === 'hermes') return 'OPENROUTER_API_KEY';
    return 'OPENAI_API_KEY';
  }
  function serializeAccountForBackend(account) { return JSON.stringify(account); }
  function renderClientReport(report) { return report.message || ''; }
  function formatTimeShort() { return '刚刚'; }
`, context);

vm.runInContext(fs.readFileSync(path.join(__dirname, 'theme-config.js'), 'utf8'), context, { filename: 'theme-config.js' });
vm.runInContext(fs.readFileSync(path.join(__dirname, 'panels-core.js'), 'utf8'), context, { filename: 'panels-core.js' });

context.selectedConfigClientKind = 'global';
let html = context.renderConfig();
assert(html.includes('高级设置'));
assert(html.includes('网关运行'));
assert(html.includes('服务地址'));
assert(html.includes('服务监听端口'));
assert(html.includes('请求体上限'));
assert(html.includes('运行数据目录'));
assert(html.includes('<span>Claude</span>'));
assert(!html.includes('<span>Claude Code</span>'));
assert(!html.includes('Codex 响应翻译'));
assert(!html.includes('中文推理输出'));
assert(!html.includes('Hosted Prompts 目录'));
assert(!html.includes('Codex 请求治理'));
assert(!html.includes('Codex 工具执行'));
assert(!html.includes('MCP、Computer、Playwright 与 Browser-Use 的底层执行参数'));
assert(!html.includes('config-section-collapsed'));
assert(!html.includes('基础设置'));
assert(!html.includes('中文思考模式'));
assert(!html.includes('安全策略 & 执行器'));
assert(!html.includes('工具执行策略'));
assert(!html.includes('自动注入 Codex 配置'));
assert(!html.includes('Codex 配置注入'));
assert(!html.includes('Codex CDP 调试'));
assert(!html.includes('~/.codex/config.toml'));

context.selectedConfigClientKind = 'codex';
html = context.renderConfig();
assert(html.includes('仅配置 Codex 客户端专用行为'));
assert(html.includes('Codex 配置注入'));
assert(html.includes('Codex CDP 调试'));
assert(html.includes('不属于全局网关设置'));
assert(html.includes('自动注入 Codex 配置'));
assert(html.includes('~/.codex/config.toml'));
assert(html.includes('Codex 响应翻译'));
assert(html.includes('中文推理输出'));
assert(html.includes('Hosted Prompts 目录'));
assert(html.includes('Codex 请求治理'));
assert(html.includes('Codex 工具执行'));
assert(html.includes('Codex Responses tools 的 MCP、Computer、Playwright 与 Browser-Use'));
assert(html.includes('config-section-collapsed'));

context.selectedConfigClientKind = 'claude_code';
html = context.renderConfig();
assert(html.includes('编程会话治理'));
assert(html.includes('权限模式'));
assert(html.includes('MCP 服务器'));
assert(html.includes('自定义过滤'));
assert(html.includes('x-custom-cache-noise:'));
assert(html.includes('Anthropic system 行过滤'));
assert(html.includes('claude doctor'));
assert(html.includes('claude mcp list'));
assert(!html.includes('config_client_auth_env'));
assert(!html.includes('config-client-model-input'));
assert(!html.includes('保存高级设置'));

context.selectedConfigClientKind = 'openclaw';
html = context.renderConfig();
assert(html.includes('Agent 网关治理'));
assert(html.includes('Gateway / Channels'));
assert(html.includes('执行审批'));
assert(html.includes('openclaw config validate --json'));
assert(html.includes('openclaw exec-policy show --json'));
assert(!html.includes('SecretRef 环境变量'));
assert(!html.includes('agents.defaults.model'));

context.selectedConfigClientKind = 'hermes';
html = context.renderConfig();
assert(html.includes('Agent 运行时治理'));
assert(html.includes('Skills / Tools'));
assert(html.includes('Sessions / Memory'));
assert(html.includes('hermes config check'));
assert(html.includes('hermes skills list'));
assert(!html.includes('config_client_env_path'));
assert(!html.includes('auxiliary.vision.model'));

context.selectedConfigClientKind = 'generic_client';
html = context.renderConfig();
assert(html.includes('兼容客户端模板'));
assert(html.includes('环境模板'));
assert(html.includes('OPENAI_BASE_URL'));
assert(html.includes('OPENAI_MODEL'));
assert(!html.includes('config_client_api_key_env'));
assert(!html.includes('Provider / Base URL'));
assert(!html.includes('写入客户端'));

console.log('config render smoke ok');
