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
        if (isForcedUpgrade(info)) {
          showUpgradeModal(info, 'auto', { force: true, autoInstall: true });
        } else {
          showToast(`发现新版本 ${info.latest}，可在服务概览中安装`, 'info');
        }
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

  function isForcedUpgrade(info) {
    return Boolean(info && info.has_update && info.force_update_applies);
  }

  function showUpgradeModal(info, source, options = {}) {
    const forced = Boolean(options.force || isForcedUpgrade(info));
    const existing = document.getElementById('upgradeModal');
    if (existing) existing.remove();

    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';
    overlay.id = 'upgradeModal';
    overlay.innerHTML = `
        <div class="modal-box" style="max-width:640px;">
          <div class="modal-header">
            <h3>${forced ? '必须安装关键更新' : '发现新版本'} ${esc(info.latest || '')}</h3>
            ${forced ? '' : '<button class="modal-close" id="upgradeCloseBtn" type="button">✕</button>'}
          </div>
          <div class="modal-body" data-source="${escAttr(source || '')}">
            ${forced ? '<div class="info-banner warning" style="margin-bottom:14px;">当前版本低于本次发布要求，DEX AI 将自动下载并安装关键更新。更新完成后会重启应用。</div>' : ''}
            <div style="margin-bottom:16px;font-family:var(--font-mono);">
              <p style="color:var(--text-secondary);margin-bottom:4px;">当前版本</p>
              <p style="font-size:16px;color:var(--amber);margin-bottom:12px;">${esc(info.current)}</p>
              <p style="color:var(--text-secondary);margin-bottom:4px;">最新版本</p>
              <p style="font-size:18px;color:var(--green);margin-bottom:12px;">${esc(info.latest)}</p>
            </div>
            ${forced && info.force_update_reason ? '<p style="color:var(--text-primary);font-size:12px;margin:0 0 12px;">原因：' + esc(info.force_update_reason) + '</p>' : ''}
            ${forced && info.minimum_supported_version ? '<p style="color:var(--text-muted);font-size:11px;margin:0 0 12px;">最低可继续使用版本：' + esc(info.minimum_supported_version) + '</p>' : ''}
            ${info.endpoint ? '<p style="color:var(--text-muted);font-size:11px;margin:0 0 12px;">更新源：' + esc(info.endpoint) + '</p>' : ''}
            ${info.changelog ? '<div style="border-top:1px solid var(--border-subtle);padding-top:12px;"><p style="color:var(--text-secondary);margin-bottom:8px;">更新日志</p><pre style="font-family:var(--font-mono);font-size:10px;color:var(--text-primary);max-height:200px;overflow-y:auto;white-space:pre-wrap;">' + esc(info.changelog) + '</pre></div>' : ''}
            <p id="upgradeStatusText" style="margin-top:12px;color:var(--text-muted);font-size:11px;">${forced ? '正在准备安装关键更新。' : '安装完成后会询问是否立即重启。不会无提示重启。'}</p>
          </div>
          <div style="padding:12px 20px;display:flex;gap:10px;justify-content:flex-end;border-top:1px solid var(--border-subtle);">
            ${forced ? '<button class="btn btn-ghost" id="upgradeExitBtn" type="button">退出</button>' : '<button class="btn btn-ghost" id="upgradeCancelBtn" type="button">取消</button>'}
            <button class="btn btn-primary" id="btnConfirmUpgrade" type="button">${forced ? '安装关键更新' : '⇡ 下载并安装'}</button>
          </div>
        </div>`;
    overlay.addEventListener('click', (event) => {
      if (!forced && event.target === overlay) overlay.remove();
    });
    document.body.appendChild(overlay);

    document.getElementById('upgradeCloseBtn')?.addEventListener('click', () => overlay.remove());
    document.getElementById('upgradeCancelBtn')?.addEventListener('click', () => overlay.remove());
    document.getElementById('upgradeExitBtn')?.addEventListener('click', async () => {
      try {
        await invoke('exit_app');
      } catch (_) {
        window.close();
      }
    });

    const installUpgrade = async () => {
      if (!forced && !await showConfirm(`确定下载并安装 ${info.latest || '新版本'} 吗？安装完成后会询问是否立即重启 DEX AI。`)) return;
      if (!forced) overlay.remove();
      const updateBtn = document.getElementById('btnUpdate');
      const confirmBtn = document.getElementById('btnConfirmUpgrade');
      const statusText = document.getElementById('upgradeStatusText');
      if (updateBtn) updateBtn.disabled = true;
      if (confirmBtn) {
        confirmBtn.disabled = true;
        confirmBtn.textContent = forced ? '正在安装...' : '正在安装...';
      }
      if (statusText) statusText.textContent = '正在下载并安装更新...';
      showToast(forced ? '正在安装关键更新...' : '正在下载并安装更新...', 'info');
      try {
        const result = await invoke('run_upgrade');
        clearUpdateInfo();
        const message = result && result.message ? result.message : '更新已安装。请重启 DEX AI 完成切换。';
        showToast(message, 'success');
        if (result && result.restart_required) {
          if (forced) {
            if (statusText) statusText.textContent = '关键更新已安装，正在重启 DEX AI...';
            window.setTimeout(() => invoke('restart_app'), 700);
            return;
          }
          if (await showConfirm('更新已安装。是否立即重启 DEX AI 完成切换？')) {
            showToast('正在重启 DEX AI...', 'info');
            await invoke('restart_app');
            return;
          }
        }
      } catch (error) {
        const message = '升级失败: ' + error;
        showToast(message, 'error');
        if (statusText) statusText.textContent = forced ? `${message}。请检查网络后重试，或退出应用。` : message;
      }
      if (confirmBtn) {
        confirmBtn.disabled = false;
        confirmBtn.textContent = forced ? '重试安装' : '⇡ 下载并安装';
      }
      if (updateBtn) updateBtn.disabled = false;
    };

    document.getElementById('btnConfirmUpgrade')?.addEventListener('click', installUpgrade);
    if (forced && options.autoInstall) {
      window.setTimeout(installUpgrade, 300);
    }
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
