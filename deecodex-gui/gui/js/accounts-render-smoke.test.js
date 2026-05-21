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

const switcher = context.renderClientSwitcher(context.accountsData.accounts);
assert(switcher.includes('client-tab active has-issues'));
assert(switcher.includes('Hermes'));

const detail = context.renderClientAccountDetail();
assert(detail.includes('Hermes .env Key'));
assert(detail.includes('最近备份'));
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
const codexList = context.renderAccountList();
assert(codexList.includes("applyAccount('c1')"));
assert(codexList.includes("editAccount('c1')"));
assert(codexList.includes("refreshBalanceForCard('c1')"));
assert(codexList.includes("deleteAccount('c1')"));

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
assert.deepStrictEqual(
  Array.from(context.clientProviderDefaults('claude_code', 'deepseek').known_models),
  ['deepseek-v4-pro[1m]', 'deepseek-v4-pro', 'deepseek-v4-flash'],
);
context.addAccount('deepseek', 'claude_code');
assert.strictEqual(context.editingAccount.upstream, 'https://api.deepseek.com/anthropic');
assert.strictEqual(context.editingAccount.default_model, 'deepseek-v4-pro[1m]');
assert.strictEqual(context.editingAccount.client_options.auth_env, 'ANTHROPIC_AUTH_TOKEN');
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
