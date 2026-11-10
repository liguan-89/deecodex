let _threadsData = null;
// _currentThread 当前打开的详情页线程：刷新/切 tab 后能恢复。持久化到 deeStorage。
// 字段：{ clientKind, nativeId, threadKey } | null
const CURRENT_THREAD_KEY = 'dex_current_thread_v1';
let _currentThread = null;
try {
  const raw = window.deeStorage?.getItem?.(CURRENT_THREAD_KEY);
  if (raw) {
    const parsed = JSON.parse(raw);
    if (parsed && typeof parsed === 'object' && parsed.clientKind && parsed.nativeId) {
      _currentThread = {
        clientKind: normalizeThreadClientKind(parsed.clientKind),
        nativeId: String(parsed.nativeId),
        threadKey: String(parsed.threadKey || ''),
      };
    }
  }
} catch (_) { /* localStorage 解析失败时忽略，保留 null */ }
let selectedThreadClientKind = 'all';

function persistCurrentThread() {
  const storage = typeof window !== 'undefined' ? window.deeStorage : null;
  if (!storage) return;
  if (_currentThread) {
    storage.setItem(CURRENT_THREAD_KEY, JSON.stringify(_currentThread));
  } else {
    storage.removeItem(CURRENT_THREAD_KEY);
  }
}

const THREAD_CLIENT_LABELS = {
  all: '全部',
  codex: 'Codex',
  claude_code: 'Claude',
  hermes: 'Hermes',
  generic_client: '通用',
};

function normalizeThreadClientKind(kind) {
  const value = String(kind || 'codex');
  if (value === 'ClaudeCode') return 'claude_code';
  if (value === 'GenericClient') return 'generic_client';
  if (value === 'Codex') return 'codex';
  if (value === 'Hermes') return 'hermes';
  return ['codex', 'claude_code', 'hermes', 'generic_client'].includes(value) ? value : 'codex';
}

function threadClientLabel(kind) {
  const normalized = normalizeThreadClientKind(kind);
  return THREAD_CLIENT_LABELS[normalized] || normalized;
}

