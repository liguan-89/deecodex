const CODEX_MODEL_LIST = ['gpt-5.5', 'gpt-5.4', 'gpt-5.4-mini', 'gpt-5.3-codex', 'gpt-5', 'codex-auto-review'];
const CLIENT_KIND_LABELS = {
  codex: 'Codex',
  claude_code: 'Claude',
  openclaw: 'OpenClaw',
  hermes: 'Hermes',
  generic_client: '通用客户端',
};

function providerBadgeClass(p) {
  return 'badge-provider badge-' + (p || 'custom');
}

function providerLogoSlug(p) {
  return ['openrouter', 'deepseek', 'kimi', 'minimax', 'mimo', 'longcat', 'qwen', 'glm', 'openai', 'anthropic', 'google-ai', 'openclaw', 'hermes', 'claude', 'codex', 'claude-code'].includes(p)
    ? p
    : 'custom';
}

function providerLogoSrc(p) {
  const files = {
    openrouter: 'openrouter.webp',
    deepseek: 'deepseek.png',
    kimi: 'kimi.png',
    minimax: 'minimax.webp',
    mimo: 'mimo.webp',
    longcat: 'longcat.png',
    qwen: 'qwen.svg',
    anthropic: 'anthropic.png',
    claude: 'anthropic.png',
    glm: 'glm-wordmark.png',
    openai: 'openai.png',
    codex: 'codex.png',
    'claude-code': 'claude-code.png',
    openclaw: 'openclaw.png',
    hermes: 'hermes.png',
  };
  const slug = providerLogoSlug(p);
  return `assets/provider-logos/${files[slug] || slug + '.svg'}`;
}

function providerIcon(p, label) {
  const slug = providerLogoSlug(p);
  return `<img class="provider-logo-img provider-logo-${escAttr(slug)}" src="${providerLogoSrc(p)}" alt="${escAttr(label || p || '自定义')}">`;
}

function renderProviderBadge(p) {
  const slug = p || 'custom';
  return `<span class="${providerBadgeClass(slug)}"><img src="${providerLogoSrc(slug)}" alt="" aria-hidden="true">${esc(slug)}</span>`;
}

function normalizeClientKind(kind) {
  const value = String(kind || 'codex');
  if (value === 'ClaudeCode') return 'claude_code';
  if (value === 'Openclaw') return 'openclaw';
  if (value === 'GenericClient') return 'generic_client';
  if (value === 'Codex') return 'codex';
  if (value === 'Hermes') return 'hermes';
  return ['codex', 'claude_code', 'openclaw', 'hermes', 'generic_client'].includes(value) ? value : 'codex';
}

function accountClientKind(a) {
  return normalizeClientKind(a?.client_kind || a?.target || 'codex');
}

function isCodexAccount(a) {
  return accountClientKind(a) === 'codex';
}

function clientAccountHasIssue(a) {
  if (!a || isCodexAccount(a)) return false;
  const status = a._client_status_report || a.last_check;
  if (!status) return false;
  return status.ok === false;
}

function getClientProfile(kind) {
  const slug = normalizeClientKind(kind);
  return clientProfiles.find(profile => normalizeClientKind(profile.slug || profile.kind) === slug) || null;
}

function clientSecretLabel(kind) {
  const slug = normalizeClientKind(kind);
  if (slug === 'claude_code') return 'Claude 鉴权变量';
  if (slug === 'openclaw') return 'SecretRef 环境变量';
  if (slug === 'hermes') return 'Hermes .env Key';
  return 'Key 环境变量名';
}

function renderClaudeAuthEnvSelect(value) {
  const current = value || 'ANTHROPIC_API_KEY';
  return `<select id="edit_client_auth_env">
    <option value="ANTHROPIC_API_KEY" ${current === 'ANTHROPIC_API_KEY' ? 'selected' : ''}>ANTHROPIC_API_KEY</option>
    <option value="ANTHROPIC_AUTH_TOKEN" ${current === 'ANTHROPIC_AUTH_TOKEN' ? 'selected' : ''}>ANTHROPIC_AUTH_TOKEN</option>
  </select>`;
}

function clientModelSlots(kind) {
  const profile = getClientProfile(kind);
  if (profile && Array.isArray(profile.model_slots) && profile.model_slots.length) return profile.model_slots;
  const slug = normalizeClientKind(kind);
  if (slug === 'claude_code') return [
    { key: 'default', label: '主模型', target: 'ANTHROPIC_MODEL', required: true },
    { key: 'sonnet', label: 'Sonnet 模型', target: 'ANTHROPIC_DEFAULT_SONNET_MODEL' },
    { key: 'opus', label: 'Opus 模型', target: 'ANTHROPIC_DEFAULT_OPUS_MODEL' },
    { key: 'haiku', label: 'Haiku 模型', target: 'ANTHROPIC_DEFAULT_HAIKU_MODEL' },
  ];
  if (slug === 'openclaw') return [
    { key: 'default', label: '默认 Agent 模型', target: 'agents.defaults.model', required: true },
    { key: 'image', label: '图片理解模型', target: 'agents.defaults.imageModel' },
    { key: 'image_generation', label: '图片生成模型', target: 'agents.defaults.imageGenerationModel' },
    { key: 'video_generation', label: '视频生成模型', target: 'agents.defaults.videoGenerationModel' },
  ];
  if (slug === 'hermes') return [
    { key: 'default', label: '主模型', target: 'model.default', required: true },
    { key: 'vision', label: '视觉辅助模型', target: 'auxiliary.vision.model' },
    { key: 'web_extract', label: '网页提取模型', target: 'auxiliary.web_extract.model' },
    { key: 'compression', label: '压缩模型', target: 'auxiliary.compression.model' },
    { key: 'session_search', label: '会话检索模型', target: 'auxiliary.session_search.model' },
    { key: 'title_generation', label: '标题生成模型', target: 'auxiliary.title_generation.model' },
  ];
  return [
    { key: 'default', label: '默认模型', target: 'OPENAI_MODEL', required: true },
    { key: 'fast', label: '快速模型', target: 'OPENAI_FAST_MODEL' },
    { key: 'reasoning', label: '推理模型', target: 'OPENAI_REASONING_MODEL' },
    { key: 'vision', label: '视觉模型', target: 'OPENAI_VISION_MODEL' },
  ];
}

function clientModelMap(account) {
  const map = account?.client_options?.model_map;
  return map && typeof map === 'object' && !Array.isArray(map) ? map : {};
}

function clientIcon(kind) {
  const slug = normalizeClientKind(kind);
  const logo = slug === 'claude_code' ? 'claude-code' : (slug === 'generic_client' ? 'custom' : slug);
  return `<span class="client-logo-box client-logo-${escAttr(slug)}"><img class="client-logo-img" src="${providerLogoSrc(logo)}" alt="" aria-hidden="true"></span>`;
}

function renderCardActionMenu(a, isClient) {
  return '';
}

function renderClientSwitcher(list) {
  const counts = accountsData.client_counts || {};
  const profiles = clientProfiles.length ? clientProfiles : [
    { slug: 'codex', label: 'Codex', description: 'deecodex 代理账号' },
    { slug: 'claude_code', label: 'Claude Code', description: 'Claude 本地配置' },
    { slug: 'openclaw', label: 'OpenClaw', description: 'OpenClaw 配置' },
    { slug: 'hermes', label: 'Hermes', description: 'Hermes 配置' },
    { slug: 'generic_client', label: '通用客户端', description: 'OpenAI 兼容 Env' },
  ];
  return `<div class="client-switcher" role="tablist" aria-label="账号客户端分类">
    ${profiles.map(profile => {
      const kind = normalizeClientKind(profile.slug || profile.kind);
      const clientAccounts = list.filter(a => accountClientKind(a) === kind);
      const count = counts[kind] || clientAccounts.length || 0;
      const issueCount = clientAccounts.filter(clientAccountHasIssue).length;
      const active = kind === selectedClientKind ? ' active' : '';
      const issueClass = issueCount ? ' has-issues' : '';
      return `<button type="button" class="client-tab${active}${issueClass}" onclick="selectClientKind('${escAttr(kind)}')" title="${escAttr(profile.description || '')}" role="tab" aria-selected="${kind === selectedClientKind}">
        ${clientIcon(kind)}
        <span>${esc(CLIENT_KIND_LABELS[kind] || profile.label || kind)}</span>
        <em>${count}</em>
        ${issueCount ? `<strong class="client-tab-alert" title="${escAttr(issueCount + ' 个账号最近检查异常')}">${issueCount}</strong>` : ''}
      </button>`;
    }).join('')}
  </div>`;
}

function renderClientAccountDetail() {
  const a = editingAccount;
  const kind = accountClientKind(a);
  const profile = getClientProfile(kind) || {};
  const configPath = a.client_options?.config_path || '';
  const apiKeyEnv = a.client_options?.api_key_env || defaultApiKeyEnvForClient(a);
  const authEnv = a.client_options?.auth_env || apiKeyEnv;
  const proxyEnabled = Boolean(a.client_options?.proxy_recording_enabled);
  const proxyBaseUrl = a.client_options?.proxy_base_url || '';
  const secretHint = kind === 'openclaw'
    ? 'OpenClaw 会写入 SecretRef，不把 Key 放进命令参数。'
    : (kind === 'hermes'
      ? 'Hermes 会把非密钥配置写入 config.yaml，密钥写入 .env。'
      : '写入前会展示脱敏 diff，不显示完整密钥。');
  return `<div class="breadcrumb">
    <span class="back-link" onclick="navigateAccounts('list')">← 账号列表</span>
    <span> / ${esc(a.name)}</span>
  </div>
  <div class="page-header account-detail-header">
    <div class="account-detail-title">
      ${clientIcon(kind)}
      <div>
        <div class="account-detail-heading">
          <h2>${esc(a.name)}</h2>
          <span class="client-kind-badge">${esc(CLIENT_KIND_LABELS[kind] || kind)}</span>
          ${renderProviderBadge(a.provider)}
        </div>
      </div>
    </div>
  </div>

  <div class="account-form client-account-form">
    <section class="account-edit-section">
      <div class="account-section-head">
        <div class="section-sub-label">客户端账号</div>
        <div class="account-section-desc">此账号直接写入 ${esc(profile.label || '外部客户端')} 配置，不经过 deecodex 端点翻译。</div>
      </div>
      <div class="config-fields">
        <div class="config-field">
          <label>账号名称</label>
          <input type="text" id="edit_name" value="${escAttr(a.name)}" placeholder="输入账号显示名">
        </div>
        <div class="config-field">
          <label>供应商</label>
          <select id="edit_client_provider" onchange="updateClientProviderDefaults()">
            ${providersForClientKind(kind).map(p => `<option value="${escAttr(p.slug)}" ${a.provider === p.slug ? 'selected' : ''}>${esc(p.label)}</option>`).join('')}
          </select>
        </div>
        <div class="config-field wide">
          <label>目标客户端 Base URL</label>
          <input type="text" id="edit_upstream" value="${escAttr(a.upstream)}" placeholder="${escAttr(profile.default_base_url || 'https://api.example.com/v1')}">
          <span class="hint">这个 URL 会写入目标客户端配置；非 Codex 账号不会走 deecodex 代理翻译。</span>
        </div>
        <div class="config-field">
          <label>默认模型</label>
          <input type="text" id="edit_default_model" value="${escAttr(a.default_model || '')}" placeholder="${escAttr(profile.default_model || 'model-name')}">
        </div>
        <div class="config-field">
          <label>API Key</label>
          <div class="pass-group">
            <input type="password" id="edit_api_key" value="${escAttr(a.api_key)}" placeholder="输入 API 密钥" autocomplete="off">
            <button type="button" class="pass-toggle" onclick="togglePass('edit_api_key', this)" title="显示/隐藏 API Key" aria-label="显示或隐藏 API Key">
              <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M2.5 12s3.5-6 9.5-6 9.5 6 9.5 6-3.5 6-9.5 6-9.5-6-9.5-6z"></path><circle cx="12" cy="12" r="2.5"></circle></svg>
            </button>
          </div>
        </div>
      </div>
    </section>

    <section class="account-edit-section">
      <div class="account-section-head">
        <div class="section-sub-label">客户端模型映射</div>
        <div class="account-section-desc">不同客户端会写入不同模型槽位；默认模型会同步到上方默认模型字段。</div>
      </div>
      <div class="section-action-row">
        <button class="btn btn-ghost" onclick="fetchClientModels()">从上游获取模型列表</button>
        <span id="clientModelFetchStatus"></span>
      </div>
      <div class="model-map-head client-model-map-head client-model-template">
        <span>客户端槽位</span>
        <span>上游模型</span>
        <span></span>
      </div>
      <div id="clientModelMapRows">${renderClientModelMappingRows(a)}</div>
      <div class="model-add-row"><button onclick="addClientModelRow()">+ 添加自定义槽位</button></div>
    </section>

    <section class="account-edit-section">
      <div class="account-section-head">
        <div class="section-sub-label">配置写入</div>
        <div class="account-section-desc">写入前会做 dry-run 并生成备份；OpenClaw 优先使用官方 config dry-run/validate。</div>
      </div>
      <div class="config-fields">
        <div class="config-field">
          <label>配置路径 <span class="optional-label">可选</span></label>
          <input type="text" id="edit_client_config_path" value="${escAttr(configPath)}" placeholder="${escAttr(profile.config_path_hint || '')}">
        </div>
        <div class="config-field">
          <label>请求历史代理</label>
          <label class="history-toggle${proxyEnabled ? ' on' : ''}" id="edit_client_proxy_toggle" onclick="toggleClientProxyRecording()">
            <div class="toggle-dot"></div> 启用记录
          </label>
          <span class="hint">${proxyEnabled && proxyBaseUrl ? esc(proxyBaseUrl) : '开启后写入本地代理 URL，并用代理 token 识别账号。'}</span>
        </div>
        <div class="config-field">
          <label>Key 环境变量名</label>
          <input type="text" id="edit_client_api_key_env" value="${escAttr(apiKeyEnv)}" placeholder="OPENAI_API_KEY">
          <span class="hint">${esc(secretHint)}</span>
        </div>
        ${kind === 'claude_code' ? `<div class="config-field">
          <label>${clientSecretLabel(kind)}</label>
          ${renderClaudeAuthEnvSelect(authEnv)}
          <span class="hint">兼容 Claude Code 的 API Key 和 Auth Token 两种本地环境变量。</span>
        </div>` : `<div class="config-field">
          <label>${clientSecretLabel(kind)}</label>
          <input type="text" value="${escAttr(apiKeyEnv)}" disabled>
        </div>
        `}
      </div>
      <div class="section-action-row">
        ${a.id ? `<button class="btn btn-ghost" onclick="refreshClientAccountStatus('${escAttr(a.id)}')">刷新状态</button>` : ''}
        ${a.id ? `<button class="btn btn-ghost" onclick="editConfigFile('${escAttr(a.id)}')">编辑配置文件</button>` : ''}
        <button class="btn btn-ghost" onclick="dryRunEditingClientAccount()">预检当前表单</button>
        <span id="clientDryRunStatus"></span>
      </div>
      <div id="clientApplyPreview" class="client-apply-preview"></div>
    </section>

    <section class="account-edit-section">
      <div class="account-section-head">
        <div>
          <div class="section-sub-label">最近备份</div>
          <div class="account-section-desc">正式写入或手动恢复前都会生成备份；恢复也会先备份当前文件。</div>
        </div>
        ${a.id ? `<button class="btn btn-ghost btn-small" onclick="fetchClientBackupsForDetail('${escAttr(a.id)}')">刷新</button>` : ''}
      </div>
      <div id="clientBackupList" class="client-backup-list">
        <span class="status-muted">${a.id ? '加载中...' : '保存账号后显示备份'}</span>
      </div>
    </section>

    <section class="account-edit-section">
      <div class="account-section-head">
        <div>
          <div class="section-sub-label">最近配置事件</div>
          <div class="account-section-desc">只记录外部客户端配置操作，不混入 deecodex 代理请求历史。</div>
        </div>
        ${a.id ? `<button class="btn btn-ghost btn-small" onclick="fetchClientEventsForDetail('${escAttr(a.id)}')">刷新</button>` : ''}
      </div>
      <div id="clientEventLog" class="client-event-log">
        <span class="status-muted">${a.id ? '加载中...' : '保存账号后显示事件'}</span>
      </div>
    </section>

    <div class="accounts-actions">
      <button class="btn btn-primary" onclick="saveAccount()">保存账号</button>
      ${a.id ? `<button class="btn btn-ghost" onclick="saveAndApplyClientAccount('${escAttr(a.id)}')">保存并写入配置</button>` : ''}
      <button class="btn btn-danger" onclick="deleteAccount('${escAttr(a.id)}')">删除账号</button>
    </div>
  </div>`;
}

