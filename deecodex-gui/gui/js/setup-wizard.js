let codexQuickStartStatus = null;
let codexQuickStartBusy = false;
let codexQuickStartDismissed = false;

async function checkSetupWizard() {
  hideWizardBar();
  try {
    const status = await invoke('get_codex_quick_start_status');
    codexQuickStartStatus = status || null;
    window.codexQuickStartStatus = codexQuickStartStatus;
    if (status?.should_show && !codexQuickStartDismissed) {
      showCodexQuickStartModal(status, { force: false });
    }
  } catch (error) {
    console.warn('[deecodex] Codex Desktop 快速配置状态读取失败:', error);
  }
}

async function openCodexQuickStart(force) {
  return openCodexQuickStartWithMode(force, null);
}

async function openCodexRelayStart(force) {
  return openCodexQuickStartWithMode(force, 'relay');
}

async function openCodexQuickStartWithMode(force, preferredMode) {
  hideWizardBar();
  try {
    const status = await invoke('get_codex_quick_start_status');
    codexQuickStartStatus = status || null;
    window.codexQuickStartStatus = codexQuickStartStatus;
    showCodexQuickStartModal(status, { force: force !== false, preferredMode });
  } catch (error) {
    showToast('快速配置状态读取失败: ' + error, 'error');
  }
}

function closeCodexQuickStartModal() {
  document.getElementById('codexQuickStartModal')?.remove();
}

function dismissCodexQuickStart() {
  codexQuickStartDismissed = true;
  closeCodexQuickStartModal();
}

function hideWizardBar() {
  const main = document.getElementById('mainContent');
  if (main) main.style.paddingTop = '';
}

function renderCodexQuickModeCard(mode, status, selectedMode) {
  const isSmart = mode === 'smart';
  const isRelay = mode === 'relay';
  const disabled = isSmart && !status?.has_official_login;
  const title = isRelay ? '中转模式' : (isSmart ? '智能路由' : 'API 模式');
  const desc = isRelay
    ? '不改变 Codex 线程/模型标识；只填写中转 URL 和 API Key，使用 Codex 内置 GPT 模型名。'
    : (isSmart
      ? '检测到 Codex 登录态时使用；DeepSeek 负责常规任务，官方登录态承接原生工具。'
      : '不依赖官方登录态；Codex Desktop 直接使用 DeepSeek 账号模型。');
  const badge = isRelay
    ? '推荐中转'
    : (isSmart
      ? (status?.has_official_login ? '已发现登录态' : '需要先登录')
      : '最少配置');
  return `
    <label class="codex-quick-mode-card ${selectedMode === mode ? 'active' : ''} ${disabled ? 'disabled' : ''}">
      <input type="radio" name="codexQuickMode" value="${escAttr(mode)}" ${selectedMode === mode ? 'checked' : ''} ${disabled ? 'disabled' : ''}>
      <span class="codex-quick-mode-main">
        <span class="codex-quick-mode-title">${esc(title)}</span>
        <span class="codex-quick-mode-desc">${esc(desc)}</span>
      </span>
      <span class="codex-quick-mode-badge">${esc(badge)}</span>
    </label>`;
}

function codexQuickModelOptions(models, selectedModel) {
  const seen = new Set();
  return (models || [])
    .map(model => String(model || '').trim())
    .filter(model => {
      if (!model || seen.has(model)) return false;
      seen.add(model);
      return true;
    })
    .map(model => `<option value="${escAttr(model)}" ${model === selectedModel ? 'selected' : ''}>${esc(model)}</option>`)
    .join('');
}

function codexQuickDeepseekModels() {
  const status = window.codexQuickStartStatus || {};
  const models = Array.isArray(status.known_models) && status.known_models.length
    ? status.known_models
    : ['deepseek-v4-pro', 'deepseek-v4-flash', 'deepseek-chat', 'deepseek-reasoner'];
  return models.filter(model => !/^gpt-/i.test(String(model || '').trim()));
}

function codexQuickDeepseekDefaultModel() {
  const status = window.codexQuickStartStatus || {};
  const models = codexQuickDeepseekModels();
  const preferred = String(status.default_model || '').trim();
  if (preferred && !/^gpt-/i.test(preferred) && models.includes(preferred)) return preferred;
  if (models.includes('deepseek-v4-pro')) return 'deepseek-v4-pro';
  return models[0] || 'deepseek-v4-pro';
}

function codexQuickDeepseekUpstream() {
  const status = window.codexQuickStartStatus || {};
  const upstream = String(status.upstream || '').trim();
  return upstream || 'https://api.deepseek.com/v1';
}

