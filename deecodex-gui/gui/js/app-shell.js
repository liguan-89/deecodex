let currentPanel = 'status';
let currentConfig = null;

let accountsView = 'list';
let accountsData = { accounts: [], active_id: null };
let editingAccount = null;
let providerPresets = [];
let clientProfiles = [];
const savedAccountsKind = deeStorage.getItem('deecodex.accountsClientKind');
const savedAccountsSurface = deeStorage.getItem('deecodex.accountsClientSurface');
let selectedClientKind = ['codex', 'claude_code', 'hermes', 'generic_client', 'dsh', 'pi', 'opencode', 'workbuddy'].includes(savedAccountsKind) ? savedAccountsKind : 'codex';
var selectedClientSurface = (savedAccountsSurface === 'cli' || savedAccountsSurface === 'desktop') ? savedAccountsSurface : 'desktop';
const CONFIG_CLIENT_STORAGE_KEY = 'deecodex.configClientKind';
let selectedConfigClientKind = deeStorage.getItem(CONFIG_CLIENT_STORAGE_KEY) || 'global';
let endpointTemplates = [];
let upstreamModels = [];
let appRefreshTimer = null;
let transientScrollbarTimers = new WeakMap();
let transientScrollbarsReady = false;

function cleanupPanelEffects(panelId) {
  if (panelId === 'sessions') window.stopHistoryAutoRefresh?.();
  if (panelId === 'sessions') window.stopHistoryReconnectPolling?.();
  if (panelId === 'plugins') window.stopPluginAutoRefresh?.();
  if (panelId === 'plugins') window.stopPluginEventRefresh?.();
  if (panelId === 'plugins') window.clearPluginQrPolling?.();
  if (panelId === 'dex-assistant') window.dexDisposePanel?.();
  if (panelId === 'profile') window.profileLoginTeardown?.();
}

// ═══════════════════════════════════════════════════════════════
// 初始化
// ═══════════════════════════════════════════════════════════════
async function init() {
  initTheme();
  initWindowDragging();
  initWindowControls();
  initTransientScrollbars();
  if (window.DexTelemetry?.init) window.DexTelemetry.init();
  loadNav();
  const startupPanel = deeStorage.getItem('deecodex.startupPanel') || 'status';
  switchPanel(startupPanel);
  normalizeThreadsOnStartup();
  await Promise.all([loadStatus(), loadConfig(), loadAccountsData()]);
  if (currentPanel === 'status') renderPanel('status');
  if (typeof checkSetupWizard === 'function') checkSetupWizard();

  // 启动后延迟自动检查更新，避免抢占首屏初始化。
  window.setTimeout(() => autoCheckUpgrade(), 10000);

  // 监听托盘账号切换事件，自动刷新
  if (typeof window.DeeCodexTauri?.listen === 'function') {
    (async () => {
      try {
        await window.DeeCodexTauri.listen('account-switched', () => {
          if (currentPanel === 'status') loadStatus();
          if (currentPanel === 'accounts' && accountsView === 'list') loadAccountsData();
        });
        await window.DeeCodexTauri.listen('show-update', () => {
          if (typeof showStoredUpdatePrompt === 'function') showStoredUpdatePrompt('tray');
        });
      } catch (e) {
        console.warn('[deecodex] 事件监听注册失败:', e);
      }
    })();
  } else {
    console.warn('[deecodex] Tauri 事件不可用，托盘切换后需手动刷新');
  }

  if (appRefreshTimer) clearInterval(appRefreshTimer);
  appRefreshTimer = setInterval(async () => {
    if (currentPanel === 'status') await loadStatus();
  }, 10000);
}

async function normalizeThreadsOnStartup() {
  try {
    const diff = await invoke('normalize_threads');
    const changed = Number(diff?.changed_count || 0);
    const rolloutFixed = Number(diff?.rollout_metadata_fixed_count || 0);
    const remaining = Number(diff?.remaining_non_unified_count || 0);
    const desktopFixed = Number(diff?.desktop_project_fixed_count || 0);
    const desktopPending = Number(diff?.desktop_project_pending_count || 0);
    if (changed || rolloutFixed || remaining || desktopFixed || desktopPending) {
      console.info('[deecodex] Codex Desktop 线程已自动归一', diff);
      if (currentPanel === 'threads') await refreshThreads();
    }
    return diff;
  } catch (error) {
    console.warn('[deecodex] Codex Desktop 线程自动归一失败:', error);
    return null;
  }
}

function isWindowDragBlocked(target) {
  if (!(target instanceof HTMLElement)) return false;
  return Boolean(target.closest('button, a, input, select, textarea, label, summary, [role="button"], [role="link"], [contenteditable="true"]'));
}

