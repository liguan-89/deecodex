const assert = require('assert');
const fs = require('fs');
const path = require('path');
const vm = require('vm');

const profileFiles = ['legal-content.js', 'profile-login.js', 'placeholder-pages.js'];

const html = fs.readFileSync(path.join(__dirname, '..', 'index.html'), 'utf8');
const loadedProfileScripts = Array.from(html.matchAll(/<script src="js\/(legal-content\.js|placeholder-pages\.js|profile-login\.js)\?/g))
  .map(match => match[1]);
assert.deepStrictEqual(loadedProfileScripts, profileFiles);

// 个人中心函数不能与插件详情页账号函数重名，避免全局命名冲突。
const owners = new Map();
['placeholder-pages.js', 'plugins-detail.js'].forEach(file => {
  const source = fs.readFileSync(path.join(__dirname, file), 'utf8');
  const re = /^(?:async\s+)?function\s+([A-Za-z0-9_]+)\s*\(/gm;
  let match;
  while ((match = re.exec(source))) {
    const list = owners.get(match[1]) || [];
    list.push(file);
    owners.set(match[1], list);
  }
});
const duplicates = Array.from(owners.entries()).filter(([, files]) => files.length > 1);
assert.deepStrictEqual(duplicates, []);

const elements = new Map();
const invokeCalls = [];
const context = {
  console,
  crypto: globalThis.crypto,
  btoa: globalThis.btoa,
  TextEncoder: globalThis.TextEncoder,
  setInterval: () => 1,
  clearInterval() {},
  setTimeout: () => 1,
  clearTimeout() {},
  addEventListener() {},
  removeEventListener() {},
  window: {},
  document: {
    activeElement: null,
    body: { appendChild() {} },
    createElement() {
      return {
        className: '',
        id: '',
        innerHTML: '',
        value: '',
        addEventListener() {},
        querySelector() { return null; },
        remove() {},
      };
    },
    getElementById(id) {
      if (!elements.has(id)) {
        elements.set(id, {
          id,
          value: '',
          innerHTML: '',
          textContent: '',
          style: {},
          classList: { toggle() {}, add() {}, remove() {} },
          setAttribute() {},
          focus() {},
          select() {},
          querySelector() { return null; },
          remove() {},
        });
      }
      return elements.get(id);
    },
    querySelector() { return null; },
    querySelectorAll() { return []; },
    addEventListener() {},
    removeEventListener() {},
  },
  deeStorage: {
    data: {},
    getItem(key) { return this.data[key] || ''; },
    setItem(key, value) { this.data[key] = String(value); },
    removeItem(key) { delete this.data[key]; },
  },
  esc: value => String(value ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;'),
  escAttr: value => String(value ?? '')
    .replace(/&/g, '&amp;')
    .replace(/"/g, '&quot;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;'),
  invoke: async (command, args) => {
    invokeCalls.push({ command, args });
    if (command === 'dexai_master_status') {
      return { loggedIn: true, email: 'master@example.com', accountId: 'acc-master', scope: 'dexai:profile.read dexai:consent dexai:grants' };
    }
    if (command === 'dexai_oauth_grants') {
      return {
        grants: [
          { grantId: 'g1', clientId: 'dexai-mobile-codex', clientName: 'Mobile Codex 插件', firstParty: true, scopes: ['mobile-codex:service'], status: 'active' }
        ]
      };
    }
    if (command === 'dexai_oauth_audit') {
      return {
        events: [
          { action: 'oauth.token.issue', outcome: 'ok', clientId: 'dexai-mobile-codex', clientName: 'Mobile Codex 插件', clientKey: 'dexai-mobile-codex', requestId: 'r1', createdAt: '2026-08-16T12:00:00Z' }
        ]
      };
    }
    if (command === 'dexai_oauth_authorize') return { code: 'AUTHCODE123', expiresIn: 60 };
    if (command === 'dexai_master_login') return { loggedIn: true };
    if (command === 'dexai_plugin_auth_manifest') {
      return {
        plugins: [
          { pluginId: 'deecodex-mobile-codex', name: 'Mobile Codex 插件', version: '0.1.19', enabled: true, clientId: 'dexai-mobile-codex', scopes: ['mobile-codex:service', 'mobile-codex:devices', 'mobile-codex:p2p'], reason: '管理本机 Companion 与绑定设备', redirectUri: 'dexai://mobile-codex/callback', firstParty: true, methods: { exchange: 'mobile_codex.oauth.exchange', status: 'mobile_codex.oauth.status', logout: 'mobile_codex.oauth.logout' } }
        ]
      };
    }
    if (command === 'invoke_plugin_rpc') {
      if (args && args.method === 'mobile_codex.oauth.status') {
        return { oauthEnabled: true, clientId: 'dexai-mobile-codex', scopes: 'mobile-codex:service mobile-codex:devices mobile-codex:p2p' };
      }
      if (args && args.method === 'mobile_codex.oauth.exchange') {
        return { ok: true, scope: 'mobile-codex:service mobile-codex:devices mobile-codex:p2p', expiresIn: 600, clientId: 'dexai-mobile-codex' };
      }
      return { ok: true };
    }
    if (command === 'invoke_plugin_account_action') {
      if (args.action === 'current_user') return { loggedIn: true, email: 'demo@example.com', accountId: 'acc-1', deviceId: 'dev-1', platform: 'macOS' };
      if (args.action === 'status') return { running: true, ownership: 'plugin', network: { networkPath: 'lan' } };
      if (args.action === 'list_devices') {
        return {
          devices: [
            { id: 'dev-1', name: '我的 Mac', deviceType: 'computer', platform: 'macOS', companionVersion: '1.2.3', online: true },
            { id: 'dev-2', name: '我的手机', deviceType: 'mobile', platform: 'Android', appVersion: '2.0.0', online: false },
          ]
        };
      }
      return { ok: true };
    }
    return {};
  },
  showToast() {},
  showConfirm: async () => true,
  switchPanel() {},
  wrapPrimaryPanel: (_panel, content) => content,
  DexTelemetry: {
    loadPreferences: async () => ({ enabled: true, install_id: 'inst-1', session_id: 'sess-1', endpoint: 'https://telemetry.example' }),
    setEnabled: async enabled => ({ enabled: Boolean(enabled) }),
  },
};
context.window = context;
context.window.deeStorage = context.deeStorage;

vm.createContext(context);
profileFiles.forEach(file => {
  const source = fs.readFileSync(path.join(__dirname, file), 'utf8');
  vm.runInContext(source, context, { filename: file });
});

// 法律文档：内容常量、渲染与弹窗打开
const legal = vm.runInContext('window.DexLegal', context);
assert(legal && legal.PRIVACY && legal.TOS, 'DexLegal 应暴露隐私政策与服务条款');
assert(legal.PRIVACY.sections.length >= 10, '隐私政策应覆盖主要章节');
assert(legal.TOS.sections.length >= 15, '服务条款应覆盖主要章节');
const privacyHtml = vm.runInContext('window.DexLegal.render(window.DexLegal.PRIVACY)', context);
assert(privacyHtml.includes('本地代理'));
assert(privacyHtml.includes('调试助手'));
assert(privacyHtml.includes('微信通道'));
assert(privacyHtml.includes('云同步'));
assert(privacyHtml.includes('匿名使用统计'));
assert(privacyHtml.includes('广告位'));
const tosHtml = vm.runInContext('window.DexLegal.render(window.DexLegal.TOS)', context);
assert(tosHtml.includes('多账号'));
assert(tosHtml.includes('中转'));
assert(tosHtml.includes('强制更新'));
assert(tosHtml.includes('邀请码'));
assert(tosHtml.includes('责任限制'));
assert.strictEqual(vm.runInContext("window.DexLegal.open('privacy')", context), true);
assert.strictEqual(vm.runInContext("window.DexLegal.open('tos')", context), true);

// 初始渲染：未加载时只显示主账号读取状态，不露出兼容层/统计
let panel = vm.runInContext('renderProfile()', context);
assert(panel.includes('个人中心'));
assert(panel.includes('DEX AI 主账号'));
assert(panel.includes('正在读取主账号状态'));
assert(!panel.includes('Mobile Codex 账号'));
assert(!panel.includes('匿名使用统计'));

// 未登录：整页只显示一套登录视图（左右分栏 + 动画角色），不含兼容层/统计
vm.runInContext(`
  profileMasterState = { loading: false, loaded: true, loggedIn: false, email: null, accountId: null, scope: null, grants: [], error: '' };
`, context);
panel = vm.runInContext('renderProfile()', context);
assert(panel.includes('登录 DEX AI'));
assert(panel.includes('pl-characters'));
assert(panel.includes('联系作者'));
assert(panel.includes('pfMasterEmail'));
assert(panel.includes('pfMasterPassword'));
assert(panel.includes("DexLegal.open('privacy')"));
assert(panel.includes("DexLegal.open('tos')"));
assert(!panel.includes('个人中心'));
assert(!panel.includes('Mobile Codex 账号'));
assert(!panel.includes('匿名使用统计'));

// 注册模式：同一右栏切换为注册表单（邀请码 / 邮箱 / 密码 / 确认密码）
const registerForm = vm.runInContext('profileLoginRenderForm("register")', context);
assert(registerForm.includes('注册 DEX AI'));
assert(registerForm.includes('pfRegisterInvitation'));
assert(registerForm.includes('pfRegisterEmail2'));
assert(registerForm.includes('pfRegisterPassword2'));
assert(registerForm.includes('pfRegisterPasswordConfirm'));
assert(registerForm.includes('profileLoginShowLogin'));
assert(registerForm.includes('profileLoginRegisterSubmit'));

// 已登录：恢复完整个人中心（主账号信息 + Mobile Codex 兼容层 + 匿名统计）
vm.runInContext(`
  profileMasterState = {
    loading: false, loaded: true, loggedIn: true,
    email: 'master@example.com', accountId: 'acc-master',
    scope: 'dexai:profile.read dexai:consent dexai:grants',
    grants: [], error: ''
  };
`, context);
panel = vm.runInContext('renderProfile()', context);
assert(panel.includes('Mobile Codex 设备与服务'));
assert(panel.includes('匿名使用统计'));
assert(panel.includes('profileMasterSection'));
assert(panel.includes('profileAccountSection'));
assert(panel.includes('telemetryEnabledToggle'));
assert(panel.includes('退出登录'));
assert(panel.includes('法律与隐私'));
assert(panel.includes('pm-legal-links'));

// 主账号：未登录视图
vm.runInContext(`
  profileMasterState = { loading: false, loaded: true, loggedIn: false, email: null, accountId: null, scope: null, grants: [], error: '' };
`, context);
let masterPanel = vm.runInContext('renderProfileMasterSection()', context);
assert(masterPanel.includes('pfMasterEmail'));
assert(masterPanel.includes('pfMasterPassword'));
assert(masterPanel.includes('登录 DEX AI'));
assert(masterPanel.includes('pl-characters'));
assert(masterPanel.includes('注册'));
assert(masterPanel.includes('联系作者'));

// 主账号：已登录视图（含授权管理）
vm.runInContext(`
  profileMasterState = {
    loading: false, loaded: true, loggedIn: true,
    email: 'master@example.com', accountId: 'acc-master',
    scope: 'dexai:profile.read dexai:consent dexai:grants',
    grants: [{ grantId: 'g1', clientId: 'dexai-mobile-codex', clientName: 'Mobile Codex 插件', firstParty: true, scopes: ['mobile-codex:service'], status: 'active' }],
    audit: [{ action: 'oauth.token.issue', outcome: 'ok', clientName: 'Mobile Codex 插件', createdAt: '2026-08-16T12:00:00Z' }],
    error: ''
  };
  profilePluginAuthList = [
    { pluginId: 'deecodex-mobile-codex', name: 'Mobile Codex 插件', version: '0.1.19', enabled: true, clientId: 'dexai-mobile-codex', scopes: ['mobile-codex:service', 'mobile-codex:devices', 'mobile-codex:p2p'], reason: '管理本机 Companion 与绑定设备', redirectUri: 'dexai://mobile-codex/callback', firstParty: true, methods: { exchange: 'mobile_codex.oauth.exchange', status: 'mobile_codex.oauth.status', logout: 'mobile_codex.oauth.logout' } }
  ];
  profilePluginOAuthMap = { 'deecodex-mobile-codex': { oauthEnabled: true, scopes: 'mobile-codex:service mobile-codex:devices mobile-codex:p2p' } };
`, context);
masterPanel = vm.runInContext('renderProfileMasterSection()', context);
assert(masterPanel.includes('master@example.com'));
assert(masterPanel.includes('已授权 1 个应用'));
assert(!masterPanel.includes('待授权插件')); // 已授权的插件不再出现在待授权列表，避免重复
assert(masterPanel.includes('已授权应用'));
assert(masterPanel.includes('最近授权记录'));
assert(masterPanel.includes('换取令牌'));
assert(masterPanel.includes('Mobile Codex 插件'));
assert(masterPanel.includes('第一方'));
assert(masterPanel.includes('撤销'));

// 主账号：插件授权被撤销 → 显示“需重新授权”
vm.runInContext(`
  profileMasterState.grants = [{ grantId: 'g1', clientId: 'dexai-mobile-codex', clientName: 'Mobile Codex 插件', firstParty: true, scopes: ['mobile-codex:service'], status: 'revoked' }];
  profilePluginOAuthMap = { 'deecodex-mobile-codex': { oauthEnabled: false, scopes: null } };
`, context);
masterPanel = vm.runInContext('renderProfileMasterSection()', context);
assert(masterPanel.includes('待授权插件'));
assert(masterPanel.includes('需重新授权'));
assert(masterPanel.includes('重新授权'));

// 未登录视图：登录卡 + 注册 / 验证 / 找回 / 重置折叠区
vm.runInContext(`
  profileMobileCodexState = { loading: false, loaded: true, user: {}, service: {}, devices: [], error: '' };
`, context);
panel = vm.runInContext('renderProfileAccountSection()', context);
assert(panel.includes('pfLoginEmail'));
assert(panel.includes('pfLoginPassword'));
assert(panel.includes('登录并启动 Companion'));
assert(panel.includes('邀请码注册'));
assert(panel.includes('验证邮箱'));
assert(panel.includes('忘记或重置密码'));
assert(panel.includes('Mobile Codex 兼容账号未登录'));

// 已登录视图：账号信息 + 连接状态 + 设备列表 + 退出
vm.runInContext(`
  profileMobileCodexState = {
    loading: false,
    loaded: true,
    user: { loggedIn: true, email: 'demo@example.com', accountId: 'acc-1', deviceId: 'dev-1', platform: 'macOS' },
    service: { running: true, ownership: 'plugin', network: { networkPath: 'lan' } },
    devices: [
      { id: 'dev-1', name: '我的 Mac', deviceType: 'computer', platform: 'macOS', companionVersion: '1.2.3', online: true },
      { id: 'dev-2', name: '我的手机', deviceType: 'mobile', platform: 'Android', appVersion: '2.0.0', online: false },
    ],
    error: ''
  };
`, context);
panel = vm.runInContext('renderProfileAccountSection()', context);
assert(panel.includes('demo@example.com'));
assert(panel.includes('2 台设备已绑定'));
assert(panel.includes('我的设备'));
assert(panel.includes('我的 Mac'));
assert(panel.includes('我的手机'));
assert(panel.includes('退出登录'));
assert(panel.includes('重命名'));
assert(panel.includes('撤销'));
assert(panel.includes('停止 Companion'));

// 未加载状态重置后，hydrateProfile 会走 current_user / status / list_devices
vm.runInContext(`
  profileMobileCodexState = { loading: false, loaded: false, user: null, service: null, devices: [], error: '' };
`, context);
invokeCalls.length = 0;
(async () => {
  await vm.runInContext('hydrateProfile()', context);
  const accountCalls = invokeCalls.filter(call => call.command === 'invoke_plugin_account_action');
  assert.deepStrictEqual(accountCalls.map(call => call.args.action), ['current_user', 'status', 'list_devices']);
  assert.strictEqual(vm.runInContext('profileMobileCodexState.user.email', context), 'demo@example.com');
  assert.strictEqual(vm.runInContext('profileMobileCodexState.devices.length', context), 2);

  // 登录动作：读取输入框并调用 login
  vm.runInContext(`
    profileMobileCodexState = { loading: false, loaded: true, user: {}, service: {}, devices: [], error: '' };
    renderProfileAccountSection();
  `, context);
  // 模拟 DOM 解析：先注册输入框元素，再赋值
  context.document.getElementById('pfLoginEmail').value = 'demo@example.com';
  context.document.getElementById('pfLoginPassword').value = 'secret-password-123';
  invokeCalls.length = 0;
  await vm.runInContext('profileLoginMobileCodex()', context);
  const loginCall = invokeCalls.find(call => call.command === 'invoke_plugin_account_action' && call.args.action === 'login');
  assert(loginCall);
  assert.strictEqual(loginCall.args.pluginId, 'deecodex-mobile-codex');
  assert.strictEqual(loginCall.args.params.email, 'demo@example.com');
  assert.strictEqual(loginCall.args.params.password, 'secret-password-123');
  assert.strictEqual(elements.get('pfLoginPassword').value, '');

  // 注册动作
  context.document.getElementById('pfInvitationCode').value = 'INVITE-1';
  context.document.getElementById('pfRegisterEmail').value = 'new@example.com';
  context.document.getElementById('pfRegisterPassword').value = 'another-secret-123';
  invokeCalls.length = 0;
  await vm.runInContext('profileRegisterMobileCodex()', context);
  const registerCall = invokeCalls.find(call => call.command === 'invoke_plugin_account_action' && call.args.action === 'register');
  assert(registerCall);
  assert.strictEqual(registerCall.args.params.invitationCode, 'INVITE-1');
  assert.strictEqual(registerCall.args.params.email, 'new@example.com');
  assert.strictEqual(registerCall.args.params.password, 'another-secret-123');

  // 退出动作
  invokeCalls.length = 0;
  await vm.runInContext('profileLogoutMobileCodex()', context);
  assert(invokeCalls.some(call => call.command === 'invoke_plugin_account_action' && call.args.action === 'logout'));

  // 设备撤销动作
  invokeCalls.length = 0;
  await vm.runInContext("profileRevokeMobileCodexDevice('dev-2', false)", context);
  assert(invokeCalls.some(call => call.command === 'invoke_plugin_account_action' && call.args.action === 'revoke_device'));

  // 主账号：hydrate 走 status + grants
  vm.runInContext(`
    profileMasterState = { loading: false, loaded: false, loggedIn: false, email: null, accountId: null, scope: null, grants: [], error: '' };
  `, context);
  invokeCalls.length = 0;
  await vm.runInContext('hydrateProfileMaster()', context);
  assert(invokeCalls.some(call => call.command === 'dexai_master_status'));
  assert(invokeCalls.some(call => call.command === 'dexai_oauth_grants'));
  assert.strictEqual(vm.runInContext('profileMasterState.loggedIn', context), true);
  assert.strictEqual(vm.runInContext('profileMasterState.email', context), 'master@example.com');
  assert.strictEqual(vm.runInContext('profileMasterState.grants.length', context), 1);
  assert.strictEqual(vm.runInContext('profileMasterState.audit.length', context), 1);
  assert.strictEqual(vm.runInContext('profilePluginAuthList.length', context), 1);
  assert.strictEqual(vm.runInContext("profilePluginOAuthMap['deecodex-mobile-codex'].oauthEnabled", context), true);

  // 主账号：登录动作
  vm.runInContext(`
    profileMasterState = { loading: false, loaded: true, loggedIn: false, email: null, accountId: null, scope: null, grants: [], error: '' };
    renderProfileMasterSection();
  `, context);
  context.document.getElementById('pfMasterEmail').value = 'master@example.com';
  context.document.getElementById('pfMasterPassword').value = 'master-secret-123';
  invokeCalls.length = 0;
  await vm.runInContext('profileMasterLogin()', context);
  const masterLoginCall = invokeCalls.find(call => call.command === 'dexai_master_login');
  assert(masterLoginCall);
  assert.strictEqual(masterLoginCall.args.email, 'master@example.com');
  assert.strictEqual(masterLoginCall.args.password, 'master-secret-123');

  // 主账号：授权插件生成授权码
  invokeCalls.length = 0;
  await vm.runInContext("profileMasterAuthorizePlugin('deecodex-mobile-codex')", context);
  const authorizeCall = invokeCalls.find(call => call.command === 'dexai_oauth_authorize');
  assert(authorizeCall);
  assert.strictEqual(authorizeCall.args.clientId, 'dexai-mobile-codex');
  assert(authorizeCall.args.codeChallenge.length >= 40);
  const exchangeCall = invokeCalls.find(call => call.command === 'invoke_plugin_rpc' && call.args.method === 'mobile_codex.oauth.exchange');
  assert(exchangeCall);
  assert.strictEqual(exchangeCall.args.pluginId, 'deecodex-mobile-codex');
  assert.strictEqual(exchangeCall.args.params.code, 'AUTHCODE123');
  assert(exchangeCall.args.params.codeVerifier.length >= 43);
  assert.strictEqual(vm.runInContext("profilePluginOAuthMap['deecodex-mobile-codex'].oauthEnabled", context), true);

  // 遥测 hydrate 仍可用
  await vm.runInContext('hydrateProfileTelemetry()', context);
  assert.strictEqual(elements.get('telemetryStatusText').textContent, '已开启');
  assert.strictEqual(elements.get('telemetryInstallId').textContent, 'inst-1');

  // 完整个人中心包含账号云同步与偏好设置区块
  vm.runInContext(`
    profileMasterState = { loading: false, loaded: true, loggedIn: true, email: 'master@example.com', accountId: 'acc-master', scope: 'dexai:profile.read dexai:consent dexai:grants dexai:vault dexai:prefs', grants: [], error: '' };
  `, context);
  panel = vm.runInContext('renderProfile()', context);
  assert(panel.includes('账号云同步'));
  assert(panel.includes('DEX AI 偏好设置'));
  assert(panel.includes('profileCloudSyncSection'));
  assert(panel.includes('profilePrefsSection'));

  // 账号云同步：未开启（有权限）
  vm.runInContext(`
    profileCloudState = { loggedIn: true, hasVaultScope: true, enabled: false, lastSyncAt: null, lastRestoreAt: null, vaultVersion: 0, error: '' };
  `, context);
  let cloudPanel = vm.runInContext('renderProfileCloudSync()', context);
  assert(cloudPanel.includes('账号云同步'));
  assert(cloudPanel.includes('开启云同步'));
  assert(cloudPanel.includes('云端不存明文'));

  // 账号云同步：已开启（显示同步信息与操作）
  vm.runInContext(`
    profileCloudState = { loggedIn: true, hasVaultScope: true, enabled: true, lastSyncAt: 1787000000, lastRestoreAt: null, vaultVersion: 1, error: '' };
  `, context);
  cloudPanel = vm.runInContext('renderProfileCloudSync()', context);
  assert(cloudPanel.includes('立即同步'));
  assert(cloudPanel.includes('从云端恢复'));
  assert(cloudPanel.includes('清除云端副本并关闭'));
  assert(cloudPanel.includes('最近同步'));
  assert(cloudPanel.includes('云端版本：v1'));

  // 账号云同步：缺少 scope 提示
  vm.runInContext(`
    profileCloudState = { loggedIn: true, hasVaultScope: false, enabled: false, error: '' };
  `, context);
  cloudPanel = vm.runInContext('renderProfileCloudSync()', context);
  assert(cloudPanel.includes('缺少云同步权限'));

  // 偏好设置：默认值与跟随账号文案
  vm.runInContext(`
    profilePrefs = { theme: 'dark', language: 'zh-CN', startupPanel: 'profile', updateReminder: true };
  `, context);
  const prefsPanel = vm.runInContext('renderProfilePrefs()', context);
  assert(prefsPanel.includes('偏好设置跟随账号'));
  assert(prefsPanel.includes('主题'));
  assert(prefsPanel.includes('界面语言'));
  assert(prefsPanel.includes('启动时默认面板'));
  assert(prefsPanel.includes('新版本提醒'));
  assert(prefsPanel.includes('value="dark" selected'));

  // 云同步/偏好动作：enable 与 prefs_set 调用链
  invokeCalls.length = 0;
  context.document.getElementById('cloudSyncPassphrase').value = 'sync-secret-123';
  context.document.getElementById('cloudSyncPassphrase2').value = 'sync-secret-123';
  await vm.runInContext('profileEnableCloudSync()', context);
  const enableCall = invokeCalls.find(call => call.command === 'dexai_vault_enable');
  assert(enableCall);
  assert.strictEqual(enableCall.args.passphrase, 'sync-secret-123');

  invokeCalls.length = 0;
  await vm.runInContext("profileSetPref('theme', 'light')", context);
  const prefsSetCall = invokeCalls.find(call => call.command === 'dexai_prefs_set');
  assert(prefsSetCall);
  assert.strictEqual(prefsSetCall.args.prefs.theme, 'light');

  // 登录/注册必须勾选同意《隐私政策》与《服务条款》
  // 1) 两个表单都渲染勾选框，且条款可打开弹窗
  const loginForm = vm.runInContext('profileLoginRenderForm("login")', context);
  assert(loginForm.includes('pfLoginAgree'));
  assert(loginForm.includes("DexLegal.open('privacy')"));
  assert(loginForm.includes("DexLegal.open('tos')"));
  const registerFormChecked = vm.runInContext('profileLoginRenderForm("register")', context);
  assert(registerFormChecked.includes('pfRegisterAgree'));

  // 2) 未勾选：登录被拦截并提示，不发起登录
  vm.runInContext(`
    profileLoginState.agreedLogin = false;
    profileLoginState.agreedRegister = false;
    profileLoginRenderForm("login");
  `, context);
  context.document.getElementById('pfMasterEmail').value = 'master@example.com';
  context.document.getElementById('pfMasterPassword').value = 'master-secret-123';
  invokeCalls.length = 0;
  let agreeSubmit = vm.runInContext('profileLoginSubmit({ preventDefault() {} })', context);
  assert.strictEqual(agreeSubmit, false);
  assert(!invokeCalls.some(call => call.command === 'dexai_master_login'));
  assert(vm.runInContext('profileLoginState.error', context).includes('隐私政策'));

  // 3) 勾选后：登录放行并调用 dexai_master_login
  vm.runInContext('profileLoginState.agreedLogin = true;', context);
  invokeCalls.length = 0;
  agreeSubmit = vm.runInContext('profileLoginSubmit({ preventDefault() {} })', context);
  assert.strictEqual(agreeSubmit, false); // 返回 false 阻止原生提交，但已发起登录
  assert(invokeCalls.some(call => call.command === 'dexai_master_login'));

  // 4) 未勾选：注册被拦截并提示，不发起注册
  vm.runInContext(`
    profileLoginState.agreedLogin = false;
    profileLoginState.agreedRegister = false;
    profileLoginRenderForm("register");
  `, context);
  context.document.getElementById('pfRegisterInvitation').value = 'INVITE-1';
  context.document.getElementById('pfRegisterEmail2').value = 'new@example.com';
  context.document.getElementById('pfRegisterPassword2').value = 'another-secret-123';
  context.document.getElementById('pfRegisterPasswordConfirm').value = 'another-secret-123';
  invokeCalls.length = 0;
  agreeSubmit = vm.runInContext('profileLoginRegisterSubmit({ preventDefault() {} })', context);
  assert.strictEqual(agreeSubmit, false);
  assert(!invokeCalls.some(call => call.command === 'dexai_master_register'));
  assert(vm.runInContext('profileLoginState.error', context).includes('隐私政策'));

  // 5) 勾选后：注册放行并调用 dexai_master_register
  vm.runInContext('profileLoginState.agreedRegister = true;', context);
  invokeCalls.length = 0;
  agreeSubmit = vm.runInContext('profileLoginRegisterSubmit({ preventDefault() {} })', context);
  assert.strictEqual(agreeSubmit, false);
  assert(invokeCalls.some(call => call.command === 'dexai_master_register'));
  assert(!vm.runInContext('profileLoginState.error', context).includes('隐私政策'));

  // Tauri webview 不支持原生 prompt（含 window.prompt 与裸 prompt），密码输入必须走自定义弹窗。
  const placeholderSrc = fs.readFileSync(path.join(__dirname, 'placeholder-pages.js'), 'utf8');
  assert(!/prompt\s*\(/.test(placeholderSrc), '不应使用原生 prompt，请用自定义密码弹窗');
  console.log('profile render smoke ok');
})().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
