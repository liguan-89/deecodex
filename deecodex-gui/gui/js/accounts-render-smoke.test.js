const assert = require('assert');
const fs = require('fs');
const path = require('path');
const vm = require('vm');

const context = {
  console,
  accountsData: {
    client_counts: { codex: 1, hermes: 2 },
    active_by_surface: {
      'hermes:cli': { account_id: 'h1' },
    },
    accounts: [
      {
        id: 'c1',
        name: 'Codex DeepSeek',
        provider: 'deepseek',
        client_kind: 'codex',
        upstream: 'https://api.deepseek.com/v1',
        api_key: 'sk-test123456',
        api_key_present: true,
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
        api_key_present: true,
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
        api_key_present: true,
        default_model: 'MiniMax-M2.7',
        client_options: { api_key_env: 'MINIMAX_API_KEY', model_map: { default: 'MiniMax-M2.7' } },
        last_check: { ok: false, message: 'Hermes 密钥为空' },
      },
      {
        id: 'h2',
        name: 'Hermes 旧账号',
        provider: 'anthropic',
        client_kind: 'hermes',
        upstream: 'https://api.anthropic.com',
        api_key: 'sk-old',
        api_key_present: true,
        default_model: 'claude-sonnet-4-5',
        client_options: { api_key_env: 'ANTHROPIC_API_KEY', model_map: { default: 'claude-sonnet-4-5' } },
        last_applied_at: 100,
        last_check: {
          ok: true,
          message: 'Hermes 配置正常',
          details: { ok: true, message: 'Hermes 配置正常', applied_at: 100 },
        },
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
      known_models: ['MiniMax-M3[1m]', 'MiniMax-M3', 'MiniMax-M2.7-highspeed[1m]', 'MiniMax-M2.7-highspeed', 'MiniMax-M2.7[1m]', 'MiniMax-M2.7'],
    },
    {
      slug: 'mimo',
      label: 'MiMo',
      description: 'MiMo',
      default_upstream: 'https://token-plan-cn.xiaomimimo.com/v1',
      known_models: ['mimo-v2.5-pro[1m]', 'mimo-v2.5-pro', 'mimo-v2.5[1m]', 'mimo-v2.5', 'mimo-v2-omni', 'mimo-v2-pro[1m]', 'mimo-v2-pro'],
    },
    {
      slug: 'longcat',
      label: 'LongCat',
      description: 'LongCat',
      default_upstream: 'https://api.longcat.chat/openai',
      known_models: ['LongCat-2.0-Preview', 'LongCat-Flash-Lite', 'LongCat-Flash-Chat', 'LongCat-Flash-Thinking-2601', 'LongCat-Flash-Omni-2603'],
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

[
  ['https://api.openai.com/v1/chat/completions', 'https://api.openai.com/v1'],
  ['https://api.openai.com/v1/responses', 'https://api.openai.com/v1'],
  ['https://api.deepseek.com/v1/chat/completions', 'https://api.deepseek.com/v1'],
  ['https://api.deepseek.com/v1/responses', 'https://api.deepseek.com/v1'],
  ['https://api.minimaxi.com/v1/chat/completions', 'https://api.minimaxi.com/v1'],
  ['https://api.minimaxi.com/v1/responses', 'https://api.minimaxi.com/v1'],
  ['https://token-plan-cn.xiaomimimo.com/v1/chat/completions', 'https://token-plan-cn.xiaomimimo.com/v1'],
  ['https://token-plan-cn.xiaomimimo.com/v1/responses', 'https://token-plan-cn.xiaomimimo.com/v1'],
].forEach(([input, expected]) => {
  assert.strictEqual(context.normalizeResponsesBaseUrl(input), expected);
});

context.editingAccount = context.accountsData.accounts.find(account => account.id === 'h1');
context.accountsView = 'edit';
const editablePanel = context.renderAccountsPanel();
assert(editablePanel.includes('accounts-form-shell accounts-edit-shell'));
assert(editablePanel.includes('accounts-scroll-region accounts-form-scroll'));

const switcher = context.renderClientSwitcher(context.accountsData.accounts);
assert(switcher.includes('client-tab active has-issues'));
assert(switcher.includes('Hermes'));

context.accountsView = 'list';
context.selectedClientKind = 'hermes';
context.selectedClientSurface = 'cli';
const hermesActiveList = context.renderAccountList();
assert(hermesActiveList.includes('Hermes MiniMax'));
assert(hermesActiveList.includes('Hermes 旧账号'));
assert(hermesActiveList.includes('<span class="active-badge">活跃</span>'));
assert(!hermesActiveList.includes('<span class="active-badge">已写入</span>'));
assert(hermesActiveList.indexOf('Hermes MiniMax') < hermesActiveList.indexOf('Hermes 旧账号'));

context.accountsView = 'edit';
const detail = context.renderClientAccountDetail();
assert(detail.includes('Hermes .env Key'));
assert(!detail.includes('value="sk-test123456"'));
assert(detail.includes('value="sk-t****3456"'));
assert(detail.includes('secret-copy-btn'));
assert(detail.includes("copyEditingAccountSecret('api_key')"));
assert(detail.includes('最近备份'));
assert(!detail.includes('account-section-desc'));
assert(!detail.includes('只记录外部客户端配置操作'));
assert(detail.includes('page-back-button account-back-link'));
assert(detail.includes('客户端模型槽位'));
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
assert(codexList.includes('登录态锚点已启用'));
assert(codexList.includes('仅锚点'));
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
assert(codexList.includes("toggleAccountRouting('c2', 'anchor')"));
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

context.accountsData.router_status = {
  anchor: { account_id: 'desktop-anchor', account_name: 'Codex Desktop', pool: 'pool-a' },
  requested_model: 'gpt-5',
  selected: {
    account_id: 'primary',
    account_name: 'Primary Router',
    mapped_model: 'deepseek-chat',
  },
  candidate_count: 2,
  eligible_count: 2,
  skipped_count: 0,
  candidates: [
    {
      account_id: 'primary',
      account_name: 'Primary Router',
      eligible: true,
      reason: 'ready',
      mapped_model: 'deepseek-chat',
    },
    {
      account_id: 'backup',
      account_name: 'Backup Router',
      eligible: true,
      reason: 'ready',
      mapped_model: 'deepseek-chat',
    },
  ],
};
context.accountsData.router_status_scenarios = [
  {
    scenario_id: 'text',
    scenario_label: '文本',
    selected: context.accountsData.router_status.selected,
    candidate_count: 2,
    eligible_count: 2,
    candidates: context.accountsData.router_status.candidates,
  },
];
context.selectedClientSurface = 'desktop';
const routerDesktopOverview = context.renderRouterStatusOverview();
assert.strictEqual(routerDesktopOverview, '');
context.selectedClientSurface = 'cli';

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
assert(!claudeAddPanel.includes('official-login-grid'));

context.selectedClientSurface = 'desktop';
const claudeDesktopAddPanel = context.renderAccountsPanel();
assert(claudeDesktopAddPanel.includes('官方 Claude 桌面版 登录'));
assert(!claudeDesktopAddPanel.includes('official-login-grid'));

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
assert(!codexDetail.includes('model-map-table'));
assert(!codexDetail.includes('model-map-row'));
assert(!codexDetail.includes('model-upstream-cell'));
assert(!codexDetail.includes('model-vision-cell'));
assert(!codexDetail.includes('model-remove-placeholder'));

context.editingAccount = {
  id: 'r1',
  name: 'OpenAI Responses',
  provider: 'openai',
  client_kind: 'codex',
  upstream: 'https://dex.jinpai.lat/v1/chat/completions',
  api_key: 'sk-responses',
  model_map: { 'gpt-5.5': 'other-model' },
  context_window_override: 1000000,
  reasoning_effort_override: 'high',
  thinking_tokens: 16000,
  capability_enabled: true,
  capability_account_id: 'c1',
  dev_pipeline_enabled: true,
  dev_pipeline_architect_account_id: 'c1',
  endpoints: [{
    id: 'ep-responses',
    kind: 'open_ai_chat',
    base_url: 'https://dex.jinpai.lat/v1/chat/completions',
    path: 'chat/completions',
    model_map: { 'gpt-5.5': 'other-model' },
    model_profiles: { 'other-model': { vision_mode: 'glue' } },
    vision: { mode: 'glue', base_url: 'https://vision.example.com', api_key: 'sk-vision', model: 'vision-model' },
    context_window_override: 1000000,
    reasoning_effort_override: 'high',
    thinking_tokens: 16000,
  }],
};
const responsesDirectDetail = context.renderAccountsPanel();
assert(!responsesDirectDetail.includes('model-map-table'));
assert(!responsesDirectDetail.includes('fetchAndPopulateModels()'));
assert(!responsesDirectDetail.includes('+ 添加历史模型配置'));
assert(!responsesDirectDetail.includes('Responses 直连保留 Codex 原始模型名'));
assert(!responsesDirectDetail.includes('<div class="section-sub-label">图片处理</div>'));
assert(!responsesDirectDetail.includes('能力补全'));
assert(!responsesDirectDetail.includes('开发协作编排'));
assert(!responsesDirectDetail.includes('上下文窗口覆盖'));
assert(!responsesDirectDetail.includes('推理强度覆盖'));
assert(!responsesDirectDetail.includes('<select id="edit_endpoint_kind">'));
assert(responsesDirectDetail.includes('id="edit_endpoint_kind" value="open_ai_responses"'));
assert(responsesDirectDetail.includes('id="edit_image_generation_enabled" checked'));
assert.strictEqual(context.editingAccount.endpoints[0].kind, 'open_ai_responses');
assert.strictEqual(context.editingAccount.endpoints[0].base_url, 'https://dex.jinpai.lat/v1');
assert.strictEqual(context.editingAccount.endpoints[0].path, '');
assert.strictEqual(context.editingAccount.endpoints[0].image_generation_enabled, true);
assert.strictEqual(JSON.stringify(context.editingAccount.model_map), '{}');
assert.strictEqual(JSON.stringify(context.editingAccount.endpoints[0].model_map), '{}');
assert.deepStrictEqual(Array.from(context.editingAccount.endpoints[0].known_models || []), ['other-model']);
assert.strictEqual(context.editingAccount.endpoints[0].vision.mode, 'native');
assert.strictEqual(context.editingAccount.context_window_override, null);
assert.strictEqual(context.editingAccount.endpoints[0].reasoning_effort_override, null);
assert.strictEqual(context.editingAccount.capability_enabled, false);
assert.strictEqual(context.editingAccount.dev_pipeline_enabled, false);

context.editingAccount = {
  id: 'r2',
  name: 'Custom Responses',
  provider: 'custom',
  client_kind: 'codex',
  upstream: 'https://gateway.example.com',
  api_key: 'sk-responses',
  model_map: { 'gpt-5.5': 'other-model' },
  context_window_override: 1000000,
  reasoning_effort_override: 'high',
  capability_enabled: true,
  dev_pipeline_enabled: true,
  endpoints: [{
    id: 'ep-custom-responses',
    kind: 'custom_responses',
    base_url: 'https://gateway.example.com',
    path: 'v2/responses',
    model_map: { 'gpt-5.5': 'other-model' },
    model_profiles: { 'other-model': { vision_mode: 'glue' } },
    vision: { mode: 'glue', base_url: 'https://vision.example.com', api_key: 'sk-vision', model: 'vision-model' },
    context_window_override: 1000000,
    reasoning_effort_override: 'high',
  }],
};
const customResponsesDetail = context.renderAccountsPanel();
assert(!customResponsesDetail.includes('model-map-table'));
assert(!customResponsesDetail.includes('<div class="section-sub-label">图片处理</div>'));
assert(!customResponsesDetail.includes('能力补全'));
assert(!customResponsesDetail.includes('开发协作编排'));
assert(!customResponsesDetail.includes('上下文窗口覆盖'));
assert(!customResponsesDetail.includes('推理强度覆盖'));
assert(customResponsesDetail.includes('<select id="edit_endpoint_kind" onchange="handleEndpointKindChange(this.value)">'));
assert(customResponsesDetail.includes('<option value="custom_responses" selected hidden>OpenAI Responses 直连（自定义路径）</option>'));
assert(customResponsesDetail.includes('id="edit_endpoint_path" value="v2/responses"'));
assert(customResponsesDetail.includes('id="edit_image_generation_enabled" '));
assert(!customResponsesDetail.includes('id="edit_image_generation_enabled" checked'));
assert.strictEqual(context.editingAccount.endpoints[0].kind, 'custom_responses');
assert.strictEqual(context.editingAccount.endpoints[0].path, 'v2/responses');
assert.deepStrictEqual(JSON.parse(JSON.stringify(context.editingAccount.endpoints[0].model_map)), {});
assert.deepStrictEqual(Array.from(context.editingAccount.endpoints[0].known_models || []), ['other-model']);
assert.strictEqual(context.editingAccount.endpoints[0].vision.mode, 'glue');
assert.strictEqual(context.editingAccount.capability_enabled, true);
assert.strictEqual(context.editingAccount.dev_pipeline_enabled, true);

context.editingAccount = context.accountsData.accounts.find(account => account.id === 'c2');
context.editingAccount.context_window_override = 1000000;
context.editingAccount.reasoning_effort_override = 'high';
context.editingAccount.capability_enabled = true;
context.editingAccount.capability_account_id = 'c1';
context.editingAccount.dev_pipeline_enabled = true;
context.editingAccount.dev_pipeline_architect_account_id = 'c1';
context.editingAccount.endpoints[0].model_map = { 'gpt-5': 'gpt-5' };
context.editingAccount.endpoints[0].vision = { mode: 'glue', base_url: 'https://vision.example.com' };
context.editingAccount.endpoints[0].context_window_override = 1000000;
context.editingAccount.endpoints[0].reasoning_effort_override = 'high';
context.accountsView = 'edit';
const officialDetail = context.renderAccountsPanel();
assert(officialDetail.includes('Router 路由'));
assert(officialDetail.includes('作为登录态锚点'));
assert(officialDetail.includes('参与模型执行'));
assert(officialDetail.includes('官方登录态'));
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
assert(officialDetail.includes('配额恢复中'));
assert(officialDetail.includes('runtime-state-grid'));
assert(officialDetail.includes('runtime-model-row'));
assert(officialDetail.includes('HTTP 429'));
assert(!officialDetail.includes('model-map-table'));
assert(!officialDetail.includes('fetchAndPopulateModels()'));
assert(!officialDetail.includes('Codex 官方账号使用官方模型名'));
assert(!officialDetail.includes('<div class="section-sub-label">图片处理</div>'));
assert(!officialDetail.includes('能力补全'));
assert(!officialDetail.includes('开发协作编排'));
assert(!officialDetail.includes('上下文窗口覆盖'));
assert(!officialDetail.includes('推理强度覆盖'));
assert(!officialDetail.includes('<select id="edit_endpoint_kind">'));
assert(officialDetail.includes('id="edit_endpoint_kind" value="codex_official"'));
assert.strictEqual(context.editingAccount.endpoints[0].kind, 'codex_official');
assert.strictEqual(JSON.stringify(context.editingAccount.endpoints[0].model_map), '{}');
assert.deepStrictEqual(Array.from(context.editingAccount.endpoints[0].known_models || []), ['gpt-5']);
assert.strictEqual(context.editingAccount.endpoints[0].vision.mode, 'native');
assert.strictEqual(context.editingAccount.context_window_override, null);
assert.strictEqual(context.editingAccount.endpoints[0].reasoning_effort_override, null);
assert.strictEqual(context.editingAccount.capability_enabled, false);
assert.strictEqual(context.editingAccount.dev_pipeline_enabled, false);

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
const addArgs = context.addAccountInvokeArgs({
  name: 'MiniMax',
  provider: 'minimax',
  client_kind: 'hermes',
  client_surface: 'desktop',
  target: 'client_config',
  upstream: 'https://api.minimaxi.com/v1',
  api_key: 'sk-test',
});
assert.strictEqual(addArgs.provider, 'minimax');
assert.strictEqual(addArgs.clientKind, 'hermes');
assert.strictEqual(addArgs.client_kind, 'hermes');
assert.strictEqual(addArgs.clientSurface, 'cli');
assert.strictEqual(addArgs.client_surface, 'cli');
assert.strictEqual(JSON.parse(addArgs.accountJson).client_kind, 'hermes');
assert.strictEqual(JSON.parse(addArgs.accountJson).client_surface, 'desktop');
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
assert.deepStrictEqual(JSON.parse(JSON.stringify(context.codexProviderModelMap('deepseek'))), {});
assert.deepStrictEqual(JSON.parse(JSON.stringify(context.codexProviderModelMap('longcat'))), {});
assert.deepStrictEqual(JSON.parse(JSON.stringify(context.codexProviderModelMap('mimo'))), {});
context.addAccount('deepseek', 'codex');
assert.strictEqual(context.editingAccount.client_surface, 'cli');
assert.deepStrictEqual(JSON.parse(JSON.stringify(context.editingAccount.model_map)), {});
assert.deepStrictEqual(JSON.parse(JSON.stringify(context.editingAccount.endpoints[0].model_map)), {});
assert.strictEqual(context.editingAccount.endpoints[0].kind, 'open_ai_chat');
assert.strictEqual(context.editingAccount.endpoints[0].vision.mode, 'off');
context.editingAccount.endpoints[0].path = '/v1/chat/completions';
const codexDefaultVisionDetail = context.renderAccountsPanel();
assert(codexDefaultVisionDetail.includes('data-mode="off"'));
assert(!codexDefaultVisionDetail.includes('collapsible-toggle open'));
assert(!codexDefaultVisionDetail.includes('collapsible-content open'));
assert(codexDefaultVisionDetail.includes('onchange="handleEndpointKindChange(this.value)"'));
assert(codexDefaultVisionDetail.includes('OpenAI Chat 兼容'));

context.addAccount('longcat', 'codex');
assert.strictEqual(context.editingAccount.upstream, 'https://api.longcat.chat/openai');
assert.deepStrictEqual(JSON.parse(JSON.stringify(context.editingAccount.model_map)), {});
assert.deepStrictEqual(JSON.parse(JSON.stringify(context.editingAccount.endpoints[0].model_map)), {});

context.addAccount('mimo', 'codex');
assert.strictEqual(context.editingAccount.upstream, 'https://token-plan-cn.xiaomimimo.com/v1');
assert.deepStrictEqual(JSON.parse(JSON.stringify(context.editingAccount.model_map)), {});
assert.deepStrictEqual(JSON.parse(JSON.stringify(context.editingAccount.endpoints[0].model_map)), {});
assert.strictEqual(context.editingAccount.endpoints[0].kind, 'open_ai_responses');
assert.strictEqual(context.editingAccount.endpoints[0].vision.mode, 'native');
assert.deepStrictEqual(JSON.parse(JSON.stringify(context.editingAccount.endpoints[0].model_profiles)), {});

context.addAccount('longcat', 'codex');
context.editingAccount.context_window_override = 1000000;
context.editingAccount.reasoning_effort_override = 'high';
context.editingAccount.capability_enabled = true;
context.editingAccount.dev_pipeline_enabled = true;
context.editingAccount.endpoints[0].model_map = { 'gpt-5': 'deepseek-chat' };
context.editingAccount.endpoints[0].model_profiles = { 'deepseek-chat': { vision_mode: 'glue' } };
context.editingAccount.endpoints[0].vision = { mode: 'glue', base_url: 'https://vision.example.com' };
context.editingAccount.endpoints[0].context_window_override = 1000000;
context.editingAccount.endpoints[0].reasoning_effort_override = 'high';
const mainContent = {
  classList: { toggle: () => {} },
  innerHTML: '',
};
let endpointKindValue = 'open_ai_responses';
const endpointKindControl = {
  get value() { return endpointKindValue; },
  set value(next) { endpointKindValue = next; },
};
const originalDocument = context.document;
context.document = {
  querySelectorAll: () => [],
  getElementById: id => ({
    mainContent,
    edit_endpoint_kind: endpointKindControl,
    edit_name: { value: context.editingAccount.name },
    edit_api_key: { value: '' },
    edit_upstream: { value: context.editingAccount.upstream },
    edit_balance_url: { value: '' },
  }[id] || null),
};
context.handleEndpointKindChange('open_ai_responses');
assert.strictEqual(context.editingAccount.endpoints[0].kind, 'open_ai_responses');
assert.deepStrictEqual(JSON.parse(JSON.stringify(context.editingAccount.endpoints[0].model_map)), {});
assert.deepStrictEqual(Array.from(context.editingAccount.endpoints[0].known_models || []), ['deepseek-chat']);
assert.strictEqual(context.editingAccount.endpoints[0].vision.mode, 'glue');
assert.strictEqual(context.editingAccount.capability_enabled, true);
assert.strictEqual(context.editingAccount.dev_pipeline_enabled, true);
assert(!mainContent.innerHTML.includes('model-map-table'));
assert(!mainContent.innerHTML.includes('能力补全'));
assert(!mainContent.innerHTML.includes('开发协作编排'));
assert(mainContent.innerHTML.includes('<select id="edit_endpoint_kind" onchange="handleEndpointKindChange(this.value)">'));
assert(mainContent.innerHTML.includes('<option value="open_ai_responses" selected>OpenAI Responses 直连</option>'));
endpointKindValue = 'open_ai_chat';
context.handleEndpointKindChange('open_ai_chat');
context.document = originalDocument;
assert.strictEqual(context.editingAccount.endpoints[0].kind, 'open_ai_chat');
assert(!mainContent.innerHTML.includes('model-map-table'));
assert(!mainContent.innerHTML.includes('能力补全'));
assert.deepStrictEqual(JSON.parse(JSON.stringify(context.editingAccount.endpoints[0].model_map)), {});
assert.strictEqual(context.editingAccount.endpoints[0].vision.mode, 'glue');
assert.strictEqual(context.editingAccount.capability_enabled, true);
assert.strictEqual(context.editingAccount.dev_pipeline_enabled, true);

context.editingAccount = {
  id: 'hidden-key',
  name: '隐藏密钥',
  provider: 'deepseek',
  client_kind: 'codex',
  upstream: 'https://api.deepseek.com/v1',
  api_key: 'sk-t****3456',
  api_key_present: true,
  model_map: {},
  endpoints: [{
    id: 'ep-hidden',
    kind: 'open_ai_chat',
    base_url: 'https://api.deepseek.com/v1',
    model_map: {},
    vision: { mode: 'off' },
  }],
};
context.document = {
  getElementById: id => ({
    edit_name: { value: context.editingAccount.name },
    edit_api_key: { value: '' },
  }[id] || null),
};
context.syncEditingDraftFromForm();
assert.strictEqual(context.editingAccount.api_key, 'sk-t****3456');
context.document = {
  getElementById: id => ({
    edit_name: { value: context.editingAccount.name },
    edit_api_key: { value: 'sk-new-secret' },
  }[id] || null),
};
context.syncEditingDraftFromForm();
assert.strictEqual(context.editingAccount.api_key, 'sk-new-secret');
context.document = originalDocument;

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
assert.strictEqual(context.clientProviderDefaults('claude_code', 'mimo').upstream, 'https://token-plan-cn.xiaomimimo.com/anthropic');
assert.strictEqual(context.clientProviderDefaults('claude_code', 'mimo').default_model, 'mimo-v2.5-pro[1m]');
assert.deepStrictEqual(Array.from(context.clientProviderDefaults('claude_code', 'mimo').known_models), [
  'mimo-v2.5-pro[1m]',
  'mimo-v2.5-pro',
  'mimo-v2.5[1m]',
  'mimo-v2.5',
  'mimo-v2-pro[1m]',
  'mimo-v2-pro',
  'mimo-v2-omni',
]);
assert.strictEqual(context.clientProviderDefaults('claude_code', 'longcat').upstream, 'https://api.longcat.chat/anthropic');
assert.strictEqual(context.clientProviderDefaults('claude_code', 'longcat').default_model, 'LongCat-Flash-Chat');
assert.strictEqual(context.clientProviderDefaults('claude_code', 'longcat').api_key_env, 'ANTHROPIC_AUTH_TOKEN');
assert.deepStrictEqual(Array.from(context.clientProviderDefaults('claude_code', 'longcat').known_models), [
  'LongCat-2.0-Preview',
  'LongCat-Flash-Lite',
  'LongCat-Flash-Chat',
  'LongCat-Flash-Thinking-2601',
  'LongCat-Flash-Omni-2603',
]);
assert.strictEqual(context.clientProviderDefaults('claude_code', 'qwen').upstream, 'https://dashscope.aliyuncs.com/apps/anthropic');
assert.strictEqual(context.clientProviderDefaults('claude_code', 'qwen').default_model, 'qwen3.6-plus');
assert.strictEqual(context.clientProviderDefaults('claude_code', 'glm').upstream, 'https://open.bigmodel.cn/api/anthropic');
assert.strictEqual(context.clientProviderDefaults('claude_code', 'glm').default_model, 'glm-5.1');

console.log('accounts render smoke ok');
