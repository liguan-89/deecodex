// 插件详情、配置、能力和连接操作
function pluginConfigFieldId(pluginId, key) {
  return 'pluginCfg_' + String(pluginId).replace(/[^a-zA-Z0-9_-]/g, '_') + '_' + String(key).replace(/[^a-zA-Z0-9_-]/g, '_');
}

function pluginConfigValue(p, key, schema) {
  if (p && p.config && Object.prototype.hasOwnProperty.call(p.config, key)) return p.config[key];
  if (schema && Object.prototype.hasOwnProperty.call(schema, 'default')) return schema.default;
  if (schema && schema.type === 'boolean') return false;
  if (schema && (schema.type === 'array' || schema.type === 'object')) return schema.type === 'array' ? [] : {};
  return '';
}

function renderPluginConfigField(p, key) {
  const schema = pluginSchemaProperties(p)[key] || {};
  const value = pluginConfigValue(p, key, schema);
  const fieldId = pluginConfigFieldId(p.id, key);
  const label = schema.title || key;
  const desc = schema.description ? `<span class="plugin-config-desc">${esc(schema.description)}</span>` : '';
  const type = schema.type || (Array.isArray(value) ? 'array' : typeof value);
  let control = '';

  if (Array.isArray(schema.enum)) {
    control = `<select id="${escAttr(fieldId)}" data-plugin-config-key="${escAttr(key)}" data-plugin-config-type="string" class="plugin-config-input">
      ${schema.enum.map(item => `<option value="${escAttr(item)}" ${String(value) === String(item) ? 'selected' : ''}>${esc(item)}</option>`).join('')}
    </select>`;
  } else if (type === 'boolean') {
    control = `<label class="plugin-config-switch">
      <input id="${escAttr(fieldId)}" data-plugin-config-key="${escAttr(key)}" data-plugin-config-type="boolean" type="checkbox" ${value ? 'checked' : ''}>
      <span>${value ? '开启' : '关闭'}</span>
    </label>`;
  } else if (type === 'integer' || type === 'number') {
    control = `<input id="${escAttr(fieldId)}" data-plugin-config-key="${escAttr(key)}" data-plugin-config-type="${escAttr(type)}" class="plugin-config-input" type="number" value="${escAttr(value ?? '')}">`;
  } else if (type === 'array') {
    const text = Array.isArray(value) ? value.join('\n') : String(value || '');
    control = `<textarea id="${escAttr(fieldId)}" data-plugin-config-key="${escAttr(key)}" data-plugin-config-type="array" class="plugin-config-input plugin-config-textarea">${esc(text)}</textarea>`;
  } else if (type === 'object') {
    const text = typeof value === 'object' ? JSON.stringify(value, null, 2) : String(value || '{}');
    control = `<textarea id="${escAttr(fieldId)}" data-plugin-config-key="${escAttr(key)}" data-plugin-config-type="object" class="plugin-config-input plugin-config-textarea">${esc(text)}</textarea>`;
  } else {
    control = `<input id="${escAttr(fieldId)}" data-plugin-config-key="${escAttr(key)}" data-plugin-config-type="string" class="plugin-config-input" value="${escAttr(value ?? '')}">`;
  }

  return `<label class="plugin-config-field">
    <span class="plugin-config-label">${esc(label)}</span>
    ${control}
    ${desc}
  </label>`;
}

function renderPluginConfigSection(p) {
  const keys = pluginConfigKeys(p);
  if (!keys.length) return '';
  return `<div class="plugin-detail-section plugin-config-section">
    <h3>配置</h3>
    <div class="plugin-config-grid">${keys.map(key => renderPluginConfigField(p, key)).join('')}</div>
    <div class="plugin-config-actions">
      <button class="btn-apply" onclick="savePluginConfig('${escAttr(p.id)}')">保存配置</button>
    </div>
  </div>`;
}

function renderPluginAccountsSection(p, accountsHtml) {
  const label = pluginAccountLabel(p);
  return `<div class="plugin-detail-section">
    <h3>${esc(label)}</h3>
    ${accountsHtml}
    <div class="pc-add-account">
      <input id="addAccountId_${escAttr(p.id)}" placeholder="新 ${escAttr(label)} ID">
      <button class="btn-apply" onclick="addPluginAccount('${escAttr(p.id)}')">添加${esc(label)}</button>
    </div>
    <div id="qrContainer_${escAttr(p.id)}" class="pc-qr"></div>
  </div>`;
}

function isMobileCodexPlugin(p) {
  return Boolean(p && p.id === 'deecodex-mobile-codex' && p.first_party);
}

function pluginRuntimeStateFromList(plugins, pluginId) {
  const plugin = (plugins || []).find(item => String(item.id || '') === String(pluginId || ''));
  return plugin ? normalizePluginState(plugin.state) : 'missing';
}

function mobileCodexRuntimeBecameAvailable(previousPlugins, nextPlugins, detailId) {
  if (detailId !== 'deecodex-mobile-codex') return false;
  const previousState = pluginRuntimeStateFromList(previousPlugins, detailId);
  const nextState = pluginRuntimeStateFromList(nextPlugins, detailId);
  return previousState !== 'running' && nextState === 'running';
}

function mobileCodexDeviceIsOnline(device) {
  if (!device || device.revokedAt) return false;
  if (typeof device.online === 'boolean') return device.online;
  return Boolean(device.networkPath && device.networkPath !== 'offline');
}

function renderMobileCodexDevice(device, currentDeviceId) {
  const revoked = Boolean(device.revokedAt);
  const online = mobileCodexDeviceIsOnline(device);
  const status = revoked ? '已撤销' : (online ? '在线' : '离线');
  const isCurrent = String(device.id || '') === String(currentDeviceId || '');
  return `<div class="mc-device-row ${revoked ? 'is-revoked' : ''}">
    <div class="mc-device-icon">${device.deviceType === 'mobile' ? 'M' : 'C'}</div>
    <div class="mc-device-main">
      <div class="mc-device-title">${esc(device.name || device.id)} ${isCurrent ? '<span class="mc-device-self">本机</span>' : ''}</div>
      <div class="mc-device-meta">${esc(device.platform || '-')} · ${esc(device.deviceType || '-')} · ${esc(device.companionVersion || device.appVersion || '-')}</div>
    </div>
    <div class="mc-device-network ${online ? 'is-online' : ''}"><span></span>${esc(status)}</div>
    <div class="mc-device-actions">
      ${revoked ? '' : `<button class="btn-apply" onclick="renameMobileCodexDevice('${escAttr(device.id)}')">重命名</button>
      <button class="btn-apply btn-danger" onclick="revokeMobileCodexDevice('${escAttr(device.id)}',${isCurrent ? 'true' : 'false'})">撤销</button>`}
    </div>
  </div>`;
}

function renderMobileCodexRenameModal(currentName) {
  return `<div class="modal-box plugin-action-modal mc-rename-modal">
    <div class="modal-header"><h3>重命名设备</h3></div>
    <div class="modal-body plugin-action-body">
      <label class="mc-rename-field">
        <span>设备名称</span>
        <input id="mcRenameDeviceName" class="plugin-config-input" maxlength="80" value="${escAttr(currentName || '')}" autocomplete="off">
      </label>
      <div id="mcRenameDeviceError" class="mc-rename-error" aria-live="polite"></div>
    </div>
    <div class="plugin-preview-actions">
      <button class="btn btn-primary" id="mcRenameDeviceSave" type="button">保存</button>
      <button class="btn btn-ghost" id="mcRenameDeviceCancel" type="button">取消</button>
    </div>
  </div>`;
}

function showMobileCodexRenameModal(currentName) {
  return new Promise(function (resolve) {
    const existing = document.getElementById('mcRenameDeviceModal');
    if (existing) existing.remove();
    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';
    overlay.id = 'mcRenameDeviceModal';
    overlay.innerHTML = renderMobileCodexRenameModal(currentName);
    document.body.appendChild(overlay);

    const input = document.getElementById('mcRenameDeviceName');
    const error = document.getElementById('mcRenameDeviceError');
    const save = document.getElementById('mcRenameDeviceSave');
    const cancel = document.getElementById('mcRenameDeviceCancel');
    let settled = false;

    function cleanup(value) {
      if (settled) return;
      settled = true;
      document.removeEventListener('keydown', onKeyDown);
      overlay.remove();
      resolve(value);
    }

    function submit() {
      const name = String((input && input.value) || '').trim();
      if (!name) {
        if (error) error.textContent = '设备名称不能为空';
        if (input) input.focus();
        return;
      }
      if (Array.from(name).length > 80) {
        if (error) error.textContent = '设备名称不能超过 80 个字符';
        if (input) input.focus();
        return;
      }
      cleanup(name);
    }

    function onKeyDown(event) {
      if (event.key === 'Escape') cleanup(null);
      if (event.key === 'Enter' && event.target === input) {
        event.preventDefault();
        submit();
      }
    }

    if (save) save.onclick = submit;
    if (cancel) cancel.onclick = function () { cleanup(null); };
    overlay.addEventListener('click', function (event) {
      if (event.target === overlay) cleanup(null);
    });
    document.addEventListener('keydown', onKeyDown);
    if (input) {
      input.focus();
      input.select();
    }
  });
}

