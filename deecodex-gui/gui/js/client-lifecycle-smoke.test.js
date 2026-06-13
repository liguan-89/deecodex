const assert = require('assert');
const fs = require('fs');
const path = require('path');
const vm = require('vm');

const calls = [];
let serviceRunning = true;
const context = {
  console,
  setTimeout: (fn) => {
    fn();
    return 0;
  },
  window: { _clientLifecycleMap: {}, _statusData: { running: true }, CSS: { escape: value => String(value) } },
  document: {
    querySelector() { return null; },
    querySelectorAll() { return []; },
    getElementById() { return null; },
    createElement() { return { className: '', id: '', innerHTML: '', addEventListener() {} }; },
    body: { appendChild() {} },
  },
  currentPanel: 'status',
  currentConfig: { host: '127.0.0.1', port: 4446 },
  accountsData: {
    active_account_id: 'cx1',
    accounts: [
      { id: 'cx1', client_kind: 'codex', client_surface: 'cli', provider: 'openrouter', client_options: {} },
    ],
  },
  clientProfiles: [],
  providerPresets: [],
  endpointTemplates: [],
  deeStorage: {
    data: {},
    getItem(key) { return this.data[key] || ''; },
    setItem(key, value) { this.data[key] = String(value); },
    removeItem(key) { delete this.data[key]; },
  },
  esc: value => String(value ?? '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;'),
  escAttr: value => String(value ?? '').replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;'),
  invoke: async (name, args) => {
    calls.push({ name, args });
    if (name === 'dex_client_lifecycle_status') {
      const requiresCwd = args.kind === 'codex_cli' || args.kind === 'claude_cli';
      return {
        kind: args.kind,
        label: args.kind,
        account_kind: args.kind.startsWith('codex') ? 'codex' : (args.kind.startsWith('claude') ? 'claude_code' : args.kind),
        surface: 'cli',
        cli: true,
        installed: true,
        account_exists: true,
        account_configured: true,
        next_action: 'launch',
        launch: { mode: 'terminal', requires_cwd: requiresCwd },
        runtime: { running: false, instances: [] },
      };
    }
    if (name === 'get_service_status') return { running: serviceRunning, host: '127.0.0.1', port: 4446 };
    if (name === 'start_service') {
      serviceRunning = true;
      return { running: true, host: '127.0.0.1', port: 4446 };
    }
    if (name === 'dex_pick_client_launch_dir') return '/tmp/project';
    if (name === 'dex_launch_client') return { ok: true };
    return {};
  },
  showToast() {},
  showConfirm: async () => true,
  loadClientProfiles: async () => {},
  loadProviderPresets: async () => {},
  loadEndpointTemplates: async () => {},
  loadConfig: async () => {},
  loadStatus: async () => {},
  loadAccountsData: async () => {},
  clientIcon: kind => `<span class="client-logo-box">${kind}</span>`,
  accountClientKind: account => account.client_kind || 'codex',
  accountClientSurface: account => account.client_surface || 'cli',
  normalizeClientKind: kind => String(kind || 'codex'),
};

context.window.window = context.window;
context.window.document = context.document;
context.window.deeStorage = context.deeStorage;
context.window._statusData = context.window._statusData;

vm.createContext(context);
vm.runInContext(fs.readFileSync(path.join(__dirname, 'panels-core.js'), 'utf8'), context, { filename: 'panels-core.js' });
vm.runInContext(fs.readFileSync(path.join(__dirname, 'client-lifecycle.js'), 'utf8'), context, { filename: 'client-lifecycle.js' });

const dock = context.renderStatusClientDock(true);
assert(dock.includes('handleClientDockClick'));
assert(dock.includes('client-dock-state-dot install'));
assert(dock.includes('client-dock-state-dot account'));
assert(dock.includes('client-dock-state-dot runtime'));
assert(dock.includes('client-dock-bubble'));
assert(dock.includes('把任务交给我，我去仓库里跑一圈。'));
assert(dock.includes('把任务交给我，我去仓库里跑一圈。（桌面版）'));
assert(dock.includes('先读懂代码，再稳稳下手。'));
assert(dock.includes('先读懂代码，再稳稳下手。（桌面版）'));
assert(dock.includes('我不是只会聊天，我会真的做事。'));
assert(dock.includes('我会记住项目，也会越跑越顺。'));
assert(!dock.includes('title="Codex CLI"'));

