const assert = require('assert');
const fs = require('fs');
const path = require('path');
const vm = require('vm');

const context = {
  console,
  accountsData: {
    client_counts: { codex: 1, hermes: 1 },
    accounts: [
      {
        id: 'c1',
        name: 'Codex DeepSeek',
        provider: 'deepseek',
        client_kind: 'codex',
        upstream: 'https://api.deepseek.com/v1',
        api_key: 'sk-test123456',
        model_map: { 'gpt-5': 'deepseek-chat' },
        endpoints: [
          {
            id: 'ep1',
            kind: 'open_ai_chat',
            base_url: 'https://api.deepseek.com/v1',
            model_map: { 'gpt-5': 'deepseek-chat' },
            vision: { mode: 'off' },
          },
        ],
      },
      {
        id: 'c2',
        name: 'Codex 官方',
        provider: 'codex',
        client_kind: 'codex',
        auth_mode: 'oauth',
        upstream: 'https://chatgpt.com/backend-api/codex',
        api_key: 'sk-oauth',
        routing: { enabled: true, pool: 'codex-official', priority: 20, weight: 2 },
        client_options: {
          oauth: {
            email: '522d6@edu.kg.182796.xyz',
          },
          oauth_quota: {
            status_label: '额度冷却',
            plan_type: 'plus',
            next_recover_at: 1893456000,
            title: 'Codex 额度',
            hours_5_remaining_percent: 94,
            hours_5_used_percent: 6,
            hours_5_reset_at: 1893456000,
            weekly_remaining_percent: 82,
            weekly_used_percent: 18,
            weekly_reset_at: 1893888000,
            requests_5h: 6,
            requests_7d: 18,
            tokens_5h: 1200,
            tokens_7d: 4800,
            quota_exceeded: true,
            source: 'chatgpt_wham_usage',
            confidence_level: '精确',
            message: '官方返回额度限制，已按恢复时间暂停该账号。',
          },
        },
        runtime_state: {
          status: 'quota_exceeded',
          next_retry_after: 1893456000,
          success: 8,
          failed: 3,
          model_states: {
            'gpt-5': {
              status: 'cooling_down',
              next_retry_after: 1893456000,
              status_message: 'HTTP 429',
            },
          },
        },
        endpoints: [
          {
            id: 'ep2',
            kind: 'codex_official',
            base_url: 'https://chatgpt.com/backend-api/codex',
            model_map: { 'gpt-5': 'gpt-5' },
            vision: { mode: 'native' },
          },
        ],
      },
      {
        id: 'h1',
        name: 'Hermes MiniMax',
        provider: 'minimax',
        client_kind: 'hermes',
        upstream: 'https://api.minimaxi.com/v1',
        api_key: 'sk-test123456',
        default_model: 'MiniMax-M2.7',
        client_options: { api_key_env: 'MINIMAX_API_KEY', model_map: { default: 'MiniMax-M2.7' } },
        last_check: { ok: false, message: 'Hermes 密钥为空' },
      },
    ],
  },
  accountsView: 'edit',
  selectedClientKind: 'hermes',
  clientProfiles: [
    {
      slug: 'codex',
      label: 'Codex',
      description: 'Codex 代理配置',
      config_path_hint: '~/.codex/config.toml',
      default_base_url: 'https://openrouter.ai/api/v1',
      default_model: 'gpt-5.5',
      model_slots: [],
    },
    {
      slug: 'hermes',
      label: 'Hermes',
      description: 'Hermes 配置',
      config_path_hint: '~/.hermes/config.yaml',
      default_base_url: 'https://openrouter.ai/api/v1',
      default_model: 'anthropic/claude-sonnet-4',
      model_slots: [
        { key: 'default', label: '主模型', target: 'model.default', required: true },
        { key: 'vision', label: '视觉辅助模型', target: 'auxiliary.vision.model' },
      ],
    },
    {
      slug: 'claude_code',
      label: 'Claude Code',
      description: 'Claude Code 配置',
      config_path_hint: '~/.claude/settings.json',
      default_base_url: 'https://api.anthropic.com',
      default_model: 'claude-sonnet-4-5',
      model_slots: [
        { key: 'default', label: '主模型', target: 'ANTHROPIC_MODEL', required: true },
        { key: 'sonnet', label: 'Sonnet 模型', target: 'ANTHROPIC_DEFAULT_SONNET_MODEL' },
        { key: 'opus', label: 'Opus 模型', target: 'ANTHROPIC_DEFAULT_OPUS_MODEL' },
        { key: 'haiku', label: 'Haiku 模型', target: 'ANTHROPIC_DEFAULT_HAIKU_MODEL' },
      ],
    },
  ],
  providerPresets: [
    {
      slug: 'deepseek',
      label: 'DeepSeek',
      description: 'DeepSeek',
      default_upstream: 'https://api.deepseek.com/v1',
      known_models: ['deepseek-v4-pro[1m]', 'deepseek-v4-pro', 'deepseek-v4-flash'],
    },
    {
      slug: 'anthropic',
      label: 'Anthropic',
      description: 'Anthropic',
      default_upstream: 'https://api.anthropic.com',
      known_models: ['claude-sonnet-4-5'],
    },
    {
      slug: 'minimax',
      label: 'MiniMax',
      description: 'MiniMax',
      default_upstream: 'https://api.minimaxi.com/v1',
      known_models: ['MiniMax-M2.7'],
    },
    {
      slug: 'mimo',
      label: 'MiMo',
      description: 'MiMo',
      default_upstream: 'https://api.mimo-v2.com/v1',
      known_models: ['mimo-v2.5-pro'],
    },
    {
      slug: 'longcat',
      label: 'LongCat',
      description: 'LongCat',
      default_upstream: 'https://api.longcat.chat/v1',
      known_models: ['LongCat-Flash-Chat'],
    },
    {
      slug: 'kimi',
      label: 'Kimi',
      description: 'Kimi',
      default_upstream: 'https://api.moonshot.cn/v1',
      known_models: ['kimi-k2.5'],
    },
    {
      slug: 'glm',
      label: 'GLM',
      description: 'GLM',
      default_upstream: 'https://open.bigmodel.cn/api/paas/v4',
      known_models: ['glm-5.1'],
    },
    {
      slug: 'qwen',
      label: 'Qwen',
      description: 'Qwen',
      default_upstream: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
      known_models: ['qwen-max'],
    },
    {
      slug: 'custom',
      label: '自定义',
      description: '自定义',
      default_upstream: '',
      known_models: [],
    },
  ],
  endpointTemplates: [],
  upstreamModels: [],
  editingAccount: null,
  esc: value => String(value ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;'),
  escAttr: value => String(value ?? '')
    .replace(/&/g, '&amp;')
    .replace(/"/g, '&quot;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;'),
  trunc: (value, max) => {
    const text = String(value ?? '');
    return text.length > max ? text.slice(0, max - 3) + '...' : text;
  },
  maskKey: value => value ? 'sk-t****3456' : '',
  document: {
    getElementById: () => ({
      classList: { toggle: () => {} },
      innerHTML: '',
    }),
  },
};

vm.createContext(context);
const source = fs.readFileSync(path.join(__dirname, 'accounts.js'), 'utf8');
vm.runInContext(source, context, { filename: 'accounts.js' });

context.editingAccount = context.accountsData.accounts.find(account => account.id === 'h1');
context.accountsView = 'edit';
const editablePanel = context.renderAccountsPanel();
assert(editablePanel.includes('accounts-form-shell accounts-edit-shell'));
assert(editablePanel.includes('accounts-scroll-region accounts-form-scroll'));

const switcher = context.renderClientSwitcher(context.accountsData.accounts);
assert(switcher.includes('client-tab active has-issues'));
assert(switcher.includes('Hermes'));

const detail = context.renderClientAccountDetail();
assert(detail.includes('Hermes .env Key'));
assert(detail.includes('最近备份'));
assert(!detail.includes('account-section-desc'));
assert(!detail.includes('只记录外部客户端配置操作'));
assert(detail.includes('page-back-button account-back-link'));
assert(detail.includes('客户端模型映射'));
assert(detail.includes('model.default'));
assert(detail.includes('model-map-head client-model-map-head client-model-template'));
assert(detail.includes('model-row client-model-row client-model-template'));
assert(detail.includes('编辑配置文件'));

const report = context.renderClientReport({
  ok: true,
  message: 'Hermes 配置已准备',
  risk_level: 'low',
  schema_ok: true,
  recoverable: true,
  secret_source: '~/.hermes/.env MINIMAX_API_KEY',
  changed_files: ['/tmp/config.yaml', '/tmp/.env'],
  backup_paths: ['/tmp/config.yaml.deecodex.bak.1'],
  diagnostics: [{ level: 'info', message: '配置目录可写' }],
  diff: ['config.yaml: + model:'],
});
assert(report.includes('风险 low'));
assert(report.includes('变更文件'));
assert(report.includes('备份'));

context.accountsView = 'list';
context.selectedClientKind = 'codex';
context.selectedClientSurface = 'cli';
const codexList = context.renderAccountList();
assert(codexList.includes('line-action-icon-import'));
assert(codexList.includes('aria-label="导入配置"'));
assert(codexList.includes('line-action-icon-scan'));
assert(codexList.includes('aria-label="添加账号"'));
assert(codexList.includes('line-action-icon-check'));
assert(codexList.includes("applyAccount('c1')"));
assert(codexList.includes("editAccount('c1', 'codex')"));
assert(codexList.includes("testAccountUpstreamForCard('c1')"));
assert(codexList.includes('aria-label="测试上游连接"'));
assert(codexList.includes("deleteAccount('c1')"));
assert(codexList.includes('池已启用'));
assert(!codexList.includes('official-pool-overview'));
assert(codexList.includes('account-surface-tab'));
assert(codexList.includes('surface-cli'));
assert(codexList.includes('surface-desktop'));
assert(codexList.includes('aria-label="Codex CLI"'));
assert(codexList.includes('aria-label="Codex 桌面版"'));
assert(codexList.includes('balance-official'));
assert(codexList.includes('额度冷却'));
assert(codexList.includes('plus'));
assert(codexList.includes('522d6@edu.kg.182796.xyz'));
assert(!codexList.includes('Codex · 522d6'));
assert(codexList.includes('account-plan-pill'));
assert(codexList.includes('account-pool-switch toggle-label'));
assert(codexList.includes("toggleAccountRouting('c2')"));
assert(codexList.includes('5h'));
assert(codexList.includes('7d'));
assert(!codexList.includes('codex-official</span>'));
assert(!codexList.includes('池已启用</span>'));
assert(!codexList.includes('P20'));
assert(!codexList.includes('W2'));
assert(!codexList.includes("clearAccountCooldown('c2')"));
assert(!codexList.includes("resetAccountRuntime('c2')"));
assert(!codexList.includes("refreshBalanceForCard('c2')"));
assert(codexList.includes('runtime-quota_exceeded'));
assert(!codexList.includes('account-meta-tags mid-tags'));
assert(!codexList.includes('<span class="card-context">Chat 兼容</span>'));
assert(!codexList.includes('https://api.deepseek.com/v1'));

context.accountsView = 'add';
const addPanel = context.renderAccountsPanel();
assert(addPanel.includes('accounts-add-shell'));
assert(addPanel.includes('add-account-breadcrumb'));
assert(addPanel.includes('add-back-link'));
assert(addPanel.includes('provider-picker-shell'));
assert(addPanel.includes('provider-copy'));
assert(addPanel.includes('provider-card-arrow'));
assert(addPanel.includes('role="button"'));
assert(addPanel.includes('官方 Codex CLI 登录'));
assert(addPanel.includes('Codex CLI 设备码登录'));
assert(addPanel.includes("startOAuthAccountLogin('codex', 'browser')"));

context.selectedClientKind = 'claude_code';
context.selectedClientSurface = 'cli';
const claudeAddPanel = context.renderAccountsPanel();
assert(claudeAddPanel.includes('官方 Claude CLI 登录'));
assert(claudeAddPanel.includes("startOAuthAccountLogin('claude', 'browser')"));

context.selectedClientSurface = 'desktop';
const claudeDesktopAddPanel = context.renderAccountsPanel();
assert(claudeDesktopAddPanel.includes('官方 Claude 桌面版 登录'));

context.oauthLoginState = {
  state: 'oauth-state',
  provider: 'codex',
  status: 'pending',
  url: 'https://auth.openai.com/oauth/authorize',
  user_code: 'ABCD-EFGH',
};
const oauthPanel = context.renderAccountsPanel();
assert(oauthPanel.includes('Codex 官方登录'));
assert(oauthPanel.includes('ABCD-EFGH'));
assert(oauthPanel.includes('cancelOAuthAccountLogin()'));
context.oauthLoginState = null;
context.selectedClientKind = 'codex';
context.selectedClientSurface = 'cli';

context.editingAccount = context.accountsData.accounts.find(account => account.id === 'c1');
context.editingAccount.name = 'DeepSeek 账号';
context.accountsView = 'edit';
const codexDetail = context.renderAccountsPanel();
assert(!codexDetail.includes('一个账号就是一组供应商'));
assert(!codexDetail.includes('account-section-desc'));
assert(codexDetail.includes('<h2>DeepSeek</h2>'));
assert(!codexDetail.includes('<h2>DeepSeek 账号</h2>'));
assert(!codexDetail.includes('badge-provider badge-deepseek'));
assert(codexDetail.includes('model-map-table'));
assert(codexDetail.includes('model-map-row'));
assert(codexDetail.includes('model-upstream-cell'));
assert(codexDetail.includes('model-vision-cell'));
assert(!codexDetail.includes('model-remove-placeholder'));

context.editingAccount = context.accountsData.accounts.find(account => account.id === 'c2');
context.accountsView = 'edit';
const officialDetail = context.renderAccountsPanel();
assert(officialDetail.includes('官方账号池'));
assert(officialDetail.includes('id="edit_routing_pool"'));
assert(officialDetail.includes('value="codex-official"'));
assert(officialDetail.includes('id="edit_routing_priority" value="20"'));
assert(officialDetail.includes('id="edit_routing_weight" value="2"'));
assert(officialDetail.includes("applyAccountRoutingFromDetail('c2')"));
assert(!officialDetail.includes("refreshOfficialQuotaFromDetail('c2')"));
assert(officialDetail.includes('official-runtime-summary'));
assert(officialDetail.includes('official-quota-panel'));
assert(officialDetail.includes('5h'));
assert(officialDetail.includes('94%'));
assert(officialDetail.includes('配额耗尽中'));
assert(officialDetail.includes('runtime-state-grid'));
assert(officialDetail.includes('runtime-model-row'));
assert(officialDetail.includes('HTTP 429'));

context.selectedClientKind = 'hermes';
const hermesList = context.renderAccountList();
assert(hermesList.includes("applyClientAccount('h1')"));
assert(!hermesList.includes("testAccountUpstreamForCard('h1')"));
assert(!hermesList.includes('account-meta-tags mid-tags'));
assert(!hermesList.includes('https://api.minimaxi.com/v1'));

assert(context.renderBalanceInfo({
  mode: 'official_oauth',
  official: {
    status_label: '可用',
    plan_type: 'team',
    hours_5_remaining_percent: 99,
    hours_5_used_percent: 1,
    hours_5_reset_at: 1893456000,
    weekly_remaining_percent: 47,
    weekly_used_percent: 53,
    weekly_reset_at: 1893888000,
  },
}).includes('balance-pill balance-official'));
assert(context.renderBalanceInfo({
  mode: 'coding_plan',
  model_remains: [{ model_name: 'MiniMax-M*', interval_total: 1500, interval_used: 20, weekly_total: 15000, weekly_used: 243 }],
}).includes('balance-pill balance-plan'));
assert(context.renderBalanceInfo({
  mode: 'token_credit',
  credit_remaining: -1.45,
  credit_label: 'CNY',
}).includes('balance-pill balance-credit'));

const validation = context.renderConfigValidation({
  ok: true,
  diagnostics: [{ level: 'info', message: '配置语法校验通过' }],
});
assert(validation.includes('语法正常'));
assert(validation.includes('配置语法校验通过'));

const savePayload = JSON.parse(context.serializeAccountForBackend({
  id: 'h1',
  name: 'Hermes',
  provider: 'openrouter',
  client_kind: 'hermes',
  target: 'hermes',
  _editing_endpoint_id: 'ep1',
  upstream: 'https://openrouter.ai/api/v1',
  api_key: 'sk-test',
}));
assert.strictEqual(savePayload.client_kind, 'hermes');
assert(!Object.prototype.hasOwnProperty.call(savePayload, 'target'));
assert(!Object.prototype.hasOwnProperty.call(savePayload, '_editing_endpoint_id'));
assert.strictEqual(context.accountClientKind({ name: 'OpenClaw OpenRouter', provider: 'openrouter', target: 'client_config' }), 'openclaw');
assert.strictEqual(context.accountClientKind({ name: 'Hermes MiniMax', provider: 'minimax', target: 'client_config' }), 'hermes');
assert.strictEqual(context.accountClientKind({ clientKind: 'OpenClaw', target: 'client_config' }), 'openclaw');
assert.strictEqual(context.accountClientKind({ client_type: 'Hermes', target: 'client_config' }), 'hermes');
context.editingAccount = {
  id: 'oc-legacy',
  name: 'OpenClaw OpenRouter',
  provider: 'openrouter',
  target: 'client_config',
  upstream: 'https://openrouter.ai/api/v1',
  default_model: 'anthropic/claude-sonnet-4.5',
  client_options: { model_map: { default: 'anthropic/claude-sonnet-4.5' } },
};
const openclawLegacyDetail = context.renderClientAccountDetail();
assert(openclawLegacyDetail.includes('OpenClaw'));
assert(openclawLegacyDetail.includes('agents.defaults.model'));
assert(!openclawLegacyDetail.includes('ANTHROPIC_MODEL'));
context.editingAccount = {
  id: 'hm-legacy',
  name: 'Hermes MiniMax',
  provider: 'minimax',
  target: 'client_config',
  upstream: 'https://api.minimaxi.com/v1',
  default_model: 'MiniMax-M2.7',
  client_options: { model_map: { default: 'MiniMax-M2.7' } },
};
const hermesLegacyDetail = context.renderClientAccountDetail();
assert(hermesLegacyDetail.includes('Hermes'));
assert(hermesLegacyDetail.includes('model.default'));
assert(!hermesLegacyDetail.includes('ANTHROPIC_MODEL'));

const claudeProviders = context.providersForClientKind('claude_code').map(provider => provider.slug);
assert(claudeProviders.includes('deepseek'));
assert(claudeProviders.includes('kimi'));
assert(claudeProviders.includes('minimax'));
assert(claudeProviders.includes('mimo'));
assert(claudeProviders.includes('longcat'));
assert(claudeProviders.includes('glm'));
assert(claudeProviders.includes('qwen'));
const hermesProviders = context.providersForClientKind('hermes').map(provider => provider.slug);
assert(hermesProviders.includes('qwen'));
assert.deepStrictEqual(JSON.parse(JSON.stringify(context.codexProviderModelMap('deepseek'))), {
  'gpt-5.5': 'deepseek-v4-pro',
  'gpt-5.4': 'deepseek-v4-flash',
  'gpt-5.4-mini': 'deepseek-v4-flash',
  'gpt-5.3-codex': 'deepseek-v4-flash',
  'gpt-5': 'deepseek-v4-flash',
  'codex-auto-review': 'deepseek-v4-flash',
});
context.addAccount('deepseek', 'codex');
assert.strictEqual(context.editingAccount.client_surface, 'cli');
assert.strictEqual(context.editingAccount.model_map['gpt-5.5'], 'deepseek-v4-pro');
assert.strictEqual(context.editingAccount.model_map['gpt-5'], 'deepseek-v4-flash');
assert.strictEqual(context.editingAccount.endpoints[0].model_map['gpt-5.4'], 'deepseek-v4-flash');
assert.strictEqual(context.editingAccount.endpoints[0].vision.mode, 'native');
context.editingAccount.endpoints[0].path = '/v1/chat/completions';
const codexDefaultVisionDetail = context.renderAccountsPanel();
assert(codexDefaultVisionDetail.includes('data-mode="native"'));
assert(!codexDefaultVisionDetail.includes('collapsible-toggle open'));
assert(!codexDefaultVisionDetail.includes('collapsible-content open'));

context.selectedClientSurface = 'desktop';
context.addAccount('anthropic', 'claude_code');
assert.strictEqual(context.editingAccount.client_surface, 'desktop');
assert.strictEqual(context.editingAccount.client_options.client_surface, 'desktop');
assert.deepStrictEqual(
  Array.from(context.clientProviderDefaults('claude_code', 'deepseek').known_models),
  ['deepseek-v4-pro[1m]', 'deepseek-v4-pro', 'deepseek-v4-flash'],
);
context.addAccount('deepseek', 'claude_code');
assert.strictEqual(context.editingAccount.upstream, 'https://api.deepseek.com/anthropic');
assert.strictEqual(context.editingAccount.default_model, 'deepseek-v4-pro[1m]');
assert.strictEqual(context.editingAccount.client_options.auth_env, 'ANTHROPIC_AUTH_TOKEN');
const claudeDetail = context.renderClientAccountDetail();
assert.strictEqual((claudeDetail.match(/claude-one-m-toggle/g) || []).length, 4);
assert(claudeDetail.includes('开启 1M 上下文后，模型名会追加 [1m]'));
assert(claudeDetail.includes('claude-one-m-toggle toggle-label on'));
assert(claudeDetail.includes('ANTHROPIC_DEFAULT_SONNET_MODEL'));
assert(claudeDetail.includes('ANTHROPIC_DEFAULT_OPUS_MODEL'));
assert(claudeDetail.includes('ANTHROPIC_DEFAULT_HAIKU_MODEL'));
assert.strictEqual(context.clientProviderDefaults('claude_code', 'kimi').upstream, 'https://api.moonshot.cn/anthropic');
assert.strictEqual(context.clientProviderDefaults('claude_code', 'kimi').default_model, 'kimi-k2.5');
assert.strictEqual(context.clientProviderDefaults('claude_code', 'minimax').upstream, 'https://api.minimaxi.com/anthropic');
assert.strictEqual(context.clientProviderDefaults('claude_code', 'minimax').api_key_env, 'ANTHROPIC_API_KEY');
assert.strictEqual(context.clientProviderDefaults('claude_code', 'mimo').upstream, 'https://api.mimo-v2.com/anthropic');
assert.strictEqual(context.clientProviderDefaults('claude_code', 'mimo').default_model, 'mimo-v2.5-pro');
assert.strictEqual(context.clientProviderDefaults('claude_code', 'longcat').upstream, 'https://api.longcat.chat/anthropic');
assert.strictEqual(context.clientProviderDefaults('claude_code', 'longcat').default_model, 'LongCat-Flash-Chat');
assert.strictEqual(context.clientProviderDefaults('claude_code', 'longcat').api_key_env, 'ANTHROPIC_AUTH_TOKEN');
assert.strictEqual(context.clientProviderDefaults('claude_code', 'qwen').upstream, 'https://dashscope.aliyuncs.com/apps/anthropic');
assert.strictEqual(context.clientProviderDefaults('claude_code', 'qwen').default_model, 'qwen3.6-plus');
assert.strictEqual(context.clientProviderDefaults('claude_code', 'glm').upstream, 'https://open.bigmodel.cn/api/anthropic');
assert.strictEqual(context.clientProviderDefaults('claude_code', 'glm').default_model, 'glm-5.1');

console.log('accounts render smoke ok');
