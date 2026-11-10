// ═══════════════════════════════════════════════════════════════
// 个人中心
// 包含：
//   1. Mobile Codex 账号：登录 / 邀请码注册 / 邮箱验证 / 忘记与重置密码 /
//      已登录后的账号信息、Companion 服务状态、设备管理与退出登录。
//   2. 匿名使用统计（原有功能，保持不变）。
// 账号动作全部经由 deecodex-mobile-codex 插件桥接
// （invoke_plugin_account_action），前端不直连 Mobile Codex 服务器。
// ═══════════════════════════════════════════════════════════════

let profileMobileCodexState = {
  loading: false,
  loaded: false,
  user: null,
  service: null,
  devices: [],
  error: ''
};

let profileMasterState = {
  loading: false,
  loaded: false,
  loggedIn: false,
  email: null,
  accountId: null,
  scope: null,
  grants: [],
  audit: [],
  error: ''
};

let profilePluginAuthList = [];
let profilePluginOAuthMap = {};

// 账号云同步 + 偏好设置（跟随账号）
let profileCloudState = {
  loggedIn: false,
  hasVaultScope: false,
  enabled: false,
  lastSyncAt: null,
  lastRestoreAt: null,
  vaultVersion: 0,
  vaultUpdatedAt: null,
  prefsUpdatedAt: null,
  error: ''
};
let profilePrefs = {};

// ── 管理界面内联图标（仅保留必需，保持界面克制）────────────────
const PM_ICON_MARK =
  '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round" aria-hidden="true">'
  + '<path d="M12 2L15 9H9L12 2Z"/><path d="M12 22L9 15H15L12 22Z"/>'
  + '<path d="M2 12L9 9V15L2 12Z"/><path d="M22 12L15 15V9L22 12Z"/>'
  + '</svg>';

function renderProfile() {
  const master = profileMasterState;
  // 未加载：只显示主账号读取状态，避免登录态确定前露出兼容层/统计区块。
  if (!master.loaded) {
    return '<div class="pm-shell">'
      + '<div class="pm-panel pm-panel-center">'
      + '<span class="pm-mark pm-mark-lg">' + PM_ICON_MARK + '</span>'
      + '<p class="pm-loading-title">个人中心</p>'
      + '<p class="pm-loading-text">DEX AI 主账号 · 正在读取主账号状态…</p>'
      + '</div>';
  }
  // 未登录：整页只显示一套登录界面（左右分栏 + 动画角色）。
  if (!master.loggedIn) {
    return typeof profileLoginRender === 'function'
      ? profileLoginRender()
      : '<div class="config-section"><div class="config-section-header"><div><h3>DEX AI 主账号</h3><p>登录页加载失败</p></div></div></div>';
  }
  // 已登录：账号管理模型表单风格——分区表单 + 表格式列表，登录页保持不动。
  return '<section class="primary-page-shell primary-page-shell-accounts profile-form-shell" data-primary-panel="profile">'
    + '<div class="accounts-page-shell accounts-form-shell">'
    + '<div class="accounts-scroll-region accounts-form-scroll">'
    + '<div class="account-form profile-account-form">'
    + '<header class="pm-head">'
    + '<div class="pm-title">'
    + '<h2>个人中心</h2>'
    + '<p>DEX AI 主账号 · 授权与安全中心</p>'
    + '</div>'
    + '<button class="pm-btn pm-btn-ghost" onclick="profileMasterLogout()">退出登录</button>'
    + '</header>'
    + '<div class="profile-form-body">'
    + '<div id="profileMasterSection">' + renderProfileMasterSection() + '</div>'
    + '<details class="account-edit-section profile-collapse" open>'
    + '<summary class="profile-collapse-head"><span class="section-sub-label">账号云同步</span><span class="pm-badge" id="cloudSyncBadge">—</span></summary>'
    + '<div class="profile-collapse-body"><div id="profileCloudSyncSection">' + renderProfileCloudSync() + '</div></div>'
    + '</details>'
    + '<details class="account-edit-section profile-collapse" open>'
    + '<summary class="profile-collapse-head"><span class="section-sub-label">DEX AI 偏好设置</span><span class="pm-badge" id="prefsBadge">—</span></summary>'
    + '<div class="profile-collapse-body"><div id="profilePrefsSection">' + renderProfilePrefs() + '</div></div>'
    + '</details>'
    + '<details class="account-edit-section profile-collapse" open>'
    + '<summary class="profile-collapse-head"><span class="section-sub-label">Mobile Codex 设备与服务</span></summary>'
    + '<div class="profile-collapse-body"><div id="profileAccountSection">' + renderProfileAccountSection() + '</div></div>'
    + '</details>'
    + '<details class="account-edit-section profile-collapse">'
    + '<summary class="profile-collapse-head"><span class="section-sub-label">匿名使用统计</span></summary>'
    + '<div class="profile-collapse-body">'
    + '<div class="pm-toggle-row">'
    + '<div class="pm-toggle-copy">'
    + '<strong>允许匿名统计</strong>'
    + '<p>只采集匿名使用统计、页面 PV/UV 和广告位曝光/点击；不采集 API Key、Prompt、线程内容、账号邮箱和本地路径。</p>'
    + '</div>'
    + '<label class="pm-switch">'
    + '<input id="telemetryEnabledToggle" type="checkbox" checked onchange="toggleTelemetryEnabled(this.checked)">'
    + '<span class="pm-switch-track" aria-hidden="true"></span>'
    + '</label>'
    + '</div>'
    + '<div id="telemetryStatusText" class="pm-meta-status">读取中</div>'
    + '<div class="pm-meta-lines">'
    + '<span>统计标识：<strong id="telemetryInstallId">—</strong></span>'
    + '<span>会话：<strong id="telemetrySessionId">—</strong></span>'
    + '<span>上报入口：<strong id="telemetryEndpoint">—</strong></span>'
    + '</div>'
    + '</div>'
    + '</details>'
    + '<div class="pm-legal-links">'
    + '<span class="pm-legal-label">法律与隐私</span>'
    + '<button type="button" class="pm-link-btn" onclick="window.DexLegal && window.DexLegal.open(\'privacy\')">隐私政策</button>'
    + '<span class="pm-dot">·</span>'
    + '<button type="button" class="pm-link-btn" onclick="window.DexLegal && window.DexLegal.open(\'tos\')">服务条款</button>'
    + '</div>'
    + '</div>'
    + '</div>'
    + '</div>'
    + '</div>'
    + '</section>';
}

// ── DEX AI 主账号：登录 + 授权中心 ───────────────────────────