function mobileCodexConnectionStatus(service, error) {
  const networkPath = String((service && service.network && service.network.networkPath) || 'offline');
  if (error) {
    return {
      tone: 'error',
      title: '连接需要处理',
      description: String(error)
    };
  }
  if (!service || !service.running) {
    return {
      tone: 'offline',
      title: 'Mobile Codex 未启动',
      description: '启动 Companion 后，手机才能连接这台电脑。'
    };
  }
  if (networkPath === 'offline') {
    return {
      tone: 'online',
      title: 'Mobile Codex 已就绪',
      description: '局域网连接可直接使用；公网 P2P 通道会在手机通过外部网络连接时显示。'
    };
  }
  if (networkPath === 'lan') {
    return {
      tone: 'online',
      title: 'Mobile Codex 连接正常',
      description: '手机当前通过同一 Wi-Fi 的局域网直连访问这台电脑。'
    };
  }
  return {
    tone: 'online',
    title: 'Mobile Codex 连接正常',
    description: '手机可以通过安全的 P2P 网络访问这台电脑。'
  };
}

function renderMobileCodexAccountSection(p) {
  const state = _mobileCodexState;
  const user = state.user || {};
  const loggedIn = Boolean(user.loggedIn);
  const service = state.service || {};
  if (!state.loaded && state.loading) {
    return `<div class="plugin-detail-section mc-account-shell"><h3>Mobile Codex 账号</h3><div class="plugin-empty-line">正在读取本机账号状态…</div></div>`;
  }
  if (!loggedIn) {
    return `<div class="plugin-detail-section mc-account-shell">
      <div class="plugin-section-head"><h3>Mobile Codex 账号</h3><span class="mc-security-note">密码仅进入本机第一方插件</span></div>
      ${state.error ? `<div class="mc-inline-error">${esc(state.error)}</div>` : ''}
      <div class="mc-auth-grid">
        <div class="mc-auth-primary">
          <label><span>邮箱</span><input id="mcLoginEmail" class="plugin-config-input" type="email" autocomplete="username"></label>
          <label><span>密码</span><input id="mcLoginPassword" class="plugin-config-input" type="password" autocomplete="current-password"></label>
          <button class="btn-apply mc-primary-action" onclick="loginMobileCodex()">登录并启动 Companion</button>
        </div>
        <div class="mc-auth-secondary">
          <details><summary>邀请码注册</summary>
            <input id="mcInvitationCode" class="plugin-config-input" placeholder="邀请码">
            <input id="mcRegisterEmail" class="plugin-config-input" type="email" placeholder="邮箱">
            <input id="mcRegisterPassword" class="plugin-config-input" type="password" placeholder="密码，至少 12 位">
            <button class="btn-apply" onclick="registerMobileCodex()">提交注册</button>
          </details>
          <details><summary>验证邮箱</summary>
            <input id="mcVerifyToken" class="plugin-config-input" placeholder="邮箱验证 Token">
            <button class="btn-apply" onclick="verifyMobileCodexEmail()">完成验证</button>
          </details>
          <details><summary>忘记或重置密码</summary>
            <input id="mcRecoverEmail" class="plugin-config-input" type="email" placeholder="找回密码邮箱">
            <button class="btn-apply" onclick="forgotMobileCodexPassword()">发送重置邮件</button>
            <input id="mcResetToken" class="plugin-config-input" placeholder="重置 Token">
            <input id="mcResetPassword" class="plugin-config-input" type="password" placeholder="新密码">
            <button class="btn-apply" onclick="resetMobileCodexPassword()">重置密码</button>
          </details>
        </div>
      </div>
    </div>`;
  }
  const devices = state.devices || [];
  const externallyManaged = service.running && service.ownership === 'external';
  const connection = mobileCodexConnectionStatus(service, state.error);
  const chatgptBridge = String(service.chatgptBridge || 'unknown');
  const chatgptReady = chatgptBridge === 'shim';
  const chatgptHint = chatgptReady
    ? 'ChatGPT 已以桥接模式运行，手机可直接连接。'
    : (chatgptBridge === 'official_mode'
      ? 'ChatGPT 正在运行但未走桥接，点击下方按钮将重启为桥接模式。'
      : 'ChatGPT 未运行，点击下方按钮以桥接模式启动。');
  const ent = user.entitlement || {};
  let entHtml = '';
  if (ent.known) {
    entHtml = ent.authorized
      ? `<div class="mc-entitlement-line ok"><span>套餐授权</span><b>有效</b>${ent.planName ? ' · ' + esc(ent.planName) : ''}${ent.planCode ? '（' + esc(ent.planCode) + '）' : ''}</div>`
      : `<div class="mc-entitlement-line warn"><span>套餐授权</span><b>未生效</b>（${esc(ent.serviceStatus || '未知')}）插件市场安装 / 复制 / 生成将被锁定，可在个人中心管理授权</div>`;
  } else if (user.loggedIn) {
    entHtml = `<div class="mc-entitlement-line warn"><span>套餐授权</span><b>状态未知</b>（插件版本过旧或网络不可用），可在个人中心确认账号状态</div>`;
  }
  return `<div class="plugin-detail-section mc-account-shell">
    <div class="mc-connection-card ${escAttr(connection.tone)}">
      <span class="mc-connection-indicator" aria-hidden="true"></span>
      <div class="mc-connection-copy">
        <strong>${esc(connection.title)}</strong>
        <span>${esc(connection.description)}</span>
      </div>
      <button class="btn-apply" onclick="loadMobileCodexState(true)">刷新状态</button>
    </div>
    <div class="plugin-section-head">
      <h3>账号</h3>
      <div class="mc-account-actions">
        <button class="btn-apply btn-danger" onclick="logoutMobileCodex()">退出登录</button>
      </div>
    </div>
    <div class="mc-account-profile">
      <div class="mc-account-avatar">${esc(String(user.email || 'M').slice(0, 1).toUpperCase())}</div>
      <div>
        <strong>${esc(user.email || '-')}</strong>
        <span>${devices.length} 台设备已绑定</span>
      </div>
    </div>
    ${entHtml}
    <div class="mc-service-actions">
      <button class="btn-apply" ${externallyManaged ? 'disabled' : ''} onclick="setMobileCodexService(${service.running ? 'false' : 'true'})">${externallyManaged ? '系统 Companion 运行中' : (service.running ? '停止 Companion' : '启动 Companion')}</button>
      <span>${externallyManaged ? '由系统服务管理' : (service.running ? '关闭后，手机将无法连接这台电脑。' : '启动后，手机才能连接这台电脑。')}</span>
      <button class="btn-apply" ${chatgptReady ? 'disabled' : ''} onclick="launchMobileCodexChatgpt()">${chatgptReady ? 'ChatGPT 已桥接' : '启动 ChatGPT（桥接模式）'}</button>
      <span>${chatgptHint}</span>
    </div>
    <div class="mc-device-head"><strong>我的设备</strong><span>${devices.length} 台</span></div>
    <div class="mc-device-list">${devices.length ? devices.map(device => renderMobileCodexDevice(device, user.deviceId)).join('') : '<div class="plugin-empty-line">暂无设备</div>'}</div>
  </div>`;
}

function firstPartyPluginUpdateKey(kind) {
  if (kind === 'debug') return 'debug';
  return kind === 'desktop' ? 'desktopSync' : 'mobileCodex';
}

function firstPartyPluginUpdateCommand(kind, action) {
  if (kind === 'debug') {
    return action === 'check' ? 'check_debug_plugin_update' : 'update_debug_plugin';
  }
  const desktop = kind === 'desktop';
  return action === 'check'
    ? (desktop ? 'check_desktop_sync_plugin_update' : 'check_mobile_codex_plugin_update')
    : (desktop ? 'update_desktop_sync_plugin' : 'update_mobile_codex_plugin');
}

function renderFirstPartyPluginUpdateSection(plugin, kind) {
  const key = firstPartyPluginUpdateKey(kind);
  const state = _firstPartyPluginUpdates[key];
  const label = kind === 'debug' ? 'DEXAI Debug' : (kind === 'desktop' ? 'Desktop Sync' : 'Mobile Codex');
  const plan = state.plan;
  const latest = plan && plan.latestVersion;
  const available = Boolean(plan && plan.updateAvailable);
  const notes = plan && Array.isArray(plan.releaseNotes) ? plan.releaseNotes : [];
  return `<div class="plugin-detail-section mc-update-section">
    <div class="plugin-section-head">
      <div>
        <h3>${label} 更新</h3>
        <p class="mc-sync-subtitle">当前 v${esc(plugin && plugin.version || '未安装')}。检查不会下载或安装，只有点击“立即更新”才会替换组件。</p>
      </div>
      <button class="btn-apply" ${state.checking || state.applying ? 'disabled' : `onclick="checkFirstPartyPluginUpdate('${kind}')"`}>${state.checking ? '检查中…' : '检查更新'}</button>
    </div>
    ${state.error ? `<div class="mc-inline-error">${esc(state.error)}</div>` : ''}
    ${plan ? `<div class="mc-sync-overview ${available ? 'ready' : 'waiting'}">
      <div class="mc-sync-overview-copy">
        <strong>${available ? `发现 v${esc(latest)} 更新` : '已是最新版本'}</strong>
        <span>${available ? `更新包 ${Math.ceil(Number(plan.sizeBytes || 0) / 1024 / 1024)} MB，下载后将校验 SHA-256 和签名。` : '已完成版本、平台、最低 DEX AI 版本和发布清单检查。'}</span>
        ${notes.length ? `<small>${esc(notes.join(' · '))}</small>` : ''}
      </div>
      ${available ? `<button class="btn-apply" ${state.applying ? 'disabled' : `onclick="applyFirstPartyPluginUpdate('${kind}')"`}>${state.applying ? '更新中…' : '立即更新'}</button>` : ''}
    </div>` : ''}
  </div>`;
}

