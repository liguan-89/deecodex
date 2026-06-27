(function () {
  const invoke = (...args) => window.invoke(...args);

  window._cdpLaunched = false;

  async function mgmtLaunchCodex() {
    if (window._cdpLaunched) {
      try {
        await invoke('stop_codex_cdp');
        window._cdpLaunched = false;
        window.renderPanel?.('status');
        showToast('CDP 已停止', 'info');
      } catch (error) {
        showToast('停止 CDP 失败: ' + error, 'error');
      }
      return;
    }
    try {
      const status = await invoke('get_service_status');
      if (!status.running) {
        await invoke('start_service');
        for (let i = 0; i < 20; i++) {
          await new Promise((resolve) => setTimeout(resolve, 500));
          const nextStatus = await invoke('get_service_status');
          if (nextStatus.running) break;
        }
      }
      await invoke('launch_codex_cdp');
      window._cdpLaunched = true;
      window.renderPanel?.('status');
      showToast('CDP 已启动', 'success');
    } catch (error) {
      showToast('启动 CDP 失败: ' + error, 'error');
    }
  }

  async function mgmtToggle() {
    const btn = document.getElementById('btnToggle');
    if (btn) btn.disabled = true;

    try {
      const status = await invoke('get_service_status');
      if (status.running) {
        await invoke('stop_service');
        showToast('服务正在停止', 'info');
      } else {
        await invoke('start_service');
        showToast('服务已启动', 'success');
      }
    } catch (error) {
      showToast('操作失败: ' + error, 'error');
    }

    await window.loadStatus?.();
    if (btn) btn.disabled = false;
  }

  async function autoCheckUpgrade() {
    restoreStoredUpdateInfo();
    applyUpdateIndicator(deeStorage.getItem('updateAvailable') === '1');

    const today = new Date().toISOString().slice(0, 10);
    deeStorage.setItem('lastUpgradeCheck', today);
    try {
      const info = await invoke('check_upgrade');
      if (info.has_update) {
        rememberUpdateInfo(info);
        showToast(`发现新版本 ${info.latest}，可在服务概览中安装`, 'info');
      } else {
        clearUpdateInfo();
      }
    } catch (_) {}
  }

  function restoreStoredUpdateInfo() {
    const raw = deeStorage.getItem('updateInfo');
    if (!raw) return null;
    try {
      const info = JSON.parse(raw);
      if (info && info.has_update) {
        window._updateInfo = info;
        return info;
      }
    } catch (_) {}
    return null;
  }

  function rememberUpdateInfo(info) {
    window._updateInfo = info;
    deeStorage.setItem('updateAvailable', '1');
    deeStorage.setItem('updateLatest', info.latest || '');
    deeStorage.setItem('updateChangelog', info.changelog || '');
    deeStorage.setItem('updateInfo', JSON.stringify(info));
    applyUpdateIndicator(true);
    if (currentPanel === 'status') renderPanel('status');
  }

  function clearUpdateInfo() {
    window._updateInfo = null;
    deeStorage.removeItem('updateAvailable');
    deeStorage.removeItem('updateLatest');
    deeStorage.removeItem('updateChangelog');
    deeStorage.removeItem('updateInfo');
    applyUpdateIndicator(false);
    if (currentPanel === 'status') renderPanel('status');
  }

  function applyUpdateIndicator(hasUpdate) {
    const ver = document.getElementById('dashboardVersion');
    const btn = document.getElementById('btnUpdate');
    syncGlobalUpdatePrompt(hasUpdate);
    if (hasUpdate) {
      if (ver && !ver.querySelector('.update-dot')) {
        const dot = document.createElement('span');
        dot.className = 'update-dot';
        ver.insertBefore(dot, ver.firstChild);
      }
      if (btn && !btn.querySelector('.update-dot')) {
        btn.insertAdjacentHTML('afterbegin', '<span class="update-dot" style="vertical-align:middle;margin-right:3px;"></span>');
      }
    } else {
      ver?.querySelector('.update-dot')?.remove();
      btn?.querySelector('.update-dot')?.remove();
    }
  }

  function syncGlobalUpdatePrompt(hasUpdate) {
    const old = document.getElementById('globalUpdatePrompt');
    if (!hasUpdate) {
      old?.remove();
      return;
    }

    const info = window._updateInfo || restoreStoredUpdateInfo() || {};
    const latest = info.latest || deeStorage.getItem('updateLatest') || '新版本';
    const changelog = info.changelog || deeStorage.getItem('updateChangelog') || '';
    const firstLine = changelog.split(/\r?\n/).map(line => line.trim()).find(Boolean);

    const prompt = old || document.createElement('button');
    prompt.id = 'globalUpdatePrompt';
    prompt.type = 'button';
    prompt.className = 'global-update-prompt';
    prompt.title = '查看更新';
    prompt.innerHTML = `
      <span class="update-dot" aria-hidden="true"></span>
      <span class="global-update-copy">
        <strong>发现新版本 ${esc(latest)}</strong>
        ${firstLine ? `<small>${esc(firstLine)}</small>` : '<small>查看更新内容并安装</small>'}
      </span>`;
    prompt.onclick = () => showStoredUpdatePrompt('global');
    if (!old) document.body.appendChild(prompt);
  }

  async function mgmtRestart() {
    if (!await showConfirm('确定要重启 deecodex 服务吗？')) return;

    const btn = document.getElementById('btnRestart');
    if (btn) btn.disabled = true;

    try {
      showToast('正在停止服务...', 'info');
      await invoke('stop_service');
      showToast('服务已停止，正在重新启动...', 'info');
      await new Promise((resolve) => setTimeout(resolve, 800));
      await invoke('start_service');
      showToast('服务已重启', 'success');
    } catch (error) {
      showToast('重启失败: ' + error, 'error');
    }

    await window.loadStatus?.();
    if (btn) btn.disabled = false;
  }

  function showUpgradeModal(info, source) {
    const existing = document.getElementById('upgradeModal');
    if (existing) existing.remove();

    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';
    overlay.id = 'upgradeModal';
    overlay.innerHTML = `
        <div class="modal-box" style="max-width:640px;">
          <div class="modal-header">
            <h3>发现新版本 ${esc(info.latest || '')}</h3>
            <button class="modal-close" id="upgradeCloseBtn" type="button">✕</button>
          </div>
          <div class="modal-body" data-source="${escAttr(source || '')}">
            <div style="margin-bottom:16px;font-family:var(--font-mono);">
              <p style="color:var(--text-secondary);margin-bottom:4px;">当前版本</p>
              <p style="font-size:16px;color:var(--amber);margin-bottom:12px;">${esc(info.current)}</p>
              <p style="color:var(--text-secondary);margin-bottom:4px;">最新版本</p>
              <p style="font-size:18px;color:var(--green);margin-bottom:12px;">${esc(info.latest)}</p>
            </div>
            ${info.endpoint ? '<p style="color:var(--text-muted);font-size:11px;margin:0 0 12px;">更新源：' + esc(info.endpoint) + '</p>' : ''}
            ${info.changelog ? '<div style="border-top:1px solid var(--border-subtle);padding-top:12px;"><p style="color:var(--text-secondary);margin-bottom:8px;">更新日志</p><pre style="font-family:var(--font-mono);font-size:10px;color:var(--text-primary);max-height:200px;overflow-y:auto;white-space:pre-wrap;">' + esc(info.changelog) + '</pre></div>' : ''}
            <p style="margin-top:12px;color:var(--text-muted);font-size:11px;">安装完成后会询问是否立即重启。不会无提示重启。</p>
          </div>
          <div style="padding:12px 20px;display:flex;gap:10px;justify-content:flex-end;border-top:1px solid var(--border-subtle);">
            <button class="btn btn-ghost" id="upgradeCancelBtn" type="button">取消</button>
            <button class="btn btn-primary" id="btnConfirmUpgrade" type="button">⇡ 下载并安装</button>
          </div>
        </div>`;
    overlay.addEventListener('click', (event) => {
      if (event.target === overlay) overlay.remove();
    });
    document.body.appendChild(overlay);

    document.getElementById('upgradeCloseBtn')?.addEventListener('click', () => overlay.remove());
    document.getElementById('upgradeCancelBtn')?.addEventListener('click', () => overlay.remove());
    document.getElementById('btnConfirmUpgrade')?.addEventListener('click', async () => {
      if (!await showConfirm(`确定下载并安装 ${info.latest || '新版本'} 吗？安装完成后会询问是否立即重启 DEX AI。`)) return;
      overlay.remove();
      const updateBtn = document.getElementById('btnUpdate');
      if (updateBtn) updateBtn.disabled = true;
      showToast('正在下载并安装更新...', 'info');
      try {
        const result = await invoke('run_upgrade');
        clearUpdateInfo();
        const message = result && result.message ? result.message : '更新已安装。请重启 DEX AI 完成切换。';
        showToast(message, 'success');
        if (result && result.restart_required) {
          const shouldRestart = await showConfirm('更新已安装。是否立即重启 DEX AI 完成切换？');
          if (shouldRestart) {
            showToast('正在重启 DEX AI...', 'info');
            await invoke('restart_app');
            return;
          }
        }
      } catch (error) {
        showToast('升级失败: ' + error, 'error');
      }
      if (updateBtn) updateBtn.disabled = false;
    });
  }

  function showStoredUpdatePrompt(source) {
    const info = window._updateInfo || restoreStoredUpdateInfo();
    if (info && info.has_update) {
      showUpgradeModal(info, source);
    } else {
      mgmtUpdate();
    }
  }

  async function mgmtUpdate() {
    const btn = document.getElementById('btnUpdate');
    if (btn) btn.disabled = true;
    showToast('正在检查更新...', 'info');

    try {
      const info = await invoke('check_upgrade');
      if (!info.has_update) {
        showToast('已是最新版本 (' + info.current + ')', 'success');
        clearUpdateInfo();
        if (btn) btn.disabled = false;
        return;
      }

      rememberUpdateInfo(info);
      showUpgradeModal(info, 'manual');
    } catch (error) {
      showToast('检查更新失败: ' + error, 'error');
    }

    if (btn) btn.disabled = false;
  }

  window.mgmtLaunchCodex = mgmtLaunchCodex;
  window.mgmtToggle = mgmtToggle;
  window.autoCheckUpgrade = autoCheckUpgrade;
  window.applyUpdateIndicator = applyUpdateIndicator;
  window.showStoredUpdatePrompt = showStoredUpdatePrompt;
  window.mgmtRestart = mgmtRestart;
  window.mgmtUpdate = mgmtUpdate;
})();
