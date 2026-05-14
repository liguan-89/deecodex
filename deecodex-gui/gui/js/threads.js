let _threadsData = null;

function renderThreads() {
  return `<div class="page-header">
    <h2>线程聚合</h2>
    <p>切换 Codex 侧边栏会话历史的 provider 归属</p>
  </div>
  <div class="threads-summary" id="threadsSummary">
    <div class="threads-stat"><div class="stat-value">—</div><div class="stat-label">总线程数</div></div>
    <div class="threads-stat other"><div class="stat-value">—</div><div class="stat-label">非统一线程</div></div>
    <div class="threads-stat migrated"><div class="stat-value">—</div><div class="stat-label">归属</div></div>
  </div>
  <div class="mgmt-actions" style="margin-bottom:20px;">
    <button class="btn btn-primary" id="btnMigrate" onclick="doMigrate()">⟐ 聚合所有线程</button>
    <button class="btn btn-ghost" id="btnRestore" onclick="doRestore()">↩ 还原隔离模式</button>
    <button class="btn btn-warning" id="btnCalibrate" onclick="doCalibrate()" style="display:none;">⟐ 校准</button>
    <button class="btn btn-ghost" onclick="refreshThreads()">⟳ 刷新</button>
  </div>
  <div id="threadsProviderBreakdown"></div>
  <div class="section-sub-label" style="margin-top:20px;">线程列表</div>
  <div style="max-height:50vh;overflow-y:auto;">
    <table class="threads-table">
      <thead><tr><th>标题</th><th>Provider</th><th>更新时间</th><th>状态</th></tr></thead>
      <tbody id="threadsTableBody"><tr><td colspan="4" style="text-align:center;color:var(--text-muted);">加载中...</td></tr></tbody>
    </table>
  </div>`;
}

async function refreshThreads() {
  try {
    const [status, list] = await Promise.all([
      invoke('get_threads_status'),
      invoke('list_threads'),
    ]);
    _threadsData = { status, list };

    // 更新统计卡片
    const cards = document.querySelectorAll('#threadsSummary .stat-value');
    if (cards.length >= 3) {
      cards[0].textContent = status.total;
      cards[1].textContent = status.non_unified_count;
      if (status.calibration_needed) {
        cards[2].textContent = '需要校准';
        cards[2].style.color = 'var(--amber)';
      } else {
        cards[2].textContent = status.active_provider || '—';
        cards[2].style.color = 'var(--text-primary)';
      }
    }

    // 更新按钮状态
    const btnMigrate = document.getElementById('btnMigrate');
    const btnRestore = document.getElementById('btnRestore');
    const btnCalibrate = document.getElementById('btnCalibrate');
    if (btnMigrate) btnMigrate.style.display = status.calibration_needed ? 'none' : '';
    if (btnMigrate) btnMigrate.disabled = status.migrated || status.non_unified_count === 0;
    if (btnRestore) btnRestore.style.display = status.calibration_needed ? 'none' : '';
    if (btnRestore) btnRestore.disabled = !status.migrated;
    if (btnCalibrate) btnCalibrate.style.display = status.calibration_needed ? '' : 'none';

    // 各 provider 分布
    const breakdown = document.getElementById('threadsProviderBreakdown');
    if (breakdown) {
      breakdown.innerHTML = status.summary.map(s => {
        const cls = s.provider === status.active_provider ? 'tag-current' : 'tag-other';
        return `<span class="tag ${cls}" style="margin-right:6px;margin-bottom:4px;">${esc(s.provider)}: ${s.count}</span>`;
      }).join('');
    }

    // 线程列表
    const tbody = document.getElementById('threadsTableBody');
    if (tbody) {
      if (!list || list.length === 0) {
        tbody.innerHTML = '<tr><td colspan="4" style="text-align:center;color:var(--text-muted);">无线程数据</td></tr>';
      } else {
        tbody.innerHTML = list.map(t => {
          const isActive = status.active_provider && t.model_provider === status.active_provider;
          const providerTag = isActive
            ? `<span class="tag tag-current">${esc(t.model_provider)}</span>`
            : `<span class="tag tag-other">${esc(t.model_provider)}</span>`;
          const time = t.updated_at_ms
            ? new Date(t.updated_at_ms).toLocaleString('zh-CN')
            : '—';
          const archived = t.archived ? ' [已归档]' : '';
          return `<tr class="thread-row" onclick="openThread('${escAttr(t.id)}')">
            <td title="${escAttr(t.title)}"><span class="td-title-text">${esc(t.title || '(无标题)')}${archived}</span><span class="tag-delete" onclick="event.stopPropagation();deleteThreadRow('${escAttr(t.id)}')">删除</span></td>
            <td>${providerTag}</td>
            <td>${esc(time)}</td>
            <td>${esc(t.id).substring(0,12)}...</td>
          </tr>`;
        }).join('');
      }
    }
  } catch (err) {
    showToast('加载线程数据失败: ' + err, 'error');
    const tbody = document.getElementById('threadsTableBody');
    if (tbody) tbody.innerHTML = '<tr><td colspan="4" style="text-align:center;color:var(--red);">加载失败: ' + esc(String(err)) + '</td></tr>';
  }
}

