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

  async function presetModelMapping() {
    const cfg = await invoke('get_config').catch(() => ({}));
    let map = {};
    try {
      map = JSON.parse(cfg.model_map || '{}');
    } catch (_) {}
    const defaults = {
      'gpt-5.5': 'deepseek-v4-pro',
      'gpt-5': 'deepseek-v4-pro',
      'gpt-4o': 'deepseek-v4-pro',
      'gpt-4o-mini': 'deepseek-v4-pro',
      'gpt-4.1': 'deepseek-v4-pro',
      'o3-model': 'deepseek-v4-pro',
      'o4-model': 'deepseek-v4-pro',
    };
    let changed = false;
    for (const [key, value] of Object.entries(defaults)) {
      if (!map[key]) {
        map[key] = value;
        changed = true;
      }
    }
    if (changed) {
      await invoke('save_config', { config: { ...cfg, model_map: JSON.stringify(map) } });
    }
  }

  async function autoCheckUpgrade() {
    if (!window.DeeCodexTauri?.hasTauri) {
      applyUpdateIndicator(false);
      return;
    }
    const last = deeStorage.getItem('lastUpgradeCheck');
    const today = new Date().toISOString().slice(0, 10);
    if (last === today) {
      applyUpdateIndicator(Boolean(deeStorage.getItem('updateAvailable')));
      return;
    }

    deeStorage.setItem('lastUpgradeCheck', today);
    try {
      const info = await invoke('check_upgrade');
      if (info.has_update) {
        window._updateInfo = info;
        deeStorage.setItem('updateAvailable', '1');
        applyUpdateIndicator(true);
      } else {
        window._updateInfo = null;
        deeStorage.removeItem('updateAvailable');
        applyUpdateIndicator(false);
      }
    } catch (_) {}
  }

  function applyUpdateIndicator(hasUpdate) {
    const logo = document.querySelector('.sidebar-brand .logo');
    const ver = document.getElementById('sidebarVersion');
    const btn = document.getElementById('btnUpdate');
    if (hasUpdate) {
      if (logo) {
        logo.classList.add('update-available');
        logo.style.animation = 'logo-pulse-amber 3s ease-in-out infinite';
      }
      if (ver && !ver.querySelector('.update-dot')) {
        const dot = document.createElement('span');
        dot.className = 'update-dot';
        ver.insertBefore(dot, ver.firstChild);
      }
      if (btn && !btn.querySelector('.update-dot')) {
        btn.insertAdjacentHTML('afterbegin', '<span class="update-dot" style="vertical-align:middle;margin-right:3px;"></span>');
      }
    } else {
      if (logo) {
        logo.classList.remove('update-available');
        logo.style.animation = '';
      }
      ver?.querySelector('.update-dot')?.remove();
      btn?.querySelector('.update-dot')?.remove();
    }
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

  async function mgmtUpdate() {
    const btn = document.getElementById('btnUpdate');
    if (btn) btn.disabled = true;
    showToast('正在检查更新...', 'info');

    try {
      const info = await invoke('check_upgrade');
      if (!info.has_update) {
        showToast('已是最新版本 (' + info.current + ')', 'success');
        window._updateInfo = null;
        deeStorage.removeItem('updateAvailable');
        applyUpdateIndicator(false);
        if (btn) btn.disabled = false;
        return;
      }

      window._updateInfo = info;
      deeStorage.setItem('updateAvailable', '1');
      applyUpdateIndicator(true);

      const overlay = document.createElement('div');
      overlay.className = 'modal-overlay';
      overlay.id = 'upgradeModal';
      overlay.innerHTML = `
        <div class="modal-box" style="max-width:640px;">
          <div class="modal-header">
            <h3>⇡ 发现新版本</h3>
            <button class="modal-close" id="upgradeCloseBtn" type="button">✕</button>
          </div>
          <div class="modal-body">
            <div style="margin-bottom:16px;font-family:var(--font-mono);">
              <p style="color:var(--text-secondary);margin-bottom:4px;">当前版本</p>
              <p style="font-size:16px;color:var(--amber);margin-bottom:12px;">${esc(info.current)}</p>
              <p style="color:var(--text-secondary);margin-bottom:4px;">最新版本</p>
              <p style="font-size:18px;color:var(--green);margin-bottom:12px;">${esc(info.latest)}</p>
            </div>
            ${info.changelog ? '<div style="border-top:1px solid var(--border-subtle);padding-top:12px;"><p style="color:var(--text-secondary);margin-bottom:8px;">更新日志</p><pre style="font-family:var(--font-mono);font-size:10px;color:var(--text-primary);max-height:200px;overflow-y:auto;white-space:pre-wrap;">' + esc(info.changelog) + '</pre></div>' : ''}
          </div>
          <div style="padding:12px 20px;display:flex;gap:10px;justify-content:flex-end;border-top:1px solid var(--border-subtle);">
            <button class="btn btn-ghost" id="upgradeCancelBtn" type="button">取消</button>
            <button class="btn btn-primary" id="btnConfirmUpgrade" type="button">⇡ 立即升级</button>
          </div>
        </div>`;
      overlay.addEventListener('click', (event) => {
        if (event.target === overlay) overlay.remove();
      });
      document.body.appendChild(overlay);

      document.getElementById('upgradeCloseBtn')?.addEventListener('click', () => overlay.remove());
      document.getElementById('upgradeCancelBtn')?.addEventListener('click', () => overlay.remove());
      document.getElementById('btnConfirmUpgrade')?.addEventListener('click', async () => {
        overlay.remove();
        const updateBtn = document.getElementById('btnUpdate');
        if (updateBtn) updateBtn.disabled = true;
        showToast('正在启动升级...', 'info');
        try {
          const message = await invoke('run_upgrade');
          showToast(message, 'success');
        } catch (error) {
          showToast('升级失败: ' + error, 'error');
        }
        if (updateBtn) updateBtn.disabled = false;
      });
    } catch (error) {
      showToast('检查更新失败: ' + error, 'error');
    }

    if (btn) btn.disabled = false;
  }

  window.mgmtLaunchCodex = mgmtLaunchCodex;
  window.mgmtToggle = mgmtToggle;
  window.presetModelMapping = presetModelMapping;
  window.autoCheckUpgrade = autoCheckUpgrade;
  window.applyUpdateIndicator = applyUpdateIndicator;
  window.mgmtRestart = mgmtRestart;
  window.mgmtUpdate = mgmtUpdate;
})();