function selectClientKind(kind) {
  selectedClientKind = normalizeClientKind(kind);
  if (accountsView === 'add') accountsView = 'list';
  renderMainContent();
  if (accountsView === 'list') {
    (accountsData.accounts || [])
      .filter(a => accountClientKind(a) === selectedClientKind)
      .forEach(a => {
        if (isCodexAccount(a)) fetchBalanceForCard(a);
        else fetchClientStatusForCard(a);
      });
  }
}

function toggleClientProxyRecording() {
  if (!editingAccount || isCodexAccount(editingAccount)) return;
  editingAccount.client_options = editingAccount.client_options || {};
  editingAccount.client_options.proxy_recording_enabled = !Boolean(editingAccount.client_options.proxy_recording_enabled);
  const toggle = document.getElementById('edit_client_proxy_toggle');
  if (toggle) toggle.classList.toggle('on', editingAccount.client_options.proxy_recording_enabled);
}

function getProviderPreset(provider) {
  return providerPresets.find(pp => pp.slug === provider) || null;
}

function accountCapabilityLabels(a) {
  const fromAccount = a.provider_options && Array.isArray(a.provider_options.capability_labels)
    ? a.provider_options.capability_labels
    : null;
  if (fromAccount) return fromAccount;
  const preset = getProviderPreset(a.provider);
  return preset && Array.isArray(preset.capability_labels) ? preset.capability_labels : [];
}

function renderCapabilityAccountOptions(current) {
  const list = accountsData.accounts || [];
  const currentId = current?.id || '';
  const selected = current?.capability_account_id || '';
  const options = ['<option value="">请选择能力账号</option>'];
  list
    .filter(account => account.id && account.id !== currentId)
    .forEach(account => {
      const isSelected = account.id === selected ? 'selected' : '';
      options.push(`<option value="${escAttr(account.id)}" ${isSelected}>${esc(account.name)} · ${esc(account.provider)}</option>`);
    });
  return options.join('');
}

function toggleCapabilityFields() {
  const enabled = document.getElementById('edit_capability_enabled')?.checked;
  const fields = document.getElementById('capabilityFields');
  if (fields) fields.style.display = enabled ? '' : 'none';
}

function renderDevPipelineAccountOptions(current, selected) {
  const list = accountsData.accounts || [];
  const options = [`<option value="" ${!selected ? 'selected' : ''}>活跃账号（当前主账号）</option>`];
  list
    .filter(account => account.id)
    .forEach(account => {
      const isSelected = account.id === selected ? 'selected' : '';
      options.push(`<option value="${escAttr(account.id)}" ${isSelected}>${esc(account.name)} · ${esc(account.provider)}</option>`);
    });
  return options.join('');
}

function toggleDevPipelineFields() {
  const enabled = document.getElementById('edit_dev_pipeline_enabled')?.checked;
  const fields = document.getElementById('devPipelineFields');
  if (fields) fields.style.display = enabled ? '' : 'none';
}

function currentEndpoint(a) {
  const endpoints = Array.isArray(a?.endpoints) ? a.endpoints : [];
  const selectedId = selectedEndpointId(a);
  if (selectedId) return endpoints.find(ep => ep.id === selectedId) || endpoints[0] || null;
  return endpoints[0] || null;
}

function currentEndpointIndex(a) {
  const endpoints = Array.isArray(a?.endpoints) ? a.endpoints : [];
  const ep = currentEndpoint(a);
  if (!ep) return -1;
  const idx = endpoints.findIndex(item => item.id === ep.id);
  return idx >= 0 ? idx : 0;
}

function endpointKindLabel(kind) {
  const labels = {
    open_ai_chat: 'Chat 兼容',
    open_ai_responses: 'OpenAI Responses 直连',
    anthropic_messages: 'Anthropic Messages',
    custom_chat: 'Chat 兼容',
    custom_responses: 'Responses 直连',
    OpenAiChat: 'Chat 兼容',
    OpenAiResponses: 'OpenAI Responses 直连',
    AnthropicMessages: 'Anthropic Messages',
    CustomChat: 'Chat 兼容',
    CustomResponses: 'Responses 直连',
  };
  return labels[kind] || kind || 'Chat 兼容';
}

function visionModeLabel(mode) {
  const labels = {
    off: '视觉关闭',
    native: '原生多模态',
    glue: '胶水多模态',
    Off: '视觉关闭',
    Native: '原生多模态',
    Glue: '胶水多模态',
  };
  return labels[mode] || '视觉关闭';
}

function formatTimeShort(ts) {
  const n = Number(ts || 0);
  if (!n) return '—';
  const d = new Date(n * 1000);
  return d.toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' });
}

function setVisionMode(mode) {
  const input = document.getElementById('edit_vision_mode');
  if (input) input.value = mode;
  document.querySelectorAll('.vision-mode-option').forEach(btn => {
    btn.classList.toggle('active', btn.dataset.mode === mode);
  });
  toggleVisionFields();
}

function selectedEndpointId(a) {
  if (!a) return null;
  if (a._editing_endpoint_id) return a._editing_endpoint_id;
  if (a.id === (accountsData.active_account_id || accountsData.active_id) && accountsData.active_endpoint_id) {
    return accountsData.active_endpoint_id;
  }
  return Array.isArray(a.endpoints) && a.endpoints[0] ? a.endpoints[0].id : null;
}

function providerDefaultTemplate(provider) {
  const templates = endpointTemplates || [];
  if (provider === 'anthropic') {
    return templates.find(t => t.id === 'anthropic_messages')
      || templates.find(t => t.kind === 'anthropic_messages' || t.kind === 'AnthropicMessages')
      || null;
  }
  if (provider === 'openai') {
    return templates.find(t => t.id === 'responses_direct')
      || templates.find(t => t.kind === 'open_ai_responses' || t.kind === 'OpenAiResponses')
      || null;
  }
  return templates.find(t => t.id === 'chat_compatible')
    || templates.find(t => t.kind === 'open_ai_chat' || t.kind === 'OpenAiChat')
    || templates[0]
    || null;
}

function newEndpointId() {
  return 'endpoint_' + Date.now().toString(16) + '_' + Math.random().toString(16).slice(2, 8);
}

function createEndpointFromTemplate(template, account) {
  const tpl = template || providerDefaultTemplate(account?.provider) || {};
  const hasTemplateBaseUrl = Object.prototype.hasOwnProperty.call(tpl, 'default_base_url');
  const baseUrl = hasTemplateBaseUrl
    ? (tpl.default_base_url || account?.upstream || '')
    : (account?.upstream || 'https://openrouter.ai/api/v1');
  const kind = tpl.kind || 'open_ai_chat';
  return {
    id: newEndpointId(),
    name: tpl.label || endpointKindLabel(kind),
    kind,
    base_url: baseUrl,
    path: tpl.default_path || '',
    template_id: tpl.id || '',
    template_version: 1,
    model_map: { ...(account?.model_map || {}) },
    model_profiles: {},
    vision: {
      mode: tpl.default_vision_mode || 'off',
      unsupported_image_policy: 'reject',
      glue_strategy: 'final_answer',
      adapter_id: 'minimax_coding_plan_vlm',
      base_url: account?.vision_upstream || '',
      api_key: account?.vision_api_key || '',
      model: account?.vision_model || '',
      path: account?.vision_endpoint || 'v1/coding_plan/vlm',
    },
    custom_headers: { ...(account?.custom_headers || {}) },
    request_timeout_secs: account?.request_timeout_secs || null,
    max_retries: account?.max_retries ?? null,
    context_window_override: account?.context_window_override || null,
    reasoning_effort_override: account?.reasoning_effort_override || null,
    thinking_tokens: account?.thinking_tokens || null,
    fast_mode_enabled: false,
    fast_service_tier: 'priority',
    balance_url: account?.balance_url || '',
  };
}

function ensureAccountEndpoints(a) {
  if (!a) return [];
  if (!Array.isArray(a.endpoints)) a.endpoints = [];
  if (a.endpoints.length === 0) {
    a.endpoints.push(createEndpointFromTemplate(providerDefaultTemplate(a.provider), a));
  }
  if (!selectedEndpointId(a)) a._editing_endpoint_id = a.endpoints[0].id;
  return a.endpoints;
}

function navigateAccounts(view) {
  accountsView = view;
  if (view === 'list') editingAccount = null;
  renderMainContent();
}

function renderMainContent() {
  const main = document.getElementById('mainContent');
  main.classList.toggle('accounts-main', accountsView === 'list');
  const html = renderAccountsPanel();
  main.innerHTML = typeof wrapPrimaryPanel === 'function' ? wrapPrimaryPanel('accounts', html) : html;
  afterRenderAccountsPanel();
}

function afterRenderAccountsPanel() {
  if (accountsView === 'edit' && editingAccount && !isCodexAccount(editingAccount) && editingAccount.id) {
    fetchClientEventsForDetail(editingAccount.id);
    fetchClientBackupsForDetail(editingAccount.id);
  }
}

function renderAccountsPanel() {
  if (accountsView === 'add') return renderAddAccount();
  if (accountsView === 'edit') return renderAccountDetail();
  return renderAccountList();
}

// ── Level 1: 账号列表 ──

function renderAccountList() {
  const list = accountsData.accounts || [];
  const filtered = list.filter(a => accountClientKind(a) === selectedClientKind);
  let cards = '';
  if (filtered.length === 0) {
    cards = `<div class="empty-state">暂无${esc(CLIENT_KIND_LABELS[selectedClientKind] || '客户端')}账号，点击上方按钮创建</div>`;
  } else {
    cards = '<div class="accounts-grid">' + filtered.map(a => {
      const active = a.id === (accountsData.active_account_id || accountsData.active_id);
      if (!isCodexAccount(a)) return renderClientAccountCard(a);
      const ep = currentEndpoint(a);
      const visionMode = ep?.vision?.mode || (a.vision_enabled ? 'glue' : 'off');
      const contextWindow = ep?.context_window_override ?? null;
      const reasoningEffort = ep?.reasoning_effort_override ?? null;
      const caps = accountCapabilityLabels(a).slice(0, 2);
      const capabilityAccount = a.capability_account_id
        ? list.find(candidate => candidate.id === a.capability_account_id)
        : null;
      const primaryTags = [
        endpointKindLabel(ep?.kind),
        ...caps,
        visionModeLabel(visionMode),
      ];
      const advancedTags = [];
      if (contextWindow) advancedTags.push(`上下文 ${contextWindow.toLocaleString()}`);
      if (reasoningEffort) advancedTags.push(`推理 ${reasoningEffort}`);
      if (a.capability_enabled) advancedTags.push(`能力 ${capabilityAccount ? capabilityAccount.name : '未配置'}`);
      if (a.dev_pipeline_enabled) advancedTags.push(`开发 ${a.dev_pipeline_trigger_mode === 'always' ? '始终' : (a.dev_pipeline_command || '/dev-pipeline')}`);
      const metaTags = primaryTags.slice(0, 3).map(label => `<span class="card-context">${esc(label)}</span>`).join('')
        + (advancedTags.length ? `<span class="card-context tag-muted">+${advancedTags.length}</span>` : '');
      return `<div class="account-card${active ? ' active' : ''}">
        <div class="account-card-mainline">
          <div class="account-card-primary">
            <div class="account-card-info">
              <div class="account-card-header">
                <div class="account-card-titlebar">
                  ${renderProviderBadge(a.provider)}
                  ${active ? '<span class="active-badge">活跃</span>' : ''}
                </div>
              </div>
              <div class="account-card-body">
                <div class="account-card-main">
                  <div class="card-name">${esc(a.name)}</div>
                  <div class="card-upstream" title="${escAttr(a.upstream)}">${esc(trunc(a.upstream, 64))}</div>
                </div>
              </div>
            </div>
            <div class="account-meta-tags mid-tags">${metaTags}</div>
          </div>
          <div class="account-card-side">
            <div class="card-balance" id="balance-${escAttr(a.id)}">
              <span class="balance-loading">—</span>
            </div>
            <div class="card-actions-row">
              ${active
                ? '<button class="account-action account-applied" disabled>已应用</button>'
                : `<button class="account-action account-apply" onclick="applyAccount('${escAttr(a.id)}')">应用</button>`}
              <button class="account-action" onclick="editAccount('${escAttr(a.id)}')">编辑</button>
              <button class="account-action account-refresh" onclick="refreshBalanceForCard('${escAttr(a.id)}')" title="刷新余额">刷新</button>
              <button class="account-action danger" onclick="deleteAccount('${escAttr(a.id)}')">删除</button>
            </div>
          </div>
        </div>
      </div>`;
    }).join('') + '</div>';
  }

  return `<div class="accounts-page-shell">
    <div class="accounts-static-header">
      <div class="page-header accounts-page-header">
        <div><h2>账号管理</h2></div>
        <div class="page-header-actions">
          <button class="btn btn-ghost" onclick="importFromCodex()">导入配置</button>
          <button class="btn btn-ghost" onclick="scanClientAccounts()">扫描客户端</button>
          <button class="btn btn-primary" onclick="navigateAccounts('add')">添加账号</button>
        </div>
      </div>
      ${renderClientSwitcher(list)}
    </div>
    <div class="accounts-scroll-region">
      ${cards}
    </div>
  </div>`;
}