function showCodexQuickStartModal(status, options) {
  closeCodexQuickStartModal();
  status = status || {};
  const force = options?.force === true;
  if (!force && status.completed) return;
  const recommendedMode = options?.preferredMode || status.recommended_mode || (status.has_official_login ? 'smart' : 'relay');
  const selectedMode = recommendedMode === 'smart' && status.has_official_login ? 'smart' : (recommendedMode === 'relay' ? 'relay' : 'api');
  window.codexQuickStartStatus = status;
  const models = codexQuickDeepseekModels();
  const relayModels = Array.isArray(status.relay_known_models) ? status.relay_known_models : ['gpt-5.6-sol', 'gpt-5.6-terra', 'gpt-5.6-luna', 'gpt-5.5', 'gpt-5.4', 'gpt-5.4-mini', 'gpt-5.3-codex', 'gpt-5.2'];
  const defaultModel = codexQuickDeepseekDefaultModel();
  const relayDefaultModel = status.relay_default_model || (relayModels.includes('gpt-5.5') ? 'gpt-5.5' : relayModels[0]) || 'gpt-5.5';
  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay codex-quick-overlay';
  overlay.id = 'codexQuickStartModal';
  overlay.innerHTML = `
    <div class="modal-box codex-quick-box">
      <div class="modal-header codex-quick-header">
        <div>
          <h3>开始使用 Codex Desktop</h3>
          <p>选择 API、中转或智能路由；中转模式只需要 URL 和 API Key。</p>
        </div>
        <button class="modal-close" id="codexQuickCloseBtn" type="button">✕</button>
      </div>
      <div class="modal-body codex-quick-body">
        <div class="codex-quick-status">
          <span class="${status.codex_installed ? 'ok' : 'warn'}">Codex Desktop ${status.codex_installed ? '已检测' : '未检测'}</span>
          <span class="${status.has_official_login ? 'ok' : 'muted'}">官方登录态 ${status.has_official_login ? '已发现' : '未发现'}</span>
          <span class="${status.has_execution_account ? 'ok' : 'muted'}">执行账号 ${status.has_execution_account ? '已存在' : '待配置'}</span>
        </div>

        <div class="codex-quick-mode-grid">
          ${renderCodexQuickModeCard('api', status, selectedMode)}
          ${renderCodexQuickModeCard('relay', status, selectedMode)}
          ${renderCodexQuickModeCard('smart', status, selectedMode)}
        </div>

        <div class="config-fields codex-quick-fields">
          <div class="config-field">
            <label>账号名称</label>
            <input type="text" id="codexQuickAccountName" value="${selectedMode === 'relay' ? 'DEX AI 中转' : 'DeepSeek 桌面版'}" placeholder="DeepSeek 桌面版">
          </div>
          <div class="config-field">
            <label id="codexQuickDefaultModelLabel">默认模型</label>
            <select id="codexQuickDefaultModel" disabled title="中转模式固定使用 Codex 内置 GPT 模型；API/智能路由模式新手请保持默认">
              ${codexQuickModelOptions(selectedMode === 'relay' ? relayModels : models, selectedMode === 'relay' ? relayDefaultModel : defaultModel)}
            </select>
          </div>
          <div class="config-field codex-quick-upstream-field">
            <label id="codexQuickUpstreamLabel">${selectedMode === 'relay' ? '中转 Base URL' : 'DeepSeek Base URL'}</label>
            <div class="codex-quick-url-field">
              <input type="text" id="codexQuickUpstream" value="${escAttr(selectedMode === 'relay' ? (status.relay_upstream || 'https://api.dexai.vip/v1') : codexQuickDeepseekUpstream())}" placeholder="https://api.dexai.vip/v1" ${selectedMode === 'relay' ? '' : 'readonly'}>
              <span id="codexQuickRelayDisclaimer" class="codex-quick-relay-disclaimer" style="${selectedMode === 'relay' ? '' : 'display:none;'}">只做教学和演示专用</span>
            </div>
          </div>
          <div class="config-field codex-quick-key-field">
            <label id="codexQuickApiKeyLabel">${selectedMode === 'relay' ? '中转 API Key' : 'DeepSeek API Key'}</label>
            <div class="input-with-action">
              <input type="password" id="codexQuickApiKey" value="" placeholder="${selectedMode === 'relay' ? '输入中转 API Key' : '输入 DeepSeek API Key'}" autocomplete="off">
              <button type="button" class="btn-open-key" id="codexQuickPasteRelayBtn" title="从剪切板识别 newapi_channel_conn 链接信息">
                <span>粘贴中转</span>
              </button>
              <button type="button" class="btn-open-key" id="codexQuickOpenDeepseekBtn" title="前往 DeepSeek 开放平台获取 API Key">
                <span>获取 Key</span>
                <span aria-hidden="true" class="open-key-arrow">↗</span>
              </button>
            </div>
          </div>
        </div>
      </div>
      <div class="codex-quick-actions">
        <button type="button" class="btn btn-ghost" id="codexQuickFetchModelsBtn">获取模型</button>
        <button type="button" class="btn btn-ghost" id="codexQuickLaterBtn">稍后</button>
        <button type="button" class="btn btn-primary" id="codexQuickApplyBtn">完成配置</button>
      </div>
    </div>`;
  overlay.addEventListener('click', event => {
    if (event.target === overlay) dismissCodexQuickStart();
  });
  document.body.appendChild(overlay);
  window.DexTelemetry?.track?.('quick_config_open', { mode: selectedMode, force: options.force !== false });
  bindCodexQuickModeCards();
  document.getElementById('codexQuickCloseBtn')?.addEventListener('click', dismissCodexQuickStart);
  document.getElementById('codexQuickLaterBtn')?.addEventListener('click', dismissCodexQuickStart);
  document.getElementById('codexQuickFetchModelsBtn')?.addEventListener('click', codexQuickFetchModels);
  document.getElementById('codexQuickApplyBtn')?.addEventListener('click', codexQuickApply);
  document.getElementById('codexQuickOpenDeepseekBtn')?.addEventListener('click', codexQuickOpenDeepseekPlatform);
  document.getElementById('codexQuickPasteRelayBtn')?.addEventListener('click', codexQuickPasteRelayConnection);
  codexQuickUpdateModeFields();
}

