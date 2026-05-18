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
  ],
  providerPresets: [
    {
      slug: 'deepseek',
      label: 'DeepSeek',
      description: 'DeepSeek',
      default_upstream: 'https://api.deepseek.com/v1',
      known_models: ['deepseek-chat'],
    },
    {
      slug: 'minimax',
      label: 'MiniMax',
      description: 'MiniMax',
      default_upstream: 'https://api.minimaxi.com/v1',
      known_models: ['MiniMax-M2.7'],
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
assert(codexList.includes("editConfigFile('c1')"));
assert(codexList.includes('配置'));

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

console.log('accounts render smoke ok');