async function checkFirstPartyPluginUpdate(kind) {
  const state = _firstPartyPluginUpdates[firstPartyPluginUpdateKey(kind)];
  if (state.checking || state.applying) return;
  state.checking = true;
  state.error = '';
  await loadPluginsData();
  try {
    state.plan = await invoke(firstPartyPluginUpdateCommand(kind, 'check'));
  } catch (e) {
    state.error = String(e);
    showToast('检查更新失败: ' + esc(String(e)), 'error');
  } finally {
    state.checking = false;
    await loadPluginsData();
  }
}

async function applyFirstPartyPluginUpdate(kind) {
  const state = _firstPartyPluginUpdates[firstPartyPluginUpdateKey(kind)];
  if (state.applying || !state.plan || !state.plan.updateAvailable) return;
  state.applying = true;
  state.error = '';
  await loadPluginsData();
  try {
    const result = await invoke(firstPartyPluginUpdateCommand(kind, 'apply'));
    // apply 返回的是更新前的计划，updateAvailable 仍为 true；
    // 必须重新检查，让页面回到“已是最新版本”，避免同版本重复更新。
    try {
      state.plan = await invoke(firstPartyPluginUpdateCommand(kind, 'check'));
    } catch (e) {
      state.plan = result && result.update || null;
    }
    const label = kind === 'debug' ? 'DEXAI Debug' : (kind === 'desktop' ? 'Desktop Sync' : 'Mobile Codex');
    showToast(label + ' 已更新到 v' + ((result.manifest && result.manifest.version) || (state.plan && state.plan.latestVersion) || ''), 'success');
    await loadMobileCodexState(true);
    await refreshDesktopSyncState();
  } catch (e) {
    state.error = String(e);
    showToast('更新失败: ' + esc(String(e)), 'error');
  } finally {
    state.applying = false;
    await loadPluginsData();
  }
}

function desktopSyncPlugin() {
  return (_pluginsData || []).find(plugin => plugin.id === 'deecodex-desktop-sync') || null;
}

function desktopSyncPathLabel(path) {
  if (path === 'lan') return '局域网直连';
  if (path === 'derp') return '公网中继';
  if (path === 'direct') return '直连';
  return '';
}

function desktopSyncLastSeenLabel(value) {
  const timestamp = Number(value || 0);
  if (!Number.isFinite(timestamp) || timestamp <= 0) return '';
  const elapsed = Math.max(0, Date.now() - timestamp);
  if (elapsed < 3 * 60 * 1000) return '刚刚在线';
  if (elapsed < 60 * 60 * 1000) return `${Math.max(1, Math.round(elapsed / 60000))} 分钟前在线`;
  if (elapsed < 24 * 60 * 60 * 1000) return `${Math.max(1, Math.round(elapsed / 3600000))} 小时前在线`;
  return new Date(timestamp).toLocaleString('zh-CN', {
    month: 'numeric',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit'
  });
}

function desktopSyncResultSummary(result) {
  const summary = (result && result.summary) || {};
  return {
    applied: Number(summary.applied || 0),
    unchanged: Number(summary.unchanged || 0),
    conflicts: Number(summary.conflicts || 0),
    skipped: Number(summary.skipped || 0),
    failedPeers: Number(summary.failedPeers || 0)
  };
}

function renderDesktopSyncLastResult() {
  const result = _desktopSyncState.lastResult;
  if (!result) return '';
  const summary = desktopSyncResultSummary(result);
  const tone = result.ok ? (summary.conflicts || summary.skipped ? 'warning' : 'success') : 'error';
  return `<div class="mc-sync-result ${escAttr(tone)}">
    <div>
      <strong>${esc(result.message || (result.ok ? '同步完成' : '同步失败'))}</strong>
      <span>新增 ${summary.applied} · 已是最新 ${summary.unchanged} · 冲突 ${summary.conflicts} · 跳过 ${summary.skipped}</span>
    </div>
    ${summary.conflicts ? '<small>冲突会话保留本机版本，不会自动覆盖。</small>' : ''}
    ${summary.failedPeers ? `<small>${summary.failedPeers} 台电脑连接失败，请确认对方在线。</small>` : ''}
  </div>`;
}

function renderDesktopSyncPeer(peer, ready) {
  const syncing = String(_desktopSyncState.syncingDeviceId || '') === String(peer.deviceId || '');
  const pathLabel = desktopSyncPathLabel(peer.networkPath);
  const lastSeen = desktopSyncLastSeenLabel(peer.lastSeenAtMs);
  const meta = [peer.platform || '电脑', pathLabel, lastSeen].filter(Boolean).join(' · ');
  return `<div class="mc-sync-peer">
    <div class="mc-sync-peer-icon" aria-hidden="true"><span></span><span></span></div>
    <div class="mc-sync-peer-main">
      <strong>${esc(peer.name || '未命名电脑')}</strong>
      <span>${esc(meta || '同账号电脑')}</span>
    </div>
    <span class="mc-device-network is-online"><span></span>在线</span>
    <button class="btn-apply mc-sync-action" ${ready && !syncing ? `onclick="syncFromDesktopPeer('${escAttr(peer.deviceId)}')"` : 'disabled'}>
      ${syncing ? '正在同步…' : '同步到本机'}
    </button>
  </div>`;
}

function renderDesktopSyncSection() {
  const user = (_mobileCodexState && _mobileCodexState.user) || {};
  if (!user.loggedIn) return '';
  const state = _desktopSyncState;
  const plugin = desktopSyncPlugin();
  const updateSection = renderFirstPartyPluginUpdateSection(plugin, 'desktop');
  if (!plugin || !pluginIsEnabled(plugin)) {
    return `${updateSection}<div class="plugin-detail-section mc-sync-shell">
      <div class="plugin-section-head"><h3>多机同步</h3></div>
      <div class="mc-inline-error">多机同步组件未安装或已停用。</div>
    </div>`;
  }
  const status = state.status || {};
  const peers = state.peers || [];
  const onlinePeers = peers.filter(peer => peer.online && peer.syncable);
  const hiddenOfflineCount = Math.max(0, peers.length - onlinePeers.length);
  const ready = Boolean(status.ready) && !state.loading && !state.syncingDeviceId;
  return `${updateSection}<div class="plugin-detail-section mc-sync-shell">
    <div class="plugin-section-head">
      <div>
        <h3>多机同步</h3>
        <p class="mc-sync-subtitle">使用当前 Mobile Codex 账号，不需要再次登录</p>
      </div>
      <button class="btn-apply" ${state.loading ? 'disabled' : 'onclick="refreshDesktopSyncState()"'}>${state.loading ? '刷新中…' : '刷新'}</button>
    </div>
    <div class="mc-sync-overview ${status.ready ? 'ready' : 'waiting'}">
      <div class="mc-sync-overview-copy">
        <strong>${esc(status.ready ? '可以同步' : (status.message || '正在检查同步状态'))}</strong>
        <span>只同步已在本机添加项目的会话，不复制源码或项目文件。</span>
      </div>
      <div class="mc-sync-metrics">
        <span><strong>${Number(status.projectCount || 0)}</strong> 本机项目</span>
        <span><strong>${Number(status.threadCount || 0)}</strong> 本机会话</span>
        <span><strong>${onlinePeers.length}</strong> 在线电脑</span>
      </div>
    </div>
    ${state.error ? `<div class="mc-inline-error">${esc(state.error)}</div>` : ''}
    <div class="mc-sync-list">
      ${onlinePeers.length
        ? onlinePeers.map(peer => renderDesktopSyncPeer(peer, ready)).join('')
        : `<div class="plugin-empty-line">${state.loading ? '正在查找同账号电脑…' : '当前没有其他在线电脑'}</div>`}
    </div>
    ${hiddenOfflineCount ? `<div class="mc-sync-footnote">${hiddenOfflineCount} 台离线或历史电脑已隐藏</div>` : ''}
    ${renderDesktopSyncLastResult()}
  </div>`;
}

function renderPluginToolsSection(dexTools) {
  if (!dexTools.length) return '';
  return `<div class="plugin-detail-section">
    <h3>DEX 工具</h3>
    <div class="plugin-perm-tags">${dexTools.map(t => `<span class="plugin-perm-tag" title="${escAttr(t.description || '')}">${esc(t.name)} · L${Number(t.level || 0)}</span>`).join('')}</div>
  </div>`;
}