function bindCodexQuickModeCards() {
  document.querySelectorAll('.codex-quick-mode-card input').forEach(input => {
    input.addEventListener('change', () => {
      document.querySelectorAll('.codex-quick-mode-card').forEach(card => {
        const radio = card.querySelector('input');
        card.classList.toggle('active', Boolean(radio?.checked));
      });
      codexQuickUpdateModeFields();
    });
  });
}

function codexQuickRelayModels() {
  const status = window.codexQuickStartStatus || {};
  return Array.isArray(status.relay_known_models) && status.relay_known_models.length
    ? status.relay_known_models
    : ['gpt-5.6-sol', 'gpt-5.6-terra', 'gpt-5.6-luna', 'gpt-5.5', 'gpt-5.4', 'gpt-5.4-mini', 'gpt-5.3-codex', 'gpt-5.2'];
}

function codexQuickUpdateModeFields() {
  const mode = codexQuickSelectedMode();
  const status = window.codexQuickStartStatus || {};
  const relay = mode === 'relay';
  const name = document.getElementById('codexQuickAccountName');
  const upstream = document.getElementById('codexQuickUpstream');
  const upstreamLabel = document.getElementById('codexQuickUpstreamLabel');
  const relayDisclaimer = document.getElementById('codexQuickRelayDisclaimer');
  const key = document.getElementById('codexQuickApiKey');
  const keyLabel = document.getElementById('codexQuickApiKeyLabel');
  const modelLabel = document.getElementById('codexQuickDefaultModelLabel');
  const fetchBtn = document.getElementById('codexQuickFetchModelsBtn');
  const openBtn = document.getElementById('codexQuickOpenDeepseekBtn');
  const pasteBtn = document.getElementById('codexQuickPasteRelayBtn');
  if (name && (!name.value || name.value === 'DeepSeek 桌面版' || name.value === 'DEX AI 中转')) name.value = relay ? 'DEX AI 中转' : 'DeepSeek 桌面版';
  if (upstream) {
    upstream.readOnly = !relay;
    upstream.title = relay ? '填写中转服务 Base URL' : 'API/智能路由模式固定使用 DeepSeek Base URL，避免误改';
    if (!relay) {
      upstream.value = codexQuickDeepseekUpstream();
    } else if (!upstream.value || upstream.value === codexQuickDeepseekUpstream() || upstream.value === 'https://api.dexai.vip' || upstream.value === 'https://api.dexai.vip/v1') {
      upstream.value = status.relay_upstream || 'https://api.dexai.vip/v1';
    }
  }
  if (upstreamLabel) upstreamLabel.textContent = relay ? '中转 Base URL' : 'DeepSeek Base URL';
  if (relayDisclaimer) relayDisclaimer.style.display = relay ? '' : 'none';
  if (keyLabel) keyLabel.textContent = relay ? '中转 API Key' : 'DeepSeek API Key';
  if (key) key.placeholder = relay ? '输入中转 API Key' : '输入 DeepSeek API Key';
  if (modelLabel) modelLabel.textContent = relay ? 'Codex 内置 GPT 映射' : '默认模型';
  if (fetchBtn) fetchBtn.style.display = relay ? 'none' : '';
  if (openBtn) {
    openBtn.style.display = '';
    openBtn.title = relay ? '前往 DEX AI 获取中转 API Key' : '前往 DeepSeek 开放平台获取 API Key';
    const label = openBtn.querySelector('span:first-child');
    if (label) label.textContent = relay ? '获取 API Key' : '获取 Key';
  }
  if (pasteBtn) pasteBtn.style.display = relay ? '' : 'none';
  if (relay) {
    const relayModels = codexQuickRelayModels();
    codexQuickSetModels(relayModels, status.relay_default_model || (relayModels.includes('gpt-5.5') ? 'gpt-5.5' : relayModels[0]) || 'gpt-5.5', { preserveExisting: false });
  } else {
    codexQuickSetModels(codexQuickDeepseekModels(), codexQuickDeepseekDefaultModel(), { preserveExisting: false });
  }
}

