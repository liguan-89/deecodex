let currentPanel = 'status';
let currentConfig = null;

let accountsView = 'list';
let accountsData = { accounts: [], active_id: null };
let editingAccount = null;
let providerPresets = [];
let clientProfiles = [];
let selectedClientKind = 'codex';
const CONFIG_CLIENT_STORAGE_KEY = 'deecodex.configClientKind';
let selectedConfigClientKind = deeStorage.getItem(CONFIG_CLIENT_STORAGE_KEY) || 'global';
let endpointTemplates = [];
let upstreamModels = [];
let appRefreshTimer = null;

function cleanupPanelEffects(panelId) {
  if (panelId === 'sessions') window.stopHistoryAutoRefresh?.();
  if (panelId === 'sessions') window.stopHistoryReconnectPolling?.();
  if (panelId === 'plugins') window.stopPluginAutoRefresh?.();
  if (panelId === 'plugins') window.clearPluginQrPolling?.();
  if (panelId === 'dex-assistant') window.dexDisposePanel?.();
}

// ═══════════════════════════════════════════════════════════════
// 初始化
// ═══════════════════════════════════════════════════════════════
async function init() {
  initTheme();
  loadNav();
  switchPanel('status');
  await Promise.all([loadStatus(), loadConfig(), loadAccountsData()]);
  if (currentPanel === 'status') renderPanel('status');

  // 每日自动检查更新
  autoCheckUpgrade();

  // 首次安装 / 更新后配置引导（异步，不阻塞）
  checkSetupWizard();

  // 监听托盘账号切换事件，自动刷新
  if (typeof window.DeeCodexTauri?.listen === 'function') {
    (async () => {
      try {
        await window.DeeCodexTauri.listen('account-switched', () => {
          if (currentPanel === 'status') loadStatus();
          if (currentPanel === 'accounts' && accountsView === 'list') loadAccountsData();
        });
        console.log('[deecodex] 已注册 account-switched 事件监听');
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
    else if (currentPanel === 'accounts' && accountsView === 'list') await loadAccountsData();
  }, 10000);
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
  if (main) main.classList.toggle('dex-main', panelId === 'dex-assistant');
  if (main) main.classList.toggle('accounts-main', panelId === 'accounts' && accountsView === 'list');
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
}

function goToConfig(sectionId) {
  if (typeof configKindForSection === 'function') {
    selectedConfigClientKind = configKindForSection(sectionId);
    deeStorage.setItem(CONFIG_CLIENT_STORAGE_KEY, selectedConfigClientKind);
  }
  switchPanel('config');
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      const el = document.getElementById('cfg-' + sectionId);
      if (el) {
        if (el.tagName === 'DETAILS') el.open = true;
        el.scrollIntoView({ behavior: 'smooth', block: 'start' });
      }
    });
  });
}

function renderPanel(panelId) {
  const container = document.getElementById('mainContent');
  if (container) container.classList.toggle('dex-main', panelId === 'dex-assistant');
  if (container) container.classList.toggle('accounts-main', panelId === 'accounts' && accountsView === 'list');
  switch (panelId) {
    case 'status': container.innerHTML = renderStatus(); break;
    case 'config': container.innerHTML = renderConfig(); break;
    case 'diagnostics': container.innerHTML = renderDiagnostics(); break;
    case 'help': container.innerHTML = renderHelp(); break;
    case 'sessions': container.innerHTML = renderHistory(); refreshHistory(); break;
    case 'threads': container.innerHTML = renderThreads(); refreshThreads(); break;
    case 'accounts': container.innerHTML = renderAccountsPanel(); loadAccountsData(); break;
    case 'plugins': container.innerHTML = renderPluginsPanel(); loadPluginsData(); break;
    case 'dex-assistant': container.innerHTML = renderDexAssistant(); break;
    case 'profile': container.innerHTML = renderProfile(); break;
    default: container.innerHTML = '<div class="empty-state">未知面板</div>';
  }
}

// ═══════════════════════════════════════════════════════════════