function threadLineActionIcon(name) {
  return `<span class="line-action-icon line-action-icon-${escAttr(name)}" aria-hidden="true"></span>`;
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

function renderThreads() {
  return `<div class="page-header threads-page-header">
    <h2>线程聚合</h2>
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

function renderThreadsIntoMain(html) {
  const container = document.getElementById('mainContent');
  if (!container) return null;
  container.innerHTML = typeof wrapPrimaryPanel === 'function'
    ? wrapPrimaryPanel('threads', html)
    : html;
  return container;
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
  const desktopBlocked = !!status.desktop_project_repair_blocked;
  const desktopRunning = !!status.codex_desktop_running;
  const desktopPending = Number(status.desktop_project_pending_count || 0);
  const actionNeeded = pendingUnified > 0;
  const migrateDisabled = actionNeeded ? '' : ' disabled';
  const desktopTitle = desktopPending
    ? ' title="项目索引待补齐；立即归一会写入并复查，若状态刷新后仍未补齐可再次重试"'
    : '';
  const desktopIndexTag = desktopPending > 0
    ? `<span class="tag tag-warning"${desktopTitle}>索引待同步: ${desktopPending}</span>`
    : '';
  return `<div class="codex-thread-tools codex-thread-strip">
    <div class="codex-thread-meta">
      <span class="codex-thread-label">Codex 专属操作</span>
      <span class="tag tag-current">归属: ${esc(active)}</span>
      ${desktopRunning ? '<span class="tag tag-warning">Codex Desktop运行中</span>' : ''}
      <span class="tag ${pendingUnified ? 'tag-warning' : 'tag-other'}">待统一: ${pendingUnified}</span>
      <span class="tag tag-current">已统一: ${Number(status.provider_unified_count || 0)}</span>
      <span class="tag tag-current">Codex 可见: ${Number(status.codex_visible_count || 0)}</span>
      ${desktopIndexTag}
    </div>
    <div class="codex-thread-tool-row">
      <button class="btn btn-primary" id="btnMigrate" onclick="doMigrate()"${migrateDisabled}>立即归一</button>
      <button class="btn btn-ghost" id="btnRestore" onclick="doRestore()"${restoreDisabled}>旧备份还原</button>
      <button class="btn btn-ghost" id="btnSnapshotRestore" onclick="doSnapshotRestore()" title="从 Time Machine 快照恢复被 Codex 升级重置的历史线程" aria-label="快照恢复">快照恢复</button>
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
    const cwdText = String(t.cwd || '').trim();
    const gitBranch = String(t.git_branch || '').trim();
    const tokensUsed = Number(t.tokens_used || 0);
    const metaParts = [];
    if (messageCount) metaParts.push(`${messageCount} 条消息`);
    if (preview) metaParts.push(trunc(preview, 72));
    // 项目 + 分支副标题：有 cwd 时显示「~/.../path › branch」；无 cwd 但有 branch 也显示分支
    const projectHint = (() => {
      if (cwdText && gitBranch) return `${trunc(cwdText, 40).replace(/^\/Users\/[^/]+/, '~')} › ${gitBranch}`;
      if (cwdText) return trunc(cwdText, 50).replace(/^\/Users\/[^/]+/, '~');
      if (gitBranch) return `branch: ${gitBranch}`;
      return '';
    })();
    if (projectHint) metaParts.push(projectHint);
    // token 消耗摘要（只在 > 0 时显示，>1000 折成 k/M）
    if (tokensUsed > 0) {
      const tok = tokensUsed >= 1_000_000
        ? `${(tokensUsed / 1_000_000).toFixed(1)}M tok`
        : tokensUsed >= 1000
          ? `${Math.round(tokensUsed / 1000)}k tok`
          : `${tokensUsed} tok`;
      metaParts.push(tok);
    }
    const meta = metaParts.length ? `<span class="thread-meta-line">${esc(metaParts.join(' · '))}</span>` : '';
    const threadKey = String(t.thread_key || '');
    const archived = !!t.archived;
    // 归档按钮：仅 codex + delete_available 线程可点（与 delete 同行）
    // 已归档时显示「取消归档」，未归档时显示「归档」
    const archiveAction = (kind === 'codex' && t.delete_available)
      ? `<button type="button" class="thread-row-action ${archived ? 'is-archived' : ''}" onclick="event.stopPropagation();toggleArchive('${escAttr(threadJsArg(t.native_id))}', ${!archived})" title="${archived ? '取消归档' : '归档'}" aria-label="${archived ? '取消归档' : '归档'}">${threadLineActionIcon(archived ? 'unarchive' : 'archive')}</button>`
      : '';
    const deleteAction = t.delete_available
      ? `<button type="button" class="thread-row-action danger" onclick="event.stopPropagation();deleteThreadRow('${escAttr(threadJsArg(kind))}','${escAttr(threadJsArg(t.native_id))}')" title="删除 Codex 线程" aria-label="删除 Codex 线程">${threadLineActionIcon('trash')}</button>`
      : '';
    // 客户端列：subagent 时显示「subagent: Aquinas」+ role 副标签
    const sourceRaw = String(t.source || '').trim();
    const threadSource = String(t.thread_source || '').trim();
    const agentNickname = String(t.agent_nickname || '').trim();
    const agentRole = String(t.agent_role || '').trim();
    const isSubagent = sourceRaw.startsWith('{') || threadSource === 'subagent' || !!agentNickname;
    const clientCell = isSubagent
      ? `<span class="tag tag-subagent" title="subagent${agentRole ? ' · ' + agentRole : ''}">subagent</span>${agentNickname ? `<div class="thread-client-sub">${esc(agentNickname)}${agentRole ? ' · ' + esc(agentRole) : ''}</div>` : ''}`
      : (sourceRaw && sourceRaw !== 'vscode' && sourceRaw !== kind
          ? `<span class="tag tag-current">${esc(threadClientLabel(kind))}</span><div class="thread-client-sub">${esc(sourceRaw)}</div>`
          : `<span class="tag tag-current">${esc(threadClientLabel(kind))}</span>`);
    // 当前活跃判定：_currentThread 必须 clientKind + nativeId 同时匹配才算
    const isActive = !!(t.detail_available && _currentThread
      && normalizeThreadClientKind(_currentThread.clientKind) === kind
      && String(_currentThread.nativeId) === String(t.native_id));
    let baseClass = t.detail_available ? 'thread-row' : 'thread-row thread-row-muted';
    // 已归档：行加 thread-row-archived（灰度降低），但仍保留 detail_available 可点击
    if (archived && t.detail_available) baseClass = `${baseClass} thread-row-archived`;
    const rowClass = isActive ? `${baseClass} thread-row-active` : baseClass;
    const rowClick = t.detail_available ? ` onclick="openThread('${escAttr(threadJsArg(kind))}','${escAttr(threadJsArg(t.native_id))}','${escAttr(threadJsArg(threadKey))}')"` : '';
    const rowTitle = [t.title, t.native_id ? `线程 ID: ${t.native_id}` : ''].filter(Boolean).join('\n');
    // 侧边栏不可见标记：exec / subagent 线程在 Codex 桌面侧边栏不显示（客户端白名单外），
    // deecodex 聚合视图仍如实展示，加标记避免用户困惑。
    const sidebarTag = (kind === 'codex' && !t.sidebar_visible)
      ? `<span class="tag tag-other" title="该线程不在 Codex 桌面侧边栏显示（客户端仅展示 vscode/cli 来源线程）">侧边栏隐藏</span> `
      : '';
    return `<tr class="${rowClass}"${rowClick}>
      <td title="${escAttr(rowTitle)}"><span class="td-title-text">${esc(t.title || '(无标题)')}</span>${sidebarTag}${meta}</td>
      <td>${clientCell}</td>
      <td>${esc(provider)}</td>
      <td title="${escAttr(fullTime)}">${esc(time)}</td>
      <td class="thread-actions-cell"><span class="thread-actions-inline">${archiveAction}${deleteAction}</span></td>
    </tr>`;
  }).join('');
}

