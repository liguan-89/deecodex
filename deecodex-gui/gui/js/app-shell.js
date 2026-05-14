let currentPanel = 'status';
let currentConfig = {};

let accountsView = 'list';
let accountsData = { accounts: [], active_id: null };
let editingAccount = null;
let providerPresets = [];
let upstreamModels = [];

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
  const tauriEvent = window.__TAURI__?.event;
  if (tauriEvent?.listen) {
    (async () => {
      try {
        await tauriEvent.listen('account-switched', () => {
          if (currentPanel === 'status') loadStatus();
          if (currentPanel === 'accounts' && accountsView === 'list') loadAccountsData();
        });
        console.log('[deecodex] 已注册 account-switched 事件监听');
      } catch (e) {
        console.warn('[deecodex] 事件监听注册失败:', e);
      }
    })();
  } else {
    console.warn('[deecodex] __TAURI__.event 不可用，托盘切换后需手动刷新');
  }

  setInterval(async () => {
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
  currentPanel = panelId;
  document.querySelectorAll('.nav-item').forEach(el => {
    el.classList.toggle('active', el.dataset.panel === panelId);
  });
  if (panelId === 'accounts') accountsView = 'list';
  const saveBtn = document.getElementById('sidebarSaveBtn');
  saveBtn.style.display = (panelId === 'config') ? '' : 'none';
  // 清理侧边栏残留消息
  const sidebarMsg = document.getElementById('sidebarMsg');
  if (sidebarMsg) { sidebarMsg.textContent = ''; sidebarMsg.className = 'sidebar-status'; }
  renderPanel(panelId);
}

function goToConfig(sectionId) {
  switchPanel('config');
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      const el = document.getElementById('cfg-' + sectionId);
      if (el) el.scrollIntoView({ behavior: 'smooth', block: 'start' });
    });
  });
}

function renderPanel(panelId) {
  const container = document.getElementById('mainContent');
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
