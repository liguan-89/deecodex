// 客户端启动图标功能提示（dock-tips）
// 覆盖：常驻提示条 + 「?」功能面板 + 首次引导卡 + hover 气泡
// 持久化键 deecodex.dockTipsSeen：首次引导只出现一次，可随时用「?」再看。
(function () {
  const DOCK_TIPS_SEEN_KEY = 'deecodex.dockTipsSeen';

  function tipsSeen() {
    return typeof deeStorage !== 'undefined' && deeStorage?.getItem?.(DOCK_TIPS_SEEN_KEY) === '1';
  }

  function markTipsSeen() {
    if (typeof deeStorage !== 'undefined') deeStorage?.setItem?.(DOCK_TIPS_SEEN_KEY, '1');
  }

  function escText(value) {
    return typeof esc === 'function' ? esc(value) : String(value ?? '');
  }

  // 常驻提示条 + 「?」入口（渲染在「客户端启动」标题右侧）
  function renderDockTipsEntry() {
    return '' +
      '<span class="client-dock-tips-hint">左键启动/下载 · 右键更多操作 · 悬停看状态</span>' +
      '<button type="button" class="client-dock-tips-btn" onclick="openDockTipsPanel()" title="查看客户端图标全部功能" aria-label="查看客户端图标全部功能">?</button>';
  }

  // 首次进入的一次性引导卡（已看过返回空字符串）
  function renderDockFirstRunCard() {
    if (tipsSeen()) return '';
    return '' +
      '<div class="client-dock-first-tips" id="clientDockFirstTips">' +
        '<div class="client-dock-first-tips-head">' +
          '<strong>这排图标还能这样用</strong>' +
          '<button type="button" class="client-dock-first-tips-close" onclick="dismissDockFirstRunTips()" aria-label="关闭提示">✕</button>' +
        '</div>' +
        '<ul>' +
          '<li><b>左键单击</b>：下载 / 启动 / 停止对应客户端</li>' +
          '<li><b>右键图标</b>：快捷配置 · 更新 · 清理配置文件 · 卸载 · 强制关闭</li>' +
          '<li><b>悬停图标</b>：查看安装 / 账号 / 运行状态，右下绿点 = 正在运行</li>' +
        '</ul>' +
        '<button type="button" class="client-dock-first-tips-ok" onclick="dismissDockFirstRunTips()">知道了</button>' +
      '</div>';
  }

  function dismissDockFirstRunTips() {
    markTipsSeen();
    document.getElementById('clientDockFirstTips')?.remove();
  }

  // hover 气泡：一句介绍 + 运行状态（复活原有 client-dock-bubble）
  function renderDockBubble(item, info) {
    const welcome = typeof statusClientWelcomeText === 'function' ? statusClientWelcomeText(item, null) : '';
    const state = info && info.text ? info.text : '';
    const line = [welcome, state].filter(Boolean).join(' · ');
    return '<span class="client-dock-bubble" aria-hidden="true">' + escText(line) + '</span>';
  }

  function renderPluginCenterBubble() {
    return '<span class="client-dock-bubble" aria-hidden="true">点击进入插件市场，安装 / 管理 DEX AI 插件</span>';
  }

  // 「?」功能面板：通用操作 + 每个客户端一句话
  function openDockTipsPanel() {
    document.getElementById('dockTipsPanel')?.remove();
    const kinds = typeof statusClientKinds === 'function' ? statusClientKinds() : [];
    const items = kinds.map(function (item) {
      const label = item.label || (typeof clientKindUiLabel === 'function' ? clientKindUiLabel(item.accountKind) : item.slug);
      const welcome = typeof statusClientWelcomeText === 'function' ? statusClientWelcomeText(item, null) : '';
      return '<li><b>' + escText(label) + '</b><span>' + escText(welcome || '客户端') + '</span></li>';
    }).join('');

    const panel = document.createElement('div');
    panel.className = 'dock-tips-overlay';
    panel.id = 'dockTipsPanel';
    panel.innerHTML = '' +
      '<div class="dock-tips-modal" role="dialog" aria-modal="true" aria-label="客户端图标功能说明">' +
        '<div class="dock-tips-modal-head">' +
          '<h3>客户端图标能做什么</h3>' +
          '<button type="button" class="dock-tips-modal-close" onclick="closeDockTipsPanel()" aria-label="关闭">✕</button>' +
        '</div>' +
        '<div class="dock-tips-modal-body">' +
          '<section class="dock-tips-section">' +
            '<h4>通用操作</h4>' +
            '<ul class="dock-tips-actions">' +
              '<li><b>左键单击</b>：下载 / 启动 / 停止对应客户端</li>' +
              '<li><b>右键图标</b>：快捷配置 · 更新 · 清理配置文件 · 卸载 · 强制关闭</li>' +
              '<li><b>悬停图标</b>：查看安装 / 账号 / 运行状态（三色圆点）</li>' +
              '<li><b>右下绿点</b>：客户端进程正在运行</li>' +
            '</ul>' +
          '</section>' +
          '<section class="dock-tips-section">' +
            '<h4>每个客户端</h4>' +
            '<ul class="dock-tips-clients">' + items + '</ul>' +
          '</section>' +
          '<p class="dock-tips-note">插件中心：进入插件市场，安装 / 管理 DEX AI 插件。</p>' +
        '</div>' +
      '</div>';
    document.body.appendChild(panel);
    panel.addEventListener('mousedown', function (e) {
      if (e.target === panel) closeDockTipsPanel();
    });
    function onKey(e) {
      if (e.key === 'Escape') {
        closeDockTipsPanel();
        document.removeEventListener('keydown', onKey);
      }
    }
    document.addEventListener('keydown', onKey);
  }

  function closeDockTipsPanel() {
    document.getElementById('dockTipsPanel')?.remove();
  }

  window.renderDockTipsEntry = renderDockTipsEntry;
  window.renderDockFirstRunCard = renderDockFirstRunCard;
  window.dismissDockFirstRunTips = dismissDockFirstRunTips;
  window.renderDockBubble = renderDockBubble;
  window.renderPluginCenterBubble = renderPluginCenterBubble;
  window.openDockTipsPanel = openDockTipsPanel;
  window.closeDockTipsPanel = closeDockTipsPanel;
})();