function renderProfileMasterSection() {
  const state = profileMasterState;
  if (!state.loaded) {
    return '<div class="pm-empty">正在读取主账号状态…</div>';
  }
  if (!state.loggedIn) {
    return typeof profileLoginRender === 'function' ? profileLoginRender() : '<div class="pm-empty">登录页加载失败</div>';
  }
  const grants = state.grants || [];
  const authPlugins = profilePluginAuthList || [];
  const activeGrantClientIds = new Set(
    grants.filter(grant => String(grant.status || '') === 'active').map(grant => grant.clientId)
  );
  // 只列出尚未获得有效授权的插件，避免与“已授权应用”重复。
  const pendingAuthPlugins = authPlugins.filter(plugin => !activeGrantClientIds.has(plugin.clientId));
  const grantCount = grants.length;
  return '<section class="account-edit-section">'
    + '<div class="account-section-head">'
    + '<div class="section-sub-label">主账号信息</div>'
    + '<div class="account-section-desc">个人中心是唯一登录入口，插件与功能通过这里申请授权。</div>'
    + '</div>'
    + '<div class="config-fields">'
    + '<div class="config-field">'
    + '<label>邮箱</label>'
    + '<input type="text" value="' + escAttr(state.email || '-') + '" disabled>'
    + '</div>'
    + '<div class="config-field">'
    + '<label>账号 ID</label>'
    + '<input type="text" value="' + escAttr(state.accountId || '-') + '" disabled>'
    + '</div>'
    + (state.scope
      ? '<div class="config-field"><label>授权范围</label><input type="text" value="' + escAttr(state.scope) + '" disabled></div>'
      : '')
    + '<div class="config-field">'
    + '<label>已授权应用</label>'
    + '<input type="text" value="已授权 ' + grantCount + ' 个应用" disabled>'
    + '</div>'
    + '</div>'
    + '</section>'
    + (pendingAuthPlugins.length
      ? '<section class="account-edit-section">'
        + '<div class="account-section-head">'
        + '<div class="section-sub-label">待授权插件</div>'
        + '<div class="account-section-desc">尚未获得有效授权的插件，可在此申请授权。</div>'
        + '<span class="pm-badge">' + pendingAuthPlugins.length + '</span>'
        + '</div>'
        + '<div class="codex-model-table profile-model-table">'
        + '<div class="codex-model-head profile-model-head"><span>插件</span><span>状态</span><span>操作</span></div>'
        + pendingAuthPlugins.map(profileRenderPluginAuth).join('')
        + '</div>'
        + '</section>'
      : '')
    + '<section class="account-edit-section">'
    + '<div class="account-section-head">'
    + '<div class="section-sub-label">已授权应用</div>'
    + '<div class="account-section-desc">已授予访问权限的应用。</div>'
    + '<span class="pm-badge">' + grantCount + '</span>'
    + '</div>'
    + '<div class="codex-model-table profile-model-table">'
    + '<div class="codex-model-head profile-model-head"><span>应用</span><span>状态</span><span>操作</span></div>'
    + (grantCount ? grants.map(profileRenderGrant).join('') : '<div class="codex-model-empty">暂无已授权应用</div>')
    + '</div>'
    + '</section>'
    + '<details class="account-edit-section profile-collapse" open>'
    + '<summary class="profile-collapse-head"><span class="section-sub-label">最近授权记录</span><span class="pm-badge">' + (state.audit || []).length + '</span></summary>'
    + '<div class="profile-collapse-body"><div class="codex-model-table profile-model-table">'
    + '<div class="codex-model-head profile-model-head"><span>动作</span><span>应用</span><span>结果</span></div>'
    + ((state.audit || []).length ? state.audit.map(profileRenderAuditEvent).join('') : '<div class="codex-model-empty">暂无授权记录</div>')
    + '</div></div>'
    + '</details>';
}

function profileRenderAuditEvent(event) {
  const actionText = {
    'oauth.authorize': '发起授权',
    'oauth.token.issue': '换取令牌',
    'oauth.token.refresh': '刷新令牌',
    'oauth.token.device': '设备流换令牌',
    'oauth.token.password': '主账号登录',
    'oauth.device.approve': '批准设备流',
    'oauth.grant.revoke': '撤销授权'
  }[event.action] || event.action;
  const clientLabel = event.clientName || event.clientKey || '系统';
  const time = event.createdAt ? new Date(event.createdAt).toLocaleString() : '';
  return '<div class="codex-model-row profile-model-row">'
    + '<span class="codex-model-name profile-model-name" title="' + escAttr(clientLabel) + (time ? ' · ' + escAttr(time) : '') + '">' + esc(actionText) + '</span>'
    + '<span class="profile-model-status profile-model-status-text">' + esc(clientLabel) + '</span>'
    + '<span class="profile-model-actions"><span class="pm-status ' + (event.outcome === 'ok' ? 'is-ok' : 'is-off') + '">' + esc(event.outcome === 'ok' ? '成功' : (event.outcome || '')) + '</span></span>'
    + '</div>';
}

function profileRenderPluginAuth(plugin) {
  const oauth = profilePluginOAuthMap[plugin.pluginId] || { oauthEnabled: false, scopes: null };
  const enabled = Boolean(oauth.oauthEnabled);
  const revokedGrant = (profileMasterState.grants || []).some(grant =>
    grant.clientId === plugin.clientId && String(grant.status || '') === 'revoked'
  );
  const scopes = Array.isArray(plugin.scopes) ? plugin.scopes : [];
  return '<div class="codex-model-row profile-model-row">'
    + '<span class="codex-model-name profile-model-name" title="' + escAttr(plugin.reason || '') + (scopes.length ? ' · ' + escAttr(scopes.join(' ')) : '') + '">' + esc(plugin.name || plugin.clientId || '-') + (plugin.firstParty ? '<span class="pm-tag">第一方</span>' : '') + '</span>'
    + '<span class="profile-model-status"><span class="pm-status ' + (enabled ? 'is-ok' : (revokedGrant ? 'is-warn' : 'is-off')) + '">' + esc(enabled ? '已授权' : (revokedGrant ? '需重新授权' : '未授权')) + '</span></span>'
    + '<span class="profile-model-actions">'
    + '<button class="pm-btn pm-btn-primary" onclick="profileMasterAuthorizePlugin(\'' + escAttr(plugin.pluginId) + '\')">' + (enabled ? '重新授权' : '授权') + '</button>'
    + '</span>'
    + '</div>';
}