function codexQuickSelectedMode() {
  return document.querySelector('input[name="codexQuickMode"]:checked')?.value || 'api';
}

function codexQuickCurrentModels() {
  const select = document.getElementById('codexQuickDefaultModel');
  return Array.from(select?.options || []).map(option => option.value).filter(Boolean);
}

function codexQuickSetModels(models, preferred, options = {}) {
  const select = document.getElementById('codexQuickDefaultModel');
  if (!select) return;
  const preserveExisting = options.preserveExisting !== false;
  const existing = codexQuickCurrentModels();
  const merged = [];
  const push = model => {
    model = String(model || '').trim();
    if (model && !merged.includes(model)) merged.push(model);
  };
  push(preferred);
  (models || []).forEach(push);
  if (preserveExisting) existing.forEach(push);
  select.innerHTML = codexQuickModelOptions(merged, preferred || merged[0]);
}

async function codexQuickOpenDeepseekPlatform() {
  if (codexQuickStartBusy) return;
  const relay = codexQuickSelectedMode() === 'relay';
  const url = relay ? 'https://api.dexai.vip' : 'https://platform.deepseek.com/';
  try {
    await invoke('open_external_url', { url });
  } catch (error) {
    showToast((relay ? '打开 DEX AI 控制台失败: ' : '打开 DeepSeek 平台失败: ') + error, 'error');
  }
}

function parseDexRelayConnection(text) {
  const raw = String(text || '').trim();
  if (!raw) throw new Error('剪切板为空');
  let parsed = null;
  try {
    parsed = JSON.parse(raw);
  } catch (_) {
    const match = raw.match(/\{[\s\S]*\}/);
    if (match) parsed = JSON.parse(match[0]);
  }
  if (!parsed || typeof parsed !== 'object') throw new Error('未识别到中转连接 JSON');
  if (parsed._type && parsed._type !== 'newapi_channel_conn') throw new Error('剪切板不是 newapi_channel_conn');
  const url = String(parsed.url || parsed.base_url || parsed.baseUrl || '').trim();
  const key = String(parsed.key || parsed.api_key || parsed.apiKey || '').trim();
  if (!url || !key) throw new Error('连接信息缺少 url 或 key');
  return { url, key };
}

async function codexQuickPasteRelayConnection() {
  try {
    const text = await navigator.clipboard.readText();
    const conn = parseDexRelayConnection(text);
    const urlInput = document.getElementById('codexQuickUpstream');
    const keyInput = document.getElementById('codexQuickApiKey');
    if (urlInput) urlInput.value = conn.url;
    if (keyInput) keyInput.value = conn.key;
    const relayRadio = document.querySelector('input[name="codexQuickMode"][value="relay"]');
    if (relayRadio) {
      relayRadio.checked = true;
      document.querySelectorAll('.codex-quick-mode-card').forEach(card => {
        const radio = card.querySelector('input');
        card.classList.toggle('active', Boolean(radio?.checked));
      });
    }
    codexQuickUpdateModeFields();
    showToast('已从剪切板填入中转 URL 和 API Key', 'success');
  } catch (error) {
    showToast('剪切板识别失败: ' + error.message, 'error');
  }
}

