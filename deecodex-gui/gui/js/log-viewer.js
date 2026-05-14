(function () {
  const EMPTY_LOG = '(暂无日志)';
  const AUTO_REFRESH_KEY = 'logAutoRefresh';

  function getInvoke() {
    return window.DeeCodexTauri?.invoke || window.invoke;
  }

  function toast(message, type) {
    if (typeof window.showToast === 'function') window.showToast(message, type);
  }

  function escapeHtml(value) {
    if (typeof window.esc === 'function') return window.esc(value);
    if (value === null || value === undefined) return '';
    return String(value)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  function escapeAttr(value) {
    if (typeof window.escAttr === 'function') return window.escAttr(value);
    return escapeHtml(value);
  }

  function closeLogModal() {
    clearInterval(window._logAutoRefresh);
    window._logAutoRefresh = null;
    document.getElementById('logModal')?.remove();
  }

  async function mgmtLogs() {
    closeLogModal();

    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';
    overlay.id = 'logModal';

    const autoRefresh = deeStorage.getItem(AUTO_REFRESH_KEY) === 'true';
    overlay.innerHTML = `
      <div class="modal-box" style="max-width:940px;">
        <div class="modal-header">
          <h3>☰ 服务日志</h3>
          <div style="display:flex;align-items:center;gap:12px;">
            <label style="font-size:11px;color:var(--text-secondary);cursor:pointer;display:flex;align-items:center;gap:4px;">
              <input type="checkbox" id="logAutoRefreshChk" ${autoRefresh ? 'checked' : ''}> 自动刷新
            </label>
            <button class="btn btn-ghost btn-sm" id="logClearBtn" type="button">清空日志</button>
            <button class="modal-close" id="logCloseBtn" type="button">✕</button>
          </div>
        </div>
        <div class="modal-body">
          <div id="logContent">加载中...</div>
        </div>
      </div>`;

    overlay.addEventListener('click', (event) => {
      if (event.target === overlay) closeLogModal();
    });
    document.body.appendChild(overlay);

    document.getElementById('logCloseBtn')?.addEventListener('click', closeLogModal);
    document
      .getElementById('logAutoRefreshChk')
      ?.addEventListener('change', (event) => toggleLogAutoRefresh(event.currentTarget.checked));
    document
      .getElementById('logClearBtn')
      ?.addEventListener('click', (event) => clearLogs(event.currentTarget));

    await refreshLogs();
    if (autoRefresh) window._logAutoRefresh = setInterval(refreshLogs, 5000);
  }

  function toggleLogAutoRefresh(on) {
    deeStorage.setItem(AUTO_REFRESH_KEY, on);
    if (on) {
      if (!window._logAutoRefresh) window._logAutoRefresh = setInterval(refreshLogs, 5000);
    } else {
      clearInterval(window._logAutoRefresh);
      window._logAutoRefresh = null;
    }
  }

  async function clearLogs(btn) {
    if (btn && btn.dataset.confirmed !== 'true') {
      btn.dataset.confirmed = 'true';
      btn.textContent = '再次点击确认';
      toast('再次点击确认清空日志', 'info');
      clearTimeout(window._logClearConfirmTimer);
      window._logClearConfirmTimer = setTimeout(() => {
        btn.dataset.confirmed = '';
        btn.textContent = '清空日志';
      }, 3000);
      return;
    }
    if (btn) btn.disabled = true;
    try {
      await getInvoke()('clear_logs');
      await refreshLogs();
      toast('日志已清空', 'success');
    } catch (error) {
      toast('清空日志失败: ' + error, 'error');
    } finally {
      if (btn) {
        btn.disabled = false;
        btn.dataset.confirmed = '';
        btn.textContent = '清空日志';
      }
    }
  }

  async function refreshLogs() {
    try {
      const lines = await getInvoke()('get_logs');
      const container = document.getElementById('logContent');
      if (!container) return;

      if (!lines || lines.length === 0) {
        container.innerHTML = '<div class="log-empty">暂无日志</div>';
        return;
      }

      const filtered = lines.filter((line) => line && line !== EMPTY_LOG);
      if (filtered.length === 0) {
        container.innerHTML = '<div class="log-empty">暂无日志</div>';
        return;
      }

      let html = '';
      for (let i = filtered.length - 1; i >= 0; i--) {
        const parsed = parseLogLine(filtered[i]);
        html += '<div class="log-entry">' +
          '<span class="log-time">' + escapeHtml(parsed.time || '') + '</span>' +
          '<span class="log-level-badge log-level-' + (parsed.level || 'unknown') + '">' + escapeHtml((parsed.level || '?').toUpperCase()) + '</span>' +
          '<span class="log-target" title="' + escapeAttr(parsed.target || '') + '">' + escapeHtml(parsed.target || '') + '</span>' +
          '<span class="log-msg">' + escapeHtml(parsed.message || '') + '</span>' +
          '</div>';
      }
      container.innerHTML = html || '<div class="log-empty">无匹配日志条目</div>';
    } catch (error) {
      const container = document.getElementById('logContent');
      if (container) container.innerHTML = '<div class="log-empty">无法加载日志: ' + escapeHtml(String(error)) + '</div>';
    }
  }

  function parseLogLine(raw) {
    const clean = raw.replace(/\x1b\[[0-9;]*m/g, '').trim();
    if (!clean) return { level: 'unknown', time: '', target: '', message: raw };

    let time = '';
    let rest = clean;
    if (clean.length >= 27 && clean[4] === '-') {
      time = clean.substring(11, 19);
      rest = clean.substring(27).trim();
    }

    const wsIdx = rest.indexOf(' ');
    if (wsIdx === -1) return { level: 'unknown', time, target: '', message: rest };

    const level = rest.substring(0, wsIdx).toUpperCase();
    const afterLevel = rest.substring(wsIdx + 1).trim();

    let target = '';
    let message = afterLevel;
    const colonIdx = afterLevel.indexOf(': ');
    if (colonIdx > 0) {
      target = afterLevel.substring(0, colonIdx);
      message = afterLevel.substring(colonIdx + 2);
    }

    return { level: level.toLowerCase(), time, target, message };
  }

  window.mgmtLogs = mgmtLogs;
  window.toggleLogAutoRefresh = toggleLogAutoRefresh;
  window.clearLogs = clearLogs;
  window.refreshLogs = refreshLogs;
})();