function profileRenderGrant(grant) {
  const revoked = String(grant.status || '') === 'revoked';
  const scopes = Array.isArray(grant.scopes) ? grant.scopes : [];
  return '<div class="codex-model-row profile-model-row' + (revoked ? ' disabled' : '') + '">'
    + '<span class="codex-model-name profile-model-name" title="' + escAttr((scopes.length ? scopes.join(' ') : '已授权')) + '">' + esc(grant.clientName || grant.clientId || '-') + (grant.firstParty ? '<span class="pm-tag">第一方</span>' : '') + '</span>'
    + '<span class="profile-model-status"><span class="pm-status ' + (revoked ? 'is-off' : 'is-ok') + '">' + esc(revoked ? '已撤销' : '已授权') + '</span></span>'
    + '<span class="profile-model-actions">' + (revoked ? '' : '<button class="pm-btn pm-btn-danger" onclick="profileMasterRevokeGrant(\'' + escAttr(grant.grantId) + '\')">撤销</button>') + '</span>'
    + '</div>';
}

function profileMasterRerender() {
  const container = document.getElementById('profileMasterSection');
  const loginDom = document.getElementById('plCharacters');
  // 视图形态不一致时整页刷新：已登录需要主账号区块，未登录需要登录视图。
  const needFullRefresh = profileMasterState.loggedIn ? !container : !loginDom;
  if (needFullRefresh) {
    if (typeof profileLoginTeardown === 'function') profileLoginTeardown();
    if (typeof switchPanel === 'function') switchPanel('profile');
    return;
  }
  if (container) container.innerHTML = renderProfileMasterSection();
  if (!profileMasterState.loggedIn && typeof profileLoginHydrate === 'function') {
    profileLoginHydrate();
  }
}

async function hydrateProfileMaster() {
  if (profileMasterState.loading) return;
  profileMasterState.loading = true;
  profileMasterState.error = '';
  profileMasterRerender();
  try {
    const status = await invoke('dexai_master_status', {});
    profileMasterState.loggedIn = Boolean(status && status.loggedIn);
    profileMasterState.email = (status && status.email) || null;
    profileMasterState.accountId = (status && status.accountId) || null;
    profileMasterState.scope = (status && status.scope) || null;
    if (status && status.error) profileMasterState.error = String(status.error);
    profileMasterState.grants = [];
    if (profileMasterState.loggedIn) {
      try {
        const result = await invoke('dexai_oauth_grants', {});
        profileMasterState.grants = (result && result.grants) || [];
      } catch (error) {
        profileMasterState.error = String(error);
      }
      try {
        const audit = await invoke('dexai_oauth_audit', {});
        profileMasterState.audit = (audit && audit.events) || [];
      } catch (error) {
        profileMasterState.audit = [];
      }
      await profileRefreshPluginAuth();
      await hydrateProfileCloud();
      await hydrateProfilePrefs();
    } else {
      profileCloudState = Object.assign({}, profileCloudState, {
        loggedIn: false,
        hasVaultScope: false,
        enabled: false,
        error: ''
      });
      profileRerenderCloudSync();
    }
  } catch (error) {
    profileMasterState.error = String(error);
  } finally {
    profileMasterState.loading = false;
    profileMasterState.loaded = true;
    profileMasterRerender();
  }
}

async function profileRefreshPluginAuth() {
  try {
    const manifest = await invoke('dexai_plugin_auth_manifest', {});
    profilePluginAuthList = (manifest && manifest.plugins) || [];
    profilePluginOAuthMap = {};
    for (const plugin of profilePluginAuthList) {
      const statusMethod = plugin.methods && plugin.methods.status;
      if (!statusMethod) continue;
      try {
        const status = await invoke('invoke_plugin_rpc', {
          pluginId: plugin.pluginId,
          method: statusMethod,
          params: {}
        });
        profilePluginOAuthMap[plugin.pluginId] = {
          oauthEnabled: Boolean(status && status.oauthEnabled),
          scopes: (status && status.scopes) || null
        };
      } catch (error) {
        profilePluginOAuthMap[plugin.pluginId] = { oauthEnabled: false, scopes: null };
      }
    }
  } catch (error) {
    profilePluginAuthList = [];
    profilePluginOAuthMap = {};
  }
}

function profilePkceBase64Url(bytes) {
  let binary = '';
  bytes.forEach(function (byte) { binary += String.fromCharCode(byte); });
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

async function profilePkce() {
  const random = new Uint8Array(48);
  crypto.getRandomValues(random);
  const verifier = profilePkceBase64Url(random);
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(verifier));
  return { verifier: verifier, challenge: profilePkceBase64Url(new Uint8Array(digest)) };
}

async function profileMasterLogin() {
  const email = profileMobileCodexFieldValue('pfMasterEmail');
  const password = profileMobileCodexFieldValue('pfMasterPassword');
  if (!email || !password) { showToast('请填写邮箱和密码', 'error'); return; }
  try {
    await invoke('dexai_master_login', { email: email, password: password });
    const passwordInput = document.getElementById('pfMasterPassword');
    if (passwordInput) passwordInput.value = '';
    showToast('已登录 DEX AI 主账号', 'success');
    profileMasterState.loaded = false;
    await hydrateProfileMaster();
  } catch (error) {
    showToast('登录失败: ' + esc(String(error)), 'error');
    if (typeof profileLoginFail === 'function') profileLoginFail(String(error));
  }
}

async function profileMasterLogout() {
  if (!await showConfirm('退出 DEX AI 主账号？本机主会话将清除。')) return;
  try {
    await invoke('dexai_master_logout', {});
    showToast('已退出主账号', 'success');
    profileMasterState.loaded = false;
    await hydrateProfileMaster();
  } catch (error) {
    showToast('退出失败: ' + esc(String(error)), 'error');
  }
}

async function profileMasterRevokeGrant(grantId) {
  if (!await showConfirm('撤销该应用的授权？其令牌将立即失效，需重新授权。')) return;
  try {
    const grant = (profileMasterState.grants || []).find(item => String(item.grantId || '') === String(grantId || ''));
    await invoke('dexai_oauth_revoke_grant', { grantId: grantId });
    showToast('已撤销授权', 'success');
    const plugin = (profilePluginAuthList || []).find(item => item.clientId === (grant && grant.clientId));
    const logoutMethod = plugin && plugin.methods && plugin.methods.logout;
    if (plugin && logoutMethod) {
      try {
        await invoke('invoke_plugin_rpc', {
          pluginId: plugin.pluginId,
          method: logoutMethod,
          params: {}
        });
        profilePluginOAuthMap[plugin.pluginId] = { oauthEnabled: false, scopes: null };
      } catch (pluginError) {
        console.warn('[deecodex] 撤销插件 OAuth 失败（插件下次调用会回退）:', pluginError);
      }
    }
    await hydrateProfileMaster();
  } catch (error) {
    showToast('撤销失败: ' + esc(String(error)), 'error');
  }
}