function renderPluginPermissionsSection(p) {
  const details = p.permission_details || [];
  const perms = p.permissions || [];
  if (!details.length && !perms.length) return '';
  const rows = details.length
    ? details.map(item => `<div class="plugin-permission-row">
        <span class="plugin-permission-name">${esc(item.permission)}</span>
        <span class="plugin-risk-badge ${escAttr(PLUGIN_RISK_CLASS[item.risk] || 'low')}">${esc(pluginRiskLabel(item.risk))}</span>
        <span class="plugin-permission-desc">${esc(item.description || '')}</span>
      </div>`).join('')
    : `<div class="plugin-perm-tags">${perms.map(perm => `<span class="plugin-perm-tag">${esc(perm)}</span>`).join('')}</div>`;
  return `<div class="plugin-detail-section">
    <h3>权限</h3>
    <div class="plugin-permission-list">${rows}</div>
  </div>`;
}

function renderPluginSourceSection(p) {
  if (!p.source_hash && !p.source_path) return '';
  return `<div class="plugin-detail-section">
    <h3>来源</h3>
    <div class="plugin-source-grid">
      ${p.source_path ? `<span>路径</span><code>${esc(p.source_path)}</code>` : ''}
      ${p.source_hash ? `<span>SHA-256</span><code>${esc(p.source_hash)}</code>` : ''}
    </div>
  </div>`;
}

function pluginBytes(bytes) {
  const value = Number(bytes || 0);
  if (!Number.isFinite(value) || value <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB'];
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit++;
  }
  return `${size >= 10 || unit === 0 ? Math.round(size) : size.toFixed(1)} ${units[unit]}`;
}

function renderPluginAssetsSection(p, compact) {
  const assets = p && p.assets;
  if (!assets || !assets.paths) return '';
  const paths = assets.paths || {};
  return `<div class="plugin-detail-section plugin-assets-section">
    <div class="plugin-section-head">
      <h3>资产</h3>
      ${compact ? `<span class="plugin-asset-inline">数据占用 ${esc(pluginBytes(assets.data_bytes))} · 缓存 ${esc(pluginBytes(assets.cache_bytes))}</span>` : ''}
      <button class="btn-apply" onclick="clearPluginCache('${escAttr(p.id)}')">清理缓存</button>
    </div>
    ${compact ? `
      <div class="plugin-detail-text">更新插件会保留资产；卸载插件会清理这些目录。</div>
    ` : `
    <div class="plugin-asset-metrics">
      <div><strong>${esc(pluginBytes(assets.total_bytes))}</strong><span>总占用</span></div>
      <div><strong>${esc(pluginBytes(assets.data_bytes))}</strong><span>数据</span></div>
      <div><strong>${esc(pluginBytes(assets.cache_bytes))}</strong><span>缓存</span></div>
      <div><strong>${Number(assets.secret_count || 0)}</strong><span>密钥</span></div>
      <div><strong>${Number(assets.account_count || 0)}</strong><span>连接资产</span></div>
    </div>
    <div class="plugin-source-grid plugin-asset-paths">
      <span>数据</span><code>${esc(paths.data_dir || '-')}</code>
      <span>缓存</span><code>${esc(paths.cache_dir || '-')}</code>
      <span>密钥</span><code>${esc(paths.secrets_dir || '-')}</code>
    </div>
    <p class="plugin-detail-text plugin-asset-note">更新插件会保留资产；卸载插件会清理这些目录。</p>
    `}
  </div>`;
}

function pluginFeatureActionOrder(feature) {
  const methods = feature.methods || {};
  const keys = Object.keys(methods);
  const preferred = PLUGIN_WORKFLOW_ACTIONS[feature.kind] || [];
  const ordered = preferred.filter(action => keys.includes(action));
  keys.forEach(action => {
    if (!ordered.includes(action)) ordered.push(action);
  });
  return ordered;
}

function pluginFeatureResultKey(pluginId, featureId, action) {
  return [pluginId, featureId, action].map(value => String(value || '')).join('::');
}

function renderPluginFeatureResult(pluginId, featureId) {
  const prefix = pluginFeatureResultKey(pluginId, featureId, '');
  const entryKey = Object.keys(_pluginFeatureResults).reverse().find(key => key.startsWith(prefix));
  if (!entryKey) return '';
  const entry = _pluginFeatureResults[entryKey];
  const text = JSON.stringify(entry.result == null ? null : entry.result, null, 2);
  return `<div class="plugin-workflow-result">
    <div class="plugin-workflow-result-head">
      <span>${esc(pluginActionLabel(entry.action))}</span>
      <button class="btn-apply" onclick="showPluginFeatureResult(_pluginFeatureResults['${escAttr(entryKey)}'].result)">展开</button>
    </div>
    <pre>${esc(text)}</pre>
  </div>`;
}

function renderPluginFeaturesSection(p) {
  const features = pluginFeatures(p);
  if (!features.length) return '';
  const enabled = pluginIsEnabled(p);
  return `<div class="plugin-detail-section">
    <h3>能力</h3>
    <div class="plugin-workflow-grid">${features.map(feature => {
      const methodKeys = pluginFeatureActionOrder(feature);
      const kind = String(feature.kind || 'integration');
      return `<div class="plugin-feature-card plugin-feature-card-${escAttr(kind)}">
        <div class="plugin-feature-main">
          <div class="plugin-feature-title-row">
            <span class="plugin-feature-label">${esc(feature.label || pluginFeatureKindLabel(feature.kind))}</span>
            <span class="plugin-feature-kind">${esc(pluginFeatureKindLabel(feature.kind))}</span>
          </div>
          ${feature.description ? `<span class="plugin-feature-desc">${esc(feature.description)}</span>` : ''}
        </div>
        ${methodKeys.length ? `<div class="plugin-feature-actions">${methodKeys.map(action => `<button class="btn-apply" ${enabled ? `onclick="event.stopPropagation(); executePluginFeatureAction('${escAttr(p.id)}','${escAttr(feature.id)}','${escAttr(action)}')"` : 'disabled title="插件已停用"'}>${esc(pluginActionLabel(action))}</button>`).join('')}</div>` : '<div class="plugin-empty-line">暂无可执行动作</div>'}
        ${renderPluginFeatureResult(p.id, feature.id)}
      </div>`;
    }).join('')}</div>
  </div>`;
}

function pluginFeatureActionSchema(feature, action) {
  const schemas = (feature && feature.params_schema) || {};
  const schema = schemas[action] || null;
  if (!schema || schema.type !== 'object' || !schema.properties) return null;
  return schema;
}

function renderPluginActionParamField(key, schema, requiredKeys) {
  const type = schema.type || 'string';
  const value = Object.prototype.hasOwnProperty.call(schema, 'default') ? schema.default : '';
  const label = schema.title || key;
  const required = requiredKeys.includes(key);
  const desc = schema.description ? `<span class="plugin-config-desc">${esc(schema.description)}</span>` : '';
  const attrs = `data-plugin-action-key="${escAttr(key)}" data-plugin-action-type="${escAttr(type)}"`;
  let control = '';
  if (Array.isArray(schema.enum)) {
    control = `<select ${attrs} class="plugin-config-input">
      ${schema.enum.map(item => `<option value="${escAttr(item)}" ${String(value) === String(item) ? 'selected' : ''}>${esc(item)}</option>`).join('')}
    </select>`;
  } else if (type === 'boolean') {
    control = `<label class="plugin-config-switch">
      <input ${attrs} type="checkbox" ${value ? 'checked' : ''}>
      <span>${value ? '开启' : '关闭'}</span>
    </label>`;
  } else if (type === 'integer' || type === 'number') {
    control = `<input ${attrs} class="plugin-config-input" type="number" value="${escAttr(value ?? '')}">`;
  } else if (type === 'array') {
    const text = Array.isArray(value) ? value.join('\n') : String(value || '');
    control = `<textarea ${attrs} class="plugin-config-input plugin-config-textarea">${esc(text)}</textarea>`;
  } else if (type === 'object') {
    const text = typeof value === 'object' ? JSON.stringify(value, null, 2) : String(value || '{}');
    control = `<textarea ${attrs} class="plugin-config-input plugin-config-textarea">${esc(text)}</textarea>`;
  } else {
    control = `<input ${attrs} class="plugin-config-input" value="${escAttr(value ?? '')}">`;
  }
  return `<label class="plugin-config-field">
    <span class="plugin-config-label">${esc(label)}${required ? ' *' : ''}</span>
    ${control}
    ${desc}
  </label>`;
}

function parsePluginTypedInput(el, typeAttr) {
  const type = el.getAttribute(typeAttr) || 'string';
  if (type === 'boolean') return Boolean(el.checked);
  if (type === 'integer') {
    const value = parseInt(el.value, 10);
    return Number.isFinite(value) ? value : 0;
  }
  if (type === 'number') {
    const value = parseFloat(el.value);
    return Number.isFinite(value) ? value : 0;
  }
  if (type === 'array') {
    return String(el.value || '').split('\n').map(item => item.trim()).filter(Boolean);
  }
  if (type === 'object') {
    const text = String(el.value || '').trim();
    return text ? JSON.parse(text) : {};
  }
  return el.value || '';
}

function collectPluginActionParams(schema) {
  if (!schema) return null;
  const params = {};
  const fields = Array.from(document.querySelectorAll('[data-plugin-action-key]'));
  fields.forEach(el => {
    const key = el.getAttribute('data-plugin-action-key');
    if (!key) return;
    params[key] = parsePluginTypedInput(el, 'data-plugin-action-type');
  });
  return params;
}