async function doMigrate() {
  if (!await showConfirm('确定要聚合所有线程吗？\n\n所有线程将统一到当前活跃 provider，Codex 侧边栏将聚合显示所有会话。\n迁移前会自动备份，可随时还原。')) return;

  const btn = document.getElementById('btnMigrate');
  if (btn) btn.disabled = true;
  showToast('迁移中...', 'info');

  try {
    const diff = await invoke('migrate_threads');
    showToast(`已迁移 ${diff.changed_count} 条线程`, 'success');
    await refreshThreads();
  } catch (err) {
    showToast('迁移失败: ' + err, 'error');
  } finally {
    if (btn) btn.disabled = false;
  }
}

async function doRestore() {
  if (!await showConfirm('确定要切换到隔离模式吗？\n\n所有线程将还原到各自的原始 provider。\n\n还原后备份将被删除。')) return;

  const btn = document.getElementById('btnRestore');
  if (btn) btn.disabled = true;
  showToast('还原中...', 'info');

  try {
    const diff = await invoke('restore_threads');
    showToast(`已还原 ${diff.changed_count} 条线程`, 'success');
    await refreshThreads();
  } catch (err) {
    showToast('还原失败: ' + err, 'error');
  } finally {
    if (btn) btn.disabled = false;
  }
}

async function doCalibrate() {
  const activeProvider = (_threadsData && _threadsData.status && _threadsData.status.active_provider) || '';
  if (!await showConfirm(`检测到活跃 provider 已变更，需要重新校准。\n\n将还原原始 provider 后重新聚合到「${activeProvider}」。`)) return;

  const btn = document.getElementById('btnCalibrate');
  if (btn) btn.disabled = true;
  showToast('校准中...', 'info');

  try {
    const diff = await invoke('calibrate_threads');
    showToast(`已校准 ${diff.changed_count} 条线程到 ${diff.target_provider}`, 'success');
    await refreshThreads();
  } catch (err) {
    showToast('校准失败: ' + err, 'error');
  } finally {
    if (btn) btn.disabled = false;
  }
}

// ── 线程详情面板 ──

let _currentThreadId = null;

function openThread(threadId) {
  _currentThreadId = threadId;
  const container = document.getElementById('mainContent');
  if (!container) return;
  container.innerHTML = `<div class="detail-panel">
    <div class="detail-header">
      <button class="detail-back-btn" onclick="closeThreadDetail()">← 返回</button>
      <h2 id="detailTitle">加载中...</h2>
      <button class="detail-delete-btn" onclick="deleteThreadFromDetail('${escAttr(threadId)}')">删除</button>
    </div>
    <div class="detail-messages" id="detailMessages">
      <div class="detail-loading">加载中...</div>
    </div>
  </div>`;

  invoke('get_thread_content', { threadId })
    .then(content => {
      const titleEl = document.getElementById('detailTitle');
      if (titleEl) titleEl.textContent = content.thread.title || '(无标题)';
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
  _currentThreadId = null;
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
  const displayBody = truncated ? body.substring(0, 500) : body;
  const expandBtn = truncated
    ? `<button class="msg-expand-btn" onclick="this.previousElementSibling.classList.toggle('expanded');this.textContent=this.previousElementSibling.classList.contains('expanded')?'收起':'展开全部';">展开全部</button>`
    : '';

  return `<div class="msg-bubble ${roleClass}">
    <div class="msg-role">${esc(role)}</div>
    <div class="msg-body">${esc(displayBody)}</div>
    ${expandBtn}
  </div>`;
}

async function deleteThreadFromDetail(threadId) {
  if (!await showConfirm('确定要永久删除此线程吗？\n\n此操作不可恢复，线程将从 SQLite、会话文件和迁移备份中同时移除。')) return;
  try {
    await invoke('delete_thread', { threadId });
    showToast('线程已永久删除', 'success');
    closeThreadDetail();
  } catch (err) {
    showToast('删除失败: ' + err, 'error');
  }
}

async function deleteThreadRow(threadId) {
  if (!await showConfirm('确定要永久删除此线程吗？\n\n此操作不可恢复，线程将从 SQLite、会话文件和迁移备份中同时移除。')) return;
  try {
    await invoke('delete_thread', { threadId });
    showToast('线程已永久删除', 'success');
    refreshThreads();
  } catch (err) {
    showToast('删除失败: ' + err, 'error');
  }
}

// ═══════════════════════════════════════════════════════════════
// 账号管理
// ═══════════════════════════════════════════════════════════════