function renderClientAccountCard(a) {
  const kind = accountClientKind(a);
  const profile = getClientProfile(kind);
  const statusId = `client-status-${escAttr(a.id)}`;
  const path = a.client_options?.config_path || profile?.config_path_hint || '';
  const last = a.last_applied_at ? formatTimeShort(a.last_applied_at) : '未写入';
  const model = a.default_model || '未配置模型';
  const metaTags = `<span class="card-context">${esc(model)}</span><span class="card-context tag-muted">${esc(last)}</span>`;
  return `<div class="account-card client-account-card">
    <div class="account-card-mainline">
      <div class="account-card-primary">
        <div class="account-card-info">
          <div class="account-card-header">
            <div class="account-card-titlebar">
              <span class="client-kind-badge">${clientIcon(kind)}${esc(CLIENT_KIND_LABELS[kind] || kind)}</span>
              ${renderProviderBadge(a.provider)}
            </div>
          </div>
          <div class="account-card-body">
            <div class="account-card-main">
              <div class="card-name">${esc(a.name)}</div>
              <div class="card-upstream" title="${escAttr(a.upstream)}">${esc(trunc(a.upstream || path, 70))}</div>
            </div>
          </div>
        </div>
        <div class="account-meta-tags mid-tags">${metaTags}</div>
      </div>
      <div class="account-card-side">
        <div class="card-balance client-status-box" id="${statusId}"><span class="balance-loading">待检查</span></div>
        <div class="card-actions-row">
          <button class="account-action account-apply" onclick="applyClientAccount('${escAttr(a.id)}')">写入</button>
          <button class="account-action" onclick="editAccount('${escAttr(a.id)}')">编辑</button>
          <button class="account-action" onclick="refreshClientAccountStatus('${escAttr(a.id)}')">刷新</button>
          <button class="account-action danger" onclick="deleteAccount('${escAttr(a.id)}')">删除</button>
        </div>
      </div>
    </div>
  </div>`;
}

// ── Level 2: 添加账号 ──

function renderAddAccount() {
  let cards = '';
  if (providerPresets.length === 0) {
    cards = '<div class="empty-state">加载供应商列表...</div>';
  } else {
    const providers = providersForClientKind(selectedClientKind);
    cards = '<div class="provider-grid">' + providers.map(p => {
      const upstream = p.default_upstream || '(自定义)';
      return `<div class="provider-card" onclick="addAccount('${escAttr(p.slug)}', '${escAttr(selectedClientKind)}')">
        <div class="provider-icon">${providerIcon(p.slug, p.label)}</div>
        <div class="provider-name">${esc(p.label)}</div>
        <div class="provider-desc">${esc(p.description)}</div>
        <div class="provider-default-upstream" title="${escAttr(upstream)}">${esc(trunc(upstream, 42))}</div>
      </div>`;
    }).join('') + '</div>';
  }

  return `<div class="breadcrumb">
    <span class="back-link" onclick="navigateAccounts('list')">← 账号列表</span>
    <span> / 添加账号</span>
  </div>
  <div class="page-header"><h2>选择供应商</h2><p>为 ${esc(CLIENT_KIND_LABELS[selectedClientKind] || '客户端')} 创建新账号配置</p></div>
  ${cards}`;
}

function providersForClientKind(kind) {
  const normalized = normalizeClientKind(kind);
  if (normalized === 'codex') return providerPresets;
  const bySlug = slug => providerPresets.find(p => p.slug === slug);
  const custom = bySlug('custom') || {
    slug: 'custom',
    label: '自定义',
    description: 'OpenAI-compatible 或客户端原生 Base URL',
    default_upstream: '',
    known_models: [],
  };
  if (normalized === 'claude_code') return [
    bySlug('anthropic'),
    bySlug('deepseek'),
    bySlug('kimi'),
    bySlug('minimax'),
    bySlug('mimo'),
    bySlug('longcat'),
    bySlug('qwen'),
    bySlug('glm'),
    bySlug('openrouter'),
    custom,
  ].filter(Boolean);
  if (normalized === 'openclaw') return [bySlug('openrouter'), bySlug('anthropic'), bySlug('openai'), bySlug('qwen'), bySlug('minimax'), custom].filter(Boolean);
  if (normalized === 'hermes') return [bySlug('openrouter'), bySlug('anthropic'), bySlug('openai'), bySlug('qwen'), bySlug('minimax'), custom].filter(Boolean);
  return [bySlug('openai'), bySlug('openrouter'), bySlug('qwen'), custom].filter(Boolean);
}

function clientProviderDefaults(kind, provider, preset = null) {
  const normalized = normalizeClientKind(kind);
  const p = preset || getProviderPreset(provider) || {};
  if (normalized === 'claude_code' && provider === 'deepseek') {
    return {
      upstream: 'https://api.deepseek.com/anthropic',
      default_model: 'deepseek-v4-pro[1m]',
      known_models: ['deepseek-v4-pro[1m]', 'deepseek-v4-pro', 'deepseek-v4-flash'],
      api_key_env: 'ANTHROPIC_AUTH_TOKEN',
    };
  }
  if (normalized === 'claude_code' && provider === 'kimi') {
    return {
      upstream: 'https://api.moonshot.cn/anthropic',
      default_model: 'kimi-k2.5',
      known_models: ['kimi-k2.5', 'kimi-k2-turbo-preview', 'kimi-k2-0711-preview'],
      api_key_env: 'ANTHROPIC_AUTH_TOKEN',
    };
  }
  if (normalized === 'claude_code' && provider === 'minimax') {
    return {
      upstream: 'https://api.minimaxi.com/anthropic',
      default_model: 'MiniMax-M2.7',
      known_models: ['MiniMax-M2.7', 'MiniMax-M2', 'MiniMax-M1'],
      api_key_env: 'ANTHROPIC_API_KEY',
    };
  }
  if (normalized === 'claude_code' && provider === 'mimo') {
    return {
      upstream: 'https://api.mimo-v2.com/anthropic',
      default_model: 'mimo-v2.5-pro',
      known_models: ['mimo-v2.5-pro', 'mimo-v2.5', 'mimo-v2-pro'],
      api_key_env: 'ANTHROPIC_API_KEY',
    };
  }
  if (normalized === 'claude_code' && provider === 'longcat') {
    return {
      upstream: 'https://api.longcat.chat/anthropic',
      default_model: 'LongCat-Flash-Chat',
      known_models: [
        'LongCat-Flash-Chat',
        'LongCat-Flash-Thinking-2601',
        'LongCat-Flash-Thinking',
        'LongCat-Flash-Lite',
        'LongCat-2.0-Preview',
      ],
      api_key_env: 'ANTHROPIC_AUTH_TOKEN',
    };
  }
  if (normalized === 'claude_code' && provider === 'qwen') {
    return {
      upstream: 'https://dashscope.aliyuncs.com/apps/anthropic',
      default_model: 'qwen3.6-plus',
      known_models: ['qwen3.6-plus', 'qwen-plus', 'qwen-max', 'qwen-turbo'],
      api_key_env: 'ANTHROPIC_API_KEY',
    };
  }
  if (normalized === 'claude_code' && provider === 'glm') {
    return {
      upstream: 'https://open.bigmodel.cn/api/anthropic',
      default_model: 'glm-5.1',
      known_models: ['glm-5.1', 'glm-5', 'glm-4.7', 'glm-4.5-air'],
      api_key_env: 'ANTHROPIC_AUTH_TOKEN',
    };
  }
  return {
    upstream: p.default_upstream || '',
    default_model: (Array.isArray(p.known_models) && p.known_models[0]) || '',
    known_models: Array.isArray(p.known_models) ? p.known_models : [],
    api_key_env: defaultApiKeyEnvForClient({ provider, client_kind: normalized }),
  };
}

// ── Level 3: 编辑账号 ──