function showPluginFeatureActionModal(p, feature, action) {
  return new Promise(function (resolve) {
    var existing = document.getElementById('pluginActionModal');
    if (existing) existing.remove();

    const schema = pluginFeatureActionSchema(feature, action);
    const requiredKeys = Array.isArray(schema && schema.required) ? schema.required : [];
    const paramFields = schema
      ? Object.keys(schema.properties || {}).map(key => renderPluginActionParamField(key, schema.properties[key] || {}, requiredKeys)).join('')
      : '';
    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';
    overlay.id = 'pluginActionModal';
    overlay.innerHTML = `<div class="modal-box plugin-action-modal">
      <div class="modal-header"><h3>执行插件动作</h3></div>
      <div class="modal-body plugin-action-body">
        <div class="plugin-preview-title">
          <strong>${esc(feature.label || feature.id || '插件能力')}</strong>
          <span>${esc(action)}</span>
          <span>${esc(p.name || p.id)}</span>
        </div>
        ${schema
          ? `<div class="plugin-action-form">${paramFields || '<div class="plugin-empty-line">该动作不需要参数</div>'}</div>`
          : '<textarea id="pluginActionParamsJson" class="plugin-action-json" spellcheck="false">{}</textarea>'}
        <pre id="pluginActionResult" class="plugin-action-result" style="display:none;"></pre>
      </div>
      <div class="plugin-preview-actions">
        <button class="btn btn-primary" id="pluginActionRun" type="button">执行</button>
        <button class="btn btn-ghost" id="pluginActionCancel" type="button">取消</button>
      </div>
    </div>`;
    document.body.appendChild(overlay);

    function cleanup(value) { overlay.remove(); resolve(value); }
    const run = document.getElementById('pluginActionRun');
    const cancel = document.getElementById('pluginActionCancel');
    const textarea = document.getElementById('pluginActionParamsJson');
    const firstField = document.querySelector('[data-plugin-action-key]') || textarea;
    if (firstField) firstField.focus();
    if (run) run.onclick = function () {
      try {
        if (schema) {
          cleanup(collectPluginActionParams(schema) || {});
        } else {
          const text = String((textarea && textarea.value) || '').trim();
          cleanup(text ? JSON.parse(text) : {});
        }
      } catch(e) {
        showToast('参数 JSON 格式错误: ' + esc(String(e)), 'error');
      }
    };
    if (cancel) cancel.onclick = function () { cleanup(null); };
    overlay.addEventListener('click', function (e) { if (e.target === overlay) cleanup(null); });
  });
}

function showPluginFeatureResult(result) {
  var existing = document.getElementById('pluginActionResultModal');
  if (existing) existing.remove();
  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay';
  overlay.id = 'pluginActionResultModal';
  const text = JSON.stringify(result == null ? null : result, null, 2);
  overlay.innerHTML = `<div class="modal-box plugin-action-modal">
    <div class="modal-header"><h3>执行结果</h3></div>
    <div class="modal-body plugin-action-body">
      <pre class="plugin-action-result">${esc(text)}</pre>
    </div>
    <div class="plugin-preview-actions">
      <button class="btn btn-primary" id="pluginActionResultClose" type="button">完成</button>
    </div>
  </div>`;
  document.body.appendChild(overlay);
  const close = document.getElementById('pluginActionResultClose');
  const cleanup = function () { overlay.remove(); };
  if (close) close.onclick = cleanup;
  overlay.addEventListener('click', function (e) { if (e.target === overlay) cleanup(); });
}

function renderPluginCard(p) {
  var state = normalizePluginState(p.state);
  var enabled = pluginIsEnabled(p);
  var running = enabled && pluginIsRunning(p);
  var stateLabel = enabled ? (PLUGIN_STATE_LABEL[state] || PLUGIN_STATE_LABEL.stopped) : '已停用';
  var sc = running ? 'var(--green)' : 'var(--text-muted)';
  var dexTools = p.dex_tools || [];
  var features = pluginFeatures(p);
  var meta = pluginCategoryLabels(p).concat(features.slice(0, 2).map(feature => feature.label));
  if (features.length > 2) meta.push('+' + (features.length - 2));
  var tags = [pluginCategoryLabel(pluginPrimaryCategory(p)), p.source_label || '已安装'];
  if (p.template) tags.push('模板');
  var action = !enabled
    ? `<button class="btn-apply" onclick="event.stopPropagation(); setPluginEnabled('${escAttr(p.id)}', true)">启用</button>`
    : running
      ? `<button class="btn-apply btn-danger" onclick="event.stopPropagation(); stopPlugin('${escAttr(p.id)}')">停止</button>`
      : `<button class="btn-apply" onclick="event.stopPropagation(); startPlugin('${escAttr(p.id)}')">启动</button>`;

  return `<div class="plugin-market-card ${running ? 'installed' : ''}" onclick="showPluginDetail('${escAttr(p.id)}')">
    <div class="plugin-market-head">
      <div class="plugin-market-title">
        <span>${esc(p.name)}</span>
        <em>v${esc(p.version)}</em>
      </div>
      <span class="plugin-market-status installed"><span class="plugin-status-dot" style="color:${sc};background:${sc}"></span>${esc(stateLabel)}</span>
    </div>
    ${p.description ? `<p class="plugin-market-desc">${esc(p.description)}</p>` : ''}
    <div class="plugin-market-tags">${tags.map(t => `<span>${esc(t)}</span>`).join('')}</div>
    <div class="plugin-market-foot">
      <span title="${escAttr(meta.join(' · '))}">${esc(meta.join(' · ') || '基础插件')}</span>
      ${action}
    </div>
  </div>`;
}

function renderPluginOverviewSection(p) {
  const features = pluginFeatures(p);
  const risk = pluginPermissionRisk(p);
  const categories = pluginCategoryLabels(p).join(' / ');
  const accountText = pluginHasAccountFeature(p) ? pluginAccountLabel(p) : '无需连接';
  const toolCount = (p.dex_tools || []).length;
  const permissionList = p.permission_details && p.permission_details.length ? p.permission_details : (p.permissions || []);
  const permissionCount = permissionList.length;
  const items = [
    { label: '分类', value: categories || '工具插件' },
    { label: '能力', value: `${features.length || 0} 项${toolCount ? ` · ${toolCount} 工具` : ''}` },
    { label: '权限', value: `${pluginRiskLabel(risk)} · ${permissionCount} 项`, cls: `risk-${PLUGIN_RISK_CLASS[risk] || 'low'}` },
    { label: '连接', value: accountText }
  ];
  return `<div class="plugin-detail-section plugin-overview-section">
    <div class="plugin-overview-grid">
      ${items.map(item => `<div class="plugin-overview-item">
        <span>${esc(item.label)}</span>
        <strong class="${escAttr(item.cls || '')}">${esc(item.value)}</strong>
      </div>`).join('')}
    </div>
  </div>`;
}

function renderMobileCodexAdvancedSection(p, enabled, running, dexTools) {
  return `<details class="plugin-advanced-panel">
    <summary>
      <span>
        <strong>高级设置与诊断</strong>
        <small>权限、插件信息、运行日志和危险操作</small>
      </span>
      <span class="plugin-advanced-chevron" aria-hidden="true"></span>
    </summary>
    <div class="plugin-advanced-content">
      <div class="plugin-detail-section plugin-advanced-actions">
        <div>
          <h3>插件控制</h3>
          <p class="plugin-detail-text">仅在排障或不再使用 Mobile Codex 时操作。</p>
        </div>
        <div class="plugin-detail-actions">
          ${enabled
            ? `<button class="btn-apply" onclick="setPluginEnabled('${escAttr(p.id)}', false)">停用插件</button>`
            : `<button class="btn-apply" onclick="setPluginEnabled('${escAttr(p.id)}', true)">启用插件</button>`}
          ${enabled
            ? (running
              ? `<button class="btn-apply btn-danger" onclick="stopPlugin('${escAttr(p.id)}')">停止插件进程</button>`
              : `<button class="btn-apply" onclick="startPlugin('${escAttr(p.id)}')">启动插件进程</button>`)
            : ''}
        </div>
      </div>
      ${renderPluginOverviewSection(p)}
      ${renderPluginToolsSection(dexTools)}
      ${renderPluginConfigSection(p)}
      ${renderPluginPermissionsSection(p)}
      ${renderPluginSourceSection(p)}
      ${renderPluginAssetsSection(p)}
      ${renderPluginEventsSection(p)}
      <div class="plugin-danger-zone">
        <h3>卸载插件</h3>
        <p class="plugin-detail-text">卸载后将删除插件及其本机数据，此操作不可恢复。</p>
        <button class="btn-apply btn-danger" onclick="uninstallPlugin('${escAttr(p.id)}')">卸载插件</button>
      </div>
    </div>
  </details>`;
}

