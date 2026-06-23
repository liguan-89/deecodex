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
  hideWizardBar();
  try {
    const status = await invoke('get_codex_quick_start_status');
    codexQuickStartStatus = status || null;
    window.codexQuickStartStatus = codexQuickStartStatus;
    showCodexQuickStartModal(status, { force: force !== false });
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
  const existing = document.getElementById('setupWizard');
  if (existing) existing.remove();
  const main = document.getElementById('mainContent');
  if (main) main.style.paddingTop = '';
}

function showWizardBar() {
  hideWizardBar();
}

function autoWizardStep() {
  // 旧顶部引导条已移除，保留空函数兼容旧调用。
}

function renderCodexQuickModeCard(mode, status, selectedMode) {
  const isSmart = mode === 'smart';
  const disabled = isSmart && !status?.has_official_login;
  const title = isSmart ? '智能路由' : 'API 模式';
  const desc = isSmart
    ? '检测到 Codex 登录态时使用；DeepSeek 负责常规任务，官方登录态承接原生工具。'
    : '不依赖官方登录态；Codex Desktop 直接使用 DeepSeek 账号模型。';
  const badge = isSmart
    ? (status?.has_official_login ? '已发现登录态' : '需要先登录')
    : '最少配置';
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

function showCodexQuickStartModal(status, options) {
  closeCodexQuickStartModal();
  status = status || {};
  const force = options?.force === true;
  if (!force && status.completed) return;
  const recommendedMode = status.recommended_mode || (status.has_official_login ? 'smart' : 'api');
  const selectedMode = recommendedMode === 'smart' && status.has_official_login ? 'smart' : 'api';
  const models = Array.isArray(status.known_models) ? status.known_models : [];
  const defaultModel = status.default_model || models[0] || 'deepseek-v4-pro';
  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay codex-quick-overlay';
  overlay.id = 'codexQuickStartModal';
  overlay.innerHTML = `
    <div class="modal-box codex-quick-box">
      <div class="modal-header codex-quick-header">
        <div>
          <h3>开始使用 Codex Desktop</h3>
          <p>用 DeepSeek 完成最小配置，复杂设置之后再调。</p>
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
          ${renderCodexQuickModeCard('smart', status, selectedMode)}
        </div>

        <div class="config-fields codex-quick-fields">
          <div class="config-field">
            <label>账号名称</label>
            <input type="text" id="codexQuickAccountName" value="DeepSeek 桌面版" placeholder="DeepSeek 桌面版">
          </div>
          <div class="config-field">
            <label>默认模型</label>
            <select id="codexQuickDefaultModel">
              ${codexQuickModelOptions(models, defaultModel)}
            </select>
          </div>
          <div class="config-field wide">
            <label>DeepSeek Base URL</label>
            <input type="text" id="codexQuickUpstream" value="${escAttr(status.upstream || 'https://api.deepseek.com/v1')}" placeholder="https://api.deepseek.com/v1">
          </div>
          <div class="config-field wide">
            <label>DeepSeek API Key</label>
            <input type="password" id="codexQuickApiKey" value="" placeholder="输入 DeepSeek API Key" autocomplete="off">
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
  bindCodexQuickModeCards();
  document.getElementById('codexQuickCloseBtn')?.addEventListener('click', dismissCodexQuickStart);
  document.getElementById('codexQuickLaterBtn')?.addEventListener('click', dismissCodexQuickStart);
  document.getElementById('codexQuickFetchModelsBtn')?.addEventListener('click', codexQuickFetchModels);
  document.getElementById('codexQuickApplyBtn')?.addEventListener('click', codexQuickApply);
}

function bindCodexQuickModeCards() {
  document.querySelectorAll('.codex-quick-mode-card input').forEach(input => {
    input.addEventListener('change', () => {
      document.querySelectorAll('.codex-quick-mode-card').forEach(card => {
        const radio = card.querySelector('input');
        card.classList.toggle('active', Boolean(radio?.checked));
      });
    });
  });
}

function codexQuickSelectedMode() {
  return document.querySelector('input[name="codexQuickMode"]:checked')?.value || 'api';
}

function codexQuickCurrentModels() {
  const select = document.getElementById('codexQuickDefaultModel');
  return Array.from(select?.options || []).map(option => option.value).filter(Boolean);
}

function codexQuickSetModels(models, preferred) {
  const select = document.getElementById('codexQuickDefaultModel');
  if (!select) return;
  const existing = codexQuickCurrentModels();
  const merged = [];
  const push = model => {
    model = String(model || '').trim();
    if (model && !merged.includes(model)) merged.push(model);
  };
  push(preferred);
  (models || []).forEach(push);
  existing.forEach(push);
  select.innerHTML = codexQuickModelOptions(merged, preferred || merged[0]);
}

async function codexQuickFetchModels() {
  if (codexQuickStartBusy) return;
  const btn = document.getElementById('codexQuickFetchModelsBtn');
  const upstream = document.getElementById('codexQuickUpstream')?.value?.trim() || '';
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
  const payload = {
    mode: codexQuickSelectedMode(),
    account_name: document.getElementById('codexQuickAccountName')?.value || 'DeepSeek 桌面版',
    upstream: document.getElementById('codexQuickUpstream')?.value || '',
    api_key: document.getElementById('codexQuickApiKey')?.value || '',
    default_model: document.getElementById('codexQuickDefaultModel')?.value || 'deepseek-v4-pro',
    known_models: codexQuickCurrentModels(),
  };
  if (!payload.upstream.trim()) { showToast('DeepSeek Base URL 不能为空', 'error'); return; }
  if (!payload.api_key.trim()) { showToast('DeepSeek API Key 不能为空', 'error'); return; }
  codexQuickStartBusy = true;
  if (btn) { btn.disabled = true; btn.textContent = '配置中'; }
  try {
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
window.closeCodexQuickStartModal = closeCodexQuickStartModal;