function selectThreadClient(kind) {
  selectedThreadClientKind = kind === 'all' ? 'all' : normalizeThreadClientKind(kind);
  if (_threadsData) {
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
  if (!await showConfirm('确定要立即归一 Codex Desktop 线程吗？\n\n会把 Codex 桌面版线程统一到当前 Codex 配置的 provider；CLI、subagent 和其他客户端历史不会被改写。')) return;

  const btn = document.getElementById('btnMigrate');
  if (btn) btn.disabled = true;
  showToast('归一中...', 'info');

  try {
    const diff = await invoke('migrate_threads');
    const remaining = Number(diff.remaining_non_unified_count || 0);
    const target = diff.target_provider || '当前归属';
    const changed = Number(diff.changed_count || 0);
    const rolloutFixed = Number(diff.rollout_metadata_fixed_count || 0);
    const remainingText = remaining ? `，仍有 ${remaining} 条未统一` : '，已全部统一';
    const rolloutText = rolloutFixed ? `，修复 ${rolloutFixed} 个线程元数据` : '';
    const message = changed || rolloutFixed || remaining
      ? `已归一 ${changed} 条 Codex Desktop 线程到 ${target}${rolloutText}${remainingText}`
      : '已检查 Codex 线程，无需变更';
    showToast(message, remaining ? 'warning' : 'success');
    await refreshThreads();
  } catch (err) {
    showToast('归一失败: ' + err, 'error');
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

// ── 快照恢复（从 Time Machine 快照找回被 Codex 升级重置的历史线程）──

function snapshotRestoreModal(html) {
  let existing = document.getElementById('snapshotRestoreModal');
  if (existing) existing.remove();
  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay';
  overlay.id = 'snapshotRestoreModal';
  overlay.innerHTML = `<div class="modal-box" style="max-width:560px;">
    <div class="modal-header"><h3>快照恢复</h3></div>
    <div class="modal-body" style="padding:20px;text-align:left;font-size:13px;color:var(--text-secondary);line-height:1.7;max-height:60vh;overflow:auto;">${html}</div>
    <div style="display:flex;gap:12px;padding:0 20px 16px;justify-content:flex-end;">
      <button class="btn btn-ghost" id="snapRestoreClose" type="button">关闭</button>
    </div>
  </div>`;
  document.body.appendChild(overlay);
  overlay.querySelector('#snapRestoreClose').onclick = () => overlay.remove();
  overlay.addEventListener('click', e => { if (e.target === overlay) overlay.remove(); });
  return overlay;
}

async function doSnapshotRestore() {
  const overlay = snapshotRestoreModal('<div class="detail-loading">正在扫描 Time Machine 本地快照...</div>');
  let snaps = [];
  try {
    snaps = await invoke('list_thread_snapshots');
    if (!Array.isArray(snaps) || snaps.length === 0) {
      overlay.querySelector('.modal-body').innerHTML = '未找到 Time Machine 本地快照。<br>请确认已开启 Time Machine（含本地快照），或已有升级前的快照。';
      return;
    }
  } catch (err) {
    overlay.querySelector('.modal-body').innerHTML = '获取快照列表失败: ' + esc(String(err));
    return;
  }
  const rows = snaps.map((s, i) => `<label style="display:flex;align-items:center;gap:8px;padding:6px 0;border-bottom:1px solid var(--border);cursor:pointer;">
    <input type="radio" name="snapChoice" value="${escAttr(s.name)}" ${i === 0 ? 'checked' : ''} style="flex:none;">
    <span style="flex:1;">
      <strong style="color:var(--text-primary);">${esc(s.created_at)}</strong>
      <span style="color:var(--text-secondary);font-size:12px;margin-left:8px;">${esc(s.name)}</span>
    </span>
  </label>`).join('');
  overlay.querySelector('.modal-body').innerHTML =
    `<div style="margin-bottom:10px;">选择升级前的快照，恢复被 Codex 客户端升级重置的历史线程：</div>${rows}
     <div style="margin-top:14px;display:flex;gap:12px;justify-content:flex-end;">
       <button class="btn btn-ghost" id="snapPreviewBtn" type="button">预览差异</button>
       <button class="btn btn-primary" id="snapGoBtn" type="button">开始恢复</button>
     </div>`;
  const choice = () => (overlay.querySelector('input[name="snapChoice"]:checked') || {}).value;
  overlay.querySelector('#snapPreviewBtn').onclick = async () => {
    const name = choice();
    if (!name) return;
    const body = overlay.querySelector('.modal-body');
    body.innerHTML = '<div class="detail-loading">正在挂载快照并统计差异...</div>';
    try {
      const p = await invoke('preview_thread_snapshot_restore', { snapshotName: name });
      const gb = (p.snapshot_rollout_bytes / 1073741824).toFixed(2);
      body.innerHTML = `<div style="margin-bottom:10px;color:var(--text-primary);"><strong>快照 ${esc(p.snapshot_codex_path.split('/').pop() || name)} 预览</strong></div>
        <table style="width:100%;border-collapse:collapse;font-size:13px;">
          <tr><td style="padding:4px 0;color:var(--text-secondary);">快照中线程</td><td style="padding:4px 0;text-align:right;">${Number(p.snapshot_threads || 0)} 条（归档 ${Number(p.snapshot_archived || 0)}）</td></tr>
          <tr><td style="padding:4px 0;color:var(--text-secondary);">快照对话文件</td><td style="padding:4px 0;text-align:right;">${Number(p.snapshot_rollouts || 0)} + 归档 ${Number(p.snapshot_archived_rollouts || 0)}（${gb}GB）</td></tr>
          <tr><td style="padding:4px 0;color:var(--text-secondary);">将新增线程</td><td style="padding:4px 0;text-align:right;color:var(--green);font-weight:bold;">+${Number(p.new_threads || 0)}</td></tr>
          <tr><td style="padding:4px 0;color:var(--text-secondary);">将新增对话文件</td><td style="padding:4px 0;text-align:right;color:var(--green);font-weight:bold;">+${Number(p.new_rollouts || 0)}</td></tr>
          <tr><td style="padding:4px 0;color:var(--text-secondary);">将新增索引条目</td><td style="padding:4px 0;text-align:right;color:var(--green);font-weight:bold;">+${Number(p.new_index_entries || 0)}</td></tr>
        </table>
        <div style="margin-top:12px;color:var(--text-secondary);">确认无误后点击「开始恢复」。恢复前会自动备份当前线程库。</div>`;
    } catch (err) {
      body.innerHTML = '预览失败: ' + esc(String(err));
    }
  };
  overlay.querySelector('#snapGoBtn').onclick = async () => {
    const name = choice();
    if (!name) return;
    if (!await showConfirm('确定要从快照恢复历史线程吗？\n\n1. 会先备份当前线程库到 ~/.deecodex/recovery/\n2. 合并快照中的历史线程（跳过已存在的）\n3. 复制缺失的对话文件\n\n不会覆盖或删除任何现有线程。')) return;
    const body = overlay.querySelector('.modal-body');
    const goBtn = overlay.querySelector('#snapGoBtn');
    goBtn.disabled = true;
    body.innerHTML = '<div class="detail-loading">正在恢复（合并线程库 + 复制对话文件，可能需要几分钟）...</div>';
    try {
      const r = await invoke('execute_thread_snapshot_restore', { snapshotName: name });
      const warn = Number(r.missing_rollouts || 0) > 0 ? `<div style="color:var(--orange);margin-top:8px;">⚠ ${r.missing_rollouts} 个线程对话文件缺失</div>` : '';
      body.innerHTML = `<div style="color:var(--green);font-weight:bold;margin-bottom:10px;">✅ 恢复完成</div>
        <table style="width:100%;border-collapse:collapse;font-size:13px;">
          <tr><td style="padding:4px 0;color:var(--text-secondary);">当前线程总数</td><td style="padding:4px 0;text-align:right;">${Number(r.threads_total || 0)} 条（归档 ${Number(r.archived_total || 0)}）</td></tr>
          <tr><td style="padding:4px 0;color:var(--text-secondary);">本次新增线程</td><td style="padding:4px 0;text-align:right;color:var(--green);font-weight:bold;">+${Number(r.new_threads || 0)}</td></tr>
          <tr><td style="padding:4px 0;color:var(--text-secondary);">新增对话文件</td><td style="padding:4px 0;text-align:right;">+${Number(r.new_rollouts || 0)}</td></tr>
          <tr><td style="padding:4px 0;color:var(--text-secondary);">新增索引条目</td><td style="padding:4px 0;text-align:right;">+${Number(r.new_index_entries || 0)}</td></tr>
          <tr><td style="padding:4px 0;color:var(--text-secondary);">恢复前备份</td><td style="padding:4px 0;text-align:right;font-size:12px;">${esc(r.backup_path || '')}</td></tr>
        </table>${warn}
        <div style="margin-top:12px;color:var(--text-secondary);">若 ChatGPT.app 侧边栏仍显示旧列表，重启应用后可见全部历史线程。</div>`;
      await refreshThreads();
    } catch (err) {
      body.innerHTML = '恢复失败: ' + esc(String(err)) + '<div style="margin-top:8px;color:var(--text-secondary);">数据未损坏：恢复前已自动备份当前线程库。</div>';
    } finally {
      goBtn.disabled = false;
    }
  };
}

// ── 线程详情面板 ──

function openThread(clientKind, nativeId, threadKey) {
  const kind = normalizeThreadClientKind(clientKind);
  _currentThread = { clientKind: kind, nativeId, threadKey: String(threadKey || '') };
  persistCurrentThread();
  const container = renderThreadsIntoMain(`<div class="detail-panel">
    <div class="detail-header">
      <button class="page-back-button detail-back-btn" onclick="closeThreadDetail()" title="返回线程列表" aria-label="返回线程列表">${threadLineActionIcon('back')}</button>
      <h2 id="detailTitle">加载中...</h2>
      <button class="detail-delete-btn" id="detailDeleteBtn" style="display:none;" onclick="deleteThreadFromDetail()" title="删除 Codex 线程" aria-label="删除 Codex 线程">${threadLineActionIcon('trash')}</button>
    </div>
    <div class="detail-messages" id="detailMessages">
      <div class="detail-loading">加载中...</div>
    </div>
  </div>`);
  if (!container) return;

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
  persistCurrentThread();
  renderThreadsIntoMain(renderThreads());
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

/// 切换 Codex 线程归档状态。
/// 乐观更新本地 _threadsData（不等待后端），失败时回滚并提示。
async function toggleArchive(nativeId, nextArchived) {
  if (!_threadsData) {
    showToast('线程数据尚未加载', 'error');
    return;
  }
  const idx = _threadsData.list.findIndex(t => String(t.native_id) === String(nativeId));
  if (idx < 0) {
    showToast('未找到线程 ' + nativeId, 'error');
    return;
  }
  const oldArchived = !!_threadsData.list[idx].archived;
  const oldArchivedAt = _threadsData.list[idx].archived_at_ms || null;
  // 乐观更新
  _threadsData.list[idx].archived = nextArchived;
  _threadsData.list[idx].archived_at_ms = nextArchived ? Date.now() : null;
  try {
    const resp = await invoke('archive_thread', { threadId: String(nativeId), archived: !!nextArchived });
    // 用后端返回的权威值覆盖
    if (resp && typeof resp.archived === 'boolean') {
      _threadsData.list[idx].archived = resp.archived;
      _threadsData.list[idx].archived_at_ms = resp.archived_at_ms || null;
    }
    showToast(nextArchived ? '已归档' : '已取消归档', 'success');
    const tbody = document.getElementById('threadsTableBody');
    if (tbody) tbody.innerHTML = renderThreadRows(filteredThreadList(_threadsData.list));
  } catch (err) {
    // 回滚
    _threadsData.list[idx].archived = oldArchived;
    _threadsData.list[idx].archived_at_ms = oldArchivedAt;
    showToast('归档操作失败: ' + err, 'error');
    const tbody = document.getElementById('threadsTableBody');
    if (tbody) tbody.innerHTML = renderThreadRows(filteredThreadList(_threadsData.list));
  }
}