async function profileMasterAuthorizePlugin(pluginId) {
  const plugin = (profilePluginAuthList || []).find(item => String(item.pluginId || '') === String(pluginId || ''));
  if (!plugin) { showToast('插件未声明 OAuth 授权', 'error'); return; }
  const scopes = Array.isArray(plugin.scopes) ? plugin.scopes.join(' ') : '';
  const redirectUri = plugin.redirectUri || (plugin.clientId + '://callback');
  const exchangeMethod = (plugin.methods && plugin.methods.exchange) || 'mobile_codex.oauth.exchange';
  try {
    const pkce = await profilePkce();
    const result = await invoke('dexai_oauth_authorize', {
      clientId: plugin.clientId,
      scope: scopes,
      redirectUri: redirectUri,
      codeChallenge: pkce.challenge,
      codeChallengeMethod: 'S256'
    });
    const code = result && result.code;
    if (!code) { throw new Error('未返回授权码'); }
    const exchange = await invoke('invoke_plugin_rpc', {
      pluginId: plugin.pluginId,
      method: exchangeMethod,
      params: {
        clientId: plugin.clientId,
        code: code,
        codeVerifier: pkce.verifier,
        redirectUri: redirectUri
      }
    });
    showToast((plugin.name || '插件') + ' 已获得 OAuth 授权: ' + esc((exchange && exchange.scope) || ''), 'success');
    await hydrateProfileMaster();
  } catch (error) {
    showToast('授权失败: ' + esc(String(error)), 'error');
    await hydrateProfileMaster();
  }
}

// ── Mobile Codex 账号：状态与渲染 ──────────────────────────────

function profileMobileCodexFieldValue(id) {
  const el = document.getElementById(id);
  return String((el && el.value) || '').trim();
}

function profileInvokeMobileCodex(action, params) {
  return invoke('invoke_plugin_account_action', {
    pluginId: 'deecodex-mobile-codex',
    action: action,
    params: params || {}
  });
}

function renderProfileAccountSection() {
  const state = profileMobileCodexState;
  if (!state.loaded) {
    return '<div class="pm-empty">正在读取本机账号状态…</div>';
  }
  const user = state.user || {};
  if (!user.loggedIn) return renderProfileLoggedOut();
  return renderProfileLoggedIn();
}

function renderProfileLoggedOut() {
  const state = profileMobileCodexState;
  return '<div class="pm-toggle-row" style="padding:2px 2px 10px;">'
    + '<div class="pm-toggle-copy">'
    + '<strong>Mobile Codex 兼容账号未登录</strong>'
    + '<p>本机 Companion 与绑定设备使用独立的 Mobile Codex 账号；新版本建议直接使用 DEX AI 主账号授权。</p>'
    + '</div>'
    + '<span class="pm-status is-off">未登录</span>'
    + '</div>'
    + (state.error ? '<div class="pm-err">' + esc(state.error) + '</div>' : '')
    + '<details class="pm-sub">'
    + '<summary>登录 / 注册 / 找回兼容账号</summary>'
    + '<div class="pm-sub-body">'
    + '<div class="config-fields">'
    + '<div class="config-field wide"><label>邮箱</label><input id="pfLoginEmail" type="email" autocomplete="username" placeholder="name@example.com"></div>'
    + '<div class="config-field wide"><label>密码</label><input id="pfLoginPassword" type="password" autocomplete="current-password" placeholder="至少 12 位"></div>'
    + '</div>'
    + '<button class="pm-btn pm-btn-primary" style="width:100%;height:34px;" onclick="profileLoginMobileCodex()">登录并启动 Companion</button>'
    + '<details class="pm-sub"><summary>邀请码注册</summary><div class="pm-sub-body">'
    + '<div class="pm-field"><span>邀请码</span><input id="pfInvitationCode" class="pm-input" placeholder="邀请码"></div>'
    + '<div class="pm-field"><span>邮箱</span><input id="pfRegisterEmail" class="pm-input" type="email" placeholder="邮箱"></div>'
    + '<div class="pm-field"><span>密码</span><input id="pfRegisterPassword" class="pm-input" type="password" placeholder="密码，至少 12 位"></div>'
    + '<button class="pm-btn pm-btn-ghost" style="width:100%;" onclick="profileRegisterMobileCodex()">提交注册</button>'
    + '</div></details>'
    + '<details class="pm-sub"><summary>验证邮箱</summary><div class="pm-sub-body">'
    + '<p style="margin:0 0 8px;font-size:11px;color:var(--text-muted);">注册成功后查收验证邮件；开发环境 Token 会自动填入。</p>'
    + '<div class="pm-field"><span>邮箱验证 Token</span><input id="pfVerifyToken" class="pm-input" placeholder="邮箱验证 Token"></div>'
    + '<button class="pm-btn pm-btn-ghost" style="width:100%;" onclick="profileVerifyMobileCodexEmail()">完成验证</button>'
    + '</div></details>'
    + '<details class="pm-sub"><summary>忘记或重置密码</summary><div class="pm-sub-body">'
    + '<div class="pm-field"><span>找回密码邮箱</span><input id="pfRecoverEmail" class="pm-input" type="email" placeholder="找回密码邮箱"></div>'
    + '<button class="pm-btn pm-btn-ghost" style="width:100%;" onclick="profileForgotMobileCodexPassword()">发送重置邮件</button>'
    + '<div class="pm-field"><span>重置 Token</span><input id="pfResetToken" class="pm-input" placeholder="重置 Token"></div>'
    + '<div class="pm-field"><span>新密码</span><input id="pfResetPassword" class="pm-input" type="password" placeholder="新密码，至少 12 位"></div>'
    + '<button class="pm-btn pm-btn-ghost" style="width:100%;" onclick="profileResetMobileCodexPassword()">重置密码</button>'
    + '</div></details>'
    + '</div>'
    + '</details>';
}

function profileMobileCodexDeviceIsOnline(device) {
  if (!device || device.revokedAt) return false;
  if (typeof device.online === 'boolean') return device.online;
  return Boolean(device.networkPath && device.networkPath !== 'offline');
}

function profileRenderMobileCodexDevice(device, currentDeviceId) {
  const revoked = Boolean(device.revokedAt);
  const online = profileMobileCodexDeviceIsOnline(device);
  const status = revoked ? '已撤销' : (online ? '在线' : '离线');
  const isCurrent = String(device.id || '') === String(currentDeviceId || '');
  return '<div class="codex-model-row profile-model-row' + (revoked ? ' disabled' : '') + '">'
    + '<span class="codex-model-name profile-model-name" title="' + escAttr(device.platform || '-') + ' · ' + escAttr(device.deviceType || '-') + ' · ' + escAttr(device.companionVersion || device.appVersion || '-') + '">' + esc(device.name || device.id) + (isCurrent ? ' <span class="pm-tag">本机</span>' : '') + '</span>'
    + '<span class="profile-model-status"><span class="pm-status ' + (revoked ? 'is-off' : (online ? 'is-ok' : 'is-warn')) + '">' + esc(status) + '</span></span>'
    + '<span class="profile-model-actions">'
    + (revoked ? '' : '<button class="pm-btn pm-btn-ghost" onclick="profileRenameMobileCodexDevice(\'' + escAttr(device.id) + '\')">重命名</button>'
      + '<button class="pm-btn pm-btn-danger" onclick="profileRevokeMobileCodexDevice(\'' + escAttr(device.id) + '\',' + (isCurrent ? 'true' : 'false') + ')">撤销</button>')
    + '</span>'
    + '</div>';
}