function renderAccountDetail() {
  if (!editingAccount) return '<div class="empty-state">账号数据丢失，请返回列表</div>';
  const a = editingAccount;
  if (!isCodexAccount(a)) return renderClientAccountDetail();
  ensureAccountEndpoints(a);
  const ep = currentEndpoint(a) || {};
  const visionMode = ep.vision?.mode || (a.vision_enabled ? 'glue' : 'off');
  const contextWindow = ep.context_window_override ?? null;
  const reasoningEffort = ep.reasoning_effort_override ?? null;
  const thinkingTokens = ep.thinking_tokens ?? null;
  const fastEnabled = ep.fast_mode_enabled === true;
  const fastServiceTier = ep.fast_service_tier || 'priority';
  const customHeaders = ep.custom_headers || {};
  const requestTimeout = ep.request_timeout_secs ?? null;
  const maxRetries = ep.max_retries ?? a.max_retries;
  const knownModels = getProviderKnownModels(a.provider);

  return `<div class="breadcrumb">
    <span class="back-link" onclick="navigateAccounts('list')">← 账号列表</span>
    <span> / ${esc(a.name)}</span>
  </div>
  <div class="page-header account-detail-header">
    <div class="account-detail-title">
      <img src="${providerLogoSrc(a.provider)}" alt="" aria-hidden="true">
      <div>
        <div class="account-detail-heading">
          <h2>${esc(a.name)}</h2>
          ${renderProviderBadge(a.provider)}
        </div>
      </div>
    </div>
  </div>

  <div class="account-form">
    <section class="account-edit-section">
      <div class="account-section-head">
        <div class="section-sub-label">账号凭据</div>
        <div class="account-section-desc">一个账号就是一组供应商、协议、URL 和 Key；同一个 Key 可创建多个账号分别保存。</div>
      </div>
      <div class="config-fields">
        <div class="config-field account-name-field">
          <label>账号名称</label>
          <input type="text" id="edit_name" value="${escAttr(a.name)}" placeholder="输入账号显示名">
        </div>
        <div class="config-field account-key-field">
          <label>API Key</label>
          <div class="pass-group">
            <input type="password" id="edit_api_key" value="${escAttr(a.api_key)}" placeholder="输入 API 密钥" autocomplete="off">
            <button type="button" class="pass-toggle" onclick="togglePass('edit_api_key', this)" title="显示/隐藏 API Key" aria-label="显示或隐藏 API Key">
              <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M2.5 12s3.5-6 9.5-6 9.5 6 9.5 6-3.5 6-9.5 6-9.5-6-9.5-6z"></path><circle cx="12" cy="12" r="2.5"></circle></svg>
            </button>
          </div>
        </div>
        <div class="config-field wide">
          <label>上游 URL</label>
          <div class="upstream-test-group">
            <input type="text" id="edit_upstream" value="${escAttr(ep.base_url || a.upstream)}" placeholder="https://api.example.com/v1">
            <button class="btn btn-ghost" onclick="testUpstreamConnectivity()">测试连通性</button>
          </div>
          <div class="inline-test-result">
            <span id="connectivityResult"></span>
          </div>
        </div>
      </div>
    </section>

    <section class="account-edit-section">
      <div class="account-section-head">
        <div class="section-sub-label">上游</div>
        <div class="account-section-desc">配置当前账号的协议模式、上游 URL 和余额探测。</div>
      </div>
      <div class="config-fields">
        <div class="config-field">
          <label>上游 API 类型</label>
          <select id="edit_endpoint_kind">
            ${ep.kind === 'custom_chat' || ep.kind === 'CustomChat' ? '<option value="custom_chat" selected hidden>OpenAI Chat 兼容（自定义路径）</option>' : ''}
            ${ep.kind === 'custom_responses' || ep.kind === 'CustomResponses' ? '<option value="custom_responses" selected hidden>OpenAI Responses 直连（自定义路径）</option>' : ''}
            <option value="open_ai_chat" ${(ep.kind || 'open_ai_chat') === 'open_ai_chat' || ep.kind === 'OpenAiChat' ? 'selected' : ''}>OpenAI Chat 兼容（推荐）</option>
            <option value="open_ai_responses" ${ep.kind === 'open_ai_responses' || ep.kind === 'OpenAiResponses' ? 'selected' : ''}>OpenAI Responses 直连</option>
            <option value="anthropic_messages" ${ep.kind === 'anthropic_messages' || ep.kind === 'AnthropicMessages' ? 'selected' : ''}>Anthropic Messages</option>
          </select>
          <span class="hint">DeepSeek、OpenRouter 这类一般选 OpenAI Chat 兼容；只有上游原生支持 Responses API 时才选直连。</span>
        </div>
        <div class="config-field">
          <label>余额查询 URL <span class="optional-label">可选</span></label>
          <input type="text" id="edit_balance_url" value="${escAttr(ep.balance_url || '')}" placeholder="留空则自动探测">
        </div>
      </div>
    </section>

    <section class="account-edit-section">
      <div class="account-section-head">
        <div class="section-sub-label">模型</div>
        <div class="account-section-desc">每一行就是一个模型的映射和图片处理方式。</div>
      </div>
      <div class="section-action-row">
        <button class="btn btn-ghost" onclick="fetchAndPopulateModels()">从上游获取模型列表</button>
        <span id="modelFetchStatus"></span>
      </div>
      <div class="model-map-head">
        <span>Codex 请求模型</span>
        <span>上游模型</span>
        <span>图片处理</span>
      </div>
      <div id="modelMapRows">${renderModelMappingRows(knownModels)}</div>
      <div class="model-add-row"><button onclick="addModelRow('modelMapRows', '${escAttr(JSON.stringify(knownModels))}')">+ 添加模型映射</button></div>
    </section>

    <section class="account-edit-section">
      <div class="account-section-head">
        <div class="section-sub-label">其他模型图片处理</div>
        <div class="account-section-desc">上方没有单独配置的模型，才使用这里。</div>
      </div>
      <div class="config-fields">
        <div class="config-field">
          <label>其他模型</label>
          <input type="hidden" id="edit_vision_mode" value="${escAttr(String(visionMode).toLowerCase())}">
          <div class="vision-mode-segments" role="group" aria-label="视觉能力">
            <button type="button" class="vision-mode-option ${visionMode === 'off' || visionMode === 'Off' ? 'active' : ''}" data-mode="off" onclick="setVisionMode('off')">关闭</button>
            <button type="button" class="vision-mode-option ${visionMode === 'native' || visionMode === 'Native' ? 'active' : ''}" data-mode="native" onclick="setVisionMode('native')">原生</button>
            <button type="button" class="vision-mode-option ${visionMode === 'glue' || visionMode === 'Glue' ? 'active' : ''}" data-mode="glue" onclick="setVisionMode('glue')">胶水</button>
          </div>
          <span class="hint">模型映射行优先生效；这里处理临时模型或未列出的模型。</span>
        </div>
        <div class="config-field">
          <label>不支持图片时</label>
          <select id="edit_unsupported_image_policy">
            <option value="reject" ${(ep.vision?.unsupported_image_policy || 'reject') === 'reject' ? 'selected' : ''}>拒绝请求并提示</option>
            <option value="strip_with_warning" ${ep.vision?.unsupported_image_policy === 'strip_with_warning' ? 'selected' : ''}>剥离图片并继续</option>
          </select>
          <span class="hint">关闭视觉或模型覆盖为关闭时生效</span>
        </div>
      </div>
      <div id="visionFields" style="${visionMode === 'glue' || visionMode === 'Glue' ? '' : 'display:none;'}">
        <div class="config-fields nested-fields">
          <div class="config-field">
            <label>胶水适配器</label>
            <select id="edit_vision_adapter">
              <option value="minimax_coding_plan_vlm" ${(ep.vision?.adapter_id || 'minimax_coding_plan_vlm') === 'minimax_coding_plan_vlm' ? 'selected' : ''}>MiniMax Coding Plan VLM</option>
            </select>
            <span class="hint">第一版仅实现 MiniMax 胶水适配器</span>
          </div>
          <div class="config-field">
            <label>胶水策略</label>
            <select id="edit_glue_strategy">
              <option value="final_answer" ${(ep.vision?.glue_strategy || 'final_answer') === 'final_answer' ? 'selected' : ''}>视觉模型直接回答</option>
              <option value="caption_then_main" ${ep.vision?.glue_strategy === 'caption_then_main' ? 'selected' : ''}>先识图再交给主模型</option>
            </select>
          </div>
          <div class="config-field">
            <label>视觉上游 URL</label>
            <input type="text" id="edit_vision_upstream" value="${escAttr(ep.vision?.base_url || a.vision_upstream || '')}" placeholder="https://api.minimaxi.com">
          </div>
          <div class="config-field">
            <label>视觉 API Key</label>
            <div class="pass-group">
              <input type="password" id="edit_vision_api_key" value="${escAttr(ep.vision?.api_key || a.vision_api_key || '')}" placeholder="视觉模型密钥" autocomplete="off">
              <button type="button" class="pass-toggle" onclick="togglePass('edit_vision_api_key', this)" title="显示/隐藏视觉 API Key" aria-label="显示或隐藏视觉 API Key">
                <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M2.5 12s3.5-6 9.5-6 9.5 6 9.5 6-3.5 6-9.5 6-9.5-6-9.5-6z"></path><circle cx="12" cy="12" r="2.5"></circle></svg>
              </button>
            </div>
          </div>
          <div class="config-field">
            <label>视觉模型名</label>
            <input type="text" id="edit_vision_model" value="${escAttr(ep.vision?.model || a.vision_model || '')}" placeholder="MiniMax-M2.7">
          </div>
          <div class="config-field">
            <label>视觉端点路径</label>
            <input type="text" id="edit_vision_endpoint" value="${escAttr(ep.vision?.path || a.vision_endpoint || 'v1/coding_plan/vlm')}" placeholder="v1/coding_plan/vlm">
            <div class="inline-test-row">
              <button class="btn btn-ghost" onclick="testVisionConnectivity()">测试视觉端点</button>
              <span id="visionConnectivityResult"></span>
            </div>
          </div>
        </div>
      </div>
    </section>

    <section class="account-edit-section">
      <div class="account-section-head">
        <div class="section-sub-label">运行参数</div>
        <div class="account-section-desc">按账号覆盖上下文窗口和推理预算。</div>
      </div>
      <div class="config-fields">
        <div class="config-field">
          <label class="toggle-label">
            <input type="checkbox" id="edit_cw_enabled" ${contextWindow ? 'checked' : ''} onchange="toggleContextWindowFields()">
            上下文窗口覆盖
          </label>
          <span class="hint">勾选后 Codex 使用下方 token 数作为该账号上下文。</span>
        </div>
        <div class="config-field" id="cwSizeField" style="${contextWindow ? '' : 'display:none;'}">
          <label>上下文窗口大小 (token)</label>
          <input type="number" id="edit_cw_size" value="${contextWindow || 1000000}" min="1" max="10000000" step="1" placeholder="1000000">
          <span class="hint">Codex 有效上下文约为此值 × 95%。</span>
        </div>
        <div class="config-field">
          <label class="toggle-label">
            <input type="checkbox" id="edit_reasoning_enabled" ${reasoningEffort ? 'checked' : ''} onchange="toggleReasoningFields()">
            推理强度覆盖
          </label>
          <span class="hint">用于 Claude、R1 等需要固定思考预算的账号。</span>
        </div>
        <div class="config-field">
          <label class="toggle-label">
            <input type="checkbox" id="edit_fast_enabled" ${fastEnabled ? 'checked' : ''} onchange="toggleFastFields()">
            GPT Fast 服务层
          </label>
          <span class="hint">仅 OpenAI Responses 直连端点生效；保持推理预算，只注入 service_tier。</span>
        </div>
      </div>
      <div id="reasoningFields" style="${reasoningEffort ? '' : 'display:none;'}">
        <div class="config-fields nested-fields">
          <div class="config-field">
            <label>推理强度</label>
            <select id="edit_reasoning_effort">
              <option value="" ${!reasoningEffort ? 'selected' : ''}>不覆盖（跟随 Codex 请求）</option>
              <option value="low" ${reasoningEffort === 'low' ? 'selected' : ''}>low - 低推理</option>
              <option value="medium" ${reasoningEffort === 'medium' ? 'selected' : ''}>medium - 中等推理</option>
              <option value="high" ${reasoningEffort === 'high' ? 'selected' : ''}>high - 高推理</option>
              <option value="max" ${reasoningEffort === 'max' ? 'selected' : ''}>max - 最大推理</option>
            </select>
          </div>
          <div class="config-field">
            <label>思考 Token 预算 <span class="optional-label">可选</span></label>
            <input type="number" id="edit_thinking_tokens" value="${thinkingTokens || ''}" min="1024" max="128000" step="1024" placeholder="留空不设限制，如 16000">
            <span class="hint">Claude Extended Thinking 的 token 预算，留空则不限制</span>
          </div>
        </div>
      </div>
      <div id="fastFields" style="${fastEnabled ? '' : 'display:none;'}">
        <div class="config-fields nested-fields">
          <div class="config-field">
            <label>service_tier</label>
            <input type="text" id="edit_fast_service_tier" value="${escAttr(fastServiceTier)}" placeholder="priority">
            <span class="hint">默认 priority；旧配置中的 fast 会在请求时自动转为 priority。</span>
          </div>
        </div>
      </div>
    </section>

    <section class="account-edit-section">
      <div class="account-section-head">
        <div class="section-sub-label">能力补全</div>
        <div class="account-section-desc">触发图片、computer、浏览器、MCP 或插件工具时，可先由另一个账号执行观察。</div>
      </div>
      <div class="config-fields">
        <div class="config-field">
          <label class="toggle-label">
            <input type="checkbox" id="edit_capability_enabled" ${a.capability_enabled ? 'checked' : ''} onchange="toggleCapabilityFields()">
            启用能力补全
          </label>
          <span class="hint">能力账号不能选择当前账号，建议使用支持原生工具和多模态的账号。</span>
        </div>
        <div class="config-field" id="capabilityFields" style="${a.capability_enabled ? '' : 'display:none;'}">
          <label>能力账号</label>
          <select id="edit_capability_account_id">${renderCapabilityAccountOptions(a)}</select>
        </div>
      </div>
    </section>

    <section class="account-edit-section">
      <div class="account-section-head">
        <div class="section-sub-label">开发协作编排</div>
        <div class="account-section-desc">用角色账号协作完成开发任务：方案设计、实现填充、验收收口；不绑定任何固定供应商。</div>
      </div>
      <div class="config-fields">
        <div class="config-field wide">
          <label class="toggle-label">
            <input type="checkbox" id="edit_dev_pipeline_enabled" ${a.dev_pipeline_enabled ? 'checked' : ''} onchange="toggleDevPipelineFields()">
            启用开发协作编排
          </label>
          <span class="hint">Codex 中输入触发命令即可进入协作流程，默认以当前活跃账号为主。</span>
        </div>
        <div class="dev-pipeline-fields" id="devPipelineFields" style="${a.dev_pipeline_enabled ? '' : 'display:none;'}">
          <div class="config-fields nested-fields">
            <div class="config-field">
              <label>触发方式</label>
              <select id="edit_dev_pipeline_trigger_mode">
                <option value="manual" ${(a.dev_pipeline_trigger_mode || 'manual') === 'manual' ? 'selected' : ''}>手动触发</option>
                <option value="always" ${a.dev_pipeline_trigger_mode === 'always' ? 'selected' : ''}>始终触发</option>
              </select>
            </div>
            <div class="config-field">
              <label>触发命令</label>
              <input type="text" id="edit_dev_pipeline_command" value="${escAttr(a.dev_pipeline_command || '/dev-pipeline')}" placeholder="/dev-pipeline">
            </div>
            <div class="config-field">
              <label>方案设计账号</label>
              <select id="edit_dev_pipeline_architect_account_id">${renderDevPipelineAccountOptions(a, a.dev_pipeline_architect_account_id || '')}</select>
            </div>
            <div class="config-field">
              <label>实现填充账号</label>
              <select id="edit_dev_pipeline_implementer_account_id">${renderDevPipelineAccountOptions(a, a.dev_pipeline_implementer_account_id || '')}</select>
            </div>
            <div class="config-field">
              <label>验收收口账号</label>
              <select id="edit_dev_pipeline_reviewer_account_id">${renderDevPipelineAccountOptions(a, a.dev_pipeline_reviewer_account_id || '')}</select>
            </div>
            <div class="config-field">
              <label>实现阶段工具能力</label>
              <select id="edit_dev_pipeline_tool_mode">
                <option value="controlled_tools" ${(a.dev_pipeline_tool_mode || 'controlled_tools') === 'controlled_tools' ? 'selected' : ''}>受控工具执行</option>
                <option value="patch_only" ${a.dev_pipeline_tool_mode === 'patch_only' ? 'selected' : ''}>仅生成补丁</option>
                <option value="full_agent" ${a.dev_pipeline_tool_mode === 'full_agent' ? 'selected' : ''}>完整 Codex 执行能力</option>
              </select>
              <span class="hint">当前版本会把能力模式注入阶段指令；工具执行仍继承 deecodex 的安全策略。</span>
            </div>
            <div class="config-field">
              <label>最大修正轮数</label>
              <input type="number" id="edit_dev_pipeline_max_iterations" value="${Number(a.dev_pipeline_max_iterations || 3)}" min="1" max="10" step="1">
            </div>
            <div class="config-field">
              <label class="toggle-label">
                <input type="checkbox" id="edit_dev_pipeline_show_trace" ${a.dev_pipeline_show_trace ? 'checked' : ''}>
                在最终回答中显示阶段摘要
              </label>
            </div>
            <div class="config-field">
              <label>方案角色附加指令</label>
              <textarea id="edit_dev_pipeline_architect_instruction" rows="2" placeholder="可选">${escAttr(a.dev_pipeline_architect_instruction || '')}</textarea>
            </div>
            <div class="config-field">
              <label>实现角色附加指令</label>
              <textarea id="edit_dev_pipeline_implementer_instruction" rows="2" placeholder="可选">${escAttr(a.dev_pipeline_implementer_instruction || '')}</textarea>
            </div>
            <div class="config-field">
              <label>验收角色附加指令</label>
              <textarea id="edit_dev_pipeline_reviewer_instruction" rows="2" placeholder="可选">${escAttr(a.dev_pipeline_reviewer_instruction || '')}</textarea>
            </div>
          </div>
        </div>
      </div>
    </section>

  <div class="collapsible-section">
    <button class="collapsible-toggle${Object.keys(customHeaders).length > 0 || requestTimeout || ep.path ? ' open' : ''}" onclick="this.classList.toggle('open');this.nextElementSibling.classList.toggle('open')">
      <span class="arrow">▸</span> 高级端点
    </button>
    <div class="collapsible-content${Object.keys(customHeaders).length > 0 || requestTimeout || ep.path ? ' open' : ''}">
      <div class="config-fields nested-fields">
        <div class="config-field">
          <label>请求路径 <span class="optional-label">可选</span></label>
          <input type="text" id="edit_endpoint_path" value="${escAttr(ep.path || '')}" placeholder="留空自动使用所选 API 类型">
          <span class="hint">私有代理或非标准网关才需要填写，例如 /v1/chat/completions。</span>
        </div>
        <div class="config-field">
          <label>自定义 HTTP 头 <span class="optional-label">可选</span></label>
          <textarea class="mono-textarea" id="edit_custom_headers" rows="3" placeholder="每行一个: Header-Name: value&#10;例: X-Org-Id: org-xxx&#10;例: X-Custom-Auth: token123">${escAttr(Object.entries(customHeaders).map(([k, v]) => k + ': ' + v).join('\n'))}</textarea>
          <span class="hint">每行一个头，格式: 头名称: 头值，将在每次上游请求时附加</span>
        </div>
        <div class="config-field">
          <label>请求超时（秒） <span class="optional-label">可选</span></label>
          <input type="number" id="edit_request_timeout" value="${requestTimeout || ''}" min="1" max="600" step="1" placeholder="留空使用默认 300s">
          <span class="hint">此账号的上游请求超时时间，Claude 扩展思考建议设 180-300</span>
        </div>
        <div class="config-field">
          <label>最大重试次数 <span class="optional-label">可选</span></label>
          <input type="number" id="edit_max_retries" value="${maxRetries ?? ''}" min="0" max="10" step="1" placeholder="留空使用默认 3 次">
          <span class="hint">上游请求失败（401/429/502/503/连接错误）时的重试次数，0 表示不重试</span>
        </div>
      </div>
    </div>
  </div>

  <div class="accounts-actions">
    <button class="btn btn-primary" onclick="saveAccount()">保存账号</button>
    ${a.id ? `<button class="btn btn-ghost" onclick="editConfigFile('${escAttr(a.id)}')">编辑配置文件</button>` : ''}
    <button class="btn btn-danger" onclick="deleteAccount('${escAttr(a.id)}')">删除账号</button>
  </div>
  </div>`;
}

