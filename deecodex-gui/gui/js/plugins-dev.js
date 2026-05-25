// 插件开发入口
function pluginTemplateItems() {
  return (_pluginMarketplaceData || []).filter(item => item.template);
}

function renderPluginDevBar() {
  const templates = pluginTemplateItems();
  const options = templates.length
    ? templates.map(item => `<option value="${escAttr(item.id)}" ${_pluginDevDraft.templateId === item.id ? 'selected' : ''}>${esc(item.name || item.id)}</option>`).join('')
    : '<option value="">暂无模板</option>';
  return `<div class="plugin-dev-entry" id="pluginDevEntry">
    <div class="plugin-dev-head">
      <button class="plugin-dev-toggle ${_pluginDevOpen ? 'active' : ''}" type="button" onclick="togglePluginDevBar()" aria-expanded="${_pluginDevOpen ? 'true' : 'false'}">
        <strong>开发入口</strong>
        <span>模板创建 / 校验 / 打包</span>
        <em>${_pluginDevOpen ? '收起' : '展开'}</em>
      </button>
      <button class="btn btn-ghost" type="button" onclick="openPluginMarketplaceRoot()">市场目录</button>
    </div>
    ${_pluginDevOpen ? `<div class="plugin-dev-bar">
      <div class="plugin-dev-group plugin-dev-create">
        <span class="plugin-dev-label">创建</span>
        <select class="history-select" id="pluginTemplateSelect" ${templates.length ? '' : 'disabled'}>
          ${options}
        </select>
        <input id="pluginDevId" placeholder="插件 ID" value="${escAttr(_pluginDevDraft.pluginId)}">
        <input id="pluginDevName" placeholder="插件名称" value="${escAttr(_pluginDevDraft.name)}">
        <input id="pluginDevRoot" placeholder="创建到目录" value="${escAttr(_pluginDevDraft.root)}">
        <button class="btn btn-ghost" onclick="browsePluginDevRoot()">目录</button>
        <button class="btn btn-primary" onclick="createPluginFromTemplate()">创建</button>
      </div>
      <div class="plugin-dev-group plugin-dev-tools">
        <span class="plugin-dev-label">检查</span>
        <input id="pluginDevPath" placeholder="插件目录或包路径" value="${escAttr(_pluginDevDraft.path)}">
        <button class="btn btn-ghost" onclick="browsePluginDevPath()">目录</button>
        <button class="btn btn-ghost" onclick="browsePluginDevPackage()">包</button>
        <button class="btn btn-ghost" onclick="validatePluginDevPath()">校验</button>
        <button class="btn btn-ghost" onclick="packagePluginDevPath()">打包</button>
        <button class="btn btn-ghost" onclick="openPluginDevPath()">打开</button>
      </div>
    </div>` : ''}
  </div>`;
}

function pluginDevInputValue(id) {
  const el = document.getElementById(id);
  return el ? String(el.value || '').trim() : '';
}

function pluginDevMaybeValue(id, fallback) {
  const el = document.getElementById(id);
  return el ? String(el.value || '').trim() : (fallback || '');
}

function capturePluginDevDraft() {
  _pluginDevDraft = {
    templateId: pluginDevMaybeValue('pluginTemplateSelect', _pluginDevDraft.templateId),
    pluginId: pluginDevMaybeValue('pluginDevId', _pluginDevDraft.pluginId),
    name: pluginDevMaybeValue('pluginDevName', _pluginDevDraft.name),
    root: pluginDevMaybeValue('pluginDevRoot', _pluginDevDraft.root),
    path: pluginDevMaybeValue('pluginDevPath', _pluginDevDraft.path)
  };
}

function rememberPluginDevValue(id, value) {
  const next = value || '';
  if (id === 'pluginTemplateSelect') _pluginDevDraft.templateId = next;
  if (id === 'pluginDevId') _pluginDevDraft.pluginId = next;
  if (id === 'pluginDevName') _pluginDevDraft.name = next;
  if (id === 'pluginDevRoot') _pluginDevDraft.root = next;
  if (id === 'pluginDevPath') _pluginDevDraft.path = next;
}

function setPluginDevInputValue(id, value) {
  const el = document.getElementById(id);
  if (el) el.value = value || '';
  rememberPluginDevValue(id, value);
}