function profileMobileCodexConnectionStatus(service, error) {
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

function renderProfileLoggedIn() {
  const state = profileMobileCodexState;
  const user = state.user || {};
  const service = state.service || {};
  const devices = state.devices || [];
  const externallyManaged = service.running && service.ownership === 'external';
  const connection = profileMobileCodexConnectionStatus(service, state.error);
  const chatgptBridge = String(service.chatgptBridge || 'unknown');
  const chatgptReady = chatgptBridge === 'shim';
  const chatgptHint = chatgptReady
    ? 'ChatGPT 已以桥接模式运行。'
    : (chatgptBridge === 'official_mode'
      ? 'ChatGPT 正在运行但未走桥接，点击下方按钮将重启为桥接模式。'
      : 'ChatGPT 未运行，点击下方按钮以桥接模式启动。');
  return '<div class="pm-conn">'
    + '<span class="pm-conn-dot ' + (connection.tone === 'online' ? 'is-ok' : (connection.tone === 'offline' ? 'is-warn' : 'is-error')) + '" aria-hidden="true"></span>'
    + '<div class="pm-conn-copy">'
    + '<strong>' + esc(connection.title) + '</strong>'
    + '<p>' + esc(connection.description) + '</p>'
    + '</div>'
    + '<button class="pm-btn pm-btn-ghost" onclick="profileRefreshMobileCodex()">刷新状态</button>'
    + '</div>'
    + (state.error ? '<div class="pm-err">' + esc(state.error) + '</div>' : '')
    + '<div class="pm-account pm-account-sm">'
    + '<div class="pm-avatar">' + esc(String(user.email || 'M').slice(0, 1).toUpperCase()) + '</div>'
    + '<div class="pm-account-main">'
    + '<div class="pm-account-line"><strong>' + esc(user.email || '-') + '</strong></div>'
    + '<p>' + devices.length + ' 台设备已绑定</p>'
    + '</div>'
    + '<button class="pm-btn pm-btn-danger" onclick="profileLogoutMobileCodex()">退出登录</button>'
    + '</div>'
    + '<div class="pm-svc">'
    + '<button class="pm-btn pm-btn-ghost" ' + (externallyManaged ? 'disabled' : '') + ' onclick="profileSetMobileCodexService(' + (service.running ? 'false' : 'true') + ')">' + (externallyManaged ? '系统 Companion 运行中' : (service.running ? '停止 Companion' : '启动 Companion')) + '</button>'
    + '<span>' + (externallyManaged ? '由系统服务管理' : (service.running ? '关闭后，手机将无法连接这台电脑。' : '启动后，手机才能连接这台电脑。')) + '</span>'
    + '<button class="pm-btn pm-btn-ghost" ' + (chatgptReady ? 'disabled' : '') + ' onclick="profileLaunchMobileCodexChatgpt()">' + (chatgptReady ? 'ChatGPT 已桥接' : '启动 ChatGPT（桥接模式）') + '</button>'
    + '<span>' + chatgptHint + '</span>'
    + '</div>'
    + '<div class="account-section-head" style="margin-top:8px;">'
    + '<div class="section-sub-label">我的设备</div>'
    + '<span class="pm-badge">' + devices.length + '</span>'
    + '</div>'
    + '<div class="codex-model-table profile-model-table">'
    + '<div class="codex-model-head profile-model-head"><span>设备</span><span>状态</span><span>操作</span></div>'
    + (devices.length ? devices.map(device => profileRenderMobileCodexDevice(device, user.deviceId)).join('') : '<div class="codex-model-empty">暂无设备</div>')
    + '</div>';
}

function profileRerenderAccountSection() {
  const container = document.getElementById('profileAccountSection');
  if (container) container.innerHTML = renderProfileAccountSection();
}

async function hydrateProfile() {
  if (profileMobileCodexState.loading) return;
  profileMobileCodexState.loading = true;
  profileMobileCodexState.error = '';
  profileRerenderAccountSection();
  try {
    const user = await profileInvokeMobileCodex('current_user', {});
    const service = await profileInvokeMobileCodex('status', {});
    const devices = user && user.loggedIn
      ? ((await profileInvokeMobileCodex('list_devices', {})).devices || [])
      : [];
    profileMobileCodexState.user = user || {};
    profileMobileCodexState.service = service || {};
    profileMobileCodexState.devices = devices;
  } catch (error) {
    profileMobileCodexState.error = String(error);
  } finally {
    profileMobileCodexState.loading = false;
    profileMobileCodexState.loaded = true;
    profileRerenderAccountSection();
  }
}

async function profileRefreshMobileCodex() {
  if (profileMobileCodexState.loading) return;
  profileMobileCodexState.loaded = false;
  await hydrateProfile();
}

// ── Mobile Codex 账号：动作 ────────────────────────────────────

async function profileLoginMobileCodex() {
  const email = profileMobileCodexFieldValue('pfLoginEmail');
  const password = profileMobileCodexFieldValue('pfLoginPassword');
  if (!email || !password) { showToast('请填写邮箱和密码', 'error'); return; }
  try {
    await profileInvokeMobileCodex('login', { email, password });
    const passwordInput = document.getElementById('pfLoginPassword');
    if (passwordInput) passwordInput.value = '';
    showToast('Mobile Codex 已登录', 'success');
    await hydrateProfile();
  } catch (error) {
    showToast('登录失败: ' + esc(String(error)), 'error');
    hydrateProfile();
  }
}

async function profileRegisterMobileCodex() {
  const invitationCode = profileMobileCodexFieldValue('pfInvitationCode');
  const email = profileMobileCodexFieldValue('pfRegisterEmail');
  const password = profileMobileCodexFieldValue('pfRegisterPassword');
  if (!invitationCode || !email || !password) { showToast('请补全注册信息', 'error'); return; }
  try {
    const result = await profileInvokeMobileCodex('register', { invitationCode, email, password });
    const suffix = result && result.developmentToken ? '，开发验证 Token 已填入' : '，请查收验证邮件';
    showToast('注册成功' + suffix, 'success');
    if (result && result.developmentToken) {
      const input = document.getElementById('pfVerifyToken');
      if (input) input.value = result.developmentToken;
    }
  } catch (error) {
    showToast('注册失败: ' + esc(String(error)), 'error');
  }
}

async function profileVerifyMobileCodexEmail() {
  const token = profileMobileCodexFieldValue('pfVerifyToken');
  if (!token) { showToast('请填写验证 Token', 'error'); return; }
  try {
    await profileInvokeMobileCodex('verify_email', { token });
    showToast('邮箱验证完成，可以登录', 'success');
  } catch (error) {
    showToast('验证失败: ' + esc(String(error)), 'error');
  }
}

async function profileForgotMobileCodexPassword() {
  const email = profileMobileCodexFieldValue('pfRecoverEmail');
  if (!email) { showToast('请填写邮箱', 'error'); return; }
  try {
    await profileInvokeMobileCodex('forgot_password', { email });
    showToast('如账号存在，重置邮件已经发送', 'success');
  } catch (error) {
    showToast('请求失败: ' + esc(String(error)), 'error');
  }
}

async function profileResetMobileCodexPassword() {
  const token = profileMobileCodexFieldValue('pfResetToken');
  const password = profileMobileCodexFieldValue('pfResetPassword');
  if (!token || !password) { showToast('请填写重置 Token 和新密码', 'error'); return; }
  try {
    await profileInvokeMobileCodex('reset_password', { token, password });
    profileMobileCodexState = { loading: false, loaded: false, user: null, service: null, devices: [], error: '' };
    showToast('密码已重置，所有设备会话已撤销', 'success');
    await hydrateProfile();
  } catch (error) {
    showToast('重置失败: ' + esc(String(error)), 'error');
  }
}

async function profileLogoutMobileCodex() {
  if (!await showConfirm('退出 Mobile Codex？本机 Companion 和 P2P 网络将停止。')) return;
  try {
    await profileInvokeMobileCodex('logout', {});
    profileMobileCodexState = { loading: false, loaded: false, user: null, service: null, devices: [], error: '' };
    showToast('已退出 Mobile Codex', 'success');
    await hydrateProfile();
  } catch (error) {
    showToast('退出失败: ' + esc(String(error)), 'error');
  }
}

async function profileSetMobileCodexService(start) {
  try {
    await profileInvokeMobileCodex(start ? 'start' : 'stop', {});
    showToast(start ? 'Companion 已启动' : 'Companion 已停止', 'success');
    await hydrateProfile();
  } catch (error) {
    showToast('操作失败: ' + esc(String(error)), 'error');
  }
}

async function profileLaunchMobileCodexChatgpt() {
  try {
    await profileInvokeMobileCodex('launch_chatgpt', {});
    showToast('已请求启动 ChatGPT（桥接模式）', 'success');
    profileMobileCodexState.loaded = false;
    await hydrateProfile();
  } catch (error) {
    showToast('操作失败: ' + esc(String(error)), 'error');
  }
}

async function profileRenameMobileCodexDevice(deviceId) {
  const device = (profileMobileCodexState.devices || []).find(item => String(item.id || '') === String(deviceId || ''));
  const name = await showProfileRenameDeviceModal(device && device.name);
  if (!name) return;
  try {
    await profileInvokeMobileCodex('rename_device', { deviceId, name });
    showToast('设备已重命名', 'success');
    await hydrateProfile();
  } catch (error) {
    showToast('重命名失败: ' + esc(String(error)), 'error');
  }
}

async function profileRevokeMobileCodexDevice(deviceId, isCurrent) {
  const message = isCurrent
    ? '撤销当前电脑后会立即退出账号并停止 Companion，确定继续？'
    : '撤销此设备后，它将无法继续连接。确定继续？';
  if (!await showConfirm(message)) return;
  try {
    await profileInvokeMobileCodex('revoke_device', { deviceId });
    showToast('设备已撤销', 'success');
    await hydrateProfile();
  } catch (error) {
    showToast('撤销失败: ' + esc(String(error)), 'error');
  }
}

function showProfileRenameDeviceModal(currentName) {
  return new Promise(function (resolve) {
    const existing = document.getElementById('profileRenameDeviceModal');
    if (existing) existing.remove();
    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';
    overlay.id = 'profileRenameDeviceModal';
    overlay.innerHTML = '<div class="modal-box plugin-action-modal mc-rename-modal">'
      + '<div class="modal-header"><h3>重命名设备</h3></div>'
      + '<div class="modal-body plugin-action-body">'
      + '<label class="mc-rename-field"><span>设备名称</span>'
      + '<input id="profileRenameDeviceName" class="plugin-config-input" maxlength="80" value="' + escAttr(currentName || '') + '" autocomplete="off"></label>'
      + '<div id="profileRenameDeviceError" class="mc-rename-error" aria-live="polite"></div>'
      + '</div>'
      + '<div class="plugin-preview-actions">'
      + '<button class="btn btn-primary" id="profileRenameDeviceSave" type="button">保存</button>'
      + '<button class="btn btn-ghost" id="profileRenameDeviceCancel" type="button">取消</button>'
      + '</div>'
      + '</div>';
    document.body.appendChild(overlay);

    const input = document.getElementById('profileRenameDeviceName');
    const error = document.getElementById('profileRenameDeviceError');
    const save = document.getElementById('profileRenameDeviceSave');
    const cancel = document.getElementById('profileRenameDeviceCancel');
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

// ── 账号云同步（端到端加密配置保险箱） ───────────────────────

function renderProfileCloudSync() {
  const c = profileCloudState || {};
  if (!c.loggedIn) {
    return '<div class="pm-empty">登录 DEX AI 主账号后可启用账号云同步</div>';
  }
  if (!c.hasVaultScope) {
    return '<div class="pm-err">当前主账号会话缺少云同步权限（dexai:vault），请退出后重新登录 DEX AI 主账号。</div>';
  }
  const toggleRow = '<div class="pm-toggle-row">'
    + '<div class="pm-toggle-copy">'
    + '<strong>账号云同步</strong>'
    + '<p>将本机账号与配置端到端加密后同步到云端；换电脑登录主账号即可一键恢复。</p>'
    + '</div>'
    + '<label class="pm-switch">'
    + '<input type="checkbox" id="cloudSyncToggle" ' + (c.enabled ? 'checked' : '') + ' onchange="profileToggleCloudSync(this.checked)">'
    + '<span class="pm-switch-track" aria-hidden="true"></span>'
    + '</label>'
    + '</div>';
  if (!c.enabled) {
    return toggleRow
      + '<div class="pm-meta-lines"><span>开启后上传 accounts.json 与 config.json 的端到端加密密文，云端不存明文与密码。</span></div>'
      + '<div id="cloudSyncPassphraseBox" style="display:none;">'
      + '<div class="config-fields">'
      + '<div class="config-field wide"><label>云同步密码</label><input id="cloudSyncPassphrase" type="password" autocomplete="new-password" placeholder="至少 8 位，请务必牢记"></div>'
      + '<div class="config-field wide"><label>确认密码</label><input id="cloudSyncPassphrase2" type="password" autocomplete="new-password" placeholder="再次输入"></div>'
      + '</div>'
      + '<button class="pm-btn pm-btn-primary" style="width:100%;height:34px;" onclick="profileEnableCloudSync()">开启并上传</button>'
      + '</div>'
      + '<button class="pm-btn pm-btn-ghost" onclick="profileShowCloudPassphraseBox()">开启云同步</button>';
  }
  const lines = ['同步状态：已开启'];
  if (c.lastSyncAt) lines.push('最近同步：' + new Date(c.lastSyncAt * 1000).toLocaleString());
  if (c.lastRestoreAt) lines.push('最近恢复：' + new Date(c.lastRestoreAt * 1000).toLocaleString());
  if (c.vaultVersion) lines.push('云端版本：v' + c.vaultVersion);
  return toggleRow
    + '<div class="pm-meta-lines">' + lines.map(function (line) { return '<span>' + esc(line) + '</span>'; }).join('') + '</div>'
    + '<div class="pm-btn-row" style="display:flex;gap:8px;flex-wrap:wrap;margin-top:8px;">'
    + '<button class="pm-btn pm-btn-ghost" onclick="profileSyncCloudNow()">立即同步</button>'
    + '<button class="pm-btn pm-btn-ghost" onclick="profileRestoreCloud()">从云端恢复</button>'
    + '<button class="pm-btn pm-btn-danger" onclick="profileClearCloudSync()">清除云端副本并关闭</button>'
    + '</div>';
}

function profileRerenderCloudSync() {
  const section = document.getElementById('profileCloudSyncSection');
  if (section) section.innerHTML = renderProfileCloudSync();
  const badge = document.getElementById('cloudSyncBadge');
  if (badge) {
    const c = profileCloudState || {};
    badge.textContent = c.loggedIn && c.enabled ? '已开启' : '未开启';
  }
}

function profileShowCloudPassphraseBox() {
  const box = document.getElementById('cloudSyncPassphraseBox');
  if (box) box.style.display = '';
}

async function profileEnableCloudSync() {
  const p1 = profileMobileCodexFieldValue('cloudSyncPassphrase');
  const p2 = profileMobileCodexFieldValue('cloudSyncPassphrase2');
  if (!p1 || p1.length < 8) { showToast('云同步密码至少 8 位', 'error'); return; }
  if (p1 !== p2) { showToast('两次输入的密码不一致', 'error'); return; }
  try {
    const status = await invoke('dexai_vault_enable', { passphrase: p1 });
    profileCloudState = Object.assign({}, profileCloudState, status || {});
    showToast('账号云同步已开启并上传', 'success');
    profileRerenderCloudSync();
  } catch (error) {
    showToast('开启失败: ' + esc(String(error)), 'error');
  }
}

async function profileToggleCloudSync(checked) {
  if (checked) {
    profileShowCloudPassphraseBox();
    const toggle = document.getElementById('cloudSyncToggle');
    if (toggle) toggle.checked = false;
    return;
  }
  if (!await showConfirm('关闭账号云同步？本机数据保留，云端副本不会被清除。')) return;
  try {
    const status = await invoke('dexai_vault_disable', { clearRemote: false });
    profileCloudState = Object.assign({}, profileCloudState, status || {});
    showToast('账号云同步已关闭', 'success');
    profileRerenderCloudSync();
  } catch (error) {
    showToast('关闭失败: ' + esc(String(error)), 'error');
    profileRerenderCloudSync();
  }
}

async function profileSyncCloudNow() {
  try {
    const status = await invoke('dexai_vault_sync', { passphrase: null });
    profileCloudState = Object.assign({}, profileCloudState, status || {});
    showToast('已同步到云端', 'success');
    profileRerenderCloudSync();
  } catch (error) {
    showToast('同步失败: ' + esc(String(error)), 'error');
  }
}

// 云同步密码输入弹窗：Tauri webview 不支持原生 prompt 弹窗，用自定义 modal。
function profileShowPassphraseModal() {
  return new Promise(function (resolve) {
    const existing = document.getElementById('profilePassphraseModal');
    if (existing) existing.remove();
    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';
    overlay.id = 'profilePassphraseModal';
    overlay.innerHTML = '<div class="modal-box plugin-action-modal" style="max-width:360px;">'
      + '<div class="modal-header"><h3>输入云同步密码</h3></div>'
      + '<div class="modal-body plugin-action-body">'
      + '<label class="mc-rename-field"><span>云同步密码</span>'
      + '<input id="profilePassphraseInput" class="plugin-config-input" type="password" autocomplete="off" placeholder="至少 8 位"></label>'
      + '</div>'
      + '<div class="plugin-preview-actions">'
      + '<button class="btn btn-primary" id="profilePassphraseOk" type="button">确定</button>'
      + '<button class="btn btn-ghost" id="profilePassphraseCancel" type="button">取消</button>'
      + '</div>'
      + '</div>';
    document.body.appendChild(overlay);

    const input = document.getElementById('profilePassphraseInput');
    let settled = false;

    function cleanup(value) {
      if (settled) return;
      settled = true;
      document.removeEventListener('keydown', onKeyDown);
      overlay.remove();
      resolve(value);
    }

    function onKeyDown(e) {
      if (e.key === 'Enter') {
        e.preventDefault();
        cleanup(input ? input.value : '');
      } else if (e.key === 'Escape') {
        cleanup(null);
      }
    }

    document.getElementById('profilePassphraseOk').onclick = function () {
      cleanup(input ? input.value : '');
    };
    document.getElementById('profilePassphraseCancel').onclick = function () {
      cleanup(null);
    };
    overlay.addEventListener('click', function (e) {
      if (e.target === overlay) cleanup(null);
    });
    document.addEventListener('keydown', onKeyDown);
    if (input) input.focus();
  });
}

async function profileRestoreCloud() {
  if (!await showConfirm('从云端恢复将覆盖本机 accounts.json 与 config.json（恢复前自动备份）。确定继续？')) return;
  const passphrase = await profileShowPassphraseModal();
  if (!passphrase) return;
  try {
    const status = await invoke('dexai_vault_restore', { passphrase: passphrase });
    profileCloudState = Object.assign({}, profileCloudState, status || {});
    showToast('已从云端恢复，建议重启服务使配置生效', 'success');
    profileRerenderCloudSync();
  } catch (error) {
    showToast('恢复失败: ' + esc(String(error)), 'error');
  }
}

async function profileClearCloudSync() {
  if (!await showConfirm('确定清除云端副本并关闭账号云同步？本机数据保留。')) return;
  try {
    const status = await invoke('dexai_vault_clear', {});
    profileCloudState = Object.assign({}, profileCloudState, status || {});
    showToast('云端副本已清除', 'success');
    profileRerenderCloudSync();
  } catch (error) {
    showToast('清除失败: ' + esc(String(error)), 'error');
  }
}

// ── DEX AI 偏好设置（跟随账号） ──────────────────────────────

function renderProfilePrefs() {
  const p = profilePrefs || {};
  const theme = p.theme || 'auto';
  const language = p.language || 'zh-CN';
  const startupPanel = p.startupPanel || 'status';
  const updateReminder = p.updateReminder !== false;
  return '<div class="pm-toggle-row">'
    + '<div class="pm-toggle-copy">'
    + '<strong>偏好设置跟随账号</strong>'
    + '<p>修改后自动同步到云端，换电脑登录主账号后自动恢复。</p>'
    + '</div>'
    + '</div>'
    + '<div class="config-fields">'
    + '<div class="config-field"><label>主题</label>'
    + '<select id="prefTheme" onchange="profileSetPref(\'theme\', this.value)">'
    + '<option value="auto"' + (theme === 'auto' ? ' selected' : '') + '>跟随系统</option>'
    + '<option value="light"' + (theme === 'light' ? ' selected' : '') + '>浅色</option>'
    + '<option value="dark"' + (theme === 'dark' ? ' selected' : '') + '>深色</option>'
    + '</select></div>'
    + '<div class="config-field"><label>界面语言</label>'
    + '<select id="prefLanguage" onchange="profileSetPref(\'language\', this.value)">'
    + '<option value="zh-CN"' + (language === 'zh-CN' ? ' selected' : '') + '>简体中文</option>'
    + '<option value="en"' + (language === 'en' ? ' selected' : '') + '>English</option>'
    + '</select></div>'
    + '<div class="config-field wide"><label>启动时默认面板</label>'
    + '<select id="prefStartupPanel" onchange="profileSetPref(\'startupPanel\', this.value)">'
    + '<option value="status"' + (startupPanel === 'status' ? ' selected' : '') + '>服务概览</option>'
    + '<option value="profile"' + (startupPanel === 'profile' ? ' selected' : '') + '>个人中心</option>'
    + '<option value="threads"' + (startupPanel === 'threads' ? ' selected' : '') + '>线程聚合</option>'
    + '<option value="sessions"' + (startupPanel === 'sessions' ? ' selected' : '') + '>请求历史</option>'
    + '</select></div>'
    + '</div>'
    + '<div class="pm-toggle-row">'
    + '<div class="pm-toggle-copy"><strong>新版本提醒</strong><p>有新版本时在界面显示更新提示。</p></div>'
    + '<label class="pm-switch">'
    + '<input type="checkbox" id="prefUpdateReminder" ' + (updateReminder ? 'checked' : '') + ' onchange="profileSetPref(\'updateReminder\', this.checked)">'
    + '<span class="pm-switch-track" aria-hidden="true"></span>'
    + '</label>'
    + '</div>';
}

function profileRerenderPrefs() {
  const section = document.getElementById('profilePrefsSection');
  if (section) section.innerHTML = renderProfilePrefs();
  const badge = document.getElementById('prefsBadge');
  if (badge) badge.textContent = (profilePrefs && Object.keys(profilePrefs).length) ? '已同步' : '默认';
}

async function profileSetPref(key, value) {
  const prefs = Object.assign({}, profilePrefs || {});
  prefs[key] = value;
  profilePrefs = prefs;
  if (key === 'theme' && typeof applyTheme === 'function') applyTheme(String(value));
  if (key === 'startupPanel') deeStorage.setItem('deecodex.startupPanel', String(value));
  if (key === 'updateReminder') deeStorage.setItem('deecodex.updateReminder', value ? '1' : '0');
  try {
    const result = await invoke('dexai_prefs_set', { prefs: prefs });
    profilePrefs = (result && result.prefs) || prefs;
    showToast('偏好已保存并同步', 'success');
    profileRerenderPrefs();
  } catch (error) {
    showToast('同步偏好失败（已保存在本机）: ' + esc(String(error)), 'error');
  }
}

async function hydrateProfileCloud() {
  try {
    const status = await invoke('dexai_cloud_status');
    profileCloudState = Object.assign({}, profileCloudState, status || {});
  } catch (error) {
    profileCloudState.error = String(error);
  }
  profileRerenderCloudSync();
}

async function hydrateProfilePrefs() {
  try {
    const result = await invoke('dexai_prefs_get');
    profilePrefs = (result && result.prefs) || {};
    if (typeof applyTheme === 'function' && profilePrefs.theme) applyTheme(String(profilePrefs.theme));
    if (profilePrefs.startupPanel) deeStorage.setItem('deecodex.startupPanel', String(profilePrefs.startupPanel));
    if (profilePrefs.updateReminder === false) deeStorage.setItem('deecodex.updateReminder', '0');
  } catch (error) {
    // 未登录或网络失败时保留本机偏好
  }
  profileRerenderPrefs();
}

// ── 匿名使用统计（原有功能） ───────────────────────────────────

async function hydrateProfileTelemetry() {
  const prefs = await window.DexTelemetry?.loadPreferences?.();
  const toggle = document.getElementById('telemetryEnabledToggle');
  const status = document.getElementById('telemetryStatusText');
  const installId = document.getElementById('telemetryInstallId');
  const sessionId = document.getElementById('telemetrySessionId');
  const endpoint = document.getElementById('telemetryEndpoint');
  if (!prefs) {
    if (status) status.textContent = '读取失败';
    return;
  }
  if (toggle) toggle.checked = prefs.enabled !== false;
  if (status) status.textContent = prefs.enabled !== false ? '已开启' : '已关闭';
  if (installId) installId.textContent = prefs.install_id || '—';
  if (sessionId) sessionId.textContent = prefs.session_id || '—';
  if (endpoint) endpoint.textContent = prefs.endpoint || '—';
}

async function toggleTelemetryEnabled(enabled) {
  try {
    const prefs = await window.DexTelemetry?.setEnabled?.(enabled);
    const status = document.getElementById('telemetryStatusText');
    if (status && prefs) status.textContent = prefs.enabled !== false ? '已开启' : '已关闭';
    showToast(prefs?.enabled !== false ? '匿名统计已开启' : '匿名统计已关闭', 'success');
  } catch (error) {
    showToast('保存匿名统计设置失败: ' + error, 'error');
    hydrateProfileTelemetry();
  }
}

// ═══════════════════════════════════════════════════════════════