function renderModelMappingRows(knownModels) {
  const a = editingAccount;
  if (!a) return '';
  const ep = currentEndpoint(a) || {};
  const map = ep.model_map || a.model_map || {};
  const rows = [];
  for (const codexModel of CODEX_MODEL_LIST) {
    const val = map[codexModel] || '';
    rows.push({ codexModel, val, readonly: true });
  }
  // 添加自定义映射（不在预定义列表中的）
  for (const [k, v] of Object.entries(map)) {
    if (!CODEX_MODEL_LIST.includes(k) && k) {
      rows.push({ codexModel: k, val: v, readonly: false });
    }
  }
  // 优先使用从上游获取的模型列表，否则用预设列表
  const modelSource = upstreamModels.length > 0 ? upstreamModels : (knownModels || []);
  const suggestionsJson = escAttr(JSON.stringify(modelSource));

  return rows.map((r, i) => {
    const labelExtra = r.readonly ? '' : ' (自定义)';
    const upstreamModel = r.val || r.codexModel;
    const profile = (ep.model_profiles || {})[upstreamModel] || {};
    const visionMode = profile.vision_mode || ep.vision?.mode || 'off';
    const removeControl = r.readonly
      ? '<span class="model-remove-placeholder"></span>'
      : '<button class="model-remove" onclick="removeModelMapRow(this)" title="移除">✕</button>';
    return `<div class="model-row">
      <div class="model-label codex">${esc(r.codexModel)}${labelExtra}</div>
      <div class="model-value">
        <div class="model-autocomplete">
          <input type="text" value="${escAttr(r.val)}" placeholder="未映射 (使用原名)"
            data-codex="${escAttr(r.codexModel)}" data-readonly="${r.readonly}"
            onchange="syncModelVisionTarget(this)"
            onfocus="showSuggestions(this)" oninput="filterSuggestions(this)" onblur="hideSuggestions(this)"
            autocomplete="off">
          <div class="model-suggestions" style="display:none;" data-suggestions="${suggestionsJson}"></div>
        </div>
        ${renderModelVisionSegments(upstreamModel, visionMode)}
        ${removeControl}
      </div>
    </div>`;
  }).join('');
}

function renderClientModelMappingRows(account) {
  const kind = accountClientKind(account);
  const slots = clientModelSlots(kind);
  const map = clientModelMap(account);
  const knownKeys = slots.map(slot => slot.key);
  const modelSource = upstreamModels.length > 0 ? upstreamModels : getProviderKnownModels(account.provider || '');
  const suggestionsJson = escAttr(JSON.stringify(modelSource || []));
  const rows = slots.map(slot => ({
    key: slot.key,
    label: slot.label || slot.key,
    target: slot.target || '',
    description: slot.description || '',
    required: Boolean(slot.required),
    value: map[slot.key] || (slot.key === 'default' ? (account.default_model || '') : ''),
    readonly: true,
  }));
  Object.entries(map).forEach(([key, value]) => {
    if (knownKeys.includes(key)) return;
    rows.push({ key, label: key, target: '自定义槽位', required: false, value, readonly: false });
  });
  return rows.map(row => {
    const slotCell = row.readonly
      ? `<div class="model-label codex client-model-slot">
          <strong>${esc(row.label)}${row.required ? ' *' : ''}</strong>
          <span title="${escAttr(row.description || row.target)}">${esc(row.target)}</span>
          <input type="hidden" class="client-model-slot-key" value="${escAttr(row.key)}">
        </div>`
      : `<div class="model-label client-model-slot custom">
          <input type="text" class="client-model-slot-key" value="${escAttr(row.key)}" placeholder="槽位名，如 rerank">
          <span>${esc(row.target)}</span>
        </div>`;
    return `<div class="model-row client-model-row client-model-template">
      ${slotCell}
      <div class="model-value client-model-value">
        <div class="model-autocomplete">
          <input type="text" class="client-model-value-input" value="${escAttr(row.value || '')}" placeholder="留空则不写入该槽位"
            onfocus="showSuggestions(this)" oninput="filterSuggestions(this)" onblur="hideSuggestions(this)" autocomplete="off">
          <div class="model-suggestions" style="display:none;" data-suggestions="${suggestionsJson}"></div>
        </div>
        ${row.readonly ? '<span class="model-remove-placeholder"></span>' : '<button class="model-remove" onclick="removeClientModelRow(this)" title="移除">✕</button>'}
      </div>
    </div>`;
  }).join('');
}

function addClientModelRow() {
  const container = document.getElementById('clientModelMapRows');
  if (!container) return;
  const modelSource = upstreamModels.length > 0 ? upstreamModels : getProviderKnownModels(editingAccount?.provider || '');
  const suggestionsJson = escAttr(JSON.stringify(modelSource || []));
  const row = document.createElement('div');
  row.className = 'model-row client-model-row client-model-template';
  row.innerHTML = `<div class="model-label client-model-slot custom">
      <input type="text" class="client-model-slot-key" placeholder="槽位名，如 rerank">
      <span>自定义槽位</span>
    </div>
    <div class="model-value client-model-value">
      <div class="model-autocomplete">
        <input type="text" class="client-model-value-input" placeholder="上游模型名"
          onfocus="showSuggestions(this)" oninput="filterSuggestions(this)" onblur="hideSuggestions(this)" autocomplete="off">
        <div class="model-suggestions" style="display:none;" data-suggestions="${suggestionsJson}"></div>
      </div>
      <button class="model-remove" onclick="removeClientModelRow(this)" title="移除">✕</button>
    </div>`;
  container.appendChild(row);
}

function removeClientModelRow(btn) {
  btn.closest('.client-model-row')?.remove();
}

function collectClientModelMap() {
  const result = {};
  document.querySelectorAll('#clientModelMapRows .client-model-row').forEach(row => {
    const key = row.querySelector('.client-model-slot-key')?.value?.trim();
    const value = row.querySelector('.client-model-value-input')?.value?.trim();
    if (key && value) result[key] = value;
  });
  return result;
}

function normalizedVisionMode(mode) {
  const value = String(mode || 'off').toLowerCase();
  return ['off', 'native', 'glue'].includes(value) ? value : 'off';
}

function parseOptionalInteger(value) {
  if (value === undefined || value === null || String(value).trim() === '') return null;
  const parsed = parseInt(value, 10);
  return Number.isFinite(parsed) ? parsed : null;
}

function renderModelVisionSegments(model, value) {
  const mode = normalizedVisionMode(value);
  const modelAttr = escAttr(model || '');
  const options = [
    ['off', '关闭'],
    ['native', '原生'],
    ['glue', '胶水'],
  ];
  return `<div class="model-vision-segments" data-model="${modelAttr}" data-mode="${mode}" role="group" aria-label="当前映射模型视觉能力">
    ${options.map(([value, label]) => `<button type="button" class="${mode === value ? 'active' : ''}" data-mode="${value}" onclick="setModelVisionMode(this, '${value}')">${label}</button>`).join('')}
  </div>`;
}

// ── 数据加载 ──

async function loadAccountsData() {
  try {
    const result = await invoke('list_accounts');
    accountsData = result;
    if (!clientProfiles.length) await loadClientProfiles();
    if (!providerPresets.length) await loadProviderPresets();
    if (!endpointTemplates.length) await loadEndpointTemplates();
    if (accountsView === 'list' && currentPanel === 'accounts') renderMainContent();
    // 异步加载余额（不阻塞渲染）
    if (accountsView === 'list') {
      (accountsData.accounts || [])
        .filter(a => accountClientKind(a) === selectedClientKind)
        .forEach(a => {
          if (isCodexAccount(a)) fetchBalanceForCard(a);
          else fetchClientStatusForCard(a);
        });
    }
  } catch (e) {
    showToast('加载账号失败: ' + e, 'error');
  }
}

async function loadClientProfiles() {
  try {
    clientProfiles = await invoke('get_client_profiles');
  } catch (e) {
    showToast('加载客户端分类失败: ' + e, 'error');
  }
}

async function loadProviderPresets() {
  try {
    providerPresets = await invoke('get_provider_presets');
  } catch (e) {
    showToast('加载供应商列表失败: ' + e, 'error');
  }
}

async function loadEndpointTemplates() {
  try {
    endpointTemplates = await invoke('get_endpoint_templates');
  } catch (e) {
    showToast('加载协议模板失败: ' + e, 'error');
  }
}

function getProviderKnownModels(provider) {
  const p = providerPresets.find(pp => pp.slug === provider);
  return p ? p.known_models : [];
}

function defaultApiKeyEnvForClient(account) {
  const kind = accountClientKind(account);
  if (kind === 'claude_code' && account.provider === 'deepseek') return 'ANTHROPIC_AUTH_TOKEN';
  if (kind === 'claude_code' && account.provider === 'kimi') return 'ANTHROPIC_AUTH_TOKEN';
  if (kind === 'claude_code' && account.provider === 'minimax') return 'ANTHROPIC_API_KEY';
  if (kind === 'claude_code' && account.provider === 'mimo') return 'ANTHROPIC_API_KEY';
  if (kind === 'claude_code' && account.provider === 'longcat') return 'ANTHROPIC_AUTH_TOKEN';
  if (kind === 'claude_code' && account.provider === 'qwen') return 'ANTHROPIC_API_KEY';
  if (kind === 'claude_code' && account.provider === 'glm') return 'ANTHROPIC_AUTH_TOKEN';
  if (kind === 'claude_code' || account.provider === 'anthropic') return 'ANTHROPIC_API_KEY';
  if (account.provider === 'deepseek') return 'DEEPSEEK_API_KEY';
  if (account.provider === 'openrouter') return 'OPENROUTER_API_KEY';
  if (account.provider === 'minimax') return 'MINIMAX_API_KEY';
  if (account.provider === 'mimo') return 'MIMO_API_KEY';
  if (account.provider === 'longcat') return 'LONGCAT_API_KEY';
  if (account.provider === 'qwen') return 'DASHSCOPE_API_KEY';
  if (kind === 'generic_client' || account.provider === 'openai') return 'OPENAI_API_KEY';
  return 'OPENAI_API_KEY';
}

function updateClientProviderDefaults() {
  const provider = document.getElementById('edit_client_provider')?.value || '';
  const preset = getProviderPreset(provider);
  if (!preset) return;
  const defaults = clientProviderDefaults(accountClientKind(editingAccount), provider, preset);
  const upstream = document.getElementById('edit_upstream');
  const model = document.getElementById('edit_default_model');
  const env = document.getElementById('edit_client_api_key_env');
  const authEnv = document.getElementById('edit_client_auth_env');
  const hasClientPreset = accountClientKind(editingAccount) === 'claude_code'
    && ['deepseek', 'kimi', 'minimax', 'mimo', 'longcat', 'qwen', 'glm'].includes(provider);
  if (upstream && (!upstream.value.trim() || hasClientPreset)) upstream.value = defaults.upstream || '';
  if (model && (!model.value.trim() || hasClientPreset) && defaults.default_model) {
    model.value = defaults.default_model;
  }
  if (env && (!env.value.trim() || env.value === 'OPENAI_API_KEY' || hasClientPreset)) {
    env.value = defaults.api_key_env;
  }
  if (authEnv) authEnv.value = defaults.api_key_env;
  if (editingAccount && !isCodexAccount(editingAccount)) {
    upstreamModels = [];
    syncEditingDraftFromForm();
    const clientMap = clientModelMap(editingAccount);
    const defaultModel = model?.value?.trim() || editingAccount.default_model || '';
    if (defaultModel && !clientMap.default) {
      editingAccount.client_options = editingAccount.client_options || {};
      editingAccount.client_options.model_map = { ...clientMap, default: defaultModel };
      editingAccount.default_model = defaultModel;
    }
    const rows = document.getElementById('clientModelMapRows');
    if (rows) rows.innerHTML = renderClientModelMappingRows(editingAccount);
  }
}

// ── 模型映射编辑辅助 ──

function removeModelMapRow(btn) {
  const row = btn.closest('.model-row');
  const input = row.querySelector('input');
  if (input) {
    const codex = input.dataset.codex;
    if (editingAccount && editingAccount.model_map) {
      delete editingAccount.model_map[codex];
    }
  }
  row.remove();
}

function syncEditingDraftFromForm() {
  if (!editingAccount) return;
  const a = editingAccount;
  const nameInput = document.getElementById('edit_name');
  if (nameInput) a.name = nameInput.value.trim() || a.name;
  const keyInput = document.getElementById('edit_api_key');
  if (keyInput) a.api_key = keyInput.value.trim();

  if (!isCodexAccount(a)) {
    const provider = document.getElementById('edit_client_provider');
    if (provider) a.provider = provider.value || a.provider;
    const upstream = document.getElementById('edit_upstream');
    if (upstream) a.upstream = upstream.value.trim();
    const defaultModel = document.getElementById('edit_default_model');
    if (defaultModel) a.default_model = defaultModel.value.trim();
    if (!a.client_options) a.client_options = {};
    const clientModels = collectClientModelMap();
    if (Object.keys(clientModels).length) {
      if (!clientModels.default && a.default_model) clientModels.default = a.default_model;
      a.client_options.model_map = clientModels;
      a.default_model = clientModels.default || a.default_model;
      if (defaultModel) defaultModel.value = a.default_model;
    } else {
      delete a.client_options.model_map;
    }
    const configPath = document.getElementById('edit_client_config_path');
    if (configPath) {
      const value = configPath.value.trim();
      if (value) a.client_options.config_path = value;
      else delete a.client_options.config_path;
    }
    const apiKeyEnv = document.getElementById('edit_client_api_key_env');
    if (apiKeyEnv) {
      const value = apiKeyEnv.value.trim();
      if (value) a.client_options.api_key_env = value;
      else delete a.client_options.api_key_env;
    }
    const authEnv = document.getElementById('edit_client_auth_env');
    if (authEnv) {
      a.client_options.auth_env = authEnv.value || a.client_options.api_key_env || 'ANTHROPIC_API_KEY';
      a.client_options.api_key_env = a.client_options.auth_env;
    }
    const proxyToggle = document.getElementById('edit_client_proxy_toggle');
    if (proxyToggle) {
      a.client_options.proxy_recording_enabled = proxyToggle.classList.contains('on');
    }
    a.translate_enabled = false;
    a.endpoints = [];
    return;
  }

  const endpoints = ensureAccountEndpoints(a);
  const ep = currentEndpoint(a) || endpoints[0];
  if (!ep) return;

  const endpointName = document.getElementById('edit_endpoint_name');
  if (endpointName) ep.name = endpointName.value.trim() || ep.name || '默认配置';
  const endpointKind = document.getElementById('edit_endpoint_kind');
  if (endpointKind) ep.kind = endpointKind.value;
  const upstream = document.getElementById('edit_upstream');
  if (upstream) {
    ep.base_url = upstream.value.trim();
    a.upstream = ep.base_url || a.upstream;
  }
  const endpointPath = document.getElementById('edit_endpoint_path');
  if (endpointPath) ep.path = endpointPath.value.trim();
  const balanceUrl = document.getElementById('edit_balance_url');
  if (balanceUrl) {
    ep.balance_url = balanceUrl.value.trim();
    a.balance_url = ep.balance_url;
  }

  if (!ep.vision) ep.vision = {};
  const visionMode = document.getElementById('edit_vision_mode');
  if (visionMode) {
    ep.vision.mode = visionMode.value || 'off';
    a.vision_enabled = ep.vision.mode === 'glue';
  }
  const visionUpstream = document.getElementById('edit_vision_upstream');
  if (visionUpstream) { ep.vision.base_url = visionUpstream.value.trim(); a.vision_upstream = ep.vision.base_url; }
  const visionKey = document.getElementById('edit_vision_api_key');
  if (visionKey) { ep.vision.api_key = visionKey.value.trim(); a.vision_api_key = ep.vision.api_key; }
  const visionModel = document.getElementById('edit_vision_model');
  if (visionModel) { ep.vision.model = visionModel.value.trim(); a.vision_model = ep.vision.model; }
  const visionPath = document.getElementById('edit_vision_endpoint');
  if (visionPath) { ep.vision.path = visionPath.value.trim() || 'v1/coding_plan/vlm'; a.vision_endpoint = ep.vision.path; }
  ep.vision.adapter_id = document.getElementById('edit_vision_adapter')?.value || ep.vision.adapter_id || 'minimax_coding_plan_vlm';
  ep.vision.glue_strategy = document.getElementById('edit_glue_strategy')?.value || ep.vision.glue_strategy || 'final_answer';
  ep.vision.unsupported_image_policy = document.getElementById('edit_unsupported_image_policy')?.value || ep.vision.unsupported_image_policy || 'reject';

  ep.model_map = collectModelMap();
  ep.model_profiles = collectModelProfiles();
  a.model_map = ep.model_map;

  const cwEnabled = document.getElementById('edit_cw_enabled');
  if (cwEnabled) {
    ep.context_window_override = cwEnabled.checked
      ? parseOptionalInteger(document.getElementById('edit_cw_size')?.value)
      : null;
    a.context_window_override = ep.context_window_override;
  }

  const reasoningEnabled = document.getElementById('edit_reasoning_enabled');
  if (reasoningEnabled) {
    ep.reasoning_effort_override = reasoningEnabled.checked
      ? (document.getElementById('edit_reasoning_effort')?.value || null)
      : null;
    ep.thinking_tokens = reasoningEnabled.checked
      ? parseOptionalInteger(document.getElementById('edit_thinking_tokens')?.value)
      : null;
    a.reasoning_effort_override = ep.reasoning_effort_override;
    a.thinking_tokens = ep.thinking_tokens;
  }

  const fastEnabled = document.getElementById('edit_fast_enabled');
  if (fastEnabled) {
    ep.fast_mode_enabled = fastEnabled.checked;
    ep.fast_service_tier = fastEnabled.checked
      ? (document.getElementById('edit_fast_service_tier')?.value.trim() || 'priority')
      : 'priority';
  }

  const headersText = document.getElementById('edit_custom_headers');
  if (headersText) {
    ep.custom_headers = {};
    const raw = headersText.value.trim();
    if (raw) {
      raw.split('\n').forEach(line => {
        const colonIdx = line.indexOf(':');
        if (colonIdx > 0) {
          const k = line.substring(0, colonIdx).trim();
          const v = line.substring(colonIdx + 1).trim();
          if (k && v) ep.custom_headers[k] = v;
        }
      });
    }
    a.custom_headers = ep.custom_headers;
  }

  const timeoutInput = document.getElementById('edit_request_timeout');
  if (timeoutInput) {
    ep.request_timeout_secs = parseOptionalInteger(timeoutInput.value);
    a.request_timeout_secs = ep.request_timeout_secs;
  }
  const retriesInput = document.getElementById('edit_max_retries');
  if (retriesInput) {
    ep.max_retries = parseOptionalInteger(retriesInput.value);
    a.max_retries = ep.max_retries;
  }
  a.translate_enabled = ep.kind === 'open_ai_chat' || ep.kind === 'custom_chat';
}