function initWindowDragging() {
  document.querySelectorAll('[data-window-drag-region]').forEach((region) => {
    if (region.dataset.windowDragReady === 'true') return;
    region.dataset.windowDragReady = 'true';
    region.addEventListener('mousedown', (event) => {
      if (event.button !== 0 || event.detail !== 1 || isWindowDragBlocked(event.target)) return;
      event.preventDefault();
      event.stopPropagation();
      if (typeof event.stopImmediatePropagation === 'function') event.stopImmediatePropagation();
      window.DeeCodexTauri?.startWindowDrag?.().catch((error) => {
        console.warn('[deecodex] 窗口拖动失败:', error);
      });
    }, true);
  });
}

// Windows frameless：自定义窗口控制按钮（最小化/最大化/关闭到托盘）。
// 按钮大小随窗口尺寸/分辨率自适应（WebView 随系统 DPI 自动缩放，这里用 vmin 随窗口放大），
// 最大化时图标切换为“还原”。
function initWindowControls() {
  // 仅 Windows frameless 需要自定义窗口控制；macOS 用系统红绿灯（按钮由 CSS 隐藏），直接跳过。
  if (document.documentElement.dataset.platform !== 'windows') return;

  // SVG 图标：最大化=单框，还原=双框（统一样式与尺寸，不随字号波动）
  const MAX_ICON = '<svg class="wc-icon" viewBox="0 0 12 12"><rect x="2.5" y="2.5" width="7" height="7" fill="none"/></svg>';
  const RESTORE_ICON = '<svg class="wc-icon" viewBox="0 0 12 12"><rect x="1.5" y="1.5" width="7" height="7" fill="none"/><rect x="3.5" y="3.5" width="7" height="7" fill="none" opacity="0.95"/></svg>';

  function syncMaximizeGlyph() {
    const btn = document.getElementById('windowMaximizeBtn');
    if (!btn) return;
    window.DeeCodexTauri?.isWindowMaximized?.().then((maximized) => {
      btn.innerHTML = maximized ? RESTORE_ICON : MAX_ICON;
      btn.title = maximized ? '还原' : '最大化';
      btn.setAttribute('aria-label', maximized ? '还原' : '最大化');
    }).catch(() => {});
  }

  const actions = [
    ['windowMinimizeBtn', () => window.DeeCodexTauri?.minimizeWindow?.()],
    ['windowMaximizeBtn', () => window.DeeCodexTauri?.toggleMaximizeWindow?.()],
    ['windowCloseBtn', () => window.DeeCodexTauri?.hideWindow?.()]
  ];
  actions.forEach(([id, action]) => {
    const button = document.getElementById(id);
    if (!button || button.dataset.windowControlReady === 'true') return;
    button.dataset.windowControlReady = 'true';
    button.addEventListener('click', () => {
      const result = action();
      setTimeout(syncMaximizeGlyph, 250); // 最大化动画后刷新图标
      if (result && typeof result.catch === 'function') {
        result.catch((error) => console.warn('[deecodex] 窗口控制失败:', error));
      }
    });
  });
  const unlistenPromise = window.DeeCodexTauri?.onWindowResized?.(() => syncMaximizeGlyph());
  if (unlistenPromise && typeof unlistenPromise.catch === 'function') {
    unlistenPromise.catch(() => {});
  }
  window.addEventListener('focus', syncMaximizeGlyph);
  syncMaximizeGlyph();
}

function initTransientScrollbars() {
  if (transientScrollbarsReady) return;
  transientScrollbarsReady = true;
  document.addEventListener('scroll', (event) => {
    const target = event.target === document ? document.scrollingElement : event.target;
    if (!target || !(target instanceof HTMLElement)) return;
    target.classList.add('is-scrolling');
    const oldTimer = transientScrollbarTimers.get(target);
    if (oldTimer) clearTimeout(oldTimer);
    transientScrollbarTimers.set(target, setTimeout(() => {
      target.classList.remove('is-scrolling');
      transientScrollbarTimers.delete(target);
    }, 760));
  }, true);
}

// ═══════════════════════════════════════════════════════════════
// 导航
// ═══════════════════════════════════════════════════════════════
function loadNav() {
  const nav = document.getElementById('sidebarNav');
  if (!nav) return;
  nav.innerHTML = '';
  var fragments = window._navFragments || [];
  for (var i = 0; i < fragments.length; i++) {
    nav.insertAdjacentHTML('beforeend', fragments[i]);
  }
}