const css = fs.readFileSync(path.join(__dirname, '..', 'css', 'app.css'), 'utf8');
assert(css.includes('.client-dock-state-row'));
assert(css.includes('visibility: hidden'));
assert(css.includes('.client-dock-item:hover .client-dock-state-row'));
assert(css.includes('visibility: visible'));
assert(css.includes('.client-dock-bubble'));
assert(css.includes('.client-dock-item:hover .client-dock-bubble'));
assert(css.includes('padding-top: 44px'));

context.window._clientLifecycleMap.codex_desktop = { runtime: { running: true } };
assert.strictEqual(context.statusClientProcessRunning('codex_desktop'), true);
assert.strictEqual(context.statusClientProcessRunning('codex_cli'), false);

context.window._clientLifecycleMap.codex_cli = { runtime: { running: false } };
assert.strictEqual(context.statusClientProcessRunning('codex_cli'), false);

context.window._clientLifecycleMap.codex_cli = { runtime: { running: true } };
assert.strictEqual(context.statusClientProcessRunning('codex_cli'), true);

context.window._clientLifecycleMap.claude_cli = { runtime: { running: true } };
context.window._clientLifecycleMap.hermes = { runtime: { running: true } };
assert.strictEqual(context.statusClientProcessRunning('codex_cli'), true);
assert.strictEqual(context.statusClientProcessRunning('claude_cli'), true);
assert.strictEqual(context.statusClientProcessRunning('hermes'), true);

context.window._clientLifecycleMap.hermes = { runtime: { running: false } };
assert.strictEqual(context.statusClientProcessRunning('hermes'), false);

(async () => {
  calls.length = 0;
  await context.refreshStatusClientDock();
  assert(calls.some(call => call.name === 'dex_client_lifecycle_status'));
  assert(!calls.some(call => call.name === 'dex_detect_processes'));

  calls.length = 0;
  await context.window.refreshClientLifecycleDock();
  assert(calls.some(call => call.name === 'dex_client_lifecycle_status'));
  assert.strictEqual(context.window._clientLifecycleMap.codex_cli.next_action, 'launch');

  calls.length = 0;
  await context.window.handleClientDockClick('openclaw');
  assert(!calls.some(call => call.name === 'dex_pick_client_launch_dir'));
  assert(!calls.some(call => call.name === 'get_service_status'));
  assert(!calls.some(call => call.name === 'start_service'));
  const openclawLaunch = calls.find(call => call.name === 'dex_launch_client');
  assert.strictEqual(openclawLaunch.args.kind, 'openclaw');
  assert.strictEqual(openclawLaunch.args.cwd, null);

  calls.length = 0;
  serviceRunning = false;
  await context.window.handleClientDockClick('codex_cli');
  const serviceStatusCall = calls.findIndex(call => call.name === 'get_service_status');
  const serviceStartCall = calls.findIndex(call => call.name === 'start_service');
  const pickDirCall = calls.findIndex(call => call.name === 'dex_pick_client_launch_dir');
  const launchCall = calls.findIndex(call => call.name === 'dex_launch_client');
  assert(serviceStatusCall >= 0);
  assert(serviceStartCall > serviceStatusCall);
  assert(launchCall > serviceStartCall);
  assert(calls.some(call => call.name === 'dex_pick_client_launch_dir'));
  const codexLaunch = calls.find(call => call.name === 'dex_launch_client');
  assert(pickDirCall < launchCall);
  assert.strictEqual(codexLaunch.args.cwd, '/tmp/project');
})().catch(error => {
  console.error(error);
  process.exit(1);
});
