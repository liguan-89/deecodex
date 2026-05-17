const CODEX_MODEL_LIST = ['gpt-5.5', 'gpt-5.4', 'gpt-5.4-mini', 'gpt-5.3-codex', 'gpt-5', 'codex-auto-review'];

function providerBadgeClass(p) {
  return 'badge-provider badge-' + (p || 'custom');
}

function providerIcon(p) {
  const icons = { openrouter: '◉', deepseek: '⬡', kimi: '月', minimax: 'M', glm: '智', openai: '◆', anthropic: '◈', 'google-ai': '◎', custom: '…' };
  return icons[p] || '…';
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
    open_ai_responses: 'Responses 直连',
    anthropic_messages: 'Anthropic Messages',
    custom_chat: '自定义 Chat',
    custom_responses: '自定义 Responses',
    OpenAiChat: 'Chat 兼容',
    OpenAiResponses: 'Responses 直连',
    AnthropicMessages: 'Anthropic Messages',
    CustomChat: '自定义 Chat',
    CustomResponses: '自定义 Responses',
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
  document.getElementById('mainContent').innerHTML = renderAccountsPanel();
}

function renderAccountsPanel() {
  if (accountsView === 'add') return renderAddAccount();
  if (accountsView === 'edit') return renderAccountDetail();
  return renderAccountList();
}

// ── Level 1: 账号列表 ──

function renderAccountList() {
  const list = accountsData.accounts || [];
  let cards = '';
  if (list.length === 0) {
    cards = '<div class="empty-state">暂无账号，点击下方按钮创建</div>';
  } else {
    cards = '<div class="accounts-grid">' + list.map(a => {
      const active = a.id === (accountsData.active_account_id || accountsData.active_id);
      const ep = currentEndpoint(a);
      const visionMode = ep?.vision?.mode || (a.vision_enabled ? 'glue' : 'off');
      const contextWindow = ep?.context_window_override ?? null;
      const reasoningEffort = ep?.reasoning_effort_override ?? null;
      const thinkingTokens = ep?.thinking_tokens ?? null;
      const caps = accountCapabilityLabels(a).slice(0, 3);
      const capabilityAccount = a.capability_account_id
        ? list.find(candidate => candidate.id === a.capability_account_id)
        : null;
      return `<div class="account-card${active ? ' active' : ''}">
        <div class="account-card-header">
          <span class="${providerBadgeClass(a.provider)}">${esc(a.provider)}</span>
          ${active ? '<span class="active-badge">✓ 活跃</span>' : ''}
          <button class="card-delete-btn" onclick="deleteAccount('${escAttr(a.id)}')" title="删除">✕</button>
        </div>
        <div class="card-name">${esc(a.name)}</div>
        <div class="card-key">${esc(maskKey(a.api_key) || '(未配置)')}</div>
        <div class="card-upstream" title="${escAttr(a.upstream)}">${esc(trunc(a.upstream, 36))}</div>
        <div class="card-context" title="活跃端点">${esc(ep?.name || '默认端点')} · ${esc(endpointKindLabel(ep?.kind))}</div>
        ${caps.map(label => `<div class="card-context">${esc(label)}</div>`).join('')}
        <div class="card-context" title="视觉模式">${esc(visionModeLabel(visionMode))}</div>
        <div class="card-balance" id="balance-${escAttr(a.id)}">
          <span class="balance-loading">—</span>
        </div>
        ${contextWindow ? `<div class="card-context" title="上下文窗口: ${contextWindow.toLocaleString()} tokens">⇄ ${contextWindow.toLocaleString()} tokens</div>` : ''}
        ${visionMode === 'glue' || visionMode === 'Glue' ? '<div class="card-context" title="已启用胶水多模态">👁 胶水</div>' : ''}
        ${visionMode === 'native' || visionMode === 'Native' ? '<div class="card-context" title="当前模型原生支持图片输入">👁 原生</div>' : ''}
        ${reasoningEffort ? `<div class="card-context" title="推理强度: ${reasoningEffort}${thinkingTokens ? ', 思考预算: ' + thinkingTokens.toLocaleString() + ' tokens' : ''}">🧠 ${reasoningEffort}</div>` : ''}
        ${a.capability_enabled ? `<div class="card-context" title="能力补全账号: ${escAttr(capabilityAccount ? capabilityAccount.name : '未找到')}">能力补全: ${esc(capabilityAccount ? capabilityAccount.name : '未配置')}</div>` : ''}
        ${a.dev_pipeline_enabled ? `<div class="card-context" title="开发协作编排触发: ${escAttr(a.dev_pipeline_command || '/dev-pipeline')}">开发协作: ${esc(a.dev_pipeline_trigger_mode === 'always' ? '始终' : (a.dev_pipeline_command || '/dev-pipeline'))}</div>` : ''}
        <div class="card-actions-row">
          <button class="account-action account-refresh" onclick="refreshBalanceForCard('${escAttr(a.id)}')" title="刷新余额">刷新</button>
          ${active
            ? '<button class="account-action account-applied" disabled>已应用</button>'
            : `<button class="account-action account-apply" onclick="applyAccount('${escAttr(a.id)}')">应用</button>`}
          <button class="account-action" onclick="editAccount('${escAttr(a.id)}')">编辑</button>
        </div>
      </div>`;
    }).join('') + '</div>';
  }

  return `<div class="page-header accounts-page-header">
    <div><h2>账号管理</h2><p>管理上游 LLM 账号，点击「应用」切换活跃账号</p></div>
    <div class="page-header-actions">
      <button class="btn btn-ghost" onclick="importFromCodex()">导入配置</button>
      <button class="btn btn-primary" onclick="navigateAccounts('add')">添加账号</button>
    </div>
  </div>
  ${cards}`;
}

