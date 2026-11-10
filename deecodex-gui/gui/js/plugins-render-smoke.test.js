const assert = require('assert');
const fs = require('fs');
const path = require('path');
const vm = require('vm');

const pluginScripts = [
  'plugins.js',
  'plugins-events.js',
  'plugins-dev.js',
  'plugins-detail.js',
  'plugins-market.js',
  'plugins-wb.js',
  'plugins-exports.js',
];

const html = fs.readFileSync(path.join(__dirname, '..', 'index.html'), 'utf8');
const loadedPluginScripts = Array.from(html.matchAll(/<script src="js\/(plugins(?:-[^"?]+)?\.js)\?/g))
  .map(match => match[1]);
assert.deepStrictEqual(loadedPluginScripts, pluginScripts);

const functionOwners = new Map();
pluginScripts.forEach(file => {
  const source = fs.readFileSync(path.join(__dirname, file), 'utf8');
  const re = /^(?:async\s+)?function\s+([A-Za-z0-9_]+)\s*\(/gm;
  let match;
  while ((match = re.exec(source))) {
    const owners = functionOwners.get(match[1]) || [];
    owners.push(file);
    functionOwners.set(match[1], owners);
  }
});
const duplicateFunctions = Array.from(functionOwners.entries()).filter(([, owners]) => owners.length > 1);
assert.deepStrictEqual(duplicateFunctions, []);

const elements = new Map();
const context = {
  console,
  setInterval: () => 1,
  clearInterval() {},
  window: {},
  document: {
    activeElement: null,
    body: { appendChild() {} },
    createElement() {
      return {
        className: '',
        id: '',
        innerHTML: '',
        addEventListener() {},
        remove() {},
      };
    },
    getElementById(id) {
      if (!elements.has(id)) {
        elements.set(id, {
          id,
          value: '',
          innerHTML: '',
          style: {},
          classList: { toggle() {}, add() {}, remove() {} },
          setAttribute() {},
          focus() {},
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
    data: { 'deecodex.pluginDevOpen': '1' },
    getItem(key) { return this.data[key] || ''; },
    setItem(key, value) { this.data[key] = String(value); },
    removeItem(key) { delete this.data[key]; },
  },
  esc: value => String(value ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;'),
  escAttr: value => String(value ?? '')
    .replace(/&/g, '&amp;')
    .replace(/"/g, '&quot;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;'),
  invoke: async () => [],
  showToast() {},
  showConfirm: async () => true,
  switchPanel() {},
  wrapPrimaryPanel: (_panel, html) => html,
};
context.window = context;
context.window.deeStorage = context.deeStorage;

vm.createContext(context);
pluginScripts.forEach(file => {
  const source = fs.readFileSync(path.join(__dirname, file), 'utf8');
  vm.runInContext(source, context, { filename: file });
});

vm.runInContext(`
  _pluginMarketplaceData = [
    {
      id: 'demo.plugin',
      name: 'Demo 插件',
      version: '1.2.3',
      description: '用于 smoke test',
      kind: 'tool',
      path: '/tmp/demo-plugin',
      source_label: '本地',
      permissions: ['network.http'],
      permission_details: [{ permission: 'network.http', risk: 'medium', description: '访问网络' }],
      compatibility: { compatible: true, tone: 'ok', label: '兼容' },
      features: [
        { id: 'search', kind: 'datasource', label: '搜索', description: '检索资料', methods: { search: 'search' } }
      ],
      dex_tools: [{ name: 'demo.search', description: '检索工具', level: 1 }]
    },
    {
      id: 'demo.template',
      name: 'Demo 模板',
      version: '0.1.0',
      kind: 'tool',
      template: true,
      compatibility: { compatible: true, tone: 'ok', label: '兼容' }
    }
  ];
  _pluginsData = [
    {
      id: 'demo.plugin',
      name: 'Demo 插件',
      version: '1.2.3',
      description: '用于详情渲染',
      kind: 'tool',
      state: 'stopped',
      enabled: true,
      permissions: ['network.http'],
      permission_details: [{ permission: 'network.http', risk: 'medium', description: '访问网络' }],
      source_path: '/tmp/demo-plugin',
      source_hash: 'abc123',
      assets: {
        total_bytes: 2048,
        data_bytes: 1024,
        cache_bytes: 512,
        secret_count: 1,
        account_count: 1,
        paths: { data_dir: '/tmp/data', cache_dir: '/tmp/cache', secrets_dir: '/tmp/secrets' }
      },
      account: { label: '连接' },
      accounts: [{ account_id: 'main', name: '主连接', status: 'disconnected' }],
      features: [
        { id: 'search', kind: 'datasource', label: '搜索', description: '检索资料', methods: { search: 'search' } }
      ],
      dex_tools: [{ name: 'demo.search', description: '检索工具', level: 1 }]
    },
    {
      id: 'deecodex-mobile-codex',
      name: 'Mobile Codex',
      version: '0.1.0',
      description: 'Mobile Codex 第一方 P2P 插件',
      kind: 'integration',
      state: 'running',
      enabled: true,
      first_party: true,
      permissions: ['account'],
      account: { enabled: true, label: 'Mobile Codex 账号' },
      accounts: []
    },
    {
      id: 'deecodex-desktop-sync',
      name: 'Mobile Codex 多机同步组件',
      version: '0.1.0',
      description: '内部同步组件',
      kind: 'integration',
      state: 'stopped',
      enabled: true,
      tags: ['internal-component'],
      features: [
        { id: 'desktop-sync', kind: 'automation', label: '多机同步', methods: { status: 'desktop_sync.status', peers: 'desktop_sync.peers', sync: 'desktop_sync.pull' } }
      ]
    }
  ];
`, context);

const panel = vm.runInContext('renderPluginsPanel()', context);
assert(panel.includes('插件市场'));
assert(panel.includes('wb-market-topnav'));
assert(panel.includes('Skill 专区'));
assert(panel.includes('wb-market-content'));
assert.strictEqual(vm.runInContext('_pluginDevOpen', context), false);

const dexPanel = vm.runInContext("_pluginZone = 'dex'; renderPluginsPanel()", context);
assert(dexPanel.includes('pluginList'));
assert(dexPanel.includes('plugin-catrow'));
vm.runInContext("_pluginZone = 'skill'", context);

const marketTools = vm.runInContext("_pluginPanelMode = 'market'; renderPluginMarketTools()", context);
assert(marketTools.includes('pluginMarketTools'));
assert(marketTools.includes('pluginZipPath'));
assert(marketTools.includes('pluginDevEntry'));
const installedTools = vm.runInContext("_pluginPanelMode = 'installed'; renderPluginMarketTools()", context);
assert(installedTools.includes('pluginMarketTools'));
assert(!installedTools.includes('pluginZipPath'));
assert(!installedTools.includes('pluginDevEntry'));
vm.runInContext("_pluginPanelMode = 'market'", context);
assert.strictEqual(vm.runInContext('pluginPanelData().length', context), 2);
assert(!vm.runInContext('pluginPanelData().some(plugin => plugin.id === "deecodex-desktop-sync")', context));

const marketCard = vm.runInContext('renderPluginMarketCard(_pluginMarketplaceData[0])', context);
assert(marketCard.includes('Demo 插件'));
assert(marketCard.includes('安装'));
assert(marketCard.includes('兼容'));

assert.strictEqual(vm.runInContext('pluginTemplateItems().length', context), 1);
const devBar = vm.runInContext('_pluginDevOpen = true; renderPluginDevBar()', context);
assert(devBar.includes('开发入口'));
assert(devBar.includes('Demo 模板'));

const installedCard = vm.runInContext('renderPluginCard(_pluginsData[0])', context);
assert(installedCard.includes('Demo 插件'));
assert(installedCard.includes('启动'));

const detail = vm.runInContext("_pluginDetailId = 'demo.plugin'; renderPluginDetail()", context);
assert(detail.includes('plugin-detail-shell'));
assert(detail.includes('运行事件'));
assert(detail.includes('资产'));
assert(detail.includes('DEX 工具'));
assert(detail.includes('连接'));

vm.runInContext(`
  _pluginDetailId = 'deecodex-mobile-codex';
  _mobileCodexState = {
    loading: false,
    loaded: true,
    user: { loggedIn: false },
    devices: [],
    service: { running: false },
    error: ''
  };
`, context);
const mobileLoggedOut = vm.runInContext('renderPluginDetail()', context);
assert(mobileLoggedOut.includes('Mobile Codex 账号'));
assert(mobileLoggedOut.includes('登录并启动 Companion'));
assert(mobileLoggedOut.includes('邀请码注册'));
assert(!mobileLoggedOut.includes('控制面地址'));
assert(!mobileLoggedOut.includes('https://api.liguan.me/mobile-codex'));
assert(mobileLoggedOut.includes('验证邮箱'));
assert.strictEqual(vm.runInContext(`
  mobileCodexRuntimeBecameAvailable(
    [{ id: 'deecodex-mobile-codex', state: 'stopped' }],
    [{ id: 'deecodex-mobile-codex', state: 'running' }],
    'deecodex-mobile-codex'
  )
`, context), true);
assert.strictEqual(vm.runInContext(`
  mobileCodexRuntimeBecameAvailable(
    [{ id: 'deecodex-mobile-codex', state: 'running' }],
    [{ id: 'deecodex-mobile-codex', state: 'running' }],
    'deecodex-mobile-codex'
  )
`, context), false);

vm.runInContext(`
  _mobileCodexState = {
    loading: false,
    loaded: true,
    user: {
      loggedIn: true,
      email: 'owner@example.test',
      deviceId: 'computer-1'
    },
    devices: [
      {
        id: 'computer-1',
        deviceType: 'computer',
        platform: 'macos',
        name: "Owner's Mac",
        companionVersion: '0.1.0',
        online: true,
        networkPath: 'direct'
      },
      {
        id: 'phone-1',
        deviceType: 'mobile',
        platform: 'ios',
        name: 'iPhone',
        appVersion: '0.1.0',
        online: true,
        networkPath: 'derp'
      }
    ],
    service: { running: true, network: { networkPath: 'direct' } },
    error: ''
  };
  _desktopSyncState = {
    loading: false,
    loaded: true,
    status: {
      ready: true,
      message: '可以同步',
      projectCount: 19,
      threadCount: 202
    },
    peers: [
      {
        deviceId: 'computer-2',
        name: '办公室 Mac mini',
        platform: 'macos',
        online: true,
        syncable: true,
        networkPath: 'derp',
        lastSeenAtMs: Date.now()
      },
      {
        deviceId: 'computer-old',
        name: '历史电脑',
        platform: 'macos',
        online: false,
        syncable: false,
        networkPath: 'offline',
        lastSeenAtMs: Date.now() - 86400000
      }
    ],
    syncingDeviceId: '',
    lastResult: {
      ok: true,
      message: '同步完成',
      summary: { applied: 2, unchanged: 200, conflicts: 1, skipped: 0, failedPeers: 0 }
    },
    error: ''
  };
`, context);
const mobileLoggedIn = vm.runInContext('renderPluginDetail()', context);
assert(mobileLoggedIn.includes('owner@example.test'));
assert(mobileLoggedIn.includes("Owner's Mac"));
assert(mobileLoggedIn.includes('Mobile Codex 连接正常'));
assert(mobileLoggedIn.includes('我的设备'));
assert(mobileLoggedIn.includes('重命名'));
assert(mobileLoggedIn.includes('撤销'));
assert(mobileLoggedIn.includes('高级设置与诊断'));
assert(mobileLoggedIn.includes('多机同步'));
assert(mobileLoggedIn.includes('办公室 Mac mini'));
assert(mobileLoggedIn.includes('公网中继'));
assert(mobileLoggedIn.includes('同步到本机'));
assert(mobileLoggedIn.includes('1 台离线或历史电脑已隐藏'));
assert(mobileLoggedIn.includes('新增 2'));
assert(!mobileLoggedIn.includes('tlsCertSha256'));
assert(!mobileLoggedIn.includes('tailnetAddresses'));
assert(!mobileLoggedIn.includes('<h3>能力</h3>'));
assert(!mobileLoggedIn.includes('<h3>简介</h3>'));
assert(mobileLoggedIn.includes("renameMobileCodexDevice('computer-1')"));
const internalSyncDetail = vm.runInContext(`
  _pluginDetailId = 'deecodex-desktop-sync';
  renderPluginDetail()
`, context);
assert.strictEqual(vm.runInContext('_pluginDetailId', context), 'deecodex-mobile-codex');
assert(internalSyncDetail.includes('多机同步'));
assert(internalSyncDetail.includes('办公室 Mac mini'));
assert(!internalSyncDetail.includes('<h3>能力</h3>'));
assert(!internalSyncDetail.includes('>peers<'));
assert(!internalSyncDetail.includes('>状态<'));
assert.strictEqual(vm.runInContext(`
  mobileCodexDeviceIsOnline({ online: true, networkPath: 'offline' })
`, context), true);
const onlineWaitingDevice = vm.runInContext(`
  renderMobileCodexDevice({
    id: 'computer-1',
    deviceType: 'computer',
    platform: 'macos',
    name: "Owner's Mac",
    companionVersion: '0.1.0',
    online: true,
    networkPath: 'offline'
  }, 'computer-1')
`, context);
assert(onlineWaitingDevice.includes('在线'));
assert(!onlineWaitingDevice.includes('离线'));
assert.strictEqual(vm.runInContext(`
  mobileCodexDeviceIsOnline({ online: false, networkPath: 'direct' })
`, context), false);
assert.strictEqual(vm.runInContext(`
  mobileCodexDeviceIsOnline({ online: true, networkPath: 'direct', revokedAt: '2026-07-23T00:00:00Z' })
`, context), false);
const renameModal = vm.runInContext(`renderMobileCodexRenameModal("Owner's Mac")`, context);
assert(renameModal.includes('重命名设备'));
assert(renameModal.includes('maxlength="80"'));
assert(renameModal.includes("Owner's Mac"));
assert.strictEqual(vm.runInContext(`
  mobileCodexConnectionStatus(
    { running: true, network: { networkPath: 'offline' } },
    ''
  ).title
`, context), 'Mobile Codex 已就绪');
assert.strictEqual(vm.runInContext(`
  mobileCodexConnectionStatus(
    { running: true, network: { networkPath: 'offline' } },
    ''
  ).tone
`, context), 'online');
assert(vm.runInContext(`
  mobileCodexConnectionStatus(
    { running: true, network: { networkPath: 'offline' } },
    ''
  ).description
`, context).includes('局域网连接可直接使用'));
assert(vm.runInContext(`
  mobileCodexConnectionStatus(
    { running: true, network: { networkPath: 'lan' } },
    ''
  ).description
`, context).includes('同一 Wi-Fi'));
assert.strictEqual(vm.runInContext(`desktopSyncPathLabel('lan')`, context), '局域网直连');

// WorkBuddy 插件中心授权门禁：未授权弹引导，已授权放行
let wbAuthModalHtml = '';
context.document.createElement = function () {
  let html = '';
  return {
    className: '',
    id: '',
    style: {},
    setAttribute() {},
    set innerHTML(value) {
      html = String(value);
      if (this.id === 'wbAuthRequiredModal') wbAuthModalHtml = html;
    },
    get innerHTML() { return html; },
    addEventListener() {},
    remove() {},
  };
};
vm.runInContext(`
  _wbEntitlementCache = { at: 0, known: false, authorized: false, error: '', planName: '', planCode: '', serviceStatus: '' };
  profileMasterState = { loading: false, loaded: true, loggedIn: true, error: '' };
  _mobileCodexState = {
    loading: false,
    loaded: true,
    user: { loggedIn: true, entitlement: { known: true, authorized: false, serviceStatus: 'suspended', planName: '月度套餐' } },
    devices: [],
    service: { running: true },
    error: ''
  };
  refreshMobileCodexEntitlement = async function () {
    return { loggedIn: true, entitlement: { known: true, authorized: false, serviceStatus: 'suspended', planName: '月度套餐' } };
  };
`, context);
(async () => {
  assert.strictEqual(await vm.runInContext('wbEnsureAuthorized()', context), false);
  assert(wbAuthModalHtml.includes('需要 DEX AI 授权'));
  assert(wbAuthModalHtml.includes('未生效或已过期'));
  assert(wbAuthModalHtml.includes('suspended'));
  // 主账号已退出：即使插件态仍有效，也必须拦截
  vm.runInContext(`
    _wbEntitlementCache = { at: 0, known: false, authorized: false, error: '', planName: '', planCode: '', serviceStatus: '' };
    profileMasterState = { loading: false, loaded: true, loggedIn: false, error: '' };
  `, context);
  assert.strictEqual(await vm.runInContext('wbEnsureAuthorized()', context), false);
  assert(wbAuthModalHtml.includes('需要 DEX AI 授权'));
  vm.runInContext(`
    _wbEntitlementCache = { at: 0, known: false, authorized: false, error: '', planName: '', planCode: '', serviceStatus: '' };
    profileMasterState = { loading: false, loaded: true, loggedIn: true, error: '' };
    refreshMobileCodexEntitlement = async function () {
      return { loggedIn: true, entitlement: { known: true, authorized: true, planName: '月度套餐' } };
    };
  `, context);
  assert.strictEqual(await vm.runInContext('wbEnsureAuthorized()', context), true);
})().catch(error => {
  console.error(error);
  process.exitCode = 1;
});

assert.strictEqual(typeof context.window.stopPluginAutoRefresh, 'function');
assert.strictEqual(typeof context.window.stopPluginEventRefresh, 'function');
assert.strictEqual(typeof context.window.clearPluginQrPolling, 'function');
assert.strictEqual(typeof context.window.showMobileCodexRenameModal, 'function');
assert(!fs.readFileSync(path.join(__dirname, 'plugins-detail.js'), 'utf8').includes('window.prompt'));

const cacheRefreshCalls = [];
let cacheRefreshPluginListCalls = 0;
context.invoke = async (command, args) => {
  cacheRefreshCalls.push({ command, args });
  if (command === 'list_plugins') {
    cacheRefreshPluginListCalls += 1;
    return [{
      id: 'deecodex-mobile-codex',
      name: 'Mobile Codex',
      version: '0.1.0',
      description: 'Mobile Codex 第一方 P2P 插件',
      kind: 'integration',
      state: 'running',
      enabled: true,
      first_party: true,
      permissions: ['account'],
      account: { enabled: true, label: 'Mobile Codex 账号' },
      accounts: []
    }];
  }
  if (command === 'list_plugin_marketplace' || command === 'list_plugin_events') return [];
  if (command === 'invoke_plugin_account_action') {
    if (args.action === 'current_user') {
      return { loggedIn: false };
    }
    if (args.action === 'status') return { running: false };
  }
  return {};
};
vm.runInContext(`
  _pluginDetailId = 'deecodex-mobile-codex';
  _pluginZone = 'dex';
  _pluginsData = [{
    id: 'deecodex-mobile-codex',
    name: 'Mobile Codex',
    version: '0.1.0',
    description: 'Mobile Codex 第一方 P2P 插件',
    kind: 'integration',
    state: 'stopped',
    enabled: true,
    first_party: true,
    permissions: ['account'],
    account: { enabled: true, label: 'Mobile Codex 账号' },
    accounts: []
  }];
  _mobileCodexState = {
    loading: false,
    loaded: true,
    user: {},
    devices: [],
    service: {},
    error: '旧的插件状态缓存错误'
  };
`, context);

(async () => {
  await vm.runInContext('loadPluginsData()', context);
  for (let i = 0; i < 10 && cacheRefreshPluginListCalls < 2; i += 1) {
    await new Promise(resolve => setImmediate(resolve));
  }
  assert(cacheRefreshPluginListCalls >= 2);
  assert(cacheRefreshCalls.some(call =>
    call.command === 'invoke_plugin_account_action' && call.args.action === 'current_user'
  ));
  assert.strictEqual(vm.runInContext('_mobileCodexState.loaded', context), true);
  assert.strictEqual(vm.runInContext('_mobileCodexState.error', context), '');
  console.log('plugins render smoke ok');
})().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
