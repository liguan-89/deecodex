let _threadsData = null;
let _currentThread = null;
let selectedThreadClientKind = 'all';

const THREAD_CLIENT_LABELS = {
  all: '全部',
  codex: 'Codex',
  claude_code: 'Claude',
  openclaw: 'OpenClaw',
  hermes: 'Hermes',
  generic_client: '通用客户端',
};

function normalizeThreadClientKind(kind) {
  const value = String(kind || 'codex');
  if (value === 'ClaudeCode') return 'claude_code';
  if (value === 'Openclaw') return 'openclaw';
  if (value === 'GenericClient') return 'generic_client';
  if (value === 'Codex') return 'codex';
  if (value === 'Hermes') return 'hermes';
  return ['codex', 'claude_code', 'openclaw', 'hermes', 'generic_client'].includes(value) ? value : 'codex';
}

function threadClientLabel(kind) {
  const normalized = normalizeThreadClientKind(kind);
  return THREAD_CLIENT_LABELS[normalized] || normalized;
}

function threadClientGlyph(kind) {
  const normalized = normalizeThreadClientKind(kind);
  const glyphs = { codex: 'C', claude_code: 'Cl', openclaw: 'O', hermes: 'H', generic_client: 'G' };
  return `<span class="thread-client-glyph thread-client-${escAttr(normalized)}">${esc(glyphs[normalized] || '?')}</span>`;
}

function renderThreads() {
  return `<div class="page-header">
    <h2>线程中心</h2>
  </div>
  <div class="threads-console">
    <div class="threads-summary" id="threadsSummary">
      <div class="threads-stat"><span class="stat-label">总线程</span><span class="stat-value">—</span></div>
      <div class="threads-stat"><span class="stat-label">客户端</span><span class="stat-value">—</span></div>
      <div class="threads-stat other"><span class="stat-label">可读源</span><span class="stat-value">—</span></div>
      <div class="threads-stat migrated"><span class="stat-label">筛选</span><span class="stat-value">全部</span></div>
    </div>
    <div class="threads-actions">
      <button class="btn btn-ghost" onclick="refreshThreads()">刷新</button>
    </div>
  </div>
  <div id="threadClientSwitcher" class="thread-source-switcher"></div>
  <div id="codexThreadActions" class="codex-thread-actions"></div>
  <div id="threadSourceDiagnostics"></div>
  <div class="threads-list-head">线程列表</div>
  <div class="threads-table-wrap">
    <table class="threads-table">
      <thead><tr><th>标题</th><th>客户端</th><th>模型/Provider</th><th>更新时间</th><th>线程 ID</th></tr></thead>
      <tbody id="threadsTableBody"><tr><td colspan="5" style="text-align:center;color:var(--text-muted);">加载中...</td></tr></tbody>
    </table>
  </div>`;
}

async function refreshThreads() {
  try {
    const [unified, codexStatus] = await Promise.all([
      invoke('list_client_threads'),
      invoke('get_threads_status').catch(err => ({ error: String(err) })),
    ]);
    const sources = Array.isArray(unified?.sources) ? unified.sources : [];
    const list = Array.isArray(unified?.threads) ? unified.threads : [];
    _threadsData = { sources, list, codexStatus };

    const cards = document.querySelectorAll('#threadsSummary .stat-value');
    if (cards.length >= 4) {
      cards[0].textContent = unified?.total ?? list.length;
      cards[1].textContent = sources.length;
      cards[2].textContent = sources.filter(s => s.available).length;
      cards[3].textContent = selectedThreadClientKind === 'all' ? '全部' : threadClientLabel(selectedThreadClientKind);
    }

    const switcher = document.getElementById('threadClientSwitcher');
    if (switcher) switcher.innerHTML = renderThreadClientSwitcher(sources, list.length);

    const codexActions = document.getElementById('codexThreadActions');
    if (codexActions) codexActions.innerHTML = renderCodexThreadActions(codexStatus);

    const diagnostics = document.getElementById('threadSourceDiagnostics');
    if (diagnostics) diagnostics.innerHTML = renderThreadSourceDiagnostics(sources);

    const tbody = document.getElementById('threadsTableBody');
    if (tbody) tbody.innerHTML = renderThreadRows(filteredThreadList(list));
  } catch (err) {
    showToast('加载线程数据失败: ' + err, 'error');
    const tbody = document.getElementById('threadsTableBody');
    if (tbody) tbody.innerHTML = '<tr><td colspan="5" class="threads-error-cell">加载失败: ' + esc(String(err)) + '</td></tr>';
  }
}