function showPluginDevResult(title, result) {
  var existing = document.getElementById('pluginDevResultModal');
  if (existing) existing.remove();
  const manifest = result && (result.manifest || (result.preview && result.preview.manifest));
  const compat = result && (result.compatibility || (result.preview && result.preview.compatibility));
  const rows = result && result.ok
    ? [
        ['状态', compat && compat.label ? compat.label : '通过'],
        ['插件', manifest ? `${manifest.name || manifest.id} v${manifest.version || '-'}` : '-'],
        ['ID', manifest ? manifest.id || '-' : '-'],
        ['类型', manifest ? pluginKindLabel(manifest) : '-'],
        ['路径', result.path || (result.preview && result.preview.source_path) || '-']
      ]
    : [
        ['状态', '失败'],
        ['原因', result && result.error ? result.error : '未知错误'],
        ['路径', result && result.path ? result.path : '-']
      ];
  const checks = compat && Array.isArray(compat.checks) ? compat.checks : [];
  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay';
  overlay.id = 'pluginDevResultModal';
  overlay.innerHTML = `<div class="modal-box plugin-action-modal">
    <div class="modal-header"><h3>${esc(title || '插件开发')}</h3></div>
    <div class="modal-body plugin-action-body">
      <div class="plugin-source-grid">${rows.map(row => `<span>${esc(row[0])}</span><code>${esc(row[1])}</code>`).join('')}</div>
      ${checks.length ? `<div class="plugin-compat-check-list plugin-dev-checks">${checks.map(check => `<div class="plugin-compat-check-row ${escAttr(check.tone || 'muted')}">
        <span>${esc(check.label || '-')}</span>
        <strong>${esc(check.value || '-')}</strong>
      </div>`).join('')}</div>` : ''}
    </div>
    <div class="plugin-preview-actions">
      <button class="btn btn-primary" id="pluginDevResultClose" type="button">完成</button>
    </div>
  </div>`;
  document.body.appendChild(overlay);
  const close = document.getElementById('pluginDevResultClose');
  const cleanup = function () { overlay.remove(); };
  if (close) close.onclick = cleanup;
  overlay.addEventListener('click', function (e) { if (e.target === overlay) cleanup(); });
}

async function browsePluginDevRoot() {
  try {
    const path = await invoke('browse_plugin_directory');
    if (path) setPluginDevInputValue('pluginDevRoot', path);
  } catch(e) {
    showToast('目录选择失败: ' + esc(String(e)), 'error');
  }
}

async function browsePluginDevPath() {
  try {
    const path = await invoke('browse_plugin_directory');
    if (path) setPluginDevInputValue('pluginDevPath', path);
  } catch(e) {
    showToast('插件目录选择失败: ' + esc(String(e)), 'error');
  }
}

async function browsePluginDevPackage() {
  try {
    const path = await invoke('browse_plugin_package');
    if (path) setPluginDevInputValue('pluginDevPath', path);
  } catch(e) {
    showToast('插件包选择失败: ' + esc(String(e)), 'error');
  }
}

function togglePluginDevBar() {
  capturePluginDevDraft();
  _pluginDevOpen = !_pluginDevOpen;
  if (window.deeStorage) {
    window.deeStorage.setItem('deecodex.pluginDevOpen', _pluginDevOpen ? '1' : '0');
  }
  const entry = document.getElementById('pluginDevEntry');
  if (entry) entry.outerHTML = renderPluginDevBar();
}

async function createPluginFromTemplate() {
  const templateId = pluginDevInputValue('pluginTemplateSelect');
  const pluginId = pluginDevInputValue('pluginDevId');
  const name = pluginDevInputValue('pluginDevName');
  const destinationDir = pluginDevInputValue('pluginDevRoot');
  if (!templateId || !pluginId || !name || !destinationDir) {
    showToast('请补全模板、插件 ID、名称和目录', 'error');
    return;
  }
  try {
    const result = await invoke('create_plugin_from_template', {
      templateId,
      pluginId,
      name,
      destinationDir
    });
    setPluginDevInputValue('pluginDevPath', result.path || '');
    showToast('插件草稿已创建', 'success');
    showPluginDevResult('插件草稿', result || {});
    await loadPluginsData();
  } catch(e) {
    showToast('创建失败: ' + esc(String(e)), 'error');
  }
}

async function validatePluginDevPath() {
  const path = pluginDevInputValue('pluginDevPath') || pluginDevInputValue('pluginZipPath');
  if (!path) {
    showToast('请选择插件目录或插件包', 'error');
    return;
  }
  try {
    const result = await invoke('validate_plugin_path', { path });
    showPluginDevResult('插件校验', result || {});
    showToast(result && result.ok ? '插件校验通过' : '插件校验失败', result && result.ok ? 'success' : 'error');
  } catch(e) {
    showToast('校验失败: ' + esc(String(e)), 'error');
  }
}

async function packagePluginDevPath() {
  const path = pluginDevInputValue('pluginDevPath');
  if (!path) {
    showToast('请选择插件目录', 'error');
    return;
  }
  try {
    const result = await invoke('package_plugin_directory', { path });
    if (result && result.path) setPluginDevInputValue('pluginZipPath', result.path);
    showToast('插件包已生成', 'success');
    showPluginDevResult('插件打包', result || {});
  } catch(e) {
    showToast('打包失败: ' + esc(String(e)), 'error');
  }
}

async function openPluginDevPath() {
  const path = pluginDevInputValue('pluginDevPath') || pluginDevInputValue('pluginZipPath');
  if (!path) {
    showToast('请选择要打开的路径', 'error');
    return;
  }
  try {
    await invoke('open_plugin_directory', { path });
  } catch(e) {
    showToast('打开失败: ' + esc(String(e)), 'error');
  }
}

async function openPluginMarketplaceRoot() {
  try {
    const result = await invoke('open_plugin_marketplace_directory');
    if (result && result.path) {
      showToast('已打开个人插件市场目录', 'success');
    }
  } catch(e) {
    showToast('打开市场目录失败: ' + esc(String(e)), 'error');
  }
}