function showSuggestions(input) {
  const wrap = input.closest('.model-autocomplete');
  if (!wrap) return;
  const dropdown = wrap.querySelector('.model-suggestions');
  if (!dropdown) return;
  const suggestions = JSON.parse(dropdown.dataset.suggestions || '[]');
  renderSuggestionsDropdown(input, dropdown, suggestions);
  dropdown.style.display = 'block';
}

function filterSuggestions(input) {
  const wrap = input.closest('.model-autocomplete');
  if (!wrap) return;
  const dropdown = wrap.querySelector('.model-suggestions');
  if (!dropdown) return;
  const suggestions = JSON.parse(dropdown.dataset.suggestions || '[]');
  const q = input.value.toLowerCase();
  const filtered = suggestions.filter(s => s.toLowerCase().includes(q));
  renderSuggestionsDropdown(input, dropdown, filtered);
  dropdown.style.display = 'block';
}

function hideSuggestions(input) {
  setTimeout(() => {
    const wrap = input.closest('.model-autocomplete');
    if (!wrap) return;
    const dropdown = wrap.querySelector('.model-suggestions');
    if (dropdown) dropdown.style.display = 'none';
  }, 150);
}

function renderSuggestionsDropdown(input, dropdown, items) {
  if (items.length === 0) { dropdown.innerHTML = ''; return; }
  dropdown.innerHTML = items.map(s =>
    `<div class="suggestion-item" onmousedown="event.preventDefault();selectSuggestion(this)">${esc(s)}</div>`
  ).join('');
}

function selectSuggestion(item) {
  const wrap = item.closest('.model-autocomplete');
  if (!wrap) return;
  const input = wrap.querySelector('input');
  if (input) {
    input.value = item.textContent;
    syncModelVisionTarget(input);
    input.focus();
  }
  const dropdown = wrap.querySelector('.model-suggestions');
  if (dropdown) dropdown.style.display = 'none';
}

function addModelRow(containerId, knownModelsJson) {
  const knownModels = JSON.parse(knownModelsJson);
  const suggestionsJson = escAttr(JSON.stringify(knownModels));

  const container = document.getElementById(containerId);
  const row = document.createElement('div');
  row.className = 'model-row';
  row.innerHTML = `<div class="model-label codex"><input type="text" class="custom-codex-model" placeholder="Codex 模型名"></div>
    <div class="model-value">
      <div class="model-autocomplete">
        <input type="text" placeholder="上游模型名" autocomplete="off"
          onchange="syncModelVisionTarget(this)"
          onfocus="showSuggestions(this)" oninput="filterSuggestions(this)" onblur="hideSuggestions(this)">
        <div class="model-suggestions" style="display:none;" data-suggestions="${suggestionsJson}"></div>
      </div>
      ${renderModelVisionSegments('', document.getElementById('edit_vision_mode')?.value || 'off')}
      <button class="model-remove" onclick="removeModelMapRow(this)" title="移除">✕</button>
    </div>`;
  container.appendChild(row);
}

function setModelVisionMode(button, mode) {
  const wrap = button.closest('.model-vision-segments');
  if (!wrap) return;
  wrap.querySelectorAll('button').forEach(item => {
    item.classList.toggle('active', item === button);
  });
  wrap.dataset.mode = normalizedVisionMode(mode);
}

function syncModelVisionTarget(input) {
  const row = input.closest('.model-row');
  const wrap = row?.querySelector('.model-vision-segments');
  if (!wrap) return;
  wrap.dataset.model = input.value.trim();
}

function collectModelMap() {
  const result = {};
  const rows = document.querySelectorAll('#modelMapRows .model-row');
  rows.forEach(row => {
    const inputs = row.querySelectorAll('input[type="text"]');
    let codex, upstream;
    if (inputs.length === 2) {
      // 自定义行：左输入框是 codex 模型名，右是上游
      codex = inputs[0].value.trim();
      upstream = inputs[1].value.trim();
    } else if (inputs.length === 1) {
      codex = inputs[0].dataset.codex;
      upstream = inputs[0].value.trim();
    }
    if (codex && upstream) result[codex] = upstream;
  });
  return result;
}

function collectModelProfiles() {
  const result = {};
  const rows = document.querySelectorAll('#modelMapRows .model-row');
  rows.forEach(row => {
    const inputs = row.querySelectorAll('input[type="text"]');
    const upstreamInput = inputs.length === 2 ? inputs[1] : inputs[0];
    const model = upstreamInput?.value?.trim() || upstreamInput?.dataset?.codex || row.querySelector('.custom-codex-model')?.value?.trim();
    const mode = row.querySelector('.model-vision-segments')?.dataset.mode || 'off';
    if (model) {
      result[model] = { vision_mode: mode };
    }
  });
  return result;
}

// ── CRUD 操作 ──

async function switchAccount(id) {
  try {
    await invoke('switch_account', { id });
    accountsData.active_id = id;
    showToast('已切换活跃账号', 'success');
    await loadAccountsData();
  } catch (e) {
    showToast('切换账号失败: ' + e, 'error');
  }
}

function addAccount(provider, clientKind) {
  const kind = normalizeClientKind(clientKind || selectedClientKind || 'codex');
  const preset = providerPresets.find(p => p.slug === provider);
  if (!preset) return;
  const clientProfile = getClientProfile(kind);
  const defaults = clientProviderDefaults(kind, provider, preset);
  editingAccount = {
    id: '',
    name: kind === 'codex' ? preset.label + ' 账号' : `${clientProfile?.label || CLIENT_KIND_LABELS[kind]} · ${preset.label}`,
    provider: provider,
    client_kind: kind,
    target: kind,
    upstream: kind === 'codex' ? preset.default_upstream : defaults.upstream,
    api_key: '',
    default_model: kind === 'codex' ? '' : (defaults.default_model || clientProfile?.default_model || ''),
    client_options: kind === 'codex' ? {} : {
      api_key_env: defaults.api_key_env,
      ...(kind === 'claude_code' ? { auth_env: defaults.api_key_env } : {}),
      model_map: {
        default: defaults.default_model || clientProfile?.default_model || '',
      },
    },
    last_applied_at: null,
    last_check: null,
    model_map: {},
    vision_enabled: false,
    vision_upstream: '',
    vision_api_key: '',
    vision_model: '',
    vision_endpoint: '',
    from_codex_config: false,
    balance_url: '',
    context_window_override: null,
    reasoning_effort_override: null,
    thinking_tokens: null,
    custom_headers: {},
    provider_options: preset.provider_options || { capability_labels: preset.capability_labels || [] },
    request_timeout_secs: null,
    max_retries: null,
    translate_enabled: true,
    capability_enabled: false,
    capability_account_id: null,
    dev_pipeline_enabled: false,
    dev_pipeline_trigger_mode: 'manual',
    dev_pipeline_command: '/dev-pipeline',
    dev_pipeline_architect_account_id: null,
    dev_pipeline_implementer_account_id: null,
    dev_pipeline_reviewer_account_id: null,
    dev_pipeline_tool_mode: 'controlled_tools',
    dev_pipeline_max_iterations: 3,
    dev_pipeline_show_trace: false,
    dev_pipeline_architect_instruction: '',
    dev_pipeline_implementer_instruction: '',
    dev_pipeline_reviewer_instruction: '',
  };
  if (kind === 'codex') {
    editingAccount.endpoints = [createEndpointFromTemplate(providerDefaultTemplate(provider), editingAccount)];
    editingAccount._editing_endpoint_id = editingAccount.endpoints[0].id;
  } else {
    editingAccount.translate_enabled = false;
    editingAccount.endpoints = [];
  }
  accountsView = 'edit';
  renderMainContent();
}

function editAccount(id) {
  editingAccount = accountsData.accounts.find(a => a.id === id);
  if (!editingAccount) {
    showToast('账号不存在', 'error');
    return;
  }
  // 深拷贝避免直接修改原数据
  editingAccount = JSON.parse(JSON.stringify(editingAccount));
  accountsView = 'edit';
  renderMainContent();
}

function serializeAccountForBackend(account) {
  const payload = JSON.parse(JSON.stringify(account || {}));
  if (Object.prototype.hasOwnProperty.call(payload, 'client_kind')) {
    delete payload.target;
  }
  delete payload._editing_endpoint_id;
  return JSON.stringify(payload);
}

async function saveAccount(options = {}) {
  if (!editingAccount) return;
  if (options instanceof Event) options = {};
  const a = editingAccount;
  const isNewAccount = !a.id;

  syncEditingDraftFromForm();
  const preset = getProviderPreset(a.provider);
  if (!a.provider_options || Object.keys(a.provider_options).length === 0) {
    a.provider_options = { capability_labels: preset ? (preset.capability_labels || []) : [] };
  }
  const capabilityEnabled = document.getElementById('edit_capability_enabled');
  if (capabilityEnabled) a.capability_enabled = capabilityEnabled.checked;
  const capabilityAccount = document.getElementById('edit_capability_account_id');
  a.capability_account_id = capabilityEnabled && capabilityEnabled.checked && capabilityAccount
    ? (capabilityAccount.value || null)
    : null;
  if (a.capability_enabled) {
    if (!a.capability_account_id) {
      showToast('请选择能力补全账号', 'error');
      return;
    }
    if (a.id && a.capability_account_id === a.id) {
      showToast('能力补全账号不能选择当前账号', 'error');
      return;
    }
  }
  const devPipelineEnabled = document.getElementById('edit_dev_pipeline_enabled');
  if (devPipelineEnabled) a.dev_pipeline_enabled = devPipelineEnabled.checked;
  const devTriggerMode = document.getElementById('edit_dev_pipeline_trigger_mode');
  a.dev_pipeline_trigger_mode = devTriggerMode ? devTriggerMode.value : (a.dev_pipeline_trigger_mode || 'manual');
  const devCommand = document.getElementById('edit_dev_pipeline_command');
  a.dev_pipeline_command = devCommand ? (devCommand.value.trim() || '/dev-pipeline') : (a.dev_pipeline_command || '/dev-pipeline');
  a.dev_pipeline_architect_account_id = document.getElementById('edit_dev_pipeline_architect_account_id')?.value || null;
  a.dev_pipeline_implementer_account_id = document.getElementById('edit_dev_pipeline_implementer_account_id')?.value || null;
  a.dev_pipeline_reviewer_account_id = document.getElementById('edit_dev_pipeline_reviewer_account_id')?.value || null;
  a.dev_pipeline_tool_mode = document.getElementById('edit_dev_pipeline_tool_mode')?.value || (a.dev_pipeline_tool_mode || 'controlled_tools');
  a.dev_pipeline_max_iterations = Math.max(1, Math.min(10, Number(document.getElementById('edit_dev_pipeline_max_iterations')?.value || a.dev_pipeline_max_iterations || 3)));
  a.dev_pipeline_show_trace = Boolean(document.getElementById('edit_dev_pipeline_show_trace')?.checked);
  a.dev_pipeline_architect_instruction = document.getElementById('edit_dev_pipeline_architect_instruction')?.value || '';
  a.dev_pipeline_implementer_instruction = document.getElementById('edit_dev_pipeline_implementer_instruction')?.value || '';
  a.dev_pipeline_reviewer_instruction = document.getElementById('edit_dev_pipeline_reviewer_instruction')?.value || '';
  if (a.dev_pipeline_enabled && a.dev_pipeline_trigger_mode === 'manual' && !a.dev_pipeline_command.trim()) {
    showToast('开发协作编排触发命令不能为空', 'error');
    return;
  }
  const ep = currentEndpoint(a);
  if (!isCodexAccount(a)) {
    a.translate_enabled = false;
    a.endpoints = [];
    a.model_map = {};
    a.vision_enabled = false;
  } else if (ep) {
    ep.name = endpointKindLabel(ep.kind);
    a.upstream = ep.base_url || a.upstream;
    a.balance_url = ep.balance_url || '';
    a.model_map = ep.model_map || {};
    a.context_window_override = ep.context_window_override ?? null;
    a.reasoning_effort_override = ep.reasoning_effort_override ?? null;
    a.thinking_tokens = ep.thinking_tokens ?? null;
    a.custom_headers = ep.custom_headers || {};
    a.request_timeout_secs = ep.request_timeout_secs ?? null;
    a.max_retries = ep.max_retries ?? null;
    a.translate_enabled = ep.kind === 'open_ai_chat' || ep.kind === 'custom_chat';
    a._editing_endpoint_id = ep.id;
  }
  const editingEndpointId = isCodexAccount(a) ? (ep?.id || selectedEndpointId(a)) : null;
  a.updated_at = Math.floor(Date.now() / 1000);

  try {
    let result;
    if (isNewAccount) {
      // 新账号
      result = await invoke('add_account', { provider: a.provider || 'custom', accountJson: serializeAccountForBackend(a) });
      if (!options.silent) showToast('账号已创建', 'success');
    } else {
      result = await invoke('update_account', { accountJson: serializeAccountForBackend(a) });
      if (!options.silent) showToast('账号已保存', 'success');
    }
    await loadAccountsData();
    editingAccount = accountsData.accounts.find(ac => ac.id === result.id);
    if (editingAccount) {
      editingAccount = JSON.parse(JSON.stringify(editingAccount));
      editingAccount._editing_endpoint_id = editingEndpointId;
    }
    if (isNewAccount && !options.stay) {
      accountsView = 'list';
      editingAccount = null;
    }
    if (!options.stay) renderMainContent();
    return result;
  } catch (e) {
    if (!options.silent) showToast('保存账号失败: ' + e, 'error');
    if (options.silent) throw e;
    return null;
  }
}