function switchPanel(panelId) {
  const previousPanel = currentPanel;
  if (previousPanel && previousPanel !== panelId) cleanupPanelEffects(previousPanel);
  currentPanel = panelId;
  const main = document.getElementById('mainContent');
  if (main) main.classList.toggle('primary-shell-main', panelId !== 'dex-assistant' && panelId !== 'profile');
  if (main) main.classList.toggle('status-main', panelId === 'status');
  if (main) main.classList.toggle('dex-main', panelId === 'dex-assistant');
  if (main) main.classList.toggle('accounts-main', panelId === 'accounts' && accountsView === 'list');
  if (main) main.classList.toggle('accounts-form-main', panelId === 'accounts' && accountsView !== 'list');
  document.querySelectorAll('.nav-item').forEach(el => {
    el.classList.toggle('active', el.dataset.panel === panelId);
  });
  if (panelId === 'accounts') accountsView = 'list';
  const saveBtn = document.getElementById('sidebarSaveBtn');
  saveBtn.style.display = (panelId === 'config' && (typeof configCanEditSelected !== 'function' || configCanEditSelected())) ? '' : 'none';
  // 清理侧边栏残留消息
  const sidebarMsg = document.getElementById('sidebarMsg');
  if (sidebarMsg) { sidebarMsg.textContent = ''; sidebarMsg.className = 'sidebar-status'; }
  renderPanel(panelId);
  window.DexTelemetry?.trackPageView?.(panelId, { previous_page: previousPanel || '' });
}

function wrapPrimaryPanel(panelId, html) {
  if (panelId === 'status' || panelId === 'dex-assistant') return html;
  const safePanelId = String(panelId || 'unknown').replace(/[^a-z0-9_-]/gi, '-');
  return `<section class="primary-page-shell primary-page-shell-${safePanelId}" data-primary-panel="${escAttr(safePanelId)}">${html}</section>`;
}

function renderPanel(panelId) {
  const container = document.getElementById('mainContent');
  if (container) container.classList.toggle('primary-shell-main', panelId !== 'dex-assistant' && panelId !== 'profile');
  if (container) container.classList.toggle('status-main', panelId === 'status');
  if (container) container.classList.toggle('dex-main', panelId === 'dex-assistant');
  if (container) container.classList.toggle('accounts-main', panelId === 'accounts' && accountsView === 'list');
  if (container) container.classList.toggle('accounts-form-main', panelId === 'accounts' && accountsView !== 'list');
  switch (panelId) {
    case 'status': container.innerHTML = renderStatus(); break;
    case 'config':
      container.innerHTML = wrapPrimaryPanel(panelId, renderConfig());
      if (typeof afterRenderConfigPanel === 'function') afterRenderConfigPanel();
      break;
    case 'diagnostics': container.innerHTML = wrapPrimaryPanel(panelId, renderDiagnostics()); break;
    case 'help': container.innerHTML = wrapPrimaryPanel(panelId, renderHelp()); break;
    case 'sessions': container.innerHTML = wrapPrimaryPanel(panelId, renderHistory()); refreshHistory(); break;
    case 'threads': container.innerHTML = wrapPrimaryPanel(panelId, renderThreads()); refreshThreads(); break;
    case 'accounts': container.innerHTML = wrapPrimaryPanel(panelId, renderAccountsPanel()); loadAccountsData(); break;
    case 'plugins':
      container.innerHTML = wrapPrimaryPanel(panelId, renderPluginsPanel());
      // 按当前专区加载对应数据：skill/mcp/agents 走 WorkBuddy 资源，dex 走插件市场数据。
      // 只调 loadPluginsData 时默认 skill 专区会是 no-op，导致"正在加载…"永不填充。
      syncWbSectionForZone(_pluginZone);
      if (_pluginZone === 'dex') {
        loadPluginsData();
      } else {
        loadWbResources();
      }
      break;
    case 'dex-assistant': container.innerHTML = renderDexAssistant(); break;
    case 'profile':
      {
        // 登录页与管理界面都按全屏面板布局：不包 primary-page-shell，
        // 用 .main.profile-login-main / .profile-manage-main 收紧留白。
        const masterLoaded = Boolean(profileMasterState && profileMasterState.loaded);
        const isLoginView = masterLoaded && !profileMasterState.loggedIn;
        const isManageView = masterLoaded && Boolean(profileMasterState.loggedIn);
        container.innerHTML = renderProfile();
        // 登录视图保留 primary-shell-main（登录页原有布局依赖它）；管理视图去掉
        // 以便使用自己的全屏滚动布局。
        if (container) container.classList.toggle('primary-shell-main', isLoginView);
        if (container) container.classList.toggle('profile-login-main', isLoginView);
        if (container) container.classList.toggle('profile-manage-main', isManageView);
        if (typeof hydrateProfileMaster === 'function') hydrateProfileMaster();
        if (isLoginView) {
          if (typeof profileLoginHydrate === 'function') profileLoginHydrate();
        } else if (isManageView) {
          // 已登录：Mobile Codex 兼容层与匿名统计在这一层才加载。
          if (typeof hydrateProfileTelemetry === 'function') hydrateProfileTelemetry();
          if (typeof hydrateProfile === 'function') hydrateProfile();
        }
      }
      break;
    default: container.innerHTML = wrapPrimaryPanel(panelId, '<div class="empty-state">未知面板</div>');
  }
}

// ═══════════════════════════════════════════════════════════════
