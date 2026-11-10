const assert = require('assert');
const fs = require('fs');
const path = require('path');
const vm = require('vm');

const calls = [];
let serviceRunning = true;
const createdElements = [];
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
    createElement() { const el = { className: '', id: '', innerHTML: '', style: {}, addEventListener() {} }; createdElements.push(el); return el; },
    body: { appendChild() {} },
    addEventListener() {},
  },
  currentPanel: 'status',
  currentConfig: { host: '127.0.0.1', port: 4446 },
  accountsData: {
    active_account_id: 'cx1',
    accounts: [
      { id: 'cx1', client_kind: 'codex', client_surface: 'cli', provider: 'opencode', client_options: {} },
    ],
  },
  clientProfiles: [],
  providerPresets: [
    { slug: 'deepseek', label: 'DeepSeek', default_upstream: 'https://api.deepseek.com/v1', known_models: ['deepseek-v4-pro[1m]'] },
    { slug: 'anthropic', label: 'Anthropic', default_upstream: 'https://api.anthropic.com', known_models: [] },
    { slug: 'codex', label: 'Codex 官方', default_upstream: '', known_models: [] },
    { slug: 'google-ai', label: 'Google AI', default_upstream: '', known_models: [] },
    { slug: 'minimax', label: 'MiniMax', default_upstream: 'https://api.minimaxi.com/v1', known_models: ['MiniMax-M3[1m]'] },
  ],
  endpointTemplates: [],
  providersForClientKind: () => context.providerPresets.filter(p => !['codex', 'anthropic', 'google-ai'].includes(p.slug)),
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
    if (name === 'get_config') return { codex_router_mode: context.currentConfig?.codex_router_mode || 'api' };
    if (name === 'launch_codex_cdp') return { ok: true };
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
  providerLogoSrc: slug => `assets/provider-logos/${slug}.png`,
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
vm.runInContext(fs.readFileSync(path.join(__dirname, 'dock-tips.js'), 'utf8'), context, { filename: 'dock-tips.js' });
vm.runInContext(fs.readFileSync(path.join(__dirname, 'client-lifecycle.js'), 'utf8'), context, { filename: 'client-lifecycle.js' });

const dock = context.renderStatusClientDock(true);
assert(dock.includes('handleClientDockClick'));
assert(dock.includes('oncontextmenu="handleClientDockContextMenu'));
assert(dock.includes('client-dock-item'));
assert(dock.includes('client-dock-runtime'));
assert(dock.includes('客户端启动'));
assert(dock.includes('client-dock-state-row'));
assert(dock.includes('client-dock-state-dot install'));
assert(dock.includes('client-dock-state-dot account'));
assert(dock.includes('client-dock-state-dot runtime'));
assert.strictEqual((dock.match(/client-dock-state-row/g) || []).length, 12, '11 个客户端 + 插件中心占位 = 12 组状态点');
assert.strictEqual((dock.match(/client-dock-state-dot/g) || []).length, 36, '12 组 × 3 个状态点 = 36');
const pluginCenterBlock = dock.slice(dock.indexOf('client-dock-plugin-center'));
assert(pluginCenterBlock.includes('client-dock-state-spacer'), '插件中心应含不可见占位点，保证与第二排图标对齐');
assert(!pluginCenterBlock.includes('client-dock-state-dot install'), '插件中心占位点不应参与安装/账号/运行状态');
assert(!dock.includes('客户端接入'));

context.window.openDockTipsPanel();
const tipsPanel = createdElements.find(el => el.id === 'dockTipsPanel');
assert(tipsPanel, '「?」功能面板应被创建');
assert(tipsPanel.innerHTML.includes('客户端图标能做什么'));
assert(tipsPanel.innerHTML.includes('右键图标'));
assert(tipsPanel.innerHTML.includes('下载 / 启动 / 停止对应客户端'), '「?」面板应说明左键可下载/启动/停止');
assert(tipsPanel.innerHTML.includes('清理配置文件'), '「?」面板应说明右键可清理配置文件');
assert(tipsPanel.innerHTML.includes('DeepSeek Harness'), '功能面板应列出 dsh=DeepSeek Harness');
assert(tipsPanel.innerHTML.includes('Codex 桌面'), '功能面板应列出 Codex 桌面');
context.window.dismissDockFirstRunTips();
assert.strictEqual(context.window.renderDockFirstRunCard(), '', '点击「知道了」后首次引导卡不再出现');
assert(!dock.includes('client-dock-quick-action'));
assert(dock.includes('Codex 桌面'));
assert(dock.includes('Claude 桌面'));
assert(dock.includes('Hermes 桌面'));
assert(dock.includes('DeepSeek Harness'));
assert(dock.includes('Codex CLI'));
assert(dock.includes('Claude CLI'));
assert(dock.includes('client-dock-tips-hint'));
assert(dock.includes('左键启动/下载'), '常驻提示应同时说明左键可下载/启动');
assert(dock.includes('openDockTipsPanel()'));
assert(dock.includes('client-dock-first-tips'));
assert(dock.includes('client-dock-bubble'));
assert(dock.includes('client-dock-plugin-center'));
assert(dock.includes("onclick=\"switchPanel('plugins')\""));
assert(dock.includes('插件中心'));
assert(!dock.includes('title="Codex CLI"'));