function filteredThreadList(list) {
  if (selectedThreadClientKind === 'all') return list;
  return list.filter(t => normalizeThreadClientKind(t.client_kind) === selectedThreadClientKind);
}

function renderThreadClientSwitcher(sources, total) {
  const allActive = selectedThreadClientKind === 'all' ? ' active' : '';
  const allButton = `<button type="button" class="thread-source-tab${allActive}" onclick="selectThreadClient('all')">
    <span class="thread-client-glyph thread-client-all">All</span>
    <span>全部</span>
    <em>${Number(total || 0)}</em>
  </button>`;
  const sourceButtons = sources.map(source => {
    const kind = normalizeThreadClientKind(source.client_kind);
    const active = selectedThreadClientKind === kind ? ' active' : '';
    const issueClass = Array.isArray(source.diagnostics) && source.diagnostics.length ? ' has-issues' : '';
    const title = Array.isArray(source.scan_paths) ? source.scan_paths.join('\n') : '';
    return `<button type="button" class="thread-source-tab${active}${issueClass}" onclick="selectThreadClient('${escAttr(kind)}')" title="${escAttr(title)}">
      ${threadClientGlyph(kind)}
      <span>${esc(threadClientLabel(kind))}</span>
      <em>${Number(source.count || 0)}</em>
    </button>`;
  }).join('');
  return allButton + sourceButtons;
}

function renderCodexThreadActions(status) {
  if (!status || status.error) {
    return `<div class="codex-thread-muted">Codex 专属操作不可用${status?.error ? ': ' + esc(status.error) : ''}</div>`;
  }
  const migrateHidden = status.calibration_needed ? ' style="display:none;"' : '';
  const restoreHidden = status.calibration_needed ? ' style="display:none;"' : '';
  const migrateDisabled = status.migrated || status.non_unified_count === 0 ? ' disabled' : '';
  const restoreDisabled = !status.migrated ? ' disabled' : '';
  const calibrateStyle = status.calibration_needed ? '' : ' style="display:none;"';
  const active = status.calibration_needed ? '需要校准' : (status.active_provider || '—');
  return `<div class="codex-thread-tools">
    <span class="codex-thread-label">Codex 专属操作</span>
    <span class="tag tag-current">归属: ${esc(active)}</span>
    <span class="tag tag-other">待统一: ${Number(status.non_unified_count || 0)}</span>
    <button class="btn btn-primary" id="btnMigrate" onclick="doMigrate()"${migrateHidden}${migrateDisabled}>聚合 Codex 线程</button>
    <button class="btn btn-ghost" id="btnRestore" onclick="doRestore()"${restoreHidden}${restoreDisabled}>还原 Codex 隔离</button>
    <button class="btn btn-warning" id="btnCalibrate" onclick="doCalibrate()"${calibrateStyle}>校准 Codex</button>
  </div>`;
}

function renderThreadSourceDiagnostics(sources) {
  const rows = sources
    .filter(source => Array.isArray(source.diagnostics) && source.diagnostics.length)
    .map(source => `<div class="thread-source-note">
      <strong>${esc(threadClientLabel(source.client_kind))}</strong>
      <span>${source.diagnostics.map(item => esc(item)).join('；')}</span>
    </div>`);
  return rows.length ? `<div class="thread-source-diagnostics">${rows.join('')}</div>` : '';
}