// ── 插件详情页 ──
function showPluginDetail(id) {
  stopPluginAutoRefresh();
  const requested = (_pluginsData || []).find(plugin => String(plugin.id) === String(id));
  if (pluginIsInternalComponent(requested)) {
    id = 'deecodex-mobile-codex';
  }
  _pluginDetailId = id;
  switchPanel('plugins');
  if (id === 'deecodex-mobile-codex') {
    setTimeout(() => loadMobileCodexState(false), 0);
  }
  if (id === 'dexai-debug') {
    setTimeout(() => { if (typeof debugPanelEnter === 'function') debugPanelEnter(); }, 0);
  }
}

function backToPluginList() {
  stopPluginEventRefresh();
  if (typeof stopDebugLocalPolling === 'function') stopDebugLocalPolling();
  _pluginDetailId = null;
  switchPanel('plugins');
}

function renderPluginDetail() {
  var p = _pluginsData.find(p => p.id === _pluginDetailId);
  if (pluginIsInternalComponent(p)) {
    _pluginDetailId = 'deecodex-mobile-codex';
    p = _pluginsData.find(plugin => plugin.id === _pluginDetailId);
  }
  if (!p) {
    _pluginDetailId = null;
    return renderPluginsPanel();
  }
  var state = normalizePluginState(p.state);
  var enabled = pluginIsEnabled(p);
  var running = enabled && pluginIsRunning(p);
  var stateLabel = enabled ? (PLUGIN_STATE_LABEL[state] || PLUGIN_STATE_LABEL.stopped) : '已停用';
  var sc = running ? 'var(--green)' : 'var(--text-muted)';
  var perms = p.permissions || [];
  var accounts = p.accounts || [];
  var dexTools = p.dex_tools || [];
  var tags = p.tags || [];
  var hasAccountFeature = pluginHasAccountFeature(p);
  var mobileCodex = isMobileCodexPlugin(p);
  var isDebug = p.id === 'dexai-debug';

  var accountsHtml = '';
  if (accounts.length) {
    accountsHtml = accounts.map(a => {
      var asc = ACCOUNT_STATUS_COLOR[a.status] || 'var(--text-muted)';
      var isConnected = a.status === 'connected';
      return `<div class="pc-account-row">
        <span class="pc-account-name">${esc(a.name || a.account_id)}</span>
        <span class="pc-account-status" style="color:${asc}">
          <span class="plugin-status-dot" style="color:${asc};background:${asc};"></span>
          ${ACCOUNT_STATUS_LABEL[a.status] || a.status}
        </span>
        <span class="pc-account-actions">
          ${isConnected
            ? `<button class="btn-apply btn-danger" onclick="stopPluginAccount('${escAttr(p.id)}','${escAttr(a.account_id)}')">停止连接</button>`
            : `<button class="btn-apply" onclick="scanAndStart('${escAttr(p.id)}','${escAttr(a.account_id)}')">启动连接</button>`}
          <button class="btn-apply" onclick="scanPluginQr('${escAttr(p.id)}','${escAttr(a.account_id)}')" style="background:var(--accent-dim);color:var(--accent);border-color:var(--accent);">认证</button>
          <button class="btn-apply btn-danger" onclick="deletePluginAccount('${escAttr(p.id)}','${escAttr(a.account_id)}')">删除</button>
        </span>
      </div>`;
    }).join('');
  } else {
    accountsHtml = '<div class="plugin-empty-line">暂无连接</div>';
  }
  var infoTags = [
    ...pluginCategoryLabels(p).map(label => `<span class="plugin-perm-tag">${esc(label)}</span>`),
    `<span class="plugin-perm-tag">类型 ${esc(pluginKindLabel(p))}</span>`,
    ...(tags || []).map(tag => `<span class="plugin-perm-tag">${esc(tag)}</span>`)
  ].join('');

  return `<button class="page-back-button plugin-detail-back" onclick="backToPluginList()" aria-label="返回插件市场"><span class="line-action-icon line-action-icon-back" aria-hidden="true"></span></button>

  <div class="plugin-detail-shell">
  <div class="plugin-detail-header">
    <div class="plugin-detail-title">
      <h2>
        <span class="plugin-status-dot" style="color:${sc};background:${sc};"></span>
        ${esc(p.name)}
      </h2>
      <div class="meta">
        <span>${esc(pluginKindLabel(p))}</span>
        <span>版本 v${esc(p.version)}</span>
        ${p.author ? `<span>作者 ${esc(p.author)}</span>` : ''}
        <span class="plugin-state-badge ${running ? 'on' : 'off'}"><span class="dot"></span>${stateLabel}</span>
      </div>
    </div>
    <div class="plugin-detail-actions">
      ${mobileCodex ? '' : (enabled
        ? `<button class="btn-apply" onclick="setPluginEnabled('${escAttr(p.id)}', false)" style="background:rgba(107,127,168,0.08);color:var(--text-muted);border-color:rgba(107,127,168,0.18);">停用插件</button>`
        : `<button class="btn-apply" onclick="setPluginEnabled('${escAttr(p.id)}', true)">启用插件</button>`)}
      ${mobileCodex ? '' : (enabled
        ? (running
          ? `<button class="btn-apply btn-danger" onclick="stopPlugin('${escAttr(p.id)}')">停止插件</button>`
          : `<button class="btn-apply" onclick="startPlugin('${escAttr(p.id)}')">启动插件</button>`)
        : '')}
      ${isDebug ? `<button class="btn-apply btn-danger" onclick="uninstallPlugin('${escAttr(p.id)}')">卸载插件</button>` : ''}
    </div>
  </div>

  ${p.description && !mobileCodex && !isDebug ? `<div class="plugin-detail-section">
    <h3>简介</h3>
    <p class="plugin-detail-text">${esc(p.description)}</p>
    ${infoTags && !isDebug ? `<div class="plugin-perm-tags">${infoTags}</div>` : ''}
  </div>` : ''}

  ${mobileCodex ? `${renderMobileCodexAccountSection(p)}${renderFirstPartyPluginUpdateSection(p, 'mobile')}${renderDesktopSyncSection()}` : `
  ${isDebug ? '' : renderPluginOverviewSection(p)}

  ${isDebug ? renderDebugPanel() : ''}

  ${isDebug ? renderFirstPartyPluginUpdateSection(p, 'debug') : ''}

  ${renderPluginFeaturesSection(p)}

  ${renderPluginToolsSection(dexTools)}

  ${renderPluginConfigSection(p)}

    ${isDebug ? '' : renderPluginPermissionsSection(p)}

    ${isDebug ? '' : renderPluginSourceSection(p)}

    ${renderPluginAssetsSection(p, isDebug)}

  ${hasAccountFeature ? renderPluginAccountsSection(p, accountsHtml) : ''}

  ${renderPluginEventsSection(p, isDebug ? 5 : 80)}

  ${isDebug ? '' : `<div class="plugin-danger-zone">
    <h3>卸载插件</h3>
    <p class="plugin-detail-text">卸载后将删除插件所有文件，此操作不可恢复。</p>
    <button class="btn-apply btn-danger" onclick="uninstallPlugin('${escAttr(p.id)}')">卸载插件</button>
  </div>`}
  `}
  ${mobileCodex ? renderMobileCodexAdvancedSection(p, enabled, running, dexTools) : ''}
  </div>`;
}

function parsePluginConfigInput(el) {
  const type = el.getAttribute('data-plugin-config-type') || 'string';
  if (type === 'boolean') return Boolean(el.checked);
  if (type === 'integer') {
    const value = parseInt(el.value, 10);
    return Number.isFinite(value) ? value : 0;
  }
  if (type === 'number') {
    const value = parseFloat(el.value);
    return Number.isFinite(value) ? value : 0;
  }
  if (type === 'array') {
    return String(el.value || '').split('\n').map(item => item.trim()).filter(Boolean);
  }
  if (type === 'object') {
    const text = String(el.value || '').trim();
    return text ? JSON.parse(text) : {};
  }
  return el.value || '';
}

async function savePluginConfig(pluginId) {
  const p = _pluginsData.find(p => p.id === pluginId);
  if (!p) return;
  const nextConfig = Object.assign({}, p.config || {});
  const fields = Array.from(document.querySelectorAll('[data-plugin-config-key]'));
  try {
    fields.forEach(el => {
      const key = el.getAttribute('data-plugin-config-key');
      if (!key) return;
      nextConfig[key] = parsePluginConfigInput(el);
    });
  } catch(e) {
    showToast('配置格式错误: ' + esc(String(e)), 'error');
    return;
  }
  try {
    await invoke('update_plugin_config', { pluginId: pluginId, config: nextConfig });
    showToast('配置已保存', 'success');
    await loadPluginsData();
  } catch(e) { showToast('保存失败: ' + esc(String(e)), 'error'); }
}

async function clearPluginCache(pluginId) {
  const ok = await showConfirm('清理该插件缓存？长期数据、密钥和连接资产不会被删除。');
  if (!ok) return;
  try {
    await invoke('clear_plugin_cache', { pluginId: pluginId });
    showToast('缓存已清理', 'success');
    await loadPluginsData();
    await loadPluginEvents(pluginId, true);
  } catch(e) {
    showToast('清理失败: ' + esc(String(e)), 'error');
  }
}