const indexHtml = fs.readFileSync(path.join(__dirname, '..', 'index.html'), 'utf8');
const cssFiles = Array.from(indexHtml.matchAll(/<link rel="stylesheet" href="css\/([^"?]+)/g)).map(m => m[1]);
const css = cssFiles
  .map(f => fs.readFileSync(path.join(__dirname, '..', 'css', f), 'utf8'))
  .join('\n');
assert(css.includes('.client-dock-state-row'));
assert(css.includes('visibility: hidden'));
assert(css.includes('.client-dock-item:hover .client-dock-state-row'));
assert(css.includes('visibility: visible'));
assert(css.includes('.client-dock-bubble'));
assert(css.includes('.client-dock-item:hover .client-dock-bubble'));
assert(css.includes('padding-top: 44px'));
assert(css.includes('.client-dock-plugin-center'));
assert(css.includes('.plugin-tile-bg'));
assert(css.includes('[data-theme="light"] .status-overview-shell .client-dock-plugin-center'));
assert(css.includes('.client-dock-plugin-center .client-dock-state-row'), '插件中心占位点应有永久隐藏规则');
assert(css.includes('opacity: 0 !important'), '插件中心占位点应强制不显示');

// 强制关闭成功判定：优雅退出成功（ok:true、killed 为空）不得误报为失败。
const panelsCoreSource = fs.readFileSync(path.join(__dirname, 'panels-core.js'), 'utf8');
assert(panelsCoreSource.includes('result?.ok === true'), 'forceQuitClient 应把优雅退出成功（ok:true）视为成功');
assert(panelsCoreSource.includes('${item.label} 已退出`'), 'graceful 成功应显示「已退出」而非「强制退出失败」');
assert(panelsCoreSource.includes('data-action="clean-config"'), '右键菜单应含「清理配置文件」项');
assert(panelsCoreSource.includes('cleanClientConfig(normalized, item)'), '「清理配置文件」应绑定处理函数');
assert(panelsCoreSource.includes("invoke('dex_clean_client_config_target'"), '清理前应查询只读目标');
assert(panelsCoreSource.includes("invoke('dex_clean_client_config'"), '清理应调用后端专属路径删除命令');
assert(panelsCoreSource.includes('showConfirmWithCheckbox'), '清理确认应支持可选勾选移除账号');
assert(panelsCoreSource.includes('remove_account: removeAccount'), '清理应把是否移除账号传给后端');

const uiCoreSource = fs.readFileSync(path.join(__dirname, 'ui-core.js'), 'utf8');
assert(uiCoreSource.includes('function showConfirmWithCheckbox'), 'ui-core 应提供带勾选框的确认函数');
assert(uiCoreSource.includes('window.showConfirmWithCheckbox'), 'showConfirmWithCheckbox 应挂到 window');

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

// 不要求账号的客户端（dsh 等）刷新后应显示「无需账号」，而不是误显示「需账号」。
context.window._clientLifecycleMap.dsh = { installed: true, account_exists: false, requires_account: false };
const dshMeta = context.statusClientLifecycleMeta('dsh', context.statusClientInfo('dsh', true));
assert.strictEqual(dshMeta.accountLabel, '无需账号');
assert(dshMeta.classes.includes('has-account'));
assert(dshMeta.classes.includes('config-ready'));

// pi 原生检测：已配置 → 显示「已配置」（读 ~/.pi/agent 配置）。
context.window._clientLifecycleMap.pi = { installed: true, requires_account: false, native_detected: true, native_configured: true };
const piConfiguredMeta = context.statusClientLifecycleMeta('pi', context.statusClientInfo('pi', true));
assert.strictEqual(piConfiguredMeta.accountLabel, '已配置');
assert(piConfiguredMeta.classes.includes('has-account'));
assert(piConfiguredMeta.classes.includes('config-ready'));

// pi 原生检测：未配置 → 显示「未配置」，不能误显示成已就绪。
context.window._clientLifecycleMap.pi = { installed: true, requires_account: false, native_detected: true, native_configured: false };
const piUnconfiguredMeta = context.statusClientLifecycleMeta('pi', context.statusClientInfo('pi', true));
assert.strictEqual(piUnconfiguredMeta.accountLabel, '未配置');
assert(piUnconfiguredMeta.classes.includes('needs-account'));
assert(piUnconfiguredMeta.classes.includes('config-pending'));