async function deleteAccount(id) {
  if (!await showConfirm('确定要删除此账号吗？')) return;
  try {
    await invoke('delete_account', { id });
    showToast('账号已删除', 'success');
    accountsView = 'list'; editingAccount = null;
    await loadAccountsData();
  } catch (e) {
    showToast('删除账号失败: ' + e, 'error');
  }
}

async function importFromCodex() {
  try {
    await invoke('import_codex_config');
    showToast('Codex 配置导入成功', 'success');
    await loadAccountsData();
  } catch (e) {
    showToast('导入失败: ' + e, 'error');
  }
}

async function scanClientAccounts() {
  try {
    const result = await invoke('import_client_accounts');
    showToast(result.message || '客户端扫描完成', 'success');
    await loadAccountsData();
    if (Number(result.imported || 0) > 0) {
      const imported = Array.isArray(result.accounts) ? result.accounts[0] : null;
      if (imported) selectedClientKind = accountClientKind(imported);
      accountsView = 'list';
      if (currentPanel === 'accounts') renderMainContent();
      else if (currentPanel === 'config') renderPanel('config');
    }
  } catch (e) {
    showToast('客户端扫描失败: ' + e, 'error');
  }
}

function renderClientReport(report) {
  if (!report) return '';
  const diagnostics = Array.isArray(report.diagnostics) ? report.diagnostics : [];
  const diff = Array.isArray(report.diff) ? report.diff : [];
  const changedFiles = Array.isArray(report.changed_files) ? report.changed_files : [];
  const backupPaths = Array.isArray(report.backup_paths) ? report.backup_paths : (report.backup_path ? [report.backup_path] : []);
  const levelClass = report.ok ? 'status-ok' : 'status-error';
  const envPath = report.env_path ? `<div class="client-report-path">${esc(report.env_path)}</div>` : '';
  const meta = [
    report.risk_level ? `风险 ${report.risk_level}` : '',
    report.schema_ok === false ? 'Schema 异常' : 'Schema 正常',
    report.recoverable === false ? '不可自动恢复' : '可恢复',
    report.secret_source ? `密钥: ${report.secret_source}` : '',
  ].filter(Boolean);
  return `<div class="client-report">
    <div class="${levelClass}">${esc(report.message || (report.ok ? '检查通过' : '检查失败'))}</div>
    ${meta.length ? `<div class="client-report-meta">${meta.map(item => `<span>${esc(item)}</span>`).join('')}</div>` : ''}
    ${report.config_path ? `<div class="client-report-path">${esc(report.config_path)}</div>` : ''}
    ${envPath}
    ${changedFiles.length ? `<div class="client-report-files"><strong>变更文件</strong>${changedFiles.map(path => `<span title="${escAttr(path)}">${esc(trunc(path, 72))}</span>`).join('')}</div>` : ''}
    ${backupPaths.length ? `<div class="client-report-files"><strong>备份</strong>${backupPaths.map(path => `<span title="${escAttr(path)}">${esc(trunc(path, 72))}</span>`).join('')}</div>` : ''}
    ${diagnostics.length ? `<div class="client-report-list">${diagnostics.map(d => `<div class="client-report-item ${escAttr(d.level || 'info')}">${esc(d.message || d.code || '')}</div>`).join('')}</div>` : ''}
    ${diff.length ? `<div class="client-report-diff">${diff.map(line => `<code>${esc(line)}</code>`).join('')}</div>` : ''}
  </div>`;
}

function showClientApplyConfirm(report) {
  return new Promise(resolve => {
    const existing = document.getElementById('clientApplyConfirmModal');
    if (existing) existing.remove();
    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';
    overlay.id = 'clientApplyConfirmModal';
    overlay.innerHTML = `<div class="modal-box client-apply-modal">
      <div class="modal-header">
        <h3>写入前预检</h3>
        <button class="modal-close" id="clientApplyCloseBtn" type="button">✕</button>
      </div>
      <div class="modal-body">
        <div class="client-apply-modal-intro ${report.ok ? 'status-ok' : 'status-error'}">
          ${esc(report.ok ? '预检通过，请确认脱敏 diff 后写入外部客户端配置。' : '预检未通过，已阻止写入外部客户端配置。')}
        </div>
        ${renderClientReport(report)}
      </div>
      <div class="client-apply-modal-actions">
        <button class="btn btn-primary" id="clientApplyOkBtn" type="button" ${report.ok ? '' : 'disabled'}>确认写入</button>
        <button class="btn btn-ghost" id="clientApplyCancelBtn" type="button">取消</button>
      </div>
    </div>`;
    document.body.appendChild(overlay);
    function cleanup(value) {
      overlay.remove();
      resolve(value);
    }
    document.getElementById('clientApplyCloseBtn').onclick = () => cleanup(false);
    document.getElementById('clientApplyCancelBtn').onclick = () => cleanup(false);
    document.getElementById('clientApplyOkBtn').onclick = () => cleanup(Boolean(report.ok));
    overlay.addEventListener('click', e => {
      if (e.target === overlay) cleanup(false);
    });
  });
}

async function fetchClientEventsForDetail(id) {
  const el = document.getElementById('clientEventLog');
  if (!el || !id) return;
  el.innerHTML = '<span class="status-muted">加载中...</span>';
  try {
    const events = await invoke('get_account_events', { accountId: id, limit: 12 });
    el.innerHTML = renderClientEventLog(Array.isArray(events) ? events : []);
  } catch (e) {
    el.innerHTML = `<span class="status-error">${esc(String(e))}</span>`;
  }
}

function renderClientEventLog(events) {
  if (!events.length) return '<span class="status-muted">暂无配置事件</span>';
  return events.map(event => {
    const ok = event.ok !== false;
    const action = accountEventActionLabel(event.action);
    const time = formatTimeShort(event.ts);
    const message = event.message || '';
    const details = event.details || {};
    const backupPaths = Array.isArray(details.backup_paths) ? details.backup_paths : [];
    const path = details.source_path || details.config_path || details.backup_path || backupPaths[0] || '';
    const source = path ? `<span title="${escAttr(path)}">${esc(trunc(path, 52))}</span>` : '';
    return `<div class="client-event-item ${ok ? 'ok' : 'error'}">
      <div class="client-event-dot"></div>
      <div class="client-event-main">
        <div class="client-event-line">
          <strong>${esc(action)}</strong>
          <time>${esc(time)}</time>
        </div>
        <div class="client-event-message">${esc(message)}</div>
        ${source ? `<div class="client-event-source">${source}</div>` : ''}
      </div>
    </div>`;
  }).join('');
}

function accountEventActionLabel(action) {
  const labels = {
    client_account_import: '导入账号',
    client_account_status: '状态刷新',
    client_account_dry_run: '配置预检',
    client_account_apply: '写入配置',
    client_account_restore: '恢复备份',
    client_config_open: '编辑配置',
    client_config_save: '保存配置',
  };
  return labels[action] || action || '配置事件';
}

async function fetchClientBackupsForDetail(id) {
  const el = document.getElementById('clientBackupList');
  if (!el || !id) return;
  el.innerHTML = '<span class="status-muted">加载中...</span>';
  try {
    const backups = await invoke('list_client_backups', { accountId: id });
    el.innerHTML = renderClientBackupList(id, Array.isArray(backups) ? backups : []);
  } catch (e) {
    el.innerHTML = `<span class="status-error">${esc(String(e))}</span>`;
  }
}

function renderClientBackupList(id, backups) {
  if (!backups.length) return '<span class="status-muted">暂无备份</span>';
  return backups.slice(0, 12).map(backup => {
    const time = formatTimeShort(backup.created_at);
    const sizeKb = Math.max(1, Math.round(Number(backup.size || 0) / 1024));
    return `<div class="client-backup-item">
      <div class="client-backup-main">
        <strong>${esc(backup.kind || 'config')}</strong>
        <span title="${escAttr(backup.path)}">${esc(trunc(backup.path || '', 68))}</span>
        <em>${esc(time)} · ${sizeKb} KB</em>
      </div>
      <button class="btn btn-ghost btn-small" onclick="restoreClientBackup('${escAttr(id)}', '${escAttr(backup.path || '')}')">恢复</button>
    </div>`;
  }).join('');
}

async function restoreClientBackup(id, backupPath) {
  if (!backupPath) return;
  if (!await showConfirm('确定恢复这个客户端配置备份吗？当前配置会先再次备份。')) return;
  try {
    const report = await invoke('restore_client_backup', { accountId: id, backupPath });
    showToast(report.ok ? '客户端备份已恢复' : '恢复后仍有诊断问题', report.ok ? 'success' : 'error');
    await loadAccountsData();
    fetchClientEventsForDetail(id);
    fetchClientBackupsForDetail(id);
  } catch (e) {
    showToast('恢复客户端备份失败: ' + e, 'error');
  }
}

function refreshClientSwitcherIssues() {
  const switcher = document.querySelector('.client-switcher');
  if (switcher) switcher.outerHTML = renderClientSwitcher(accountsData.accounts || []);
}

async function dryRunEditingClientAccount() {
  if (!editingAccount) return;
  syncEditingDraftFromForm();
  const statusEl = document.getElementById('clientDryRunStatus');
  const preview = document.getElementById('clientApplyPreview');
  if (statusEl) statusEl.innerHTML = '<span class="status-muted">预检中...</span>';
  try {
    const report = await invoke('test_client_account', { accountJson: serializeAccountForBackend(editingAccount) });
    if (statusEl) statusEl.innerHTML = report.ok ? '<span class="status-ok">预检通过</span>' : '<span class="status-error">预检有问题</span>';
    if (preview) preview.innerHTML = renderClientReport(report);
    if (editingAccount.id) fetchClientEventsForDetail(editingAccount.id);
  } catch (e) {
    if (statusEl) statusEl.innerHTML = '<span class="status-error">预检失败</span>';
    if (preview) preview.innerHTML = `<div class="status-error">${esc(String(e))}</div>`;
  }
}

async function dryRunClientAccount(id) {
  try {
    const report = await invoke('test_client_account', { accountId: id });
    showToast(report.ok ? '客户端预检通过' : '客户端预检发现问题', report.ok ? 'success' : 'error');
    const account = (accountsData.accounts || []).find(item => item.id === id);
    if (account) account._client_status_report = report;
    const el = document.getElementById('client-status-' + id);
    if (el) el.innerHTML = renderClientStatusSummary(report);
    refreshClientSwitcherIssues();
  } catch (e) {
    showToast('客户端预检失败: ' + e, 'error');
  }
}

async function refreshClientAccountStatus(id) {
  try {
    const report = await invoke('refresh_client_status', { accountId: id });
    showToast(report.ok ? '客户端状态正常' : '客户端状态有问题', report.ok ? 'success' : 'error');
    const account = (accountsData.accounts || []).find(item => item.id === id);
    if (account) {
      account._client_status_report = report;
      account.last_check = { ok: report.ok, message: report.message, details: report };
    }
    const el = document.getElementById('client-status-' + id);
    if (el) el.innerHTML = renderClientStatusSummary(report);
    const preview = document.getElementById('clientApplyPreview');
    if (preview) preview.innerHTML = renderClientReport(report);
    refreshClientSwitcherIssues();
    fetchClientEventsForDetail(id);
  } catch (e) {
    showToast('刷新客户端状态失败: ' + e, 'error');
  }
}

async function openClientConfig(id) {
  try {
    const result = await invoke('open_client_config', { accountId: id });
    showToast('已交给系统打开: ' + trunc(result.path || '', 64), 'success');
    fetchClientEventsForDetail(id);
  } catch (e) {
    showToast('打开配置文件失败: ' + e, 'error');
  }
}

async function editConfigFile(id) {
  if (!id) return;
  try {
    const file = await invoke('get_account_config_file', { accountId: id });
    showConfigEditorModal(id, file);
  } catch (e) {
    showToast('读取配置文件失败: ' + e, 'error');
  }
}

