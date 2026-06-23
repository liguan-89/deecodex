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
let transientScrollbarTimers = new WeakMap();
let transientScrollbarsReady = false;

function cleanupPanelEffects(panelId) {
  if (panelId === 'sessions') window.stopHistoryAutoRefresh?.();
  if (panelId === 'sessions') window.stopHistoryReconnectPolling?.();
  if (panelId === 'plugins') window.stopPluginAutoRefresh?.();
  if (panelId === 'plugins') window.stopPluginEventRefresh?.();
  if (panelId === 'plugins') window.clearPluginQrPolling?.();
  if (panelId === 'dex-assistant') window.dexDisposePanel?.();
}

// ═══════════════════════════════════════════════════════════════
// 初始化
// ═══════════════════════════════════════════════════════════════
async function init() {
  initTheme();
  initWindowDragging();
  initTransientScrollbars();
  loadNav();
  switchPanel('status');
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
  if (main) main.classList.toggle('primary-shell-main', panelId !== 'dex-assistant');
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

function wrapPrimaryPanel(panelId, html) {
  if (panelId === 'status' || panelId === 'dex-assistant') return html;
  const safePanelId = String(panelId || 'unknown').replace(/[^a-z0-9_-]/gi, '-');
  return `<section class="primary-page-shell primary-page-shell-${safePanelId}" data-primary-panel="${escAttr(safePanelId)}">${html}</section>`;
}

function renderPanel(panelId) {
  const container = document.getElementById('mainContent');
  if (container) container.classList.toggle('primary-shell-main', panelId !== 'dex-assistant');
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
    case 'plugins': container.innerHTML = wrapPrimaryPanel(panelId, renderPluginsPanel()); loadPluginsData(); break;
    case 'dex-assistant': container.innerHTML = renderDexAssistant(); break;
    case 'profile': container.innerHTML = wrapPrimaryPanel(panelId, renderProfile()); break;
    default: container.innerHTML = wrapPrimaryPanel(panelId, '<div class="empty-state">未知面板</div>');
  }
}

// ═══════════════════════════════════════════════════════════════