// ── Level 2: 添加账号 ──

function renderAddAccount() {
  let cards = '';
  if (providerPresets.length === 0) {
    cards = '<div class="empty-state">加载供应商列表...</div>';
  } else {
    cards = '<div class="provider-grid">' + providerPresets.map(p => {
      const upstream = p.default_upstream || '(自定义)';
      return `<div class="provider-card" onclick="addAccount('${escAttr(p.slug)}')">
        <div class="provider-icon">${providerIcon(p.slug)}</div>
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
  <div class="page-header"><h2>选择供应商</h2><p>选择 LLM 供应商以创建新账号配置</p></div>
  ${cards}`;
}

// ── Level 3: 编辑账号 ──

function renderAccountDetail() {
  if (!editingAccount) return '<div class="empty-state">账号数据丢失，请返回列表</div>';
  const a = editingAccount;
  ensureAccountEndpoints(a);
  const ep = currentEndpoint(a) || {};
  const visionMode = ep.vision?.mode || (a.vision_enabled ? 'glue' : 'off');
  const contextWindow = ep.context_window_override ?? null;
  const reasoningEffort = ep.reasoning_effort_override ?? null;
  const thinkingTokens = ep.thinking_tokens ?? null;
  const customHeaders = ep.custom_headers || {};
  const requestTimeout = ep.request_timeout_secs ?? null;
  const maxRetries = ep.max_retries ?? a.max_retries;
  const knownModels = getProviderKnownModels(a.provider);

  return `<div class="breadcrumb">
    <span class="back-link" onclick="navigateAccounts('list')">← 账号列表</span>
    <span> / ${esc(a.name)}</span>
  </div>
  <div class="page-header"><h2>${esc(a.name)}</h2><p><span class="${providerBadgeClass(a.provider)}">${esc(a.provider)}</span></p></div>

  <div class="account-form">
    <section class="account-edit-section">
      <div class="account-section-head">
        <div class="section-sub-label">账号凭据</div>
        <div class="account-section-desc">同一个 API Key 下可以配置多个端点。</div>
      </div>
      <div class="config-fields">
        <div class="config-field">
          <label>账号名称</label>
          <input type="text" id="edit_name" value="${escAttr(a.name)}" placeholder="输入账号显示名">
        </div>
        <div class="config-field">
          <label>API Key</label>
          <div class="pass-group">
            <input type="password" id="edit_api_key" value="${escAttr(a.api_key)}" placeholder="输入 API 密钥" autocomplete="off">
            <button type="button" onclick="togglePass('edit_api_key', this)" title="显示/隐藏">⊙</button>
          </div>
          <span class="hint">账号级凭据，切换端点时复用。</span>
        </div>
      </div>
    </section>

    <section class="account-edit-section">
      <div class="account-section-head">
        <div class="section-sub-label">端点</div>
        <div class="account-section-desc">端点决定协议、URL、路径和余额探测。</div>
      </div>
      ${renderEndpointControls(a, ep)}
      <div class="config-fields endpoint-detail-fields">
        <div class="config-field">
          <label>端点名称</label>
          <input type="text" id="edit_endpoint_name" value="${escAttr(ep.name || '')}" placeholder="例如主模型端点 / 备用端点">
        </div>
        <div class="config-field">
          <label>端点协议</label>
          <select id="edit_endpoint_kind">
            <option value="open_ai_chat" ${(ep.kind || 'open_ai_chat') === 'open_ai_chat' || ep.kind === 'OpenAiChat' ? 'selected' : ''}>Chat 兼容（Responses → Chat）</option>
            <option value="open_ai_responses" ${ep.kind === 'open_ai_responses' || ep.kind === 'OpenAiResponses' ? 'selected' : ''}>Responses 直连</option>
            <option value="anthropic_messages" ${ep.kind === 'anthropic_messages' || ep.kind === 'AnthropicMessages' ? 'selected' : ''}>Anthropic Messages</option>
            <option value="custom_chat" ${ep.kind === 'custom_chat' || ep.kind === 'CustomChat' ? 'selected' : ''}>自定义 Chat</option>
            <option value="custom_responses" ${ep.kind === 'custom_responses' || ep.kind === 'CustomResponses' ? 'selected' : ''}>自定义 Responses</option>
          </select>
          <span class="hint">选择当前上游真实支持的 API 协议。</span>
        </div>
        <div class="config-field wide">
          <label>上游 URL</label>
          <input type="text" id="edit_upstream" value="${escAttr(ep.base_url || a.upstream)}" placeholder="https://api.example.com/v1">
          <div class="inline-test-row">
            <button class="btn btn-ghost" onclick="testUpstreamConnectivity()">测试连通性</button>
            <span id="connectivityResult"></span>
          </div>
        </div>
        <div class="config-field">
          <label>端点路径 <span class="optional-label">可选</span></label>
          <input type="text" id="edit_endpoint_path" value="${escAttr(ep.path || '')}" placeholder="留空使用协议默认路径">
          <span class="hint">例如 chat/completions、responses、messages。</span>
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
            <input type="text" id="edit_vision_upstream" value="${escAttr(ep.vision?.base_url || a.vision_upstream || '')}" placeholder="https://api.minimax.chat">
          </div>
          <div class="config-field">
            <label>视觉 API Key</label>
            <div class="pass-group">
              <input type="password" id="edit_vision_api_key" value="${escAttr(ep.vision?.api_key || a.vision_api_key || '')}" placeholder="视觉模型密钥" autocomplete="off">
              <button type="button" onclick="togglePass('edit_vision_api_key', this)" title="显示/隐藏">⊙</button>
            </div>
          </div>
          <div class="config-field">
            <label>视觉模型名</label>
            <input type="text" id="edit_vision_model" value="${escAttr(ep.vision?.model || a.vision_model || '')}" placeholder="MiniMax-M1">
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
        <div class="account-section-desc">按端点覆盖上下文窗口和推理预算。</div>
      </div>
      <div class="config-fields">
        <div class="config-field">
          <label class="toggle-label">
            <input type="checkbox" id="edit_cw_enabled" ${contextWindow ? 'checked' : ''} onchange="toggleContextWindowFields()">
            上下文窗口覆盖
          </label>
          <span class="hint">勾选后 Codex 使用下方 token 数作为该端点上下文。</span>
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
          <span class="hint">用于 Claude、R1 等需要固定思考预算的端点。</span>
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
        <div class="config-field">
          <label class="toggle-label">
            <input type="checkbox" id="edit_dev_pipeline_enabled" ${a.dev_pipeline_enabled ? 'checked' : ''} onchange="toggleDevPipelineFields()">
            启用开发协作编排
          </label>
          <span class="hint">Codex 中输入触发命令即可进入协作流程，默认以当前活跃账号为主。</span>
        </div>
        <div id="devPipelineFields" style="${a.dev_pipeline_enabled ? '' : 'display:none;'}">
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
    <button class="collapsible-toggle${Object.keys(customHeaders).length > 0 || requestTimeout ? ' open' : ''}" onclick="this.classList.toggle('open');this.nextElementSibling.classList.toggle('open')">
      <span class="arrow">▸</span> 高级（自定义头 / 超时）
    </button>
    <div class="collapsible-content${Object.keys(customHeaders).length > 0 || requestTimeout ? ' open' : ''}">
      <div class="config-fields nested-fields">
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
    <button class="btn btn-danger" onclick="deleteAccount('${escAttr(a.id)}')">删除账号</button>
  </div>
  </div>`;
}

function renderEndpointControls(a, ep) {
  const endpoints = ensureAccountEndpoints(a);
  const active = a.id === (accountsData.active_account_id || accountsData.active_id);
  const currentId = ep?.id || selectedEndpointId(a) || '';
  const isSavedAccount = Boolean(a.id);
  const isCurrentActiveEndpoint = active && currentId === accountsData.active_endpoint_id;
  const applyDisabled = !isSavedAccount || !currentId || isCurrentActiveEndpoint;
  const applyLabel = !isSavedAccount ? '保存后可应用' : '应用端点';
  const templateOptions = (endpointTemplates || []).map(t =>
    `<option value="${escAttr(t.id)}">${esc(t.label)}</option>`
  ).join('');
  const endpointOptions = endpoints.map(endpoint => {
    const isCurrent = endpoint.id === currentId;
    const isActiveEndpoint = active && endpoint.id === accountsData.active_endpoint_id;
    const suffix = isActiveEndpoint ? '（活跃）' : '';
    return `<option value="${escAttr(endpoint.id)}" ${isCurrent ? 'selected' : ''}>${esc(endpoint.name || endpointKindLabel(endpoint.kind))}${suffix}</option>`;
  }).join('');
  const endpointName = ep?.name || '默认端点';
  const endpointKind = endpointKindLabel(ep?.kind);
  const endpointState = isCurrentActiveEndpoint ? '正在使用' : (active ? '未应用' : '账号未应用');
  const endpointPath = ep?.path || '协议默认路径';
  const endpointBaseUrl = ep?.base_url || a.upstream || '';
  const applyAction = isCurrentActiveEndpoint
    ? ''
    : `<button class="btn btn-primary" onclick="applyCurrentEndpoint()" ${applyDisabled ? 'disabled' : ''}>${applyLabel}</button>`;
  const deleteAction = endpoints.length > 1
    ? '<button class="btn btn-danger" onclick="deleteCurrentEndpoint()">删除</button>'
    : '';
  const currentEndpointControl = endpoints.length > 1
    ? `<div class="endpoint-switch-row">
        <label for="edit_endpoint_id">正在编辑</label>
        <select id="edit_endpoint_id" onchange="selectEditingEndpoint(this.value)">${endpointOptions}</select>
      </div>`
    : '';
  const manageClass = endpoints.length > 1 ? 'endpoint-manage-panel' : 'endpoint-manage-panel single';

  return `<div class="endpoint-box">
    <div class="endpoint-current-panel">
      <div class="endpoint-status-main">
        <div class="endpoint-status-name">${esc(endpointName)}</div>
        <div class="endpoint-status-sub" title="${escAttr(endpointBaseUrl)}">${esc(endpointKind)} · ${esc(trunc(endpointBaseUrl || '未设置 URL', 58))}</div>
      </div>
      <div class="endpoint-status-tags">
        <span class="${isCurrentActiveEndpoint ? 'active' : ''}">${esc(endpointState)}</span>
        <span>${esc(endpointPath)}</span>
      </div>
      <div class="endpoint-actions">
        ${applyAction}
        <button class="btn btn-ghost" onclick="duplicateCurrentEndpoint()" ${currentId ? '' : 'disabled'}>复制</button>
        ${deleteAction}
      </div>
    </div>
    <div class="${manageClass}">
      ${currentEndpointControl}
      <div class="endpoint-add-panel">
        <div class="endpoint-control-group endpoint-control-template">
          <label for="new_endpoint_template">添加备用端点</label>
          <select id="new_endpoint_template" title="选择要新增的协议端点">${templateOptions || '<option value="">Chat 兼容端点</option>'}</select>
          <span class="endpoint-add-hint">用于同一账号下配置备用协议或备用上游。</span>
        </div>
        <button class="btn btn-ghost" onclick="addEndpointFromSelectedTemplate()">添加</button>
      </div>
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
    if (!providerPresets.length) await loadProviderPresets();
    if (!endpointTemplates.length) await loadEndpointTemplates();
    if (accountsView === 'list' && currentPanel === 'accounts') renderMainContent();
    // 异步加载余额（不阻塞渲染）
    if (accountsView === 'list') {
      (accountsData.accounts || []).forEach(a => fetchBalanceForCard(a));
    }
  } catch (e) {
    showToast('加载账号失败: ' + e, 'error');
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
    showToast('加载端点模板失败: ' + e, 'error');
  }
}

function getProviderKnownModels(provider) {
  const p = providerPresets.find(pp => pp.slug === provider);
  return p ? p.known_models : [];
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

  const endpoints = ensureAccountEndpoints(a);
  const ep = currentEndpoint(a) || endpoints[0];
  if (!ep) return;

  const endpointName = document.getElementById('edit_endpoint_name');
  if (endpointName) ep.name = endpointName.value.trim() || ep.name || '默认端点';
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

function selectEditingEndpoint(endpointId) {
  if (!editingAccount) return;
  syncEditingDraftFromForm();
  editingAccount._editing_endpoint_id = endpointId;
  upstreamModels = [];
  renderMainContent();
}

function addEndpointFromSelectedTemplate() {
  if (!editingAccount) return;
  syncEditingDraftFromForm();
  const templateId = document.getElementById('new_endpoint_template')?.value;
  const template = (endpointTemplates || []).find(t => t.id === templateId) || providerDefaultTemplate(editingAccount.provider);
  const endpoint = createEndpointFromTemplate(template, editingAccount);
  ensureAccountEndpoints(editingAccount).push(endpoint);
  editingAccount._editing_endpoint_id = endpoint.id;
  renderMainContent();
}

function duplicateCurrentEndpoint() {
  if (!editingAccount) return;
  syncEditingDraftFromForm();
  const ep = currentEndpoint(editingAccount);
  if (!ep) return;
  const copy = JSON.parse(JSON.stringify(ep));
  copy.id = newEndpointId();
  copy.name = (copy.name || endpointKindLabel(copy.kind)) + ' 副本';
  ensureAccountEndpoints(editingAccount).push(copy);
  editingAccount._editing_endpoint_id = copy.id;
  renderMainContent();
}

async function deleteCurrentEndpoint() {
  if (!editingAccount) return;
  syncEditingDraftFromForm();
  const endpoints = ensureAccountEndpoints(editingAccount);
  const ep = currentEndpoint(editingAccount);
  if (!ep || endpoints.length <= 1) return;
  if (!await showConfirm('确定要删除当前端点吗？')) return;
  editingAccount.endpoints = endpoints.filter(item => item.id !== ep.id);
  editingAccount._editing_endpoint_id = editingAccount.endpoints[0]?.id || null;
  renderMainContent();
}

async function applyCurrentEndpoint() {
  if (!editingAccount?.id) {
    showToast('请先保存账号后再应用端点', 'error');
    return;
  }
  const ep = currentEndpoint(editingAccount);
  if (!ep) return;
  try {
    await saveAccount({ silent: true, stay: true });
    await invoke('switch_endpoint', { accountId: editingAccount.id, endpointId: ep.id });
    showToast('已应用端点', 'success');
    accountsData.active_id = editingAccount.id;
    accountsData.active_account_id = editingAccount.id;
    accountsData.active_endpoint_id = ep.id;
    await loadAccountsData();
    editingAccount = accountsData.accounts.find(ac => ac.id === editingAccount.id);
    if (editingAccount) {
      editingAccount = JSON.parse(JSON.stringify(editingAccount));
      editingAccount._editing_endpoint_id = ep.id;
    }
    renderMainContent();
  } catch (e) {
    showToast('应用端点失败: ' + e, 'error');
  }
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

function addAccount(provider) {
  const preset = providerPresets.find(p => p.slug === provider);
  if (!preset) return;
  editingAccount = {
    id: '',
    name: preset.label + ' 账号',
    provider: provider,
    upstream: preset.default_upstream,
    api_key: '',
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
  editingAccount.endpoints = [createEndpointFromTemplate(providerDefaultTemplate(provider), editingAccount)];
  editingAccount._editing_endpoint_id = editingAccount.endpoints[0].id;
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

async function saveAccount(options = {}) {
  if (!editingAccount) return;
  if (options instanceof Event) options = {};
  const a = editingAccount;

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
  if (ep) {
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
  }
  const editingEndpointId = ep?.id || selectedEndpointId(a);
  a.updated_at = Math.floor(Date.now() / 1000);

  try {
    let result;
    if (!a.id) {
      // 新账号
      result = await invoke('add_account', { provider: a.provider || 'custom', accountJson: JSON.stringify(a) });
      if (!options.silent) showToast('账号已创建', 'success');
    } else {
      result = await invoke('update_account', { accountJson: JSON.stringify(a) });
      if (!options.silent) showToast('账号已保存', 'success');
    }
    await loadAccountsData();
    editingAccount = accountsData.accounts.find(ac => ac.id === result.id);
    if (editingAccount) {
      editingAccount = JSON.parse(JSON.stringify(editingAccount));
      editingAccount._editing_endpoint_id = editingEndpointId;
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
    const result = await invoke('test_upstream_connectivity', { upstream, apiKey });
    if (result.ok) {
      const models = result.model_count != null ? `，${result.model_count} 个模型` : '';
      if (resultEl) resultEl.innerHTML = `<span class="status-ok">连通 (${result.status}, ${result.latency_ms}ms${models})</span>`;
      showToast(`视觉上游连通正常 (${result.latency_ms}ms${models})`, 'success');
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