function renderThreadRows(list) {
  if (!list || list.length === 0) {
    return '<tr><td colspan="5" style="text-align:center;color:var(--text-muted);">无线程数据</td></tr>';
  }
  return list.map(t => {
    const kind = normalizeThreadClientKind(t.client_kind);
    const provider = t.model || t.provider || '—';
    const time = formatThreadTime(t.updated_at_ms || t.created_at_ms);
    const messageCount = Number(t.message_count || 0);
    const meta = messageCount ? `<span class="thread-meta-line">${messageCount} 条消息</span>` : '';
    const deleteAction = t.delete_available
      ? `<span class="tag-delete" onclick="event.stopPropagation();deleteThreadRow('${escAttr(kind)}','${escAttr(t.native_id)}')">删除</span>`
      : '';
    const rowClass = t.detail_available ? 'thread-row' : 'thread-row thread-row-muted';
    return `<tr class="${rowClass}" onclick="openThread('${escAttr(kind)}','${escAttr(t.native_id)}')">
      <td title="${escAttr(t.title)}"><span class="td-title-text">${esc(t.title || '(无标题)')}</span>${meta}${deleteAction}</td>
      <td><span class="tag tag-current">${esc(threadClientLabel(kind))}</span></td>
      <td title="${escAttr(provider)}">${esc(provider)}</td>
      <td>${esc(time)}</td>
      <td title="${escAttr(t.native_id)}">${esc(trunc(t.native_id || '', 18))}</td>
    </tr>`;
  }).join('');
}

function selectThreadClient(kind) {
  selectedThreadClientKind = kind === 'all' ? 'all' : normalizeThreadClientKind(kind);
  if (_threadsData) {
    const cards = document.querySelectorAll('#threadsSummary .stat-value');
    if (cards.length >= 4) cards[3].textContent = selectedThreadClientKind === 'all' ? '全部' : threadClientLabel(selectedThreadClientKind);
    const switcher = document.getElementById('threadClientSwitcher');
    if (switcher) switcher.innerHTML = renderThreadClientSwitcher(_threadsData.sources, _threadsData.list.length);
    const tbody = document.getElementById('threadsTableBody');
    if (tbody) tbody.innerHTML = renderThreadRows(filteredThreadList(_threadsData.list));
  }
}

function formatThreadTime(value) {
  const ms = Number(value || 0);
  return ms ? new Date(ms).toLocaleString('zh-CN') : '—';
}

async function doMigrate() {
  if (!await showConfirm('确定要聚合 Codex 线程吗？\n\n只会修改 Codex 本地 state SQLite 中的 provider 归属；其他客户端历史不会被改写。')) return;

  const btn = document.getElementById('btnMigrate');
  if (btn) btn.disabled = true;
  showToast('迁移中...', 'info');

  try {
    const diff = await invoke('migrate_threads');
    showToast(`已迁移 ${diff.changed_count} 条 Codex 线程`, 'success');
    await refreshThreads();
  } catch (err) {
    showToast('迁移失败: ' + err, 'error');
  } finally {
    if (btn) btn.disabled = false;
  }
}

async function doRestore() {
  if (!await showConfirm('确定要还原 Codex 隔离模式吗？\n\n只会从备份恢复 Codex 线程的原始 provider。')) return;

  const btn = document.getElementById('btnRestore');
  if (btn) btn.disabled = true;
  showToast('还原中...', 'info');

  try {
    const diff = await invoke('restore_threads');
    showToast(`已还原 ${diff.changed_count} 条 Codex 线程`, 'success');
    await refreshThreads();
  } catch (err) {
    showToast('还原失败: ' + err, 'error');
  } finally {
    if (btn) btn.disabled = false;
  }
}

async function doCalibrate() {
  const activeProvider = (_threadsData && _threadsData.codexStatus && _threadsData.codexStatus.active_provider) || '';
  if (!await showConfirm(`检测到 Codex 活跃 provider 已变更，需要重新校准。\n\n将校准 Codex 备份并继续聚合到「${activeProvider}」。`)) return;

  const btn = document.getElementById('btnCalibrate');
  if (btn) btn.disabled = true;
  showToast('校准中...', 'info');

  try {
    const diff = await invoke('calibrate_threads');
    showToast(`已校准 ${diff.changed_count} 条 Codex 线程`, 'success');
    await refreshThreads();
  } catch (err) {
    showToast('校准失败: ' + err, 'error');
  } finally {
    if (btn) btn.disabled = false;
  }
}

// ── 线程详情面板 ──

