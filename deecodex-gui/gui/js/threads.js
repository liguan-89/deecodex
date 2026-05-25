let _threadsData = null;
let _currentThread = null;
let selectedThreadClientKind = 'all';

const THREAD_CLIENT_LABELS = {
  all: '全部',
  codex: 'Codex',
  claude_code: 'Claude',
  openclaw: 'OpenClaw',
  hermes: 'Hermes',
  generic_client: '通用',
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

function threadProviderLogoSrc(p) {
  const files = {
    codex: 'codex.png',
    'claude-code': 'claude-code.png',
    openclaw: 'openclaw.png',
    hermes: 'hermes.png',
    custom: 'custom.svg',
  };
  const key = files[p] ? p : 'custom';
  return `assets/provider-logos/${files[key]}`;
}

function threadClientIcon(kind) {
  if (String(kind || '') === 'all') {
    return `<span class="thread-client-glyph thread-client-all" aria-hidden="true">All</span>`;
  }
  const normalized = normalizeThreadClientKind(kind);
  const logo = normalized === 'claude_code' ? 'claude-code' : (normalized === 'generic_client' ? 'custom' : normalized);
  return `<span class="client-logo-box client-logo-${escAttr(normalized)}"><img class="client-logo-img" src="${threadProviderLogoSrc(logo)}" alt="" aria-hidden="true"></span>`;
}

function threadLineActionIcon(name) {
  return `<span class="line-action-icon line-action-icon-${escAttr(name)}" aria-hidden="true"></span>`;
}

function renderThreadIconAction(label, icon, onclick, className = '') {
  return `<button type="button" class="btn btn-ghost account-toolbar-icon thread-toolbar-icon ${className}" onclick="${onclick}" title="${escAttr(label)}" aria-label="${escAttr(label)}">${threadLineActionIcon(icon)}</button>`;
}

function threadJsArg(value) {
  return String(value ?? '')
    .replace(/\\/g, '\\\\')
    .replace(/'/g, "\\'")
    .replace(/\r/g, '\\r')
    .replace(/\n/g, '\\n');
}

function threadSourceTone(source) {
  const diagnostics = Array.isArray(source?.diagnostics) ? source.diagnostics.join(' ') : '';
  if (!diagnostics) return 'ok';
  if (/暂未发现|没有统一历史格式|后续适配|占位/.test(diagnostics)) return 'muted';
  return 'warn';
}

function formatThreadTokenCount(value) {
  const n = Number(value || 0);
  if (!Number.isFinite(n) || n <= 0) return '—';
  if (n >= 1000000) return `${(n / 1000000).toFixed(n >= 10000000 ? 0 : 1)}M`;
  if (n >= 1000) return `${Math.round(n / 1000)}K`;
  return String(Math.round(n));
}

function renderContextWindowTags(status) {
  const context = status?.context_window || {};
  const windowTokens = Number(
    context.latest_rollout_model_context_window
    || context.effective_model_context_window
    || context.configured_model_context_window
    || context.catalog_model_context_window
    || 0
  );
  const usedTokens = Number(context.latest_rollout_last_total_tokens || 0);
  const source = context.latest_rollout_token_usage_found ? 'rollout' : (windowTokens ? 'config' : '—');
  const sourceTone = context.latest_rollout_token_usage_found ? 'tag-current' : (windowTokens ? 'tag-warning' : 'tag-other');
  const titleParts = [];
  if (context.active_model) titleParts.push(`模型: ${context.active_model}`);
  if (context.configured_model_context_window) titleParts.push(`配置: ${formatThreadTokenCount(context.configured_model_context_window)}`);
  if (context.catalog_model_context_window) titleParts.push(`目录: ${formatThreadTokenCount(context.catalog_model_context_window)}`);
  if (context.effective_model_context_window) titleParts.push(`有效: ${formatThreadTokenCount(context.effective_model_context_window)}`);
  if (context.latest_rollout_path) titleParts.push(`rollout: ${context.latest_rollout_path}`);
  const title = titleParts.length ? ` title="${escAttr(titleParts.join('\n'))}"` : '';
  return `
      <span class="tag tag-current"${title}>上下文: ${formatThreadTokenCount(windowTokens)}</span>
      <span class="tag tag-current">最近已用: ${formatThreadTokenCount(usedTokens)}</span>
      <span class="tag ${sourceTone}"${title}>Token源: ${esc(source)}</span>`;
}

function renderThreads() {
  return `<div class="page-header threads-page-header">
    <h2>线程聚合</h2>
  </div>
  <div class="threads-console">
    <div class="threads-summary" id="threadsSummary">
      <div class="threads-stat"><div class="stat-value thread-summary-value">—</div><div class="stat-label thread-summary-label">总线程</div></div>
      <div class="threads-stat other"><div class="stat-value thread-summary-value">—</div><div class="stat-label thread-summary-label">可读源</div></div>
      <div class="threads-stat migrated"><div class="stat-value thread-summary-value">全部</div><div class="stat-label thread-summary-label">当前筛选</div></div>
    </div>
  </div>
  <div id="threadClientSwitcher" class="thread-source-switcher"></div>
  <div id="codexThreadActions" class="codex-thread-actions"></div>
  <div id="threadSourceDiagnostics"></div>
  <div class="threads-list-head">线程列表</div>
  <div class="threads-table-wrap">
    <table class="threads-table">
      <thead><tr><th>标题</th><th>客户端</th><th>模型/Provider</th><th>更新时间</th><th>操作</th></tr></thead>
      <tbody id="threadsTableBody"><tr><td colspan="5" class="threads-empty-cell">加载中...</td></tr></tbody>
      <tfoot><tr class="threads-table-spacer"><td colspan="5"></td></tr></tfoot>
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
    if (cards.length >= 3) {
      cards[0].textContent = unified?.total ?? list.length;
      cards[1].textContent = `${sources.filter(s => s.available).length}/${sources.length || 0}`;
      cards[2].textContent = selectedThreadClientKind === 'all' ? '全部' : threadClientLabel(selectedThreadClientKind);
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
  const allButton = `<button type="button" class="thread-source-tab${allActive}" onclick="selectThreadClient('all')" title="全部线程" aria-label="全部线程" role="tab" aria-selected="${allActive ? 'true' : 'false'}">
    <span>全部</span>
    <em>${Number(total || 0)}</em>
  </button>`;
  const sourceButtons = sources.map(source => {
    const kind = normalizeThreadClientKind(source.client_kind);
    const active = selectedThreadClientKind === kind ? ' active' : '';
    const tone = threadSourceTone(source);
    const issueClass = tone !== 'ok' ? ` has-issues source-${tone}` : '';
    const title = Array.isArray(source.scan_paths) ? source.scan_paths.join('\n') : '';
    const label = threadClientLabel(kind);
    return `<button type="button" class="thread-source-tab${active}${issueClass}" onclick="selectThreadClient('${escAttr(kind)}')" title="${escAttr(label + (title ? '\n' + title : ''))}" aria-label="${escAttr(label)}" role="tab" aria-selected="${active ? 'true' : 'false'}">
      <span>${esc(label)}</span>
      <em>${Number(source.count || 0)}</em>
    </button>`;
  }).join('');
  return `<div class="thread-client-tabs" role="tablist" aria-label="线程客户端分类">${allButton}${sourceButtons}</div>`;
}

function renderCodexThreadActions(status) {
  if (selectedThreadClientKind !== 'all' && selectedThreadClientKind !== 'codex') return '';
  if (!status || status.error) {
    return `<div class="codex-thread-muted">Codex 专属操作不可用${status?.error ? ': ' + esc(status.error) : ''}</div>`;
  }
  const pendingUnified = Number(status.non_deecodex_count ?? status.non_unified_count ?? 0);
  const restoreDisabled = !status.migrated ? ' disabled' : '';
  const active = status.active_provider || '—';
  const sourceSummary = Array.isArray(status.source_summary)
    ? status.source_summary.map(item => `${item.source || '—'} ${Number(item.count || 0)}`).join(' · ')
    : '';
  const sourceTitle = sourceSummary ? ` title="${escAttr('来源: ' + sourceSummary)}"` : '';
  const desktopBlocked = !!status.desktop_project_repair_blocked;
  const recentBlocked = !!status.desktop_recent_repair_blocked;
  const desktopPending = Number(status.desktop_project_pending_count || 0);
  const recentPending = Number(status.desktop_recent_pending_count || 0);
  const missingPreview = Number(status.missing_preview_count || 0);
  const missingUserEvent = Number(status.missing_user_event_count || 0);
  const actionNeeded = pendingUnified > 0 || desktopPending > 0 || recentPending > 0 || missingPreview > 0 || missingUserEvent > 0;
  const migrateDisabled = actionNeeded ? '' : ' disabled';
  const desktopTitle = desktopBlocked
    ? ' title="Codex Desktop 正在运行，项目索引会被运行态状态覆盖；退出 Codex Desktop 后再聚合可写入"'
    : '';
  const recentTitle = recentBlocked
    ? ' title="Codex Desktop 正在运行，Recent 时间戳修复会被运行态状态覆盖；请完全退出 Codex Desktop 后再聚合"'
    : ' title="Codex Desktop 侧边栏首屏只加载最近 20 条；这里统计项目线程是否已进入该窗口"';
  return `<div class="codex-thread-tools codex-thread-strip">
    <div class="codex-thread-meta">
      <span class="codex-thread-label">Codex 专属操作</span>
      <span class="tag tag-current">归属: ${esc(active)}</span>
      <span class="tag tag-other">待统一: ${pendingUnified}</span>
      <span class="tag tag-current"${sourceTitle}>已统一: ${Number(status.provider_unified_count || 0)}</span>
      <span class="tag tag-current">Codex 可见: ${Number(status.codex_visible_count || 0)}</span>
      <span class="tag tag-other">当前项目: ${Number(status.current_cwd_visible_count || 0)}</span>
      <span class="tag ${desktopBlocked ? 'tag-warning' : 'tag-current'}"${desktopTitle}>桌面索引: ${Number(status.desktop_project_indexed_count || 0)}</span>
      <span class="tag ${desktopPending ? 'tag-warning' : 'tag-other'}"${desktopTitle}>待写索引: ${desktopPending}</span>
      <span class="tag ${recentPending ? 'tag-warning' : 'tag-current'}"${recentTitle}>Recent: ${Number(status.desktop_recent_visible_count || 0)}</span>
      <span class="tag ${recentPending ? 'tag-warning' : 'tag-other'}"${recentTitle}>待入 Recent: ${recentPending}</span>
      <span class="tag tag-other">缺预览: ${missingPreview}</span>
      <span class="tag tag-other">缺用户事件: ${missingUserEvent}</span>
      ${renderContextWindowTags(status)}
    </div>
    <div class="codex-thread-tool-row">
      <button class="btn btn-primary" id="btnMigrate" onclick="doMigrate()"${migrateDisabled}>聚合 Codex 线程</button>
      <button class="btn btn-ghost" id="btnRestore" onclick="doRestore()"${restoreDisabled}>还原 Codex 隔离</button>
      <button class="btn btn-ghost thread-strip-refresh" onclick="refreshThreads()" title="刷新线程" aria-label="刷新线程">${threadLineActionIcon('thread-refresh')}</button>
    </div>
  </div>`;
}

function renderThreadSourceDiagnostics(sources) {
  const rows = sources
    .filter(source => Array.isArray(source.diagnostics) && source.diagnostics.length)
    .map(source => {
      const tone = threadSourceTone(source);
      if (tone === 'muted') return '';
      return `<div class="thread-source-note source-${escAttr(tone)}">
        <strong>${esc(threadClientLabel(source.client_kind))}</strong>
        <span>${source.diagnostics.map(item => esc(item)).join('；')}</span>
      </div>`;
    })
    .filter(Boolean);
  return rows.length ? `<div class="thread-source-diagnostics">
    <div class="thread-source-note-list">${rows.join('')}</div>
  </div>` : '';
}

function renderThreadRows(list) {
  if (!list || list.length === 0) {
    return '<tr><td colspan="5" class="threads-empty-cell">无线程数据</td></tr>';
  }
  return list.map(t => {
    const kind = normalizeThreadClientKind(t.client_kind);
    const provider = t.model || t.provider || '—';
    const timeValue = t.updated_at_ms || t.created_at_ms;
    const time = formatThreadTime(timeValue);
    const fullTime = formatThreadFullTime(timeValue);
    const messageCount = Number(t.message_count || 0);
    const preview = String(t.preview || '').trim();
    const metaParts = [];
    if (messageCount) metaParts.push(`${messageCount} 条消息`);
    if (preview) metaParts.push(trunc(preview, 72));
    const meta = metaParts.length ? `<span class="thread-meta-line">${esc(metaParts.join(' · '))}</span>` : '';
    const threadKey = String(t.thread_key || '');
    const deleteAction = t.delete_available
      ? `<button type="button" class="thread-row-action danger" onclick="event.stopPropagation();deleteThreadRow('${escAttr(threadJsArg(kind))}','${escAttr(threadJsArg(t.native_id))}')" title="删除 Codex 线程" aria-label="删除 Codex 线程">${threadLineActionIcon('trash')}</button>`
      : '';
    const rowClass = t.detail_available ? 'thread-row' : 'thread-row thread-row-muted';
    const rowClick = t.detail_available ? ` onclick="openThread('${escAttr(threadJsArg(kind))}','${escAttr(threadJsArg(t.native_id))}','${escAttr(threadJsArg(threadKey))}')"` : '';
    const rowTitle = [t.title, t.native_id ? `线程 ID: ${t.native_id}` : ''].filter(Boolean).join('\n');
    return `<tr class="${rowClass}"${rowClick}>
      <td title="${escAttr(rowTitle)}"><span class="td-title-text">${esc(t.title || '(无标题)')}</span>${meta}</td>
      <td><span class="tag tag-current">${esc(threadClientLabel(kind))}</span></td>
      <td>${esc(provider)}</td>
      <td title="${escAttr(fullTime)}">${esc(time)}</td>
      <td class="thread-actions-cell">${deleteAction}</td>
    </tr>`;
  }).join('');
}

function selectThreadClient(kind) {
  selectedThreadClientKind = kind === 'all' ? 'all' : normalizeThreadClientKind(kind);
  if (_threadsData) {
    const cards = document.querySelectorAll('#threadsSummary .stat-value');
    if (cards.length >= 3) cards[2].textContent = selectedThreadClientKind === 'all' ? '全部' : threadClientLabel(selectedThreadClientKind);
    const switcher = document.getElementById('threadClientSwitcher');
    if (switcher) switcher.innerHTML = renderThreadClientSwitcher(_threadsData.sources, _threadsData.list.length);
    const codexActions = document.getElementById('codexThreadActions');
    if (codexActions) codexActions.innerHTML = renderCodexThreadActions(_threadsData.codexStatus);
    const tbody = document.getElementById('threadsTableBody');
    if (tbody) tbody.innerHTML = renderThreadRows(filteredThreadList(_threadsData.list));
  }
}

function formatThreadTime(value) {
  const ms = Number(value || 0);
  if (!ms) return '—';
  return new Date(ms).toLocaleString('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  });
}

function formatThreadFullTime(value) {
  const ms = Number(value || 0);
  return ms ? new Date(ms).toLocaleString('zh-CN') : '—';
}

async function doMigrate() {
  if (!await showConfirm('确定要聚合 Codex 线程吗？\n\n将统一 Codex 线程归属并修复 Desktop 首页可见字段；其他客户端历史不会被改写。')) return;

  const btn = document.getElementById('btnMigrate');
  if (btn) btn.disabled = true;
  showToast('聚合中...', 'info');

  try {
    const diff = await invoke('migrate_threads');
    const fixed = Number(diff.visibility_fixed_count || 0);
    const desktopFixed = Number(diff.desktop_project_fixed_count || 0);
    const recentFixed = Number(diff.desktop_recent_fixed_count || 0);
    const pending = Number(diff.desktop_project_pending_count || 0);
    const recentPending = Number(diff.desktop_recent_pending_count || 0);
    const blocked = !!diff.desktop_project_repair_blocked || !!diff.desktop_recent_repair_blocked;
    const suffix = blocked
      ? `，桌面项目索引待写 ${pending} 条，Recent 待写 ${recentPending} 条；请完全退出 Codex Desktop 后再聚合`
      : `，桌面项目修复 ${desktopFixed} 条，Recent 修复 ${recentFixed} 条`;
    const changed = Number(diff.changed_count || 0);
    const message = changed || fixed || desktopFixed || recentFixed || pending
      ? `已聚合 ${changed} 条 Codex 线程，可见性修复 ${fixed} 项${suffix}`
      : '已检查 Codex 线程，无需变更';
    showToast(message, blocked ? 'warning' : 'success');
    await refreshThreads();
  } catch (err) {
    showToast('聚合失败: ' + err, 'error');
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
    const restoredCwd = Number(diff.cwd_aligned_count || 0);
    const recentRestored = Number(diff.desktop_recent_fixed_count || 0);
    showToast(`已还原 ${Number(diff.changed_count || 0)} 条 Codex 线程，路径恢复 ${restoredCwd} 条，Recent 时间恢复 ${recentRestored} 条`, 'success');
    await refreshThreads();
  } catch (err) {
    showToast('还原失败: ' + err, 'error');
  } finally {
    if (btn) btn.disabled = false;
  }
}

// ── 线程详情面板 ──

function openThread(clientKind, nativeId, threadKey) {
  const kind = normalizeThreadClientKind(clientKind);
  _currentThread = { clientKind: kind, nativeId, threadKey: String(threadKey || '') };
  const container = document.getElementById('mainContent');
  if (!container) return;
  container.innerHTML = `<div class="detail-panel">
    <div class="detail-header">
      <button class="page-back-button detail-back-btn" onclick="closeThreadDetail()" title="返回线程列表" aria-label="返回线程列表">${threadLineActionIcon('back')}</button>
      <h2 id="detailTitle">加载中...</h2>
      <button class="detail-delete-btn" id="detailDeleteBtn" style="display:none;" onclick="deleteThreadFromDetail()" title="删除 Codex 线程" aria-label="删除 Codex 线程">${threadLineActionIcon('trash')}</button>
    </div>
    <div class="detail-messages" id="detailMessages">
      <div class="detail-loading">加载中...</div>
    </div>
  </div>`;

  const args = { clientKind: kind, nativeId };
  if (_currentThread.threadKey) args.threadKey = _currentThread.threadKey;
  invoke('get_client_thread_content', args)
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