async function executePluginFeatureAction(pluginId, featureId, action) {
  const p = _pluginsData.find(p => p.id === pluginId);
  if (!p) {
    showToast('插件不存在: ' + esc(pluginId), 'error');
    return;
  }
  if (!pluginIsEnabled(p)) {
    showToast('插件已停用，请先启用', 'error');
    return;
  }
  const feature = pluginFeatures(p).find(feature => String(feature.id) === String(featureId)) || { id: featureId, label: featureId };
  const params = await showPluginFeatureActionModal(p, feature, action);
  if (params === null) return;
  let confirmed = false;
  if (p.permission_risk === 'high') {
    confirmed = await showConfirm('该插件包含高风险权限，确定执行这个能力动作吗？');
    if (!confirmed) return;
  }
  try {
    const result = await invoke('execute_plugin_feature', {
      pluginId: pluginId,
      featureId: featureId,
      action: action,
      params: params,
      confirmed: confirmed
    });
    showToast('插件动作已执行', 'success');
    _pluginFeatureResults[pluginFeatureResultKey(pluginId, featureId, action)] = {
      action: action,
      result: result,
      ts: Date.now()
    };
    showPluginFeatureResult(result);
    await loadPluginsData();
  } catch(e) {
    showToast('执行失败: ' + esc(String(e)), 'error');
    await loadPluginEvents(pluginId, true);
  }
}

async function setPluginEnabled(id, enabled) {
  try {
    await invoke('set_plugin_enabled', { pluginId: id, enabled: Boolean(enabled) });
    showToast(enabled ? '插件已启用' : '插件已停用', 'success');
    await loadPluginsData();
  } catch(e) {
    showToast('更新失败: ' + esc(String(e)), 'error');
    await loadPluginEvents(id, true);
  }
}

async function startPlugin(id) {
  try {
    await invoke('start_plugin', { pluginId: id });
    showToast('插件已启动', 'success');
    await loadPluginsData();
  } catch(e) {
    showToast('启动失败: ' + esc(String(e)), 'error');
    await loadPluginEvents(id, true);
  }
}

async function stopPlugin(id) {
  try {
    await invoke('stop_plugin', { pluginId: id });
    showToast('插件已停止', 'success');
    await loadPluginsData();
  } catch(e) {
    showToast('停止失败: ' + esc(String(e)), 'error');
    await loadPluginEvents(id, true);
  }
}

function mobileCodexFieldValue(id) {
  const el = document.getElementById(id);
  return String((el && el.value) || '').trim();
}

async function invokeMobileCodex(action, params) {
  return invoke('invoke_plugin_account_action', {
    pluginId: 'deecodex-mobile-codex',
    action: action,
    params: params || {}
  });
}

async function invokeDesktopSync(action, params) {
  return invoke('execute_plugin_feature', {
    pluginId: 'deecodex-desktop-sync',
    featureId: 'desktop-sync',
    action: action,
    params: params || {},
    confirmed: true
  });
}

async function loadDesktopSyncStateData() {
  const plugin = desktopSyncPlugin();
  if (!plugin || !pluginIsEnabled(plugin)) {
    _desktopSyncState.status = null;
    _desktopSyncState.peers = [];
    _desktopSyncState.error = '多机同步组件未安装或已停用';
    _desktopSyncState.loaded = true;
    return;
  }
  const status = await invokeDesktopSync('status', {});
  _desktopSyncState.status = status || {};
  if (status && status.loggedIn) {
    const peers = await invokeDesktopSync('peers', {});
    _desktopSyncState.peers = (peers && peers.peers) || [];
  } else {
    _desktopSyncState.peers = [];
  }
  _desktopSyncState.error = '';
  _desktopSyncState.loaded = true;
}

async function refreshDesktopSyncState() {
  if (_desktopSyncState.loading) return;
  _desktopSyncState.loading = true;
  _desktopSyncState.error = '';
  try {
    await loadDesktopSyncStateData();
  } catch(e) {
    _desktopSyncState.error = String(e);
  } finally {
    _desktopSyncState.loading = false;
    await loadPluginsData();
  }
}

async function syncFromDesktopPeer(deviceId) {
  if (_desktopSyncState.syncingDeviceId) return;
  _desktopSyncState.syncingDeviceId = String(deviceId || '');
  _desktopSyncState.error = '';
  await loadPluginsData();
  try {
    const result = await invokeDesktopSync('sync', { deviceId: deviceId });
    _desktopSyncState.lastResult = result || {};
    const summary = desktopSyncResultSummary(result);
    const message = result && result.ok
      ? (summary.applied ? `已同步 ${summary.applied} 个会话` : '本机已经是最新状态')
      : ((result && result.message) || '同步失败');
    showToast(message, result && result.ok ? 'success' : 'error');
  } catch(e) {
    _desktopSyncState.error = String(e);
    showToast('同步失败: ' + esc(String(e)), 'error');
  } finally {
    _desktopSyncState.syncingDeviceId = '';
    try {
      await loadDesktopSyncStateData();
    } catch(e) {
      _desktopSyncState.error = String(e);
    }
    await loadPluginsData();
  }
}

async function loadMobileCodexState(force) {
  if (_mobileCodexState.loading || (_mobileCodexState.loaded && !force)) return;
  _mobileCodexState.loading = true;
  _mobileCodexState.error = '';
  try {
    const user = await invokeMobileCodex('current_user', {});
    _mobileCodexState.user = user || {};
    _mobileCodexState.service = await invokeMobileCodex('status', {});
    _mobileCodexState.devices = user && user.loggedIn
      ? ((await invokeMobileCodex('list_devices', {})).devices || [])
      : [];
    if (user && user.loggedIn) {
      try {
        await loadDesktopSyncStateData();
      } catch(e) {
        _desktopSyncState.error = String(e);
        _desktopSyncState.loaded = true;
      }
    } else {
      _desktopSyncState = {
        loading: false,
        loaded: false,
        status: null,
        peers: [],
        syncingDeviceId: '',
        lastResult: null,
        error: ''
      };
    }
  } catch(e) {
    _mobileCodexState.error = String(e);
  } finally {
    _mobileCodexState.loading = false;
    _mobileCodexState.loaded = true;
    await loadPluginsData();
  }
}

// 轻量刷新 Mobile Codex 授权（套餐）状态：只更新 current_user，不重载插件列表。
// 供插件市场正向操作门禁调用。
async function refreshMobileCodexEntitlement() {
  try {
    const user = await invokeMobileCodex('current_user', {});
    _mobileCodexState.user = user || {};
    return _mobileCodexState.user;
  } catch (e) {
    _mobileCodexState.error = String(e);
    return null;
  }
}

async function loginMobileCodex() {
  const email = mobileCodexFieldValue('mcLoginEmail');
  const password = mobileCodexFieldValue('mcLoginPassword');
  if (!email || !password) { showToast('请填写邮箱和密码', 'error'); return; }
  try {
    await invokeMobileCodex('login', { email, password });
    const passwordInput = document.getElementById('mcLoginPassword');
    if (passwordInput) passwordInput.value = '';
    _mobileCodexState.loaded = false;
    showToast('Mobile Codex 已登录', 'success');
    await loadMobileCodexState(true);
  } catch(e) { showToast('登录失败: ' + esc(String(e)), 'error'); }
}

async function registerMobileCodex() {
  const invitationCode = mobileCodexFieldValue('mcInvitationCode');
  const email = mobileCodexFieldValue('mcRegisterEmail');
  const password = mobileCodexFieldValue('mcRegisterPassword');
  if (!invitationCode || !email || !password) { showToast('请补全注册信息', 'error'); return; }
  try {
    const result = await invokeMobileCodex('register', { invitationCode, email, password });
    const suffix = result && result.developmentToken ? '，开发验证 Token 已返回' : '，请查收验证邮件';
    showToast('注册成功' + suffix, 'success');
    if (result && result.developmentToken) {
      const input = document.getElementById('mcVerifyToken');
      if (input) input.value = result.developmentToken;
    }
  } catch(e) { showToast('注册失败: ' + esc(String(e)), 'error'); }
}

async function verifyMobileCodexEmail() {
  const token = mobileCodexFieldValue('mcVerifyToken');
  if (!token) { showToast('请填写验证 Token', 'error'); return; }
  try {
    await invokeMobileCodex('verify_email', { token });
    showToast('邮箱验证完成，可以登录', 'success');
  } catch(e) { showToast('验证失败: ' + esc(String(e)), 'error'); }
}

async function forgotMobileCodexPassword() {
  const email = mobileCodexFieldValue('mcRecoverEmail');
  if (!email) { showToast('请填写邮箱', 'error'); return; }
  try {
    await invokeMobileCodex('forgot_password', { email });
    showToast('如账号存在，重置邮件已经发送', 'success');
  } catch(e) { showToast('请求失败: ' + esc(String(e)), 'error'); }
}

async function resetMobileCodexPassword() {
  const token = mobileCodexFieldValue('mcResetToken');
  const password = mobileCodexFieldValue('mcResetPassword');
  if (!token || !password) { showToast('请填写重置 Token 和新密码', 'error'); return; }
  try {
    await invokeMobileCodex('reset_password', { token, password });
    _mobileCodexState = { loading: false, loaded: false, user: null, devices: [], service: null, error: '' };
    showToast('密码已重置，所有设备会话已撤销', 'success');
    await loadMobileCodexState(true);
  } catch(e) { showToast('重置失败: ' + esc(String(e)), 'error'); }
}