// 没有原生检测的免账号客户端（opencode 等）保持「无需账号」。
context.window._clientLifecycleMap.opencode = { installed: true, requires_account: false };
const opencodeMeta = context.statusClientLifecycleMeta('opencode', context.statusClientInfo('opencode', true));
assert.strictEqual(opencodeMeta.accountLabel, '无需账号');

// hermes_desktop 复用 hermes 账号：后端判定 requires_account=true + 账号存在 → 已配置。
context.window._clientLifecycleMap.hermes_desktop = { installed: true, account_exists: true, account_configured: true, requires_account: true };
const hermesDesktopMeta = context.statusClientLifecycleMeta('hermes_desktop', context.statusClientInfo('hermes_desktop', true));
assert.strictEqual(hermesDesktopMeta.accountLabel, '已配置');
assert(hermesDesktopMeta.classes.includes('has-account'));
assert(hermesDesktopMeta.classes.includes('config-ready'));

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
  await context.window.handleClientDockClick('hermes');
  assert(!calls.some(call => call.name === 'dex_pick_client_launch_dir'));
  assert(!calls.some(call => call.name === 'get_service_status'));
  assert(!calls.some(call => call.name === 'start_service'));
  const hermesLaunch = calls.find(call => call.name === 'dex_launch_client');
  assert.strictEqual(hermesLaunch.args.kind, 'hermes');
  assert.strictEqual(hermesLaunch.args.cwd, null);

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

  // Codex 桌面 + API 模式：点击应走 launch_codex_cdp（带 JS 注入）
  calls.length = 0;
  context.currentConfig.codex_router_mode = 'api';
  await context.window.handleClientDockClick('codex_desktop');
  assert(calls.some(call => call.name === 'launch_codex_cdp'));
  assert(!calls.some(call => call.name === 'dex_launch_client'));

  // Codex 桌面 + 智能路由模式：点击应走原 dex_launch_client（不注入）
  calls.length = 0;
  context.currentConfig.codex_router_mode = 'smart';
  await context.window.handleClientDockClick('codex_desktop');
  assert(calls.some(call => call.name === 'dex_launch_client'));
  assert(!calls.some(call => call.name === 'launch_codex_cdp'));

  // Codex 快捷配置：默认 DeepSeek 模板，供应商列表去掉 Codex 官方 / Anthropic / Google AI。
  createdElements.length = 0;
  await context.window.openClientQuickConfigModal('codex_cli');
  const quickOverlay = createdElements.find(el => el.id === 'clientQuickConfigModal');
  assert(quickOverlay, 'Codex 快捷配置弹窗应被创建');
  const quickHtml = quickOverlay.innerHTML;
  assert(quickHtml.includes('value="deepseek" selected'), '默认供应商应为 DeepSeek');
  assert(quickHtml.includes('value="deepseek"'));
  assert(quickHtml.includes('value="minimax"'));
  assert(!quickHtml.includes('value="codex"'), '不应包含 Codex 官方');
  assert(!quickHtml.includes('value="anthropic"'), '不应包含 Anthropic');
  assert(!quickHtml.includes('value="google-ai"'), '不应包含 Google AI');
  assert(quickHtml.includes('clientQuickModelWrap'), '默认模型应为下拉控件容器');
  assert(quickHtml.includes('clientQuickFetchModelsBtn'), '应有「获取上游模型」按钮');
  assert(quickHtml.includes('clientQuickKeyBtn'), '应有「获取 API Key」按钮');
  assert(quickHtml.includes('value="deepseek-v4-pro[1m]"'), '默认模型下拉应含 DeepSeek 预设模型');

  // 所有客户端的快捷配置都默认 DeepSeek 模板，且供应商列表一致（去掉那三家）。
  for (const kind of ['hermes', 'claude_cli', 'dsh', 'opencode', 'workbuddy']) {
    createdElements.length = 0;
    await context.window.openClientQuickConfigModal(kind);
    const overlay = createdElements.find(el => el.id === 'clientQuickConfigModal');
    assert(overlay, `${kind} 快捷配置弹窗应被创建`);
    const html = overlay.innerHTML;
    assert(html.includes('value="deepseek" selected'), `${kind} 默认供应商应为 DeepSeek`);
    assert(!html.includes('value="codex"'), `${kind} 不应包含 Codex 官方`);
    assert(!html.includes('value="anthropic"'), `${kind} 不应包含 Anthropic`);
    assert(!html.includes('value="google-ai"'), `${kind} 不应包含 Google AI`);
  }
})().catch(error => {
  console.error(error);
  process.exit(1);
});