async function codexQuickFetchModels() {
  if (codexQuickStartBusy) return;
  const btn = document.getElementById('codexQuickFetchModelsBtn');
  const upstream = codexQuickSelectedMode() === 'relay'
    ? (document.getElementById('codexQuickUpstream')?.value?.trim() || '')
    : codexQuickDeepseekUpstream();
  const apiKey = document.getElementById('codexQuickApiKey')?.value?.trim() || '';
  if (!upstream) { showToast('DeepSeek Base URL 不能为空', 'error'); return; }
  if (!apiKey) { showToast('DeepSeek API Key 不能为空', 'error'); return; }
  codexQuickStartBusy = true;
  if (btn) { btn.disabled = true; btn.textContent = '获取中'; }
  try {
    const models = await invoke('fetch_upstream_models', {
      accountId: null,
      upstream,
      apiKey,
      endpointKind: 'OpenAiChat',
    });
    codexQuickSetModels(models, models?.includes('deepseek-v4-pro') ? 'deepseek-v4-pro' : models?.[0]);
    showToast(`已获取 ${Array.isArray(models) ? models.length : 0} 个模型`, 'success');
  } catch (error) {
    showToast('获取模型失败: ' + error, 'error');
  } finally {
    codexQuickStartBusy = false;
    if (btn) { btn.disabled = false; btn.textContent = '获取模型'; }
  }
}

async function codexQuickApply() {
  if (codexQuickStartBusy) return;
  const btn = document.getElementById('codexQuickApplyBtn');
  const selectedMode = codexQuickSelectedMode();
  let defaultModel = document.getElementById('codexQuickDefaultModel')?.value || 'deepseek-v4-pro';
  if (selectedMode !== 'relay' && /^gpt-/i.test(String(defaultModel || '').trim())) {
    defaultModel = codexQuickDeepseekDefaultModel();
  }
  const payload = {
    mode: selectedMode,
    account_name: document.getElementById('codexQuickAccountName')?.value || 'DeepSeek 桌面版',
    upstream: selectedMode === 'relay'
      ? (document.getElementById('codexQuickUpstream')?.value || '')
      : codexQuickDeepseekUpstream(),
    api_key: document.getElementById('codexQuickApiKey')?.value || '',
    default_model: defaultModel,
    known_models: selectedMode === 'relay'
      ? codexQuickCurrentModels()
      : codexQuickCurrentModels().filter(model => !/^gpt-/i.test(String(model || '').trim())),
  };
  if (!payload.upstream.trim()) { showToast(payload.mode === 'relay' ? '中转 Base URL 不能为空' : 'DeepSeek Base URL 不能为空', 'error'); return; }
  if (!payload.api_key.trim()) { showToast(payload.mode === 'relay' ? '中转 API Key 不能为空' : 'DeepSeek API Key 不能为空', 'error'); return; }
  codexQuickStartBusy = true;
  if (btn) { btn.disabled = true; btn.textContent = '配置中'; }
  try {
    window.DexTelemetry?.track?.('quick_config_apply', { mode: selectedMode, model: defaultModel });
    const result = await invoke('apply_codex_quick_start', {
      requestJson: JSON.stringify(payload),
    });
    closeCodexQuickStartModal();
    codexQuickStartDismissed = true;
    await Promise.all([
      typeof loadConfig === 'function' ? loadConfig() : Promise.resolve(),
      typeof loadAccountsData === 'function' ? loadAccountsData() : Promise.resolve(),
      typeof loadStatus === 'function' ? loadStatus() : Promise.resolve(),
    ]);
    if (typeof renderPanel === 'function' && currentPanel === 'status') renderPanel('status');
    if (typeof refreshClientLifecycleDock === 'function') await refreshClientLifecycleDock();
    const serviceError = result?.service?.error;
    showToast(serviceError ? `配置已保存，服务启动失败: ${serviceError}` : (result?.message || 'Codex Desktop 已配置'), serviceError ? 'info' : 'success');
    maybeShowSupportAfterQuickStart();
  } catch (error) {
    showToast('快速配置失败: ' + error, 'error');
  } finally {
    codexQuickStartBusy = false;
    if (btn) { btn.disabled = false; btn.textContent = '完成配置'; }
  }
}

function maybeShowSupportAfterQuickStart() {
  const key = 'deecodex.support.afterQuickStartShown';
  if (deeStorage?.getItem?.(key) === '1') return;
  deeStorage?.setItem?.(key, '1');
  if (typeof showSupportProjectNudge === 'function') {
    showSupportProjectNudge();
  }
}

window.checkSetupWizard = checkSetupWizard;
window.openCodexQuickStart = openCodexQuickStart;
window.openCodexRelayStart = openCodexRelayStart;
window.closeCodexQuickStartModal = closeCodexQuickStartModal;
window.parseDexRelayConnection = parseDexRelayConnection;