function openThread(clientKind, nativeId) {
  const kind = normalizeThreadClientKind(clientKind);
  _currentThread = { clientKind: kind, nativeId };
  const container = document.getElementById('mainContent');
  if (!container) return;
  container.innerHTML = `<div class="detail-panel">
    <div class="detail-header">
      <button class="detail-back-btn" onclick="closeThreadDetail()">← 返回</button>
      <h2 id="detailTitle">加载中...</h2>
      <button class="detail-delete-btn" id="detailDeleteBtn" style="display:none;" onclick="deleteThreadFromDetail()">删除</button>
    </div>
    <div class="detail-messages" id="detailMessages">
      <div class="detail-loading">加载中...</div>
    </div>
  </div>`;

  invoke('get_client_thread_content', { clientKind: kind, nativeId })
    .then(content => {
      const thread = content.thread || {};
      const titleEl = document.getElementById('detailTitle');
      if (titleEl) titleEl.textContent = thread.title || '(无标题)';
      const deleteBtn = document.getElementById('detailDeleteBtn');
      if (deleteBtn) deleteBtn.style.display = thread.delete_available ? '' : 'none';
      const msgsEl = document.getElementById('detailMessages');
      if (!msgsEl) return;
      if (!content.messages || content.messages.length === 0) {
        msgsEl.innerHTML = '<div class="detail-loading">此会话无对话内容</div>';
        return;
      }
      msgsEl.innerHTML = content.messages.map(renderMessage).join('');
    })
    .catch(err => {
      const msgsEl = document.getElementById('detailMessages');
      if (msgsEl) msgsEl.innerHTML = `<div class="detail-loading" style="color:var(--red);">加载失败: ${esc(String(err))}</div>`;
    });
}

function closeThreadDetail() {
  _currentThread = null;
  const container = document.getElementById('mainContent');
  if (container) container.innerHTML = renderThreads();
  refreshThreads();
}

function renderMessage(msg) {
  const role = (msg.payload && msg.payload.role) || msg.role || (msg.type || 'system');
  const roleClass = ['user', 'assistant', 'developer', 'tool'].includes(role) ? role : 'system';

  let body = '';
  if (msg.payload && msg.payload.content) {
    if (Array.isArray(msg.payload.content)) {
      body = msg.payload.content
        .filter(c => c.type === 'input_text' || c.type === 'output_text' || c.type === 'text')
        .map(c => c.text || '')
        .join('\n');
    } else if (typeof msg.payload.content === 'string') {
      body = msg.payload.content;
    }
  }
  if (!body && msg.payload && msg.payload.text) {
    body = msg.payload.text;
  }
  if (!body) {
    try { body = JSON.stringify(msg.payload || msg, null, 2); } catch (_) { body = '(无法解析)'; }
  }

  const truncated = body.length > 500;
  const expandBtn = truncated
    ? `<button class="msg-expand-btn" onclick="this.previousElementSibling.classList.toggle('expanded');this.textContent=this.previousElementSibling.classList.contains('expanded')?'收起':'展开全部';">展开全部</button>`
    : '';

  return `<div class="msg-bubble ${roleClass}">
    <div class="msg-role">${esc(role)}</div>
    <div class="msg-body">${esc(body)}</div>
    ${expandBtn}
  </div>`;
}

async function deleteThreadFromDetail() {
  if (!_currentThread || _currentThread.clientKind !== 'codex') {
    showToast('只有 Codex 线程支持从这里删除', 'error');
    return;
  }
  if (!await showConfirm('确定要永久删除此 Codex 线程吗？\n\n此操作不可恢复，线程将从 SQLite、会话文件和迁移备份中同时移除。')) return;
  try {
    await invoke('delete_thread', { threadId: _currentThread.nativeId });
    showToast('线程已永久删除', 'success');
    closeThreadDetail();
  } catch (err) {
    showToast('删除失败: ' + err, 'error');
  }
}

async function deleteThreadRow(clientKind, nativeId) {
  const kind = normalizeThreadClientKind(clientKind);
  if (kind !== 'codex') {
    showToast('只有 Codex 线程支持从这里删除', 'error');
    return;
  }
  if (!await showConfirm('确定要永久删除此 Codex 线程吗？\n\n此操作不可恢复，线程将从 SQLite、会话文件和迁移备份中同时移除。')) return;
  try {
    await invoke('delete_thread', { threadId: nativeId });
    showToast('线程已永久删除', 'success');
    refreshThreads();
  } catch (err) {
    showToast('删除失败: ' + err, 'error');
  }
}