async function logoutMobileCodex() {
  if (!await showConfirm('退出 Mobile Codex？本机 Companion 和 P2P 网络将停止。')) return;
  try {
    await invokeMobileCodex('logout', {});
    _mobileCodexState = { loading: false, loaded: false, user: null, devices: [], service: null, error: '' };
    if (typeof _wbEntitlementCache !== 'undefined') {
      _wbEntitlementCache = { at: 0, known: false, authorized: false, error: '', planName: '', planCode: '', serviceStatus: '' };
    }
    showToast('已退出 Mobile Codex', 'success');
    await loadMobileCodexState(true);
  } catch(e) { showToast('退出失败: ' + esc(String(e)), 'error'); }
}

async function setMobileCodexService(start) {
  try {
    await invokeMobileCodex(start ? 'start' : 'stop', {});
    showToast(start ? 'Companion 已启动' : 'Companion 已停止', 'success');
    _mobileCodexState.loaded = false;
    await loadMobileCodexState(true);
  } catch(e) { showToast('操作失败: ' + esc(String(e)), 'error'); }
}

async function launchMobileCodexChatgpt() {
  try {
    await invokeMobileCodex('launch_chatgpt', {});
    showToast('已请求启动 ChatGPT（桥接模式）', 'success');
    _mobileCodexState.loaded = false;
    await loadMobileCodexState(true);
  } catch(e) { showToast('操作失败: ' + esc(String(e)), 'error'); }
}

async function renameMobileCodexDevice(deviceId) {
  const device = (_mobileCodexState.devices || []).find(item => String(item.id || '') === String(deviceId || ''));
  const name = await showMobileCodexRenameModal(device && device.name);
  if (!name) return;
  try {
    await invokeMobileCodex('rename_device', { deviceId, name });
    showToast('设备已重命名', 'success');
    _mobileCodexState.loaded = false;
    await loadMobileCodexState(true);
  } catch(e) { showToast('重命名失败: ' + esc(String(e)), 'error'); }
}

async function revokeMobileCodexDevice(deviceId, isCurrent) {
  const message = isCurrent
    ? '撤销当前电脑后会立即退出账号并停止 Companion，确定继续？'
    : '撤销此设备后，它将无法继续连接。确定继续？';
  if (!await showConfirm(message)) return;
  try {
    await invokeMobileCodex('revoke_device', { deviceId });
    showToast('设备已撤销', 'success');
    _mobileCodexState.loaded = false;
    await loadMobileCodexState(true);
  } catch(e) { showToast('撤销失败: ' + esc(String(e)), 'error'); }
}

async function uninstallPlugin(id) {
  var ok = await showConfirm('确定要卸载该插件吗？此操作不可恢复。');
        if (!ok) return;
  try {
    await invoke('uninstall_plugin', { pluginId: id });
    showToast('插件已卸载', 'success');
    await loadPluginsData();
  } catch(e) { showToast('卸载失败: ' + esc(String(e)), 'error'); }
}

async function stopPluginAccount(pluginId, accountId) {
  try {
    await invoke('stop_plugin_account', { pluginId: pluginId, accountId: accountId });
    showToast('连接已停止', 'success');
    await loadPluginsData();
  } catch(e) { showToast('停止连接失败: ' + esc(String(e)), 'error'); }
}

async function scanAndStart(pluginId, accountId) {
  // 先尝试直接启动连接
  try {
    const r = await invoke('start_plugin_account', { pluginId: pluginId, accountId: accountId });
    showToast('连接已启动', 'success');
    await loadPluginsData();
    return;
  } catch(e) {
    // 如果未登录，自动触发认证流程
    if (String(e).includes('bot_token') || String(e).includes('扫码') || String(e).includes('登录')) {
      showToast('需要先认证，正在获取二维码...');
    } else {
      showToast('启动失败: ' + esc(String(e)), 'error');
      return;
    }
  }
  // 扫码登录
  var oc = document.getElementById('qrOverlayContent');
  var oa = document.getElementById('qrOverlayAccount');
  if (!oc || !oa) { showToast('二维码弹窗未初始化，请重启 GUI', 'error'); return; }
  oc.innerHTML = '<span style="color:var(--text-muted);">获取二维码中...</span>';
  oa.textContent = '连接：' + accountId + '（认证后自动启动）';
  if (!showQrOverlay()) return;
  try {
    var result2 = await invoke('get_plugin_qrcode', { pluginId: pluginId, accountId: accountId });
    var url2 = result2.qrcode_data_url || result2.data_url || '';
    if (url2) {
      oc.innerHTML = `<img src="${esc(url2)}" alt="QR"><p class="qr-hint" style="color:var(--amber);">请扫码完成认证，认证后连接将自动启动</p>`;
      // 轮询状态，连接成功后自动启动
      var pollCount = 0;
      clearPluginQrPolling();
      _pluginQrPollTimer = setInterval(async () => {
        pollCount++;
        try {
          var status = await invoke('query_plugin_status', { pluginId: pluginId, accountId: accountId });
          if (status && status.status === 'connected') {
            clearPluginQrPolling();
            oc.innerHTML = '<span style="color:var(--green);">认证成功，正在启动连接...</span>';
            try {
              await invoke('start_plugin_account', { pluginId: pluginId, accountId: accountId });
              showToast('连接已启动', 'success');
              closeQrOverlay();
            } catch(e2) {
              oc.innerHTML = '<span style="color:var(--red);">连接启动失败: ' + esc(String(e2)) + '</span>';
            }
            await loadPluginsData();
          }
        } catch(ee) {}
        if (pollCount > 60) { clearPluginQrPolling(); oc.innerHTML = '<span style="color:var(--red);">登录超时，请重试</span>'; }
      }, 2000);
    } else {
      oc.innerHTML = '<span style="color:var(--red);">' + esc(JSON.stringify(result2)) + '</span>';
    }
  } catch(e2) {
    oc.innerHTML = '<span style="color:var(--red);">获取二维码失败: ' + esc(String(e2)) + '</span>';
  }
}

async function deletePluginAccount(pluginId, accountId) {
  showToast('正在删除连接 ' + esc(accountId) + '...');
  try {
    await invoke('remove_plugin_account_asset', { pluginId: pluginId, accountId: accountId });
    showToast('连接已删除', 'success');
    await loadPluginsData();
  } catch(e) { showToast('删除失败: ' + esc(String(e)), 'error'); }
}

async function addPluginAccount(pluginId) {
  const input = document.getElementById('addAccountId_' + pluginId);
  const accountId = (input && input.value || '').trim();
  if (!accountId) { showToast('请输入连接 ID'); return; }
  try {
    await invoke('upsert_plugin_account_asset', {
      pluginId: pluginId,
      accountId: accountId,
      asset: { name: accountId, enabled: true }
    });
    showToast('连接已添加', 'success');
    if (input) input.value = '';
    await loadPluginsData();
  } catch(e) { showToast('添加失败: ' + esc(String(e)), 'error'); }
}

async function scanPluginQr(pluginId, accountId) {
  if (!accountId) { showToast('请先添加连接'); return; }
  var oc = document.getElementById('qrOverlayContent');
  var oa = document.getElementById('qrOverlayAccount');
  if (!oc || !oa) { showToast('二维码弹窗未初始化，请重启 GUI', 'error'); return; }
  oc.innerHTML = '<span style="color:var(--text-muted);">获取二维码中...</span>';
  oa.textContent = '连接：' + accountId + '（认证后自动启动）';
  if (!showQrOverlay()) return;
  try {
    var result = await invoke('get_plugin_qrcode', { pluginId: pluginId, accountId: accountId });
    var url = result.qrcode_data_url || result.data_url || '';
    if (url) {
      oc.innerHTML = `<img src="${esc(url)}" alt="QR"><p class="qr-hint" style="color:var(--amber);">请扫码完成认证，认证后连接将自动启动</p>`;
      // 轮询认证状态，确认后自动启动连接
      var pollCount = 0;
      clearPluginQrPolling();
      _pluginQrPollTimer = setInterval(async () => {
        pollCount++;
        try {
          var status = await invoke('query_plugin_status', { pluginId: pluginId, accountId: accountId });
          if (status && status.status === 'connected') {
            clearPluginQrPolling();
            oc.innerHTML = '<span style="color:var(--green);">认证成功，正在启动连接...</span>';
            try {
              await invoke('start_plugin_account', { pluginId: pluginId, accountId: accountId });
              showToast('连接已启动', 'success');
              closeQrOverlay();
            } catch(e2) {
              oc.innerHTML = '<span style="color:var(--red);">连接启动失败: ' + esc(String(e2)) + '</span>';
            }
            await loadPluginsData();
          }
        } catch(ee) {}
        if (pollCount > 60) { clearPluginQrPolling(); oc.innerHTML = '<span style="color:var(--red);">登录超时，请重试</span>'; }
      }, 2000);
    } else {
      oc.innerHTML = '<span style="color:var(--red);">' + esc(JSON.stringify(result)) + '</span>';
    }
  } catch(e) { oc.innerHTML = '<span style="color:var(--red);">获取失败: ' + esc(String(e)) + '</span>'; }
}
