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
    }
  ];
`, context);

const panel = vm.runInContext('renderPluginsPanel()', context);
assert(panel.includes('插件市场'));
assert(panel.includes('pluginDevEntry'));
assert(panel.includes('pluginList'));

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

assert.strictEqual(typeof context.window.stopPluginAutoRefresh, 'function');
assert.strictEqual(typeof context.window.stopPluginEventRefresh, 'function');
assert.strictEqual(typeof context.window.clearPluginQrPolling, 'function');

console.log('plugins render smoke ok');
