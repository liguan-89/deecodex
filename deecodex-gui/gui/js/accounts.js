const CODEX_MODEL_LIST = ['gpt-5.5', 'gpt-5.4', 'gpt-5.4-mini', 'gpt-5.3-codex', 'gpt-5', 'codex-auto-review'];

function providerBadgeClass(p) {
  return 'badge-provider badge-' + (p || 'custom');
}

function providerIcon(p) {
  const icons = { openrouter: '◉', deepseek: '⬡', openai: '◆', anthropic: '◈', 'google-ai': '◎', custom: '…' };
  return icons[p] || '…';
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
      const active = a.id === accountsData.active_id;
      return `<div class="account-card${active ? ' active' : ''}">
        <div class="account-card-header">
          <span class="${providerBadgeClass(a.provider)}">${esc(a.provider)}</span>
          ${active ? '<span class="active-badge">✓ 活跃</span>' : ''}
          <button class="card-delete-btn" onclick="deleteAccount('${escAttr(a.id)}')" title="删除">✕</button>
        </div>
        <div class="card-name">${esc(a.name)}</div>
        <div class="card-key">${esc(maskKey(a.api_key) || '(未配置)')}</div>
        <div class="card-upstream" title="${escAttr(a.upstream)}">${esc(trunc(a.upstream, 36))}</div>
        <div class="card-balance" id="balance-${escAttr(a.id)}">
          <span class="balance-loading">—</span>
        </div>
        ${a.context_window_override ? `<div class="card-context" title="上下文窗口: ${a.context_window_override.toLocaleString()} tokens">⇄ ${a.context_window_override.toLocaleString()} tokens</div>` : ''}
        ${a.vision_enabled ? '<div class="card-context" title="已启用多模态视觉路由">👁 多模态</div>' : ''}
        ${a.reasoning_effort_override ? `<div class="card-context" title="推理强度: ${a.reasoning_effort_override}${a.thinking_tokens ? ', 思考预算: ' + a.thinking_tokens.toLocaleString() + ' tokens' : ''}">🧠 ${a.reasoning_effort_override}</div>` : ''}
        <div class="card-actions-row">
          <button class="btn-refresh" onclick="refreshBalanceForCard('${escAttr(a.id)}')" title="刷新余额">↻</button>
          ${active
            ? '<button class="btn-applied" disabled>✓ 已应用</button>'
            : `<button class="btn-apply" onclick="applyAccount('${escAttr(a.id)}')">▶ 应用</button>`}
          <button onclick="editAccount('${escAttr(a.id)}')">⚙ 编辑</button>
        </div>
      </div>`;
    }).join('') + '</div>';
  }

  return `<div class="page-header" style="display:flex;align-items:center;justify-content:space-between;">
    <div><h2>账号管理</h2><p>管理上游 LLM 账号，点击「应用」切换活跃账号</p></div>
    <div style="display:flex;gap:10px;">
      <button class="btn btn-ghost" onclick="importFromCodex()">📥 导入 Codex 配置</button>
      <button class="btn btn-primary" onclick="navigateAccounts('add')">+ 添加账号</button>
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
  const knownModels = getProviderKnownModels(a.provider);

  return `<div class="breadcrumb">
    <span class="back-link" onclick="navigateAccounts('list')">← 账号列表</span>
    <span> / ${esc(a.name)}</span>
  </div>
  <div class="page-header"><h2>${esc(a.name)} <span style="color:red;font-size:12px;">[v1.8.9-1M]</span></h2><p><span class="${providerBadgeClass(a.provider)}">${esc(a.provider)}</span></p></div>

  <div class="account-form">
    <div class="config-fields">
      <div class="config-field">
        <label>账号名称</label>
        <input type="text" id="edit_name" value="${escAttr(a.name)}" placeholder="输入账号显示名">
      </div>
      <div class="config-field">
        <label>上游 URL</label>
        <input type="text" id="edit_upstream" value="${escAttr(a.upstream)}" placeholder="https://api.example.com/v1">
        <span class="hint">Chat Completions API 基础地址</span>
        <button class="btn btn-ghost" onclick="testUpstreamConnectivity()" style="font-size:11px;margin-top:4px;">🔌 测试连通性</button>
        <span id="connectivityResult" style="font-size:11px;margin-left:8px;"></span>
      </div>
      <div class="config-field">
        <label>API Key</label>
        <div class="pass-group">
          <input type="password" id="edit_api_key" value="${escAttr(a.api_key)}" placeholder="输入 API 密钥" autocomplete="off">
          <button type="button" onclick="togglePass('edit_api_key', this)" title="显示/隐藏">⊙</button>
        </div>
        <span class="hint">API Key 通过密码框安全存储</span>
      </div>
      <div class="config-field">
        <label>余额查询 URL <span style="font-weight:normal;color:var(--text-muted);">（可选）</span></label>
        <input type="text" id="edit_balance_url" value="${escAttr(a.balance_url || '')}" placeholder="留空则自动探测">
        <span class="hint">自定义余额/额度查询接口地址，例如 https://api.minimax.com/v1/account/info</span>
      </div>
      <div class="config-field">
        <label style="display:flex;align-items:center;gap:8px;">
          <input type="checkbox" id="edit_translate_enabled"
            ${a.translate_enabled !== false ? 'checked' : ''}>
          启用请求翻译（Responses → Chat Completions）
        </label>
        <span class="hint">关闭后请求将直接透传至上游 Responses API，适用于原生支持 Responses API 的上游（如 OpenAI）。DeepSeek 等仅支持 Chat Completions 的供应商需保持开启。</span>
      </div>
    </div>

    <div class="section-sub-label">模型映射</div>
    <div style="font-size:11px;color:var(--text-muted);margin-bottom:10px;">左侧为 Codex 请求的模型名，右侧为上游实际模型名</div>
    <div style="margin-bottom:10px;">
      <button class="btn btn-ghost" onclick="fetchAndPopulateModels()" style="font-size:11px;">⬇ 从上游获取模型列表</button>
      <span id="modelFetchStatus" style="font-size:10px;color:var(--text-muted);margin-left:8px;"></span>
    </div>
    <div id="modelMapRows">${renderModelMappingRows(knownModels)}</div>
    <div class="model-add-row"><button onclick="addModelRow('modelMapRows', '${escAttr(JSON.stringify(knownModels))}')">+ 添加模型映射</button></div>

    <div class="collapsible-section">
      <div class="config-fields">
        <div class="config-field">
          <label style="display:flex;align-items:center;gap:8px;">
            <input type="checkbox" id="edit_vision_enabled" ${a.vision_enabled ? 'checked' : ''} onchange="toggleVisionFields()">
            启用多模态（视觉路由）
          </label>
          <span class="hint">模型支持图片输入时勾选，配置独立的视觉模型路由</span>
        </div>
      </div>
      <div id="visionFields" style="${a.vision_enabled ? '' : 'display:none;'}">
        <div class="config-fields" style="margin-top:12px;">
          <div class="config-field">
            <label>视觉上游 URL</label>
            <input type="text" id="edit_vision_upstream" value="${escAttr(a.vision_upstream || '')}" placeholder="https://api.minimax.chat">
          </div>
          <div class="config-field">
            <label>视觉 API Key</label>
            <div class="pass-group">
              <input type="password" id="edit_vision_api_key" value="${escAttr(a.vision_api_key || '')}" placeholder="视觉模型密钥" autocomplete="off">
              <button type="button" onclick="togglePass('edit_vision_api_key', this)" title="显示/隐藏">⊙</button>
            </div>
          </div>
          <div class="config-field">
            <label>视觉模型名</label>
            <input type="text" id="edit_vision_model" value="${escAttr(a.vision_model || '')}" placeholder="MiniMax-M1">
          </div>
          <div class="config-field">
            <label>视觉端点路径</label>
            <input type="text" id="edit_vision_endpoint" value="${escAttr(a.vision_endpoint || '')}" placeholder="v1/coding_plan/vlm">
          </div>
        </div>
      </div>
    </div>

    <div class="section-sub-label" style="color:red;">【测试】上下文窗口（1M 大上下文）</div>
    <div class="config-fields" style="margin-bottom:10px;">
      <div class="config-field">
        <label style="display:flex;align-items:center;gap:8px;">
          <input type="checkbox" id="edit_cw_enabled" ${a.context_window_override ? 'checked' : ''} onchange="toggleContextWindowFields()">
          启用上下文窗口覆盖
        </label>
        <span class="hint">勾选后 Codex 将使用下方设定的 token 数作为模型上下文窗口大小</span>
      </div>
      <div class="config-field" id="cwSizeField" style="${a.context_window_override ? '' : 'display:none;'}">
        <label>上下文窗口大小 (token)</label>
        <input type="number" id="edit_cw_size" value="${a.context_window_override || 1000000}" min="1" max="10000000" step="1" placeholder="1000000">
        <span class="hint">Codex 有效上下文 = 此值 × 95%（如填 1052632 可达真正 1M）</span>
      </div>
    </div>
  </div>

  <div class="collapsible-section">
    <div class="config-fields">
      <div class="config-field">
        <label style="display:flex;align-items:center;gap:8px;">
          <input type="checkbox" id="edit_reasoning_enabled" ${a.reasoning_effort_override ? 'checked' : ''} onchange="toggleReasoningFields()">
          强制推理强度（覆盖 Codex 请求）
        </label>
        <span class="hint">用于需要强制启用扩展思考的模型，如 Claude Opus、DeepSeek-R1</span>
      </div>
    </div>
    <div id="reasoningFields" style="${a.reasoning_effort_override ? '' : 'display:none;'}">
      <div class="config-fields" style="margin-top:12px;">
        <div class="config-field">
          <label>推理强度</label>
          <select id="edit_reasoning_effort">
            <option value="" ${!a.reasoning_effort_override ? 'selected' : ''}>不覆盖（跟随 Codex 请求）</option>
            <option value="low" ${a.reasoning_effort_override === 'low' ? 'selected' : ''}>low - 低推理</option>
            <option value="medium" ${a.reasoning_effort_override === 'medium' ? 'selected' : ''}>medium - 中等推理</option>
            <option value="high" ${a.reasoning_effort_override === 'high' ? 'selected' : ''}>high - 高推理</option>
            <option value="max" ${a.reasoning_effort_override === 'max' ? 'selected' : ''}>max - 最大推理</option>
          </select>
        </div>
        <div class="config-field">
          <label>思考 Token 预算 <span style="font-weight:normal;color:var(--text-muted);">（可选）</span></label>
          <input type="number" id="edit_thinking_tokens" value="${a.thinking_tokens || ''}" min="1024" max="128000" step="1024" placeholder="留空不设限制，如 16000">
          <span class="hint">Claude Extended Thinking 的 token 预算，留空则不限制</span>
        </div>
      </div>
    </div>
  </div>

  <div class="collapsible-section">
    <button class="collapsible-toggle${(a.custom_headers && Object.keys(a.custom_headers).length > 0) || a.request_timeout_secs ? ' open' : ''}" onclick="this.classList.toggle('open');this.nextElementSibling.classList.toggle('open')">
      <span class="arrow">▸</span> 高级（自定义头 / 超时）
    </button>
    <div class="collapsible-content${(a.custom_headers && Object.keys(a.custom_headers).length > 0) || a.request_timeout_secs ? ' open' : ''}">
      <div class="config-fields" style="margin-top:12px;">
        <div class="config-field">
          <label>自定义 HTTP 头 <span style="font-weight:normal;color:var(--text-muted);">（可选）</span></label>
          <textarea id="edit_custom_headers" rows="3" placeholder="每行一个: Header-Name: value&#10;例: X-Org-Id: org-xxx&#10;例: X-Custom-Auth: token123" style="font-family:monospace;font-size:12px;">${escAttr(Object.entries(a.custom_headers || {}).map(([k, v]) => k + ': ' + v).join('\n'))}</textarea>
          <span class="hint">每行一个头，格式: 头名称: 头值，将在每次上游请求时附加</span>
        </div>
        <div class="config-field">
          <label>请求超时（秒） <span style="font-weight:normal;color:var(--text-muted);">（可选）</span></label>
          <input type="number" id="edit_request_timeout" value="${a.request_timeout_secs || ''}" min="1" max="600" step="1" placeholder="留空使用默认 300s">
          <span class="hint">此账号的上游请求超时时间，Claude 扩展思考建议设 180-300</span>
        </div>
        <div class="config-field">
          <label>最大重试次数 <span style="font-weight:normal;color:var(--text-muted);">（可选）</span></label>
          <input type="number" id="edit_max_retries" value="${a.max_retries ?? ''}" min="0" max="10" step="1" placeholder="留空使用默认 3 次">
          <span class="hint">上游请求失败（401/429/502/503/连接错误）时的重试次数，0 表示不重试</span>
        </div>
      </div>
    </div>
  </div>

  <div class="accounts-actions">
    <button class="btn btn-primary" onclick="saveAccount()">保存账号</button>
    <button class="btn btn-danger" onclick="deleteAccount('${escAttr(a.id)}')">删除账号</button>
  </div>`;
}

function renderModelMappingRows(knownModels) {
  const a = editingAccount;
  if (!a) return '';
  const map = a.model_map || {};
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
    return `<div class="model-row">
      <div class="model-label codex">${esc(r.codexModel)}${labelExtra}</div>
      <div class="model-value">
        <div class="model-autocomplete">
          <input type="text" value="${escAttr(r.val)}" placeholder="未映射 (使用原名)"
            data-codex="${escAttr(r.codexModel)}" data-readonly="${r.readonly}"
            onchange="onModelMapChange(this)"
            onfocus="showSuggestions(this)" oninput="filterSuggestions(this)" onblur="hideSuggestions(this)"
            autocomplete="off">
          <div class="model-suggestions" style="display:none;" data-suggestions="${suggestionsJson}"></div>
        </div>
        <button class="model-remove" onclick="removeModelMapRow(this)" title="移除">✕</button>
      </div>
    </div>`;
  }).join('');
}

// ── 数据加载 ──

async function loadAccountsData() {
  if (!window.DeeCodexTauri?.hasTauri) {
    accountsData = { accounts: [], active_id: null };
    providerPresets = providerPresets.length ? providerPresets : [];
    if (accountsView === 'list' && currentPanel === 'accounts') renderMainContent();
    return;
  }
  try {
    const result = await invoke('list_accounts');
    accountsData = result;
    if (!providerPresets.length) await loadProviderPresets();
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

function getProviderKnownModels(provider) {
  const p = providerPresets.find(pp => pp.slug === provider);
  return p ? p.known_models : [];
}

// ── 模型映射编辑辅助 ──

function onModelMapChange(input) {
  // 预留：当用户修改上游模型名时更新占位数据
}

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
  row.innerHTML = `<div class="model-label codex"><input type="text" class="custom-codex-model" placeholder="Codex 模型名" style="width:100%;background:var(--bg-input);border:1px solid var(--border-default);border-radius:var(--radius-sm);color:var(--text-primary);padding:7px 10px;font-size:12px;font-family:var(--font-mono);outline:none;"></div>
    <div class="model-value">
      <div class="model-autocomplete">
        <input type="text" placeholder="上游模型名" autocomplete="off"
          onfocus="showSuggestions(this)" oninput="filterSuggestions(this)" onblur="hideSuggestions(this)">
        <div class="model-suggestions" style="display:none;" data-suggestions="${suggestionsJson}"></div>
      </div>
      <button class="model-remove" onclick="removeModelMapRow(this)" title="移除">✕</button>
    </div>`;
  container.appendChild(row);
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
    request_timeout_secs: null,
    max_retries: null,
    translate_enabled: true,
  };
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

async function saveAccount() {
  if (!editingAccount) return;
  const a = editingAccount;

  a.name = document.getElementById('edit_name')?.value?.trim() || a.name;
  a.upstream = document.getElementById('edit_upstream')?.value?.trim() || a.upstream;

  const keyInput = document.getElementById('edit_api_key');
  if (keyInput) a.api_key = keyInput.value.trim();

  const visionEnabled = document.getElementById('edit_vision_enabled');
  if (visionEnabled) a.vision_enabled = visionEnabled.checked;

  const vu = document.getElementById('edit_vision_upstream');
  if (vu) a.vision_upstream = vu.value.trim();
  const vk = document.getElementById('edit_vision_api_key');
  if (vk) a.vision_api_key = vk.value.trim();
  const vm = document.getElementById('edit_vision_model');
  if (vm) a.vision_model = vm.value.trim();
  const ve = document.getElementById('edit_vision_endpoint');
  if (ve) a.vision_endpoint = ve.value.trim();

  const bu = document.getElementById('edit_balance_url');
  if (bu) a.balance_url = bu.value.trim();

  // 大上下文窗口覆盖
  const cwEnabled = document.getElementById('edit_cw_enabled');
  if (cwEnabled && cwEnabled.checked) {
    const cwSize = document.getElementById('edit_cw_size');
    a.context_window_override = cwSize ? parseInt(cwSize.value, 10) || null : null;
  } else {
    a.context_window_override = null;
  }

  // 推理配置
  const reasoningEnabled = document.getElementById('edit_reasoning_enabled');
  if (reasoningEnabled && reasoningEnabled.checked) {
    const effortSel = document.getElementById('edit_reasoning_effort');
    a.reasoning_effort_override = effortSel ? (effortSel.value || null) : null;
    const thinkingTokens = document.getElementById('edit_thinking_tokens');
    a.thinking_tokens = thinkingTokens ? (parseInt(thinkingTokens.value, 10) || null) : null;
  } else {
    a.reasoning_effort_override = null;
    a.thinking_tokens = null;
  }

  // 自定义 HTTP 头
  const headersText = document.getElementById('edit_custom_headers');
  if (headersText) {
    const raw = headersText.value.trim();
    a.custom_headers = {};
    if (raw) {
      for (const line of raw.split('\n')) {
        const colonIdx = line.indexOf(':');
        if (colonIdx > 0) {
          const k = line.substring(0, colonIdx).trim();
          const v = line.substring(colonIdx + 1).trim();
          if (k && v) a.custom_headers[k] = v;
        }
      }
    }
  }

  // 请求超时
  const timeoutInput = document.getElementById('edit_request_timeout');
  if (timeoutInput) {
    a.request_timeout_secs = parseInt(timeoutInput.value, 10) || null;
  } else {
    a.request_timeout_secs = null;
  }

  // 最大重试次数
  const retriesInput = document.getElementById('edit_max_retries');
  if (retriesInput) {
    a.max_retries = parseInt(retriesInput.value, 10) || null;
  } else {
    a.max_retries = null;
  }

  const translateEnabled = document.getElementById('edit_translate_enabled');
  if (translateEnabled) a.translate_enabled = translateEnabled.checked;

  a.model_map = collectModelMap();
  a.updated_at = Math.floor(Date.now() / 1000);

  try {
    let result;
    if (!a.id) {
      // 新账号
      result = await invoke('add_account', { provider: a.provider || 'custom', accountJson: JSON.stringify(a) });
      showToast('账号已创建', 'success');
    } else {
      result = await invoke('update_account', { accountJson: JSON.stringify(a) });
      showToast('账号已保存', 'success');
    }
    await loadAccountsData();
    editingAccount = accountsData.accounts.find(ac => ac.id === result.id);
    if (editingAccount) editingAccount = JSON.parse(JSON.stringify(editingAccount));
    renderMainContent();
  } catch (e) {
    showToast('保存账号失败: ' + e, 'error');
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
  if (statusEl) statusEl.textContent = '获取中...';
  try {
    // 统一使用表单中的 upstream 和 api_key（用户可能已修改但尚未保存）
    const upstream = document.getElementById('edit_upstream')?.value?.trim();
    const keyInput = document.getElementById('edit_api_key');
    const apiKey = keyInput ? keyInput.value.trim() : '';
    if (!upstream) { showToast('请先填写上游 URL', 'error'); return; }
    upstreamModels = await invoke('fetch_upstream_models', { upstream, apiKey });
    if (upstreamModels.length > 0) {
      if (statusEl) statusEl.textContent = `✓ 获取到 ${upstreamModels.length} 个模型`;
      // 重新渲染模型映射行
      const knownModels = getProviderKnownModels(editingAccount?.provider || '');
      document.getElementById('modelMapRows').innerHTML = renderModelMappingRows(knownModels);
    } else {
      if (statusEl) statusEl.textContent = '上游未返回模型';
    }
  } catch (e) {
    if (statusEl) statusEl.textContent = '获取失败';
    showToast('获取模型列表失败: ' + e, 'error');
  }
}

async function testUpstreamConnectivity() {
  const upstream = document.getElementById('edit_upstream')?.value?.trim();
  if (!upstream) { showToast('请先填写上游 URL', 'error'); return; }
  const keyInput = document.getElementById('edit_api_key');
  const apiKey = keyInput ? keyInput.value.trim() : '';
  const resultEl = document.getElementById('connectivityResult');
  if (resultEl) resultEl.innerHTML = '<span style="color:var(--text-muted);">检测中...</span>';
  try {
    const result = await invoke('test_upstream_connectivity', { upstream, apiKey });
    if (result.ok) {
      const models = result.model_count != null ? `，${result.model_count} 个模型` : '';
      if (resultEl) resultEl.innerHTML = `<span style="color:var(--green);">✓ 连通 (${result.status}, ${result.latency_ms}ms${models})</span>`;
      showToast(`上游连通正常 (${result.latency_ms}ms${models})`, 'success');
    } else if (result.error) {
      if (resultEl) resultEl.innerHTML = `<span style="color:var(--red);">✗ ${esc(result.error)}</span>`;
      showToast('连通失败: ' + result.error, 'error');
    } else {
      if (resultEl) resultEl.innerHTML = `<span style="color:var(--amber);">⚠ HTTP ${result.status} (${result.latency_ms}ms)</span>`;
      showToast(`上游返回 HTTP ${result.status}`, 'error');
    }
  } catch (e) {
    if (resultEl) resultEl.innerHTML = `<span style="color:var(--red);">✗ ${esc(String(e))}</span>`;
    showToast('连通测试异常: ' + e, 'error');
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
      <span style="font-size:10px;">周 ${wRemain}/${coding.weekly_total}</span>
      <div class="bar-track"><div class="bar-fill" style="width:${Math.min(wPct, 100)}%"></div></div>
    </div>`;
  }
  return '<span class="balance-na">不支持</span>';
}

// ═══════════════════════════════════════════════════════════════
// 键盘快捷键