function showConfigEditorModal(id, file) {
  const existing = document.getElementById('configEditorModal');
  if (existing) existing.remove();
  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay';
  overlay.id = 'configEditorModal';
  const statusText = file.exists ? `已加载 · ${Number(file.size_bytes || 0)} bytes` : '文件不存在，将按当前账号生成初始内容';
  overlay.innerHTML = `<div class="modal-box config-editor-modal">
    <div class="modal-header">
      <h3>编辑配置文件</h3>
      <button class="modal-close" id="configEditorCloseBtn" type="button">✕</button>
    </div>
    <div class="modal-body">
      <div class="config-editor-meta">
        <strong>${esc(file.label || '配置文件')}</strong>
        <span title="${escAttr(file.path || '')}">${esc(file.path || '')}</span>
        <em>${esc((file.format || '').toUpperCase())} · ${esc(statusText)}</em>
      </div>
      <div id="configEditorValidation">${renderConfigValidation(file.validation)}</div>
      <textarea id="configEditorContent" class="config-editor-textarea" spellcheck="false">${esc(file.content || '')}</textarea>
    </div>
    <div class="config-editor-actions">
      <span id="configEditorSaveStatus"></span>
      <button class="btn btn-ghost" id="configEditorValidateBtn" type="button">校验/编译</button>
      <button class="btn btn-ghost" id="configEditorSystemOpenBtn" type="button">系统打开</button>
      <button class="btn btn-primary" id="configEditorSaveBtn" type="button">保存</button>
    </div>
  </div>`;
  document.body.appendChild(overlay);
  const cleanup = () => overlay.remove();
  document.getElementById('configEditorCloseBtn').onclick = cleanup;
  overlay.addEventListener('click', e => { if (e.target === overlay) cleanup(); });
  document.getElementById('configEditorValidateBtn').onclick = async () => {
    const content = document.getElementById('configEditorContent')?.value || '';
    const status = document.getElementById('configEditorSaveStatus');
    if (status) status.innerHTML = '<span class="status-muted">校验中...</span>';
    try {
      const validation = await invoke('validate_account_config_file', { accountId: id, content });
      document.getElementById('configEditorValidation').innerHTML = renderConfigValidation(validation);
      if (status) status.innerHTML = validation.ok ? '<span class="status-ok">校验通过</span>' : '<span class="status-error">校验失败</span>';
    } catch (e) {
      if (status) status.innerHTML = '<span class="status-error">校验异常</span>';
      showToast('配置校验失败: ' + e, 'error');
    }
  };
  document.getElementById('configEditorSaveBtn').onclick = async () => {
    const content = document.getElementById('configEditorContent')?.value || '';
    const status = document.getElementById('configEditorSaveStatus');
    if (status) status.innerHTML = '<span class="status-muted">保存中...</span>';
    try {
      const result = await invoke('save_account_config_file', { accountId: id, content });
      document.getElementById('configEditorValidation').innerHTML = renderConfigValidation(result.validation);
      if (!result.ok) {
        if (status) status.innerHTML = '<span class="status-error">未保存</span>';
        showToast(result.message || '配置校验未通过，未保存', 'error');
        return;
      }
      if (status) status.innerHTML = `<span class="status-ok">已保存${result.backup_path ? '，已备份' : ''}</span>`;
      showToast('配置文件已保存', 'success');
      fetchClientEventsForDetail(id);
      fetchClientBackupsForDetail(id);
    } catch (e) {
      if (status) status.innerHTML = '<span class="status-error">保存失败</span>';
      showToast('保存配置文件失败: ' + e, 'error');
    }
  };
  document.getElementById('configEditorSystemOpenBtn').onclick = () => openClientConfig(id);
}

function renderConfigValidation(validation) {
  const result = validation || {};
  const diagnostics = Array.isArray(result.diagnostics) ? result.diagnostics : [];
  const cls = result.ok ? 'status-ok' : 'status-error';
  const label = result.ok ? '语法正常' : '需要修正';
  return `<div class="config-editor-validation">
    <strong class="${cls}">${label}</strong>
    ${diagnostics.map(item => `<span class="${item.level === 'error' ? 'status-error' : (item.level === 'warning' ? 'status-warn' : 'status-muted')}">${esc(item.message || '')}</span>`).join('')}
  </div>`;
}

async function saveAndApplyClientAccount(id) {
  const saved = await saveAccount({ silent: true, stay: true });
  if (!saved) {
    showToast('写入前保存账号失败', 'error');
    return;
  }
  showToast('账号已保存，开始预检写入配置', 'success');
  await applyClientAccount(saved.id || id);
}

async function applyClientAccount(id) {
  try {
    showToast('正在执行写入前预检', 'info');
    const dryReport = await invoke('apply_client_account', { accountId: id, dryRun: true });
    const statusEl = document.getElementById('client-status-' + id);
    if (statusEl) statusEl.innerHTML = renderClientStatusSummary(dryReport);
    fetchClientEventsForDetail(id);
    if (!await showClientApplyConfirm(dryReport)) {
      if (!dryReport.ok) showToast('预检未通过，已取消写入', 'error');
      return;
    }
    const report = await invoke('apply_client_account', { accountId: id, dryRun: false });
    showToast(report.ok ? '客户端配置已写入' : '客户端配置写入后仍有问题', report.ok ? 'success' : 'error');
    await loadAccountsData();
    const el = document.getElementById('client-status-' + id);
    if (el) el.innerHTML = renderClientStatusSummary(report);
    fetchClientEventsForDetail(id);
    fetchClientBackupsForDetail(id);
  } catch (e) {
    showToast('写入客户端配置失败: ' + e, 'error');
  }
}

async function fetchClientStatusForCard(a) {
  const el = document.getElementById('client-status-' + a.id);
  if (!el || !a.id) return;
  el.innerHTML = '<span class="balance-loading">检查中...</span>';
  try {
    const report = await invoke('get_client_status', { accountId: a.id });
    a._client_status_report = report;
    el.innerHTML = renderClientStatusSummary(report);
    refreshClientSwitcherIssues();
  } catch (e) {
    el.innerHTML = `<span class="status-error">${esc(String(e))}</span>`;
  }
}

function renderClientStatusSummary(report) {
  const command = report.command || {};
  const version = command.version || (command.installed ? '已安装' : '未检测到');
  const diagnostics = Array.isArray(report.diagnostics) ? report.diagnostics : [];
  const hasError = diagnostics.some(item => item.level === 'error');
  const cls = hasError ? 'status-error' : (report.ok ? 'status-ok' : 'status-warn');
  const risk = report.risk_level ? `风险 ${report.risk_level}` : (report.schema_ok === false ? 'Schema 异常' : '');
  return `<div class="client-status-summary">
    <span class="${cls}">${esc(version)}</span>
    ${risk ? `<span>${esc(risk)}</span>` : ''}
  </div>`;
}

// ── 模型获取与余额查询 ──

let balanceCache = {};

async function applyAccount(id) {
  try {
    await invoke('switch_account', { id });
    accountsData.active_id = id;
    showToast('已切换活跃账号', 'success');
    await loadAccountsData();
  } catch (e) {
    showToast('切换账号失败: ' + e, 'error');
  }
}

async function fetchAndPopulateModels() {
  const statusEl = document.getElementById('modelFetchStatus');
  if (statusEl) statusEl.innerHTML = '<span class="status-muted">获取中...</span>';
  try {
    // 统一使用表单中的 upstream 和 api_key（用户可能已修改但尚未保存）
    const upstream = document.getElementById('edit_upstream')?.value?.trim();
    const keyInput = document.getElementById('edit_api_key');
    const apiKey = keyInput ? keyInput.value.trim() : '';
    if (!upstream) { showToast('请先填写上游 URL', 'error'); return; }
    const endpointKind = document.getElementById('edit_endpoint_kind')?.value || 'open_ai_chat';
    upstreamModels = await invoke('fetch_upstream_models', { upstream, apiKey, endpointKind });
    if (upstreamModels.length > 0) {
      if (statusEl) statusEl.innerHTML = `<span class="status-ok">获取到 ${upstreamModels.length} 个模型</span>`;
      // 重新渲染模型映射行
      const knownModels = getProviderKnownModels(editingAccount?.provider || '');
      document.getElementById('modelMapRows').innerHTML = renderModelMappingRows(knownModels);
    } else {
      if (statusEl) statusEl.innerHTML = '<span class="status-muted">上游未返回模型</span>';
    }
  } catch (e) {
    if (statusEl) statusEl.innerHTML = '<span class="status-error">获取失败</span>';
    showToast('获取模型列表失败: ' + e, 'error');
  }
}

async function fetchClientModels() {
  const statusEl = document.getElementById('clientModelFetchStatus');
  if (statusEl) statusEl.innerHTML = '<span class="status-muted">获取中...</span>';
  try {
    const upstream = document.getElementById('edit_upstream')?.value?.trim();
    const apiKey = document.getElementById('edit_api_key')?.value?.trim() || '';
    if (!upstream) { showToast('请先填写目标客户端 Base URL', 'error'); return; }
    const provider = document.getElementById('edit_client_provider')?.value || editingAccount?.provider || '';
    const endpointKind = accountClientKind(editingAccount) === 'claude_code' || provider === 'anthropic'
      ? 'anthropic_messages'
      : 'open_ai_chat';
    upstreamModels = await invoke('fetch_upstream_models', { upstream, apiKey, endpointKind });
    if (upstreamModels.length > 0) {
      if (statusEl) statusEl.innerHTML = `<span class="status-ok">获取到 ${upstreamModels.length} 个模型</span>`;
      syncEditingDraftFromForm();
      const rows = document.getElementById('clientModelMapRows');
      if (rows) rows.innerHTML = renderClientModelMappingRows(editingAccount);
    } else {
      if (statusEl) statusEl.innerHTML = '<span class="status-muted">上游未返回模型</span>';
    }
  } catch (e) {
    if (statusEl) statusEl.innerHTML = '<span class="status-error">获取失败</span>';
    showToast('获取客户端模型列表失败: ' + e, 'error');
  }
}

async function testUpstreamConnectivity() {
  const upstream = document.getElementById('edit_upstream')?.value?.trim();
  if (!upstream) { showToast('请先填写上游 URL', 'error'); return; }
  const keyInput = document.getElementById('edit_api_key');
  const apiKey = keyInput ? keyInput.value.trim() : '';
  const resultEl = document.getElementById('connectivityResult');
  if (resultEl) resultEl.innerHTML = '<span class="status-muted">检测中...</span>';
  try {
    const endpointKind = document.getElementById('edit_endpoint_kind')?.value || 'open_ai_chat';
    const result = await invoke('test_upstream_connectivity', { upstream, apiKey, endpointKind });
    if (result.ok) {
      const models = result.model_count != null ? `，${result.model_count} 个模型` : '';
      if (resultEl) resultEl.innerHTML = `<span class="status-ok">连通 (${result.status}, ${result.latency_ms}ms${models})</span>`;
      showToast(`上游连通正常 (${result.latency_ms}ms${models})`, 'success');
    } else if (result.error) {
      if (resultEl) resultEl.innerHTML = `<span class="status-error">${esc(result.error)}</span>`;
      showToast('连通失败: ' + result.error, 'error');
    } else {
      if (resultEl) resultEl.innerHTML = `<span class="status-warn">HTTP ${result.status} (${result.latency_ms}ms)</span>`;
      showToast(`上游返回 HTTP ${result.status}`, 'error');
    }
  } catch (e) {
    if (resultEl) resultEl.innerHTML = `<span class="status-error">${esc(String(e))}</span>`;
    showToast('连通测试异常: ' + e, 'error');
  }
}

async function testVisionConnectivity() {
  const upstream = document.getElementById('edit_vision_upstream')?.value?.trim();
  if (!upstream) { showToast('请先填写视觉上游 URL', 'error'); return; }
  const keyInput = document.getElementById('edit_vision_api_key');
  const apiKey = keyInput ? keyInput.value.trim() : '';
  const resultEl = document.getElementById('visionConnectivityResult');
  if (resultEl) resultEl.innerHTML = '<span class="status-muted">检测中...</span>';
  try {
    const visionPath = document.getElementById('edit_vision_endpoint')?.value?.trim() || 'v1/coding_plan/vlm';
    const adapterId = document.getElementById('edit_vision_adapter')?.value || 'minimax_coding_plan_vlm';
    const result = await invoke('test_vision_connectivity', { upstream, apiKey, visionPath, adapterId });
    if (result.ok) {
      const detail = result.detail ? `，${esc(result.detail)}` : '';
      if (resultEl) resultEl.innerHTML = `<span class="status-ok">连通 (${result.status}, ${result.latency_ms}ms${detail})</span>`;
      showToast(`视觉上游连通正常 (${result.latency_ms}ms)`, 'success');
    } else if (result.error) {
      if (resultEl) resultEl.innerHTML = `<span class="status-error">${esc(result.error)}</span>`;
      showToast('视觉上游连通失败: ' + result.error, 'error');
    } else {
      if (resultEl) resultEl.innerHTML = `<span class="status-warn">HTTP ${result.status} (${result.latency_ms}ms)</span>`;
      showToast(`视觉上游返回 HTTP ${result.status}`, 'error');
    }
  } catch (e) {
    if (resultEl) resultEl.innerHTML = `<span class="status-error">${esc(String(e))}</span>`;
    showToast('视觉连通测试异常: ' + e, 'error');
  }
}

async function fetchBalanceForCard(a) {
  const el = document.getElementById('balance-' + a.id);
  if (!el) return;
  const cacheKey = a.id;
  // 5 分钟内使用缓存
  if (balanceCache[cacheKey] && balanceCache[cacheKey].ts > Date.now() - 300000) {
    el.innerHTML = renderBalanceInfo(balanceCache[cacheKey].info);
    return;
  }
  el.innerHTML = '<span class="balance-loading">...</span>';
  try {
    const info = await invoke('fetch_balance', { accountId: a.id });
    balanceCache[cacheKey] = { info, ts: Date.now() };
    el.innerHTML = renderBalanceInfo(info);
  } catch (e) {
    el.innerHTML = '<span class="balance-na">不可用</span>';
  }
}

async function refreshBalanceForCard(id) {
  const a = (accountsData.accounts || []).find(acc => acc.id === id);
  if (!a) return;
  delete balanceCache[id];
  await fetchBalanceForCard(a);
}

function renderBalanceInfo(info) {
  if (info.mode === 'token_credit') {
    const remaining = info.credit_remaining;
    const limit = info.credit_limit;
    const pct = limit > 0 ? Math.round(remaining / limit * 100) : 0;
    const label = info.credit_label ? ` (${info.credit_label})` : '';
    return `<div class="balance-row">
      <span>💳 $${remaining != null ? remaining.toFixed(2) : '—'}${limit != null ? ' / $' + limit.toFixed(2) : ''}${label}</span>
      <div class="bar-track"><div class="bar-fill" style="width:${Math.min(pct, 100)}%"></div></div>
    </div>`;
  }
  if (info.mode === 'subscription') {
    return `<div class="balance-row">
      <span>📅 ${info.weekly_remaining || '—'}/${info.weekly_limit || '—'}</span>
      <span>⏱ 5h: ${info.hours_5_remaining || '—'}</span>
    </div>`;
  }
  if (info.mode === 'coding_plan' && info.model_remains) {
    const coding = info.model_remains.find(m => m.model_name === 'MiniMax-M*') || info.model_remains[0];
    if (!coding) return '<span class="balance-na">无模型数据</span>';
    const iRemain = coding.interval_total - coding.interval_used;
    const iPct = coding.interval_total > 0 ? Math.round(iRemain / coding.interval_total * 100) : 0;
    const wRemain = coding.weekly_total - coding.weekly_used;
    const wPct = coding.weekly_total > 0 ? Math.round(wRemain / coding.weekly_total * 100) : 0;
    return `<div class="balance-row">
      <span>5h ${iRemain}/${coding.interval_total}</span>
      <div class="bar-track"><div class="bar-fill" style="width:${Math.min(iPct, 100)}%"></div></div>
      <span class="balance-sub-label">周 ${wRemain}/${coding.weekly_total}</span>
      <div class="bar-track"><div class="bar-fill" style="width:${Math.min(wPct, 100)}%"></div></div>
    </div>`;
  }
  return '<span class="balance-na">不支持</span>';
}

// ═══════════════════════════════════════════════════════════════
// 键盘快捷键
