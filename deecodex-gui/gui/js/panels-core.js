// 状态面板
// ═══════════════════════════════════════════════════════════════
function clientKindUiLabel(kind, profile) {
  const normalized = typeof normalizeClientKind === 'function' ? normalizeClientKind(kind) : String(kind || '');
  const labels = (typeof CLIENT_KIND_LABELS !== 'undefined' && CLIENT_KIND_LABELS) || {
    codex: 'Codex',
    claude_code: 'Claude',
    hermes: 'Hermes',
    generic_client: '通用客户端',
  };
  return labels[normalized] || profile?.label || normalized || '未知工具';
}

function normalizeServiceHost(host) {
  const value = String(host || '').trim().replace(/\/+$/, '');
  if (!value) return '127.0.0.1';
  const withoutScheme = value.replace(/^https?:\/\//, '');
  const hostPart = withoutScheme.split('/')[0] || withoutScheme;
  if (hostPart.startsWith('[')) {
    const end = hostPart.indexOf(']');
    return end > 0 ? hostPart.slice(1, end) : hostPart;
  }
  const parts = hostPart.split(':');
  if (parts.length === 2 && /^\d+$/.test(parts[1])) return parts[0] || '127.0.0.1';
  return hostPart;
}

function serviceHostForClientUrl(host) {
  const normalized = normalizeServiceHost(host);
  const urlHost = ['0.0.0.0', '::', '*'].includes(normalized) ? '127.0.0.1' : normalized;
  return urlHost.includes(':') && !(urlHost.startsWith('[') && urlHost.endsWith(']')) ? `[${urlHost}]` : urlHost;
}

function renderStatus() {
  const s = window._statusData || {};
  const tokenStats = s.token_stats || {};
  const clientLabel = (kind, profile) => clientKindUiLabel(kind, profile);
  const kindOf = (account) => typeof accountClientKind === 'function' ? accountClientKind(account) : (account?.client_kind || account?.target || 'codex');
  const accounts = accountsData.accounts || [];

  const running = s.running;
  // 智能路由模式不启用 CDP 注入（Codex 桌面卡片已分流），"启动 CDP"按钮隐藏。
  // API/中转模式保留 CDP 注入，用于 Codex 桌面模型目录和本地代理能力。
  const routerMode = currentConfig?.codex_router_mode || 'api';
  const showCdpButton = routerMode === 'api' || routerMode === 'relay';
  const clientKinds = (() => {
    const defaults = [
      { slug: 'codex', label: 'Codex' },
      { slug: 'claude_code', label: 'Claude' },
      { slug: 'hermes', label: 'Hermes' },
      { slug: 'generic_client', label: '通用客户端' },
    ];
    const profiles = Array.isArray(clientProfiles) && clientProfiles.length
      ? clientProfiles.map(p => {
        const slug = normalizeClientKind?.(p.slug || p.kind);
        return { slug, label: clientLabel(slug, p) };
      })
      : defaults;
    const seen = new Set();
    return profiles.concat(defaults).filter(item => {
      const slug = normalizeClientKind?.(item.slug) || item.slug;
      if (!slug || seen.has(slug)) return false;
      seen.add(slug);
      return true;
    });
  })();
  const clientSummaries = clientKinds.map(kind => {
    const list = accounts.filter(a => kindOf(a) === kind.slug);
    const hasIssue = list.some(a => typeof clientAccountHasIssue === 'function' && clientAccountHasIssue(a));
    return { count: list.length, hasIssue };
  });
  const clientIssueCount = clientSummaries.filter(item => item.hasIssue).length;
  const todayRequests = Number(tokenStats.total || 0);
  const problemCount = (running ? 0 : 1) + clientIssueCount;
  const version = s.version && s.version !== '—' ? `v${s.version}` : '版本 —';
  const hasUpdate = typeof deeStorage !== 'undefined' && deeStorage?.getItem?.('updateAvailable') === '1';
  const updateInfo = typeof window !== 'undefined' ? window._updateInfo : null;
  const updateLatest = updateInfo?.latest || deeStorage?.getItem?.('updateLatest') || '';

  return `
    <section class="status-overview-shell" aria-label="接入状态总览">
      ${renderStatusHero(hasUpdate, version, updateLatest)}
      ${renderStatusMetrics(running, problemCount, todayRequests)}
      ${renderStatusOperations(running, showCdpButton)}
      ${renderStatusClientDock(running)}
    </section>
  `;
}

function renderStatusHero(hasUpdate, version, updateLatest) {
  return `<div class="gateway-hero">
    <div class="gateway-hero-main">
      <div class="gateway-version-line">
        <span class="gateway-version" id="dashboardVersion">${hasUpdate ? '<span class="update-dot"></span>' : ''}${esc(version)}</span>
        <button type="button" class="support-project-btn" onclick="showSupportProjectModal('status')" title="联系作者">
          <span class="support-project-icon" aria-hidden="true"></span>
          <span>联系作者</span>
        </button>
        ${hasUpdate ? `<button type="button" class="gateway-update-pill" onclick="showStoredUpdatePrompt('status')" title="查看更新">
          <span class="update-dot" aria-hidden="true"></span>
          <span>发现 ${esc(updateLatest || '新版本')}</span>
        </button>` : ''}
      </div>
    </div>
  </div>`;
}

function renderStatusMetrics(running, problemCount, todayRequests) {
  return `<div class="gateway-metric-grid">
    <div class="gateway-metric-card ${running ? 'ok' : 'fail'}" onclick="mgmtToggle()">
      <span class="metric-label">网关状态</span>
      <span class="gateway-status-line ${running ? 'is-running' : 'is-stopped'}" aria-label="${running ? '运行中' : '已停止'}"></span>
    </div>
    <div class="gateway-metric-card ${problemCount ? 'warn' : 'ok'}" onclick="validateConfig()">
      <span class="metric-label">待处理问题</span>
      <strong>${esc(problemCount)}</strong>
    </div>
    <div class="gateway-metric-card" onclick="switchPanel('sessions')">
      <span class="metric-label">今日消耗</span>
      <strong>${esc(todayRequests)}</strong>
    </div>
  </div>`;
}

function renderStatusOperations(running, showCdpButton) {
  return `<section class="gateway-panel operations-panel">
    <div class="gateway-section-title">服务操作</div>
    <div class="mgmt-actions gateway-actions">
      <button class="btn btn-primary" onclick="mgmtToggle()" id="btnToggle">
        <span class="status-action-icon ${running ? 'status-action-icon-stop' : 'status-action-icon-start'}" aria-hidden="true"></span>
        <span>${running ? '停止网关' : '启动网关'}</span>
      </button>
      <button class="btn btn-ghost" onclick="validateConfig()">
        <span class="status-action-icon status-action-icon-diagnostics" aria-hidden="true"></span>
        <span>运行诊断</span>
      </button>
      <button class="btn btn-ghost" onclick="mgmtLaunchCodex()" id="btnLaunchCodex"${showCdpButton ? '' : ' style="display:none"'}>
        <span class="status-action-icon ${window._cdpLaunched ? 'status-action-icon-stop' : 'status-action-icon-cdp'}" aria-hidden="true"></span>
        <span>${window._cdpLaunched ? '停止 CDP' : '启动 CDP'}</span>
      </button>
      <button class="btn btn-ghost" onclick="mgmtRestart()" id="btnRestart">
        <span class="status-action-icon status-action-icon-restart" aria-hidden="true"></span>
        <span>重启服务</span>
      </button>
      <button class="btn btn-ghost" onclick="mgmtLogs()">
        <span class="status-action-icon status-action-icon-logs" aria-hidden="true"></span>
        <span>查看日志</span>
      </button>
      <button class="btn btn-ghost" onclick="mgmtUpdate()" id="btnUpdate">
        <span class="status-action-icon status-action-icon-upgrade" aria-hidden="true"></span>
        <span>检查更新</span>
      </button>
    </div>
  </section>`;
}

function statusClientKinds() {
  return [
    // 第一行：桌面版为主
    { slug: 'codex_desktop', label: 'Codex 桌面', accountKind: 'codex', iconKind: 'codex', desktopApp: 'Codex' },
    { slug: 'dsh', label: 'DeepSeek Harness', accountKind: 'dsh', iconKind: 'dsh' },
    { slug: 'workbuddy', label: 'WorkBuddy', accountKind: 'workbuddy', iconKind: 'workbuddy' },
    { slug: 'opencode_desktop', label: 'OpenCode 桌面', accountKind: 'opencode', iconKind: 'opencode', desktopApp: 'OpenCode' },
    { slug: 'hermes_desktop', label: 'Hermes 桌面', accountKind: 'hermes', iconKind: 'hermes', desktopApp: 'Hermes' },
    { slug: 'claude_desktop', label: 'Claude 桌面', accountKind: 'claude_code', iconKind: 'claude_code', desktopApp: 'Claude' },
    // 第二行：CLI 为主
    { slug: 'claude_cli', label: 'Claude CLI', accountKind: 'claude_code', iconKind: 'claude_code' },
    { slug: 'pi', label: 'Pi', accountKind: 'pi', iconKind: 'pi' },
    { slug: 'codex_cli', label: 'Codex CLI', accountKind: 'codex', iconKind: 'codex' },
    { slug: 'opencode', label: 'OpenCode', accountKind: 'opencode', iconKind: 'opencode' },
    { slug: 'hermes', label: 'Hermes', accountKind: 'hermes', iconKind: 'hermes' },
  ];
}

function supportQrAsset(type) {
  return type === 'wechat'
    ? 'assets/support/wechat-original.png'
    : 'assets/support/alipay-original.jpg';
}

function showSupportProjectModal(source) {
  // 复用联系作者同款 qr 弹窗样式，保留支付宝 / 微信赞助二维码。
  const content = document.getElementById('qrOverlayContent');
  const title = document.querySelector('.qr-overlay .qr-title');
  const account = document.getElementById('qrOverlayAccount');
  if (title) title.textContent = '联系作者';
  if (account) account.textContent = '';
  if (content) {
    content.innerHTML = '<p class="qr-hint" style="margin:0 0 12px;">扫码联系作者；如果工具帮到了你，也可以自愿赞助。</p>'
      + '<div class="qr-grid-3" data-source="' + escAttr(source || '') + '">'
      + '<figure class="qr-fig"><img src="assets/qr-author.png" alt="作者二维码"><figcaption>联系作者</figcaption></figure>'
      + '<figure class="qr-fig"><img src="assets/qr-alipay.png" alt="支付宝收款码"><figcaption>支付宝收款</figcaption></figure>'
      + '<figure class="qr-fig"><img src="assets/qr-wechat.png" alt="微信收款码"><figcaption>微信收款</figcaption></figure>'
      + '</div>';
  }
  document.querySelector('.qr-dialog')?.classList.add('qr-dialog-wide');
  if (typeof showQrOverlay === 'function') showQrOverlay();
}

function showSupportProjectNudge() {
  const existing = document.getElementById('supportProjectNudge');
  if (existing) existing.remove();
  const nudge = document.createElement('div');
  nudge.className = 'support-project-nudge';
  nudge.id = 'supportProjectNudge';
  nudge.innerHTML = `
    <div class="support-project-nudge-copy">
      <strong>配置完成</strong>
      <span>如果 DEX AI 帮到了你，可以支持一下项目。</span>
    </div>
    <button type="button" class="btn btn-primary" id="supportProjectNudgeOpen">支持一下</button>
    <button type="button" class="support-project-nudge-close" id="supportProjectNudgeClose" aria-label="关闭">✕</button>`;
  document.body.appendChild(nudge);
  document.getElementById('supportProjectNudgeOpen')?.addEventListener('click', () => {
    nudge.remove();
    showSupportProjectModal('quick_start');
  });
  document.getElementById('supportProjectNudgeClose')?.addEventListener('click', () => nudge.remove());
  window.setTimeout(() => {
    document.getElementById('supportProjectNudge')?.remove();
  }, 14000);
}

function statusClientDockItem(kind) {
  const normalized = String(kind || '');
  return statusClientKinds().find(item => item.slug === normalized) || statusClientKinds()[0];
}

function statusClientAccountKind(kind) {
  return statusClientDockItem(kind).accountKind || kind;
}

function statusClientSurface(kind) {
  const normalized = String(kind || '');
  // dsh 是 web 型 CLI（启动后打开网页），左下角用 web 小图标。
  if (normalized === 'dsh') return 'web';
  // WorkBuddy 是纯桌面应用（不以 _desktop 结尾），固定为桌面版角标。
  if (normalized === 'workbuddy') return 'desktop';
  return normalized.endsWith('_desktop') ? 'desktop' : 'cli';
}

function statusClientPrimaryAccount(kind) {
  const accountKind = statusClientAccountKind(kind);
  const surface = statusClientSurface(kind);
  const list = ((accountsData && accountsData.accounts) || [])
    .filter(account => {
      const familyMatches = typeof accountClientKind === 'function'
        ? accountClientKind(account) === accountKind
        : normalizeClientKind?.(account?.client_kind || account?.target || 'codex') === accountKind;
      if (!familyMatches) return false;
      if (accountKind !== 'codex' && accountKind !== 'claude_code') return true;
      if (typeof accountClientSurface === 'function') return accountClientSurface(account) === surface;
      return String(account?.client_surface || account?.client_options?.client_surface || 'cli').toLowerCase() === surface;
    });
  if (!list.length) return null;
  const activeId = accountsData.active_account_id || accountsData.active_id;
  if (accountKind === 'codex') return list.find(account => account.id === activeId) || list[0];
  return list
    .slice()
    .sort((a, b) => Number(b.last_applied_at || 0) - Number(a.last_applied_at || 0))[0];
}

function statusClientProcessRunning(kind) {
  const status = (window._clientLifecycleMap || {})[kind];
  return Boolean(status?.runtime?.running);
}

function statusClientEnabled(kind, account, gatewayRunning) {
  if (kind === 'codex' || kind === 'codex_cli' || kind === 'codex_desktop') return Boolean(gatewayRunning);
  return Boolean(account?.client_options?.proxy_recording_enabled && account?.last_applied_at);
}

function statusClientInfo(kind, gatewayRunning) {
  const account = statusClientPrimaryAccount(kind);
  const enabled = statusClientEnabled(kind, account, gatewayRunning);
  const processRunning = statusClientProcessRunning(kind);
  const state = processRunning ? 'on' : 'off';
  const text = processRunning ? '运行中' : '未运行';
  return { account, enabled, processRunning, state, text };
}

function statusClientLifecycleMeta(kind, info) {
  const status = (window._clientLifecycleMap || {})[kind] || {};
  // 后端会下发 requires_account：dsh/Pi/OpenCode/WorkBuddy 等不要求账号的客户端为 false。
  // hermes_desktop 复用 hermes 账号时后端会判定为 true 并正确带出已配置状态。
  // pi/dsh 等原生检测客户端：native_detected=true 时按原生配置显示「已配置/未配置」，
  // 不再一律显示「无需账号」（那会把未配置误显示成已就绪）。
  const requiresAccount = status.requires_account !== false;
  const nativeDetected = status.native_detected === true;
  const installedKnown = typeof status.installed === 'boolean';
  const accountKnown = typeof status.account_exists === 'boolean';
  const installed = installedKnown ? status.installed : true;
  const nativeReady = nativeDetected ? status.native_configured === true : true;
  const accountReady = requiresAccount
    ? (accountKnown ? status.account_exists : Boolean(info.account))
    : nativeReady;
  const configured = requiresAccount
    ? (status.account_configured !== false && accountReady)
    : nativeReady;
  const action = status.next_action || (installed ? (accountReady ? 'launch' : 'configure') : 'install');
  const installLabel = installedKnown ? (installed ? '已安装' : '未安装') : '检测中';
  const accountLabel = requiresAccount
    ? (accountKnown ? (accountReady ? (configured ? '已配置' : '待写入') : '需账号') : (info.account ? '已配置' : '需账号'))
    : (nativeDetected ? (status.native_configured ? '已配置' : '未配置') : '无需账号');
  const runLabel = info.processRunning ? '运行中' : '待启动';
  const classes = [
    installed ? 'is-installed' : 'needs-install',
    accountReady ? 'has-account' : 'needs-account',
    configured ? 'config-ready' : 'config-pending',
    action ? `next-${action}` : '',
  ].filter(Boolean).join(' ');
  return {
    status,
    action,
    classes,
    installLabel,
    accountLabel,
    runLabel,
    installed,
    accountReady,
    configured,
    requiresAccount,
  };
}

function statusClientIcon(item, processRunning) {
  const accountKind = item.iconKind || item.accountKind || item.slug;
  const slug = accountKind === 'claude_code' ? 'claude-code' : accountKind;
  // 圆角矩形盒子（client-logo-box）+ 主体放大后的 img
  const icon = `<span class="client-logo-box client-logo-${escAttr(slug)}"><img class="client-logo-img" src="${providerLogoSrc(slug)}" alt="" aria-hidden="true"></span>`;
  const badgeKinds = ['codex', 'claude_code', 'opencode', 'hermes', 'dsh', 'workbuddy', 'pi'];
  if (badgeKinds.includes(item.accountKind)) {
    const surface = statusClientSurface(item.slug);
    // 状态点放进图标内部（stack 内 absolute），确保贴紧图标右下角
    return `<span class="surface-icon-stack">${icon}<span class="surface-glyph surface-${escAttr(surface)}" aria-hidden="true"></span><span class="client-dock-runtime ${processRunning ? 'live' : 'idle'}"></span></span>`;
  }
  return icon;
}

function statusClientWelcomeText(item, lifecycle) {
  if (lifecycle && lifecycle.installed === false) {
    return '还没接入，先把我请进来。';
  }
  const key = item.iconKind || item.accountKind || item.slug;
  const copy = {
    codex: '把任务交给我，我去仓库里跑一圈。',
    claude_code: '先读懂代码，再稳稳下手。',
    hermes: '我会记住项目，也会越跑越顺。',
    dsh: 'DeepSeek Harness，网页端开跑。',
    pi: '让 Pi 在终端里干活。',
    opencode: '开源编码代理，随开随用。',
    workbuddy: '腾讯 AI 桌面工作台，一句话交付结果。',
  };
  const text = copy[key] || '准备好了，随时开工。';
  return item.slug === 'codex_desktop' || item.slug === 'claude_desktop' || item.slug === 'opencode_desktop' || item.slug === 'hermes_desktop'
    ? `${text}（桌面版）`
    : text;
}

function renderStatusClientDock(gatewayRunning) {
  return `<section class="gateway-panel client-dock-panel" aria-label="客户端运行控制">
    <div class="gateway-section-title client-dock-title">客户端启动${typeof window.renderDockTipsEntry === 'function' ? window.renderDockTipsEntry() : ''}</div>
    ${typeof window.renderDockFirstRunCard === 'function' ? window.renderDockFirstRunCard() : ''}
    <div class="client-dock-grid">
      ${statusClientKinds().map(item => {
        const kind = item.slug;
        const label = item.label || clientKindUiLabel(item.accountKind);
        const info = statusClientInfo(kind, gatewayRunning);
        const lifecycle = statusClientLifecycleMeta(kind, info);
        return `<button type="button" class="client-dock-item ${escAttr(info.state)} ${escAttr(lifecycle.classes)}${info.processRunning ? ' process-running' : ''}" data-client-kind="${escAttr(kind)}" data-client-label="${escAttr(label)}" data-next-action="${escAttr(lifecycle.action)}" onclick="handleClientDockClick('${escAttr(kind)}')" oncontextmenu="handleClientDockContextMenu(event, '${escAttr(kind)}')" aria-label="${escAttr(label + ' · ' + info.text)}">
          <span class="client-dock-icon">${statusClientIcon(item, info.processRunning)}</span>
          ${typeof window.renderDockBubble === 'function' ? window.renderDockBubble(item, info) : ''}
          <span class="client-dock-state-row" aria-hidden="true">
            <span class="client-dock-state-dot install" title="${escAttr(lifecycle.installLabel)}"></span>
            <span class="client-dock-state-dot account" title="${escAttr(lifecycle.accountLabel)}"></span>
            <span class="client-dock-state-dot runtime" title="${escAttr(lifecycle.runLabel)}"></span>
          </span>
          <span class="client-dock-label">${esc(label)}</span>
        </button>`;
      }).join('')}
      ${renderPluginCenterDockItem()}
    </div>
  </section>`;
}

function renderPluginCenterDockItem() {
  return `<button type="button" class="client-dock-item client-dock-plugin-center" data-client-kind="plugin_center" data-client-label="插件中心" onclick="switchPanel('plugins')" aria-label="插件中心 · 打开插件商城" title="插件中心">
    <span class="client-dock-icon">${renderPluginCenterIcon()}</span>
    ${typeof window.renderPluginCenterBubble === 'function' ? window.renderPluginCenterBubble() : ''}
    <span class="client-dock-state-row client-dock-state-spacer" aria-hidden="true">
      <span class="client-dock-state-dot"></span>
      <span class="client-dock-state-dot"></span>
      <span class="client-dock-state-dot"></span>
    </span>
    <span class="client-dock-label">插件中心</span>
  </button>`;
}

// 插件中心入口图标：圆角矩形底板 + 2×2 应用格，尺寸/结构与客户端图标保持一致（54px）。
// 颜色通过 CSS 变量分层适配亮色/暗色主题（见 client-dock.css）。
function renderPluginCenterIcon() {
  return `<svg class="client-dock-plugin-icon" viewBox="0 0 44 44" aria-hidden="true" focusable="false">
    <rect class="plugin-tile-bg" x="2" y="2" width="40" height="40" rx="11.5"/>
    <rect class="plugin-tile-edge" x="2.75" y="2.75" width="38.5" height="38.5" rx="10.75" fill="none"/>
    <rect class="plugin-tile-cell" x="11" y="11" width="9" height="9" rx="2.8"/>
    <rect class="plugin-tile-cell" x="24" y="11" width="9" height="9" rx="2.8"/>
    <rect class="plugin-tile-cell" x="11" y="24" width="9" height="9" rx="2.8"/>
    <rect class="plugin-tile-cell plugin-tile-cell-accent" x="24" y="24" width="9" height="9" rx="2.8"/>
  </svg>`;
}

async function refreshStatusClientDock() {
  if (currentPanel !== 'status') return;
  try {
    const refreshLifecycle = typeof window !== 'undefined' && typeof window.refreshClientLifecycleDock === 'function'
      ? window.refreshClientLifecycleDock
      : (typeof refreshClientLifecycleDock === 'function' ? refreshClientLifecycleDock : null);
    if (refreshLifecycle) {
      await refreshLifecycle();
    }
  } catch (_) {}
}

let _clientDockMenuDocDown = null;
let _clientDockMenuDocKey = null;

function closeClientDockContextMenu() {
  const menu = document.getElementById('clientDockContextMenu');
  if (menu) menu.remove();
  if (_clientDockMenuDocDown) {
    document.removeEventListener('mousedown', _clientDockMenuDocDown);
    _clientDockMenuDocDown = null;
  }
  if (_clientDockMenuDocKey) {
    document.removeEventListener('keydown', _clientDockMenuDocKey);
    _clientDockMenuDocKey = null;
  }
}

function showClientDockContextMenu(event, kind) {
  const normalized = String(kind || '');
  const item = statusClientDockItem(normalized);
  closeClientDockContextMenu();

  const menu = document.createElement('div');
  menu.className = 'client-dock-menu';
  menu.id = 'clientDockContextMenu';
  menu.setAttribute('role', 'menu');
  menu.innerHTML = `
    <div class="client-dock-menu-header">${esc(item.label)}</div>
    <div class="client-dock-menu-actions">
      <button type="button" class="client-dock-menu-item" data-action="config" role="menuitem">
        <span class="client-dock-menu-icon client-dock-menu-icon-config" aria-hidden="true"></span>
        <span>快捷配置</span>
      </button>
      <button type="button" class="client-dock-menu-item" data-action="update" role="menuitem">
        <span class="client-dock-menu-icon client-dock-menu-icon-update" aria-hidden="true"></span>
        <span>更新</span>
      </button>
      <button type="button" class="client-dock-menu-item danger" data-action="clean-config" role="menuitem">
        <span class="client-dock-menu-icon client-dock-menu-icon-clean" aria-hidden="true"></span>
        <span>清理配置文件</span>
      </button>
      <button type="button" class="client-dock-menu-item danger" data-action="uninstall" role="menuitem">
        <span class="client-dock-menu-icon client-dock-menu-icon-uninstall" aria-hidden="true"></span>
        <span>卸载</span>
      </button>
      <button type="button" class="client-dock-menu-item danger" data-action="quit" role="menuitem">
        <span class="client-dock-menu-icon client-dock-menu-icon-quit" aria-hidden="true"></span>
        <span>强制关闭</span>
      </button>
    </div>`;
  document.body.appendChild(menu);

  const rect = menu.getBoundingClientRect();
  const pad = 8;
  const left = Math.min(Math.max(pad, event.clientX), Math.max(pad, window.innerWidth - rect.width - pad));
  const top = Math.min(Math.max(pad, event.clientY), Math.max(pad, window.innerHeight - rect.height - pad));
  menu.style.left = `${left}px`;
  menu.style.top = `${top}px`;

  menu.querySelector('[data-action="config"]').addEventListener('click', () => {
    closeClientDockContextMenu();
    openClientQuickConfig(normalized);
  });
  menu.querySelector('[data-action="update"]').addEventListener('click', () => {
    closeClientDockContextMenu();
    updateClient(normalized, item);
  });
  menu.querySelector('[data-action="clean-config"]').addEventListener('click', () => {
    closeClientDockContextMenu();
    cleanClientConfig(normalized, item);
  });
  menu.querySelector('[data-action="uninstall"]').addEventListener('click', () => {
    closeClientDockContextMenu();
    uninstallClient(normalized, item);
  });
  menu.querySelector('[data-action="quit"]').addEventListener('click', () => {
    closeClientDockContextMenu();
    forceQuitClient(normalized, item);
  });

  _clientDockMenuDocDown = function (e) {
    const m = document.getElementById('clientDockContextMenu');
    if (m && !m.contains(e.target)) closeClientDockContextMenu();
  };
  _clientDockMenuDocKey = function (e) {
    if (e.key === 'Escape') closeClientDockContextMenu();
  };
  document.addEventListener('mousedown', _clientDockMenuDocDown);
  document.addEventListener('keydown', _clientDockMenuDocKey);
}

function handleClientDockContextMenu(event, kind) {
  event?.preventDefault?.();
  event?.stopPropagation?.();
  showClientDockContextMenu(event, String(kind || ''));
}

async function openClientQuickConfig(kind) {
  const normalized = String(kind || '');
  const accountKind = statusClientAccountKind(normalized);
  if (accountKind === 'codex') {
    if (typeof openCodexQuickStart === 'function') {
      await openCodexQuickStart(true);
      return;
    }
  }
  if (typeof window.openClientQuickConfigModal === 'function') {
    window.openClientQuickConfigModal(normalized);
    return;
  }
  switchPanel('config');
}

async function forceQuitClient(kind, item) {
  const normalized = String(kind || '');
  item = item || statusClientDockItem(normalized);
  if (!statusClientProcessRunning(normalized)) {
    showToast(`${item.label} 当前未运行`, 'info');
    return;
  }
  if (!await showConfirm(`强制退出 ${item.label}？`)) return;
  try {
    const result = await invoke('dex_force_quit_client', { kind: normalized });
    const killed = Array.isArray(result?.killed) ? result.killed.length : Number(result?.killed || 0);
    const errors = Array.isArray(result?.errors) ? result.errors.length : 0;
    const remaining = Array.isArray(result?.remaining) ? result.remaining.length : 0;
    if (remaining) {
      showToast(`${item.label} 强制退出未完全成功，仍检测到 ${remaining} 个残留`, 'error');
    } else if (killed) {
      showToast(`${item.label} 已强制退出 ${killed} 个进程${errors ? `，${errors} 个失败` : ''}`, errors ? 'info' : 'success');
    } else if (result?.ok === true) {
      // 优雅退出成功（killed 为空但 ok:true）——不能当失败，显示“已退出”。
      showToast(`${item.label} 已退出`, 'success');
    } else {
      showToast(`${item.label} 强制退出失败`, 'error');
    }
    setTimeout(refreshStatusClientDock, 700);
  } catch (err) {
    showToast(`${item.label} 强制退出失败: ` + err, 'error');
  }
}

async function updateClient(kind, item) {
  const normalized = String(kind || '');
  item = item || statusClientDockItem(normalized);
  const label = item.label || '客户端';
  try {
    const result = await invoke('dex_update_client', { kind: normalized });
    const mode = result?.mode;
    if (mode === 'terminal') {
      showToast(`${label} 更新命令已在终端打开，请按提示完成更新`, 'info');
    } else if (mode === 'check_desktop') {
      const ver = result?.installed_version;
      const verText = ver ? `当前版本 ${ver}` : '未读取到当前版本';
      showToast(`${label} 已检测：${verText}，已打开官方页面确认更新`, 'info');
    } else {
      showToast(`${label} 更新已发起`, 'success');
    }
    setTimeout(refreshStatusClientDock, 1200);
  } catch (err) {
    showToast(`${label} 更新失败: ` + err, 'error');
  }
}

async function uninstallClient(kind, item) {
  const normalized = String(kind || '');
  item = item || statusClientDockItem(normalized);
  const label = item.label || '客户端';
  // 卸载前先查询真实目标（只读），让确认弹窗展示将删除的路径 / 将执行的命令，
  // 避免用户以为卸载 A 实际删除 B（例如 Codex 桌面实际指向 ChatGPT.app）。
  let targetNote = '';
  try {
    const target = await invoke('dex_uninstall_target', { kind: normalized });
    const mode = target?.mode;
    if (mode === 'remove_app' && target?.path) targetNote = `（将删除 ${target.path}）`;
    else if (mode === 'terminal' && target?.command) targetNote = `（将执行：${target.command}）`;
    else if (mode === 'unsupported' && target?.reason) targetNote = `（${target.reason}）`;
    else if (mode === 'not_installed') targetNote = '（未检测到安装）';
  } catch (err) {
    // 目标查询失败不阻塞卸载流程，保持原确认文案。
  }
  if (!await showConfirm(`确定要卸载 ${label} 吗？${targetNote}`)) return;
  if (!await showConfirm(`再次确认：卸载将删除 ${label} 的安装文件${targetNote}，此操作不可撤销，确定继续？`)) return;
  try {
    const result = await invoke('dex_uninstall_client', { kind: normalized });
    const mode = result?.mode;
    if (mode === 'terminal') {
      showToast(`${label} 卸载命令已在终端打开，请按提示完成卸载`, 'info');
    } else if (mode === 'remove_app') {
      showToast(`${label} 已卸载（${result?.path || ''}）`, 'success');
    } else if (mode === 'open_settings') {
      showToast(`请在系统设置中完成 ${label} 卸载`, 'info');
    } else {
      showToast(`${label} 卸载已发起`, 'success');
    }
    setTimeout(refreshStatusClientDock, 1200);
  } catch (err) {
    showToast(`${label} 卸载失败: ` + err, 'error');
  }
}

// 清理配置文件：按客户端专属路径删除本地配置（与卸载不同，不动安装文件）。
// 先查只读目标（展示将删除的路径 + 可一并移除的 DEX AI 账号），二次确认后再执行。
// 可勾选「同时移除 DEX AI 账号」：受「不能删除最后一个 Codex 账号」保护，删除前由后端备份。
async function cleanClientConfig(kind, item) {
  const normalized = String(kind || '');
  item = item || statusClientDockItem(normalized);
  const label = item.label || '客户端';
  let targetNote = '';
  let removableAccounts = [];
  let removeAccountBlocked = false;
  try {
    const target = await invoke('dex_clean_client_config_target', { kind: normalized });
    const mode = target?.mode;
    if (mode === 'paths' && Array.isArray(target?.paths) && target.paths.length) {
      targetNote = `（将删除：${target.paths.join('、')}）`;
    } else if (mode === 'none') {
      targetNote = '（未检测到配置文件）';
    } else if (mode === 'unsupported' && target?.reason) {
      targetNote = `（${target.reason}）`;
    }
    removableAccounts = Array.isArray(target?.removable_accounts) ? target.removable_accounts : [];
    removeAccountBlocked = target?.remove_account_blocked === true;
  } catch (err) {
    // 目标查询失败不阻塞清理流程，保持原确认文案。
  }
  const removableCount = removableAccounts.length;
  let removeAccount = false;
  if (removableCount && !removeAccountBlocked) {
    const firstNote = `确定要清理 ${label} 的配置文件吗？${targetNote}`;
    const checkboxLabel = `同时移除 DEX AI 账号（${removableCount} 个，不可恢复，可能中断当前代理会话）`;
    const result = await showConfirmWithCheckbox(firstNote, checkboxLabel);
    if (!result?.ok) return;
    removeAccount = result.checked === true;
  } else {
    if (!await showConfirm(`确定要清理 ${label} 的配置文件吗？${targetNote}`)) return;
  }
  const accountNote = removeAccount
    ? `，并移除 ${removableCount} 个 DEX AI 账号`
    : (removableCount && removeAccountBlocked ? '（最后一个 Codex 账号受保护，不可一并移除）' : '');
  if (!await showConfirm(`再次确认：清理将删除 ${label} 的本地配置${targetNote}${accountNote}，登录态与设置会丢失，此操作不可撤销，确定继续？`)) return;
  try {
    const result = await invoke('dex_clean_client_config', { kind: normalized, remove_account: removeAccount });
    const removed = Array.isArray(result?.removed) ? result.removed.length : 0;
    const failed = Array.isArray(result?.failed) ? result.failed.length : 0;
    const removedAccounts = Number(result?.removed_account_count || 0);
    if (removedAccounts) {
      showToast(`${label} 已清理配置并移除 ${removedAccounts} 个 DEX AI 账号${failed ? `，${failed} 个配置清理失败` : ''}`, failed ? 'info' : 'success');
    } else if (result?.account_failed) {
      showToast(`${label} 配置已清理，但移除账号失败：${result.account_failed}`, 'error');
    } else if (removed) {
      showToast(`${label} 已清理 ${removed} 个配置文件${failed ? `，${failed} 个失败` : ''}`, failed ? 'info' : 'success');
    } else if (failed) {
      showToast(`${label} 清理失败：${(result.failed || []).join('；')}`, 'error');
    } else {
      showToast(`${label} 没有检测到配置文件`, 'info');
    }
    setTimeout(refreshStatusClientDock, 1200);
  } catch (err) {
    showToast(`${label} 清理配置文件失败: ` + err, 'error');
  }
}

// ═══════════════════════════════════════════════════════════════
// 配置面板
// ═══════════════════════════════════════════════════════════════
function normalizeConfigClientKind(kind) {
  const value = String(kind || 'global');
  if (value === 'global') return 'global';
  if (typeof normalizeClientKind === 'function') return normalizeClientKind(value);
  return ['codex', 'claude_code', 'hermes', 'generic_client'].includes(value) ? value : 'global';
}

function configCanEditKind(kind) {
  const normalized = normalizeConfigClientKind(kind);
  return normalized === 'global' || normalized === 'codex';
}

function configCanEditSelected() {
  return configCanEditKind(selectedConfigClientKind);
}

function syncConfigSaveVisibility() {
  const saveBtn = document.getElementById('sidebarSaveBtn');
  if (saveBtn) saveBtn.style.display = currentPanel === 'config' && configCanEditSelected() ? '' : 'none';
}

function configClientProfiles() {
  if (typeof clientProfiles !== 'undefined' && Array.isArray(clientProfiles) && clientProfiles.length) return clientProfiles;
  return [
    { slug: 'codex', label: 'Codex', description: 'DEX AI 代理配置', config_path_hint: '~/.codex/config.toml', model_slots: [] },
    { slug: 'claude_code', label: 'Claude Code', description: 'Claude 本地配置', config_path_hint: '~/.claude/settings.json', model_slots: [] },
    { slug: 'hermes', label: 'Hermes', description: 'Hermes 配置', config_path_hint: '~/.hermes/config.yaml', model_slots: [] },
    { slug: 'generic_client', label: '通用客户端', description: 'OpenAI 兼容 Env', config_path_hint: '~/.deecodex/client-env', model_slots: [] },
  ];
}

function configClientLabel(kind, profile) {
  if (kind === 'global') return '全局';
  if (profile?.label) return profile.label;
  if (typeof CLIENT_KIND_LABELS !== 'undefined' && CLIENT_KIND_LABELS[kind]) return CLIENT_KIND_LABELS[kind];
  return kind;
}

function configClientTabLabel(kind, profile) {
  if (kind === 'global') return '全局';
  return clientKindUiLabel(kind, profile);
}

function configClientIcon(kind) {
  if (kind === 'global') return '<span class="config-global-icon" aria-hidden="true"></span>';
  if (typeof clientIcon === 'function') return clientIcon(kind);
  return '<span class="config-global-icon" aria-hidden="true"></span>';
}

function configClientAccounts(kind) {
  const normalized = normalizeConfigClientKind(kind);
  const list = (typeof accountsData !== 'undefined' && Array.isArray(accountsData.accounts)) ? accountsData.accounts : [];
  return list.filter(account => {
    if (typeof accountClientKind === 'function') return accountClientKind(account) === normalized;
    return normalizeConfigClientKind(account?.client_kind || account?.target || 'codex') === normalized;
  });
}

function configClientIssueCount(kind) {
  return configClientAccounts(kind).filter(account => {
    if (typeof clientAccountHasIssue === 'function') return clientAccountHasIssue(account);
    const status = account?._client_status_report || account?.last_check;
    return status?.ok === false;
  }).length;
}

function renderConfigClientSwitcher() {
  const profiles = configClientProfiles();
  const counts = (typeof accountsData !== 'undefined' && accountsData.client_counts) ? accountsData.client_counts : {};
  const globalCount = CONFIG_SECTIONS
    .filter(sec => (sec.scope || 'global') === 'global')
    .reduce((sum, sec) => sum + sec.fields.length, 0);
  const globalActive = selectedConfigClientKind === 'global' ? ' active' : '';
  return `<div class="client-switcher config-client-switcher" role="tablist" aria-label="高级设置分类">
    <button type="button" class="client-tab config-global-tab${globalActive}" onclick="selectConfigClientKind('global')" role="tab" aria-selected="${selectedConfigClientKind === 'global'}">
      ${configClientIcon('global')}
      <span>全局</span>
      <em>${globalCount}</em>
    </button>
    ${profiles.map(profile => {
      const kind = normalizeConfigClientKind(profile.slug || profile.kind);
      const accounts = configClientAccounts(kind);
      const settingsCount = CONFIG_SECTIONS
        .filter(sec => normalizeConfigClientKind(sec.scope || 'global') === kind)
        .reduce((sum, sec) => sum + sec.fields.length, 0);
      const count = kind === 'codex' && settingsCount ? settingsCount : (counts[kind] || accounts.length || 0);
      const issueCount = configClientIssueCount(kind);
      const active = kind === selectedConfigClientKind ? ' active' : '';
      const issueClass = issueCount ? ' has-issues' : '';
      return `<button type="button" class="client-tab${active}${issueClass}" onclick="selectConfigClientKind('${escAttr(kind)}')" title="${escAttr(profile.description || '')}" role="tab" aria-selected="${kind === selectedConfigClientKind}">
        ${configClientIcon(kind)}
        <span>${esc(configClientTabLabel(kind, profile))}</span>
        <em>${count}</em>
        ${issueCount ? `<strong class="client-tab-alert" title="${escAttr(issueCount + ' 个账号最近检查异常')}">${issueCount}</strong>` : ''}
      </button>`;
    }).join('')}
  </div>`;
}

function selectConfigClientKind(kind) {
  selectedConfigClientKind = normalizeConfigClientKind(kind);
  if (typeof deeStorage !== 'undefined' && typeof CONFIG_CLIENT_STORAGE_KEY !== 'undefined') {
    deeStorage.setItem(CONFIG_CLIENT_STORAGE_KEY, selectedConfigClientKind);
  }
  syncConfigSaveVisibility();
  renderPanel('config');
}

function renderEditableConfigSections(kind) {
  const sections = CONFIG_SECTIONS.filter(sec => normalizeConfigClientKind(sec.scope || 'global') === kind);
  let html = '';
  for (const sec of sections) {
    const fieldsHtml = sec.fields.map(renderField).join('');
    const sectionHeader = `
      <div class="config-section-header">
        <span class="section-icon">${sec.icon}</span>
        <h3>${sec.label}</h3>
        <span class="section-desc">${sec.fields.length} 项</span>
      </div>`;

    if (sec.collapsed) {
      html += `
    <details class="config-section config-section-collapsed" id="cfg-${sec.id}">
      <summary>
        ${sectionHeader}
        ${sec.summary ? `<p>${esc(sec.summary)}</p>` : ''}
      </summary>
      <div class="config-fields">${fieldsHtml}</div>
    </details>`;
      continue;
    }

    html += `
    <div class="config-section" id="cfg-${sec.id}">
      ${sectionHeader}
      <div class="config-fields">${fieldsHtml}</div>
    </div>`;
  }
  return html;
}

function renderCodexAdvancedSettings() {
  return `<div class="codex-advanced-layout">
    ${renderEditableConfigSections('codex')}
  </div>`;
}

function configPrimaryClientAccount(kind) {
  const accounts = configClientAccounts(kind);
  if (!accounts.length) return null;
  const activeId = accountsData?.active_id || accountsData?.active_account_id;
  const active = accounts.find(account => account.id === activeId || account.active === true || account.is_active === true);
  return active || accounts[0];
}

function escapeJsString(value) {
  return String(value ?? '')
    .replace(/\\/g, '\\\\')
    .replace(/'/g, "\\'")
    .replace(/\r/g, '\\r')
    .replace(/\n/g, '\\n');
}

function configClientAdvancedSpec(kind) {
  const specs = {
    claude_code: {
      heading: '编程会话治理',
      summary: 'Claude Code 的高级设置重点是权限边界、项目上下文、MCP 与 Hooks，而不是账号字段。',
      focus: [
        { title: '权限模式', tag: '安全', tone: 'warn', body: '审计 permission-mode、allowedTools、disallowedTools 与 bypass 权限，避免编程会话默认拥有过宽命令能力。' },
        { title: '项目上下文', tag: '上下文', tone: 'info', body: '检查 CLAUDE.md、.claude/settings.json、.claude/settings.local.json 与 --add-dir 的加载边界。' },
        { title: 'MCP 服务器', tag: '工具', tone: 'info', body: '区分用户级与项目级 MCP；项目 .mcp.json 应明确启用范围，敏感服务不要默认批准。' },
        { title: 'Hooks / Status Line', tag: '脚本', tone: 'info', body: 'Hooks 和 statusLine 会执行本地命令，适合放在这里做安全检查与诊断入口。' },
      ],
    },
    hermes: {
      heading: 'Agent 运行时治理',
      summary: 'Hermes 是带 skills、tools、memory、sessions 的 agent 产品，高级设置应帮助治理长期运行状态和工具负载。',
      focus: [
        { title: '配置体检 / 迁移', tag: '体检', tone: 'ok', body: '优先使用 hermes config check、doctor、migrate 处理缺失项和版本演进，不在这里复制账号表单。' },
        { title: 'Skills / Tools', tag: '能力', tone: 'warn', body: '控制 skills 与 toolsets 的默认启用范围，避免每个会话加载过多工具描述。' },
        { title: 'Sessions / Memory', tag: '记忆', tone: 'info', body: '关注 session 续接、memory 索引、压缩和检索配置，服务长任务与多 agent 协作。' },
        { title: 'Workspace / Runtime', tag: '运行', tone: 'info', body: '检查 worktree、terminal backend、browser 与 gateway timeout，保证 agent 能稳定执行。' },
      ],
    },
    generic_client: {
      heading: '兼容客户端模板',
      summary: '通用客户端只保留 OpenAI-compatible 环境模板、连通检查与账号管理入口，避免伪装成完整产品配置页。',
      focus: [
        { title: '环境模板', tag: '模板', tone: 'ok', body: '生成最小可用的环境变量模板；具体 Key、Provider、模型和端点仍由账号管理维护。' },
        { title: '会话边界', tag: '隔离', tone: 'info', body: '建议按终端会话或项目目录加载 env，避免全局 shell 污染多个客户端。' },
        { title: '兼容检查', tag: '协议', tone: 'info', body: '只检查 OpenAI-compatible 请求形状与认证变量，不承担特定客户端的工具权限治理。' },
      ],
    },
  };
  return specs[kind] || specs.generic_client;
}

function configClientAdvancedCommands(kind) {
  const port = currentConfig?.port || 4446;
  const host = serviceHostForClientUrl(currentConfig?.host || '127.0.0.1');
  if (kind === 'claude_code') return [
    { label: '健康检查', command: 'claude doctor' },
    { label: 'MCP 列表', command: 'claude mcp list' },
    { label: '安全模式试跑', command: 'claude --permission-mode plan --setting-sources user,project' },
  ];
  if (kind === 'hermes') return [
    { label: '配置检查', command: 'hermes config check' },
    { label: '运行体检', command: 'hermes doctor' },
    { label: '迁移补全', command: 'hermes config migrate' },
    { label: '能力列表', command: 'hermes skills list && hermes tools list' },
  ];
  return [
    { label: '最小模板', command: `export OPENAI_BASE_URL=http://${host}:${port}/v1\nexport OPENAI_API_KEY=<your-key>\nexport OPENAI_MODEL=<model-name>` },
    { label: '当前变量', command: "env | grep -E '^(OPENAI|ANTHROPIC|DEECODEX)_'" },
  ];
}

function claudeFilterAccounts() {
  return configClientAccounts('claude_code').filter(account => account?.id);
}

function claudeAccountSurface(account) {
  const raw = String(account?.client_surface || account?.client_options?.client_surface || 'cli').toLowerCase();
  return raw === 'desktop' ? 'desktop' : 'cli';
}

function claudeSurfaceSummary(accounts) {
  const surfaces = new Set((accounts || []).map(claudeAccountSurface));
  if (surfaces.has('cli') && surfaces.has('desktop')) return 'CLI / 桌面版';
  if (surfaces.has('desktop')) return '桌面版';
  if (surfaces.has('cli')) return 'CLI';
  return '未接入';
}

function claudeCustomFilterState(accountsOrAccount) {
  const accounts = Array.isArray(accountsOrAccount)
    ? accountsOrAccount
    : (accountsOrAccount ? [accountsOrAccount] : []);
  const source = accounts.find(account => {
    const opts = account?.client_options || {};
    return Boolean(opts.claude_custom_filter_enabled)
      || Array.isArray(opts.claude_custom_filter_rules)
      || Object.prototype.hasOwnProperty.call(opts, 'claude_cch_filter_enabled');
  }) || accounts[0] || null;
  const opts = source?.client_options || {};
  const rules = Array.isArray(opts.claude_custom_filter_rules)
    ? opts.claude_custom_filter_rules
        .map(rule => String(rule || '').trim())
        .filter(Boolean)
    : [];
  return {
    cchEnabled: accounts.length
      ? accounts.every(account => (account?.client_options || {}).claude_cch_filter_enabled !== false)
      : true,
    enabled: accounts.some(account => Boolean((account?.client_options || {}).claude_custom_filter_enabled)),
    rules,
  };
}

function renderClaudeCustomFilterSection(accounts) {
  const targets = Array.isArray(accounts) ? accounts : (accounts ? [accounts] : []);
  const state = claudeCustomFilterState(targets);
  const disabled = targets.length ? '' : ' disabled';
  const ruleText = state.rules.join('\n');
  return `<section class="config-client-section">
    <div class="config-section-header">
      <span class="section-icon">⌬</span>
      <h3>自定义过滤</h3>
      <span class="section-desc">${esc(claudeSurfaceSummary(targets))} · Anthropic system 行过滤</span>
    </div>
    <div class="config-filter-panel${targets.length ? '' : ' disabled'}">
      <div class="config-filter-head">
        <label class="config-filter-toggle">
          <input type="checkbox" id="claudeCchFilterEnabled" ${state.cchEnabled ? 'checked' : ''}${disabled}>
          <span>启用 cch 过滤</span>
        </label>
        <button type="button" class="btn btn-ghost" onclick="saveClaudeCustomFilters()"${disabled}>保存过滤设置</button>
      </div>
      <div class="config-filter-head secondary">
        <label class="config-filter-toggle">
          <input type="checkbox" id="claudeCustomFilterEnabled" ${state.enabled ? 'checked' : ''}${disabled}>
          <span>启用自定义过滤规则</span>
        </label>
      </div>
      <div class="config-filter-example">
        <span>每行一个匹配片段</span>
        <code>x-custom-cache-noise:</code>
        <code>session_fingerprint:</code>
      </div>
      <textarea id="claudeCustomFilterRules" spellcheck="false" aria-label="Claude 自定义过滤规则"${disabled}>${esc(ruleText)}</textarea>
      <p>作用于 Claude CLI 和 Claude 桌面版通过 Anthropic Messages 代理发送的顶层 <code>system</code> 文本；命中的整行会在转发前移除。内置已处理 <code>x-anthropic-billing-header</code> + <code>cch=</code>。</p>
      ${targets.length ? `<div class="config-filter-example"><span>同步 ${targets.length} 个 Claude 代理账号</span></div>` : '<div class="config-client-empty">暂无 Claude 账号。请先在账号管理中添加或扫描 Claude CLI / 桌面版账号，再保存过滤规则。</div>'}
    </div>
  </section>`;
}

function configClaudeDesktopAccount() {
  return configClientAccounts('claude_code').find(account => {
    const surface = String(account?.client_surface || account?.client_options?.client_surface || 'cli').toLowerCase();
    return surface === 'desktop';
  }) || null;
}

function renderClaudeDesktopDeveloperModeSection() {
  const account = configClaudeDesktopAccount();
  return `<section class="config-client-section claude-dev-mode-section">
    <div class="config-section-header">
      <span class="section-icon">⌘</span>
      <h3>桌面开发者模式</h3>
      <span class="section-desc">Claude Desktop</span>
    </div>
    <div class="config-filter-panel">
      <div class="config-filter-head">
        <label class="config-filter-toggle claude-dev-mode-toggle">
          <input type="checkbox" id="claudeDesktopDevModeSwitch" onchange="toggleClaudeDesktopDeveloperMode('${escAttr(account?.id || '')}', this.checked)">
          <span id="claudeDesktopDevModeLabel">读取中...</span>
        </label>
        <button type="button" class="btn btn-ghost" onclick="refreshClaudeDesktopDeveloperMode()">刷新</button>
        <button type="button" class="btn btn-ghost" id="claudeDesktopRestartBtn" onclick="restartClaudeDesktopForDevMode()" disabled>重启 Claude</button>
      </div>
      <p id="claudeDesktopDevModeHint">写入 Claude Desktop 的 <code>developer_settings.json</code>，只修改 <code>allowDevTools</code>。</p>
    </div>
  </section>`;
}

async function saveClaudeCustomFilters() {
  const accounts = claudeFilterAccounts();
  if (!accounts.length) {
    showToast('请先添加 Claude 账号', 'error');
    return;
  }
  const cchEnabled = Boolean(document.getElementById('claudeCchFilterEnabled')?.checked);
  const enabled = Boolean(document.getElementById('claudeCustomFilterEnabled')?.checked);
  const rules = (document.getElementById('claudeCustomFilterRules')?.value || '')
    .split(/\r?\n/)
    .map(rule => rule.trim())
    .filter(Boolean);
  const updates = accounts.map(account => {
    const next = JSON.parse(JSON.stringify(account));
    next.client_options = next.client_options || {};
    next.client_options.claude_cch_filter_enabled = cchEnabled;
    next.client_options.claude_custom_filter_enabled = enabled;
    if (rules.length) {
      next.client_options.claude_custom_filter_rules = rules;
    } else {
      delete next.client_options.claude_custom_filter_rules;
    }
    delete next._client_status_report;
    return next;
  });
  try {
    await Promise.all(updates.map(account => invoke('update_account', { accountJson: JSON.stringify(account) })));
    showToast(`Claude 自定义过滤已同步 ${updates.length} 个账号`, 'success');
    await loadAccountsData();
    renderPanel('config');
  } catch (err) {
    showToast('保存 Claude 过滤规则失败: ' + err, 'error');
  }
}

function renderConfigClientFocusRows(spec) {
  return `<section class="config-client-section">
    <div class="config-section-header">
      <span class="section-icon">◇</span>
      <h3>高级治理项</h3>
      <span class="section-desc">${spec.focus.length} 项</span>
    </div>
    <div class="config-client-focus-list">
      ${spec.focus.map(item => `<div class="config-client-focus-row">
        <div>
          <span class="config-focus-tag ${escAttr(item.tone || 'info')}">${esc(item.tag || '建议')}</span>
          <strong>${esc(item.title)}</strong>
        </div>
        <p>${esc(item.body)}</p>
      </div>`).join('')}
    </div>
  </section>`;
}

function renderConfigCommandStrip(kind) {
  const commands = configClientAdvancedCommands(kind);
  if (!commands.length) return '';
  return `<section class="config-client-section">
    <div class="config-section-header">
      <span class="section-icon">⌁</span>
      <h3>原生命令</h3>
      <span class="section-desc">复制到终端执行</span>
    </div>
    <div class="config-command-list">
      ${commands.map(item => `<div class="config-command-row">
        <span>${esc(item.label)}</span>
        <code>${esc(item.command)}</code>
        <button type="button" onclick="copyConfigCommand(this, '${escAttr(escapeJsString(item.command))}')">复制</button>
      </div>`).join('')}
    </div>
  </section>`;
}

function renderConfigRuntimeNotice() {
  const status = window._statusData || {};
  if (!status.running) return '';
  const runningHost = normalizeServiceHost(status.host || currentConfig?.host || '127.0.0.1');
  const configuredHost = normalizeServiceHost(currentConfig?.host || '127.0.0.1');
  const runningPort = String(status.port || '');
  const configuredPort = String(currentConfig?.port || '');
  const preferredPort = String(status.preferred_port || status.port || '');
  const configuredPortAuto = currentConfig?.port_auto !== false;
  const runningPortAuto = status.port_auto !== false;
  const changed = runningHost !== configuredHost
    || preferredPort !== configuredPort
    || configuredPortAuto !== runningPortAuto;
  const autoSelected = Boolean(status.auto_selected) && runningPort !== preferredPort;
  const runningEndpoint = `${runningHost.includes(':') ? `[${runningHost}]` : runningHost}:${runningPort || '—'}`;
  const configuredEndpoint = `${configuredHost.includes(':') ? `[${configuredHost}]` : configuredHost}:${configuredPort || '—'}`;
  if (!changed && autoSelected) {
    return `<div class="config-runtime-notice">
      <strong>端口已自动切换</strong>
      <span>${esc(status.port_selection_reason || `首选端口 ${preferredPort} 被占用，当前自动使用 ${runningPort}`)}。配置仍保留首选端口，网关下次启动会重新尝试。</span>
    </div>`;
  }
  return `<div class="config-runtime-notice ${changed ? 'warn' : ''}">
    <strong>${changed ? '服务地址待重启' : '运行中配置提示'}</strong>
    <span>${changed
      ? `当前监听 ${runningEndpoint}，保存配置为 ${configuredEndpoint}；重启网关后切换到新入口。`
      : '修改服务地址或端口后需要重启网关；运行中的客户端写入仍使用当前真实监听地址。'}</span>
  </div>`;
}

async function copyConfigCommand(btn, command) {
  try {
    if (typeof navigator === 'undefined' || !navigator.clipboard?.writeText) throw new Error('剪贴板不可用');
    await navigator.clipboard.writeText(command);
    const oldText = btn?.textContent || '复制';
    if (btn) {
      btn.textContent = '已复制';
      setTimeout(() => { btn.textContent = oldText; }, 1200);
    }
    showToast('命令已复制', 'success');
  } catch (err) {
    showToast('复制失败: ' + err, 'error');
  }
}

function renderClientConfigOverview(kind) {
  const profile = configClientProfiles().find(item => normalizeConfigClientKind(item.slug || item.kind) === kind) || {};
  const accounts = configClientAccounts(kind);
  const account = configPrimaryClientAccount(kind);
  const claudeAccounts = kind === 'claude_code' ? claudeFilterAccounts() : [];
  const spec = configClientAdvancedSpec(kind);
  return `
    <section class="config-client-overview">
      <div class="config-client-hero">
        <div class="config-client-title">
          ${configClientIcon(kind)}
          <div>
            <h3>${esc(configClientLabel(kind, profile))}</h3>
          </div>
        </div>
      </div>
      ${accounts.length ? '' : `<div class="config-client-empty">暂无${esc(configClientLabel(kind, profile))}账号。账号、密钥、模型和端点请在账号管理中维护。</div>`}
    </section>
    ${renderConfigClientFocusRows(spec)}
    ${kind === 'claude_code' ? renderClaudeDesktopDeveloperModeSection() : ''}
    ${kind === 'claude_code' ? renderClaudeCustomFilterSection(claudeAccounts) : ''}
    ${renderConfigCommandStrip(kind)}`;
}

function afterRenderConfigPanel() {
  if (selectedConfigClientKind !== 'claude_code') return;
  if (typeof refreshClaudeDesktopDeveloperMode === 'function') {
    refreshClaudeDesktopDeveloperMode();
  }
}

function renderConfig() {
  if (!currentConfig) {
    return `
      <div class="page-header">
        <h2>高级设置</h2>
        <p>正在加载配置...</p>
      </div>`;
  }

  selectedConfigClientKind = normalizeConfigClientKind(selectedConfigClientKind);
  syncConfigSaveVisibility();
  const canEdit = configCanEditSelected();
  let html = `
    <div class="config-page-shell">
      <div class="page-header config-page-header">
        <div>
          <h2>高级设置</h2>
        </div>
      </div>
      ${renderConfigClientSwitcher()}`;

  if (selectedConfigClientKind === 'codex') {
    html += renderCodexAdvancedSettings();
  } else {
    if (selectedConfigClientKind === 'global') {
      html += renderConfigRuntimeNotice();
    }
    html += canEdit ? renderEditableConfigSections(selectedConfigClientKind) : renderClientConfigOverview(selectedConfigClientKind);
  }

  if (canEdit) {
    html += `
    <div class="config-actions">
      <button class="btn btn-primary" id="btnSave" onclick="saveConfig()">保存配置</button>
      <button class="btn btn-ghost" id="btnValidate" onclick="validateConfig()">验证配置</button>
      <span id="configMsg" style="font-family:var(--font-mono);font-size:11px;color:var(--text-muted);align-self:center;margin-left:8px;"></span>
    </div>`;
  }

  return html + '</div>';
}

function renderField(f) {
  const val = currentConfig[f.key] !== undefined ? currentConfig[f.key] : '';
  const layout = f.layout || ((f.type === 'json' || f.type === 'textarea') ? 'wide' : 'half');
  const layoutClass = ` ${layout}`;

  let inputHtml = '';
  switch (f.type) {
    case 'password':
      inputHtml = `
        <div class="pass-group">
          <input type="password" id="field_${f.key}" name="${f.key}" value="${escAttr(String(val))}" placeholder="${escAttr(f.placeholder || '')}" autocomplete="off">
          <button type="button" class="pass-toggle" onclick="togglePass('field_${f.key}', this)" title="显示/隐藏" aria-label="显示或隐藏">
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M2.5 12s3.5-6 9.5-6 9.5 6 9.5 6-3.5 6-9.5 6-9.5-6-9.5-6z"></path><circle cx="12" cy="12" r="2.5"></circle></svg>
          </button>
        </div>`;
      break;
    case 'number':
      const step = f.step || (f.key.includes('ratio') ? '0.1' : '1');
      inputHtml = `<input type="number" id="field_${f.key}" name="${f.key}" value="${escAttr(String(val))}" min="${f.min ?? ''}" max="${f.max ?? ''}" step="${step}" placeholder="${escAttr(f.placeholder || '')}">`;
      break;
    case 'checkbox':
      inputHtml = `
        <div class="check-row">
          <input type="checkbox" id="field_${f.key}" name="${f.key}" ${val === true || val === 'true' ? 'checked' : ''}>
          <label for="field_${f.key}">${esc(f.label)}</label>
        </div>`;
      break;
    case 'select':
      const opts = (f.options || []).map(o => {
        const label = (f.optionLabels && f.optionLabels[o]) || o;
        return `<option value="${escAttr(o)}" ${String(val) === o ? 'selected' : ''}>${esc(label)}</option>`;
      }).join('');
      inputHtml = `<select id="field_${f.key}" name="${f.key}">${opts}</select>`;
      break;
    case 'json':
      inputHtml = `<textarea id="field_${f.key}" name="${f.key}" placeholder="${escAttr(f.placeholder || '{}')}" spellcheck="false">${esc(typeof val === 'object' ? JSON.stringify(val, null, 2) : String(val))}</textarea>`;
      break;
    default:
      inputHtml = `<input type="text" id="field_${f.key}" name="${f.key}" value="${escAttr(String(val))}" placeholder="${escAttr(f.placeholder || '')}">`;
  }

  if (f.type === 'checkbox') {
    return `<div class="config-field checkbox-field${layoutClass}">${inputHtml}<span class="hint">${esc(f.hint)}</span></div>`;
  }

  return `
    <div class="config-field${layoutClass}">
      <label for="field_${f.key}">${esc(f.label)}</label>
      ${inputHtml}
      <span class="hint">${esc(f.hint)}</span>
    </div>`;
}

// ═══════════════════════════════════════════════════════════════
// 诊断面板
// ═══════════════════════════════════════════════════════════════
function renderDiagnostics() {
  const report = window._diagData;
  let body = '';
  if (!report || !report.groups) {
    body = '<div class="diag-empty">尚未运行诊断。点击下方按钮验证当前配置。</div>';
  } else {
    const s = report.summary;
    const hlabels = { healthy: '正常', degraded: '降级', broken: '异常' };
    body = `
      <div class="diag-summary">
        <div class="diag-summary-bar">
          <span class="stat pass"><span class="n">${s.pass}</span><span class="l">通过</span></span>
          <span class="stat warn"><span class="n">${s.warn}</span><span class="l">警告</span></span>
          <span class="stat fail"><span class="n">${s.fail}</span><span class="l">失败</span></span>
          <span class="stat info"><span class="n">${s.info}</span><span class="l">提示</span></span>
        </div>
        <span class="health-badge ${s.health}">${hlabels[s.health] || s.health}</span>
      </div>
      ${report.groups.map(g => {
        return `
        <div class="diag-group">
          <div class="diag-group-header ${g.health}">
            <span class="group-icon" aria-hidden="true"></span> ${esc(g.name)}
          </div>
          ${g.items.map(it => `
            <div class="diag-item">
              <span class="item-icon ${it.status}" aria-hidden="true"></span>
              <div class="item-body">
                <div class="item-name">${esc(it.check_name)}</div>
                <div class="item-msg">${esc(it.message)}</div>
                ${it.detail ? '<div class="item-detail">' + esc(it.detail) + '</div>' : ''}
                ${it.suggestion ? '<div class="item-suggestion">' + esc(it.suggestion) + '</div>' : ''}
              </div>
            </div>
          `).join('')}
        </div>`;
      }).join('')}
    `;
  }

  return `
    <div class="page-header">
      <h2>执行诊断</h2>
    </div>
    <div class="diag-header">
      <button class="btn btn-primary" id="btnValidateDiag" onclick="validateConfig()">运行诊断</button>
    </div>
    ${body}
  `;
}

// ═══════════════════════════════════════════════════════════════
// 帮助面板
// ═══════════════════════════════════════════════════════════════
function renderHelp() {
  return `
    ${renderHelpHeader()}
    ${renderHelpToc()}
    ${renderHelpHero()}
    ${renderHelpVisual()}
    ${renderHelpBefore()}
    ${renderHelpModal()}
    ${renderHelpAfter()}
    ${renderHelpPick()}
    ${renderHelpRelay()}
    ${renderHelpApi()}
    ${renderHelpSmart()}
    ${renderHelpAccounts()}
    ${renderHelpCodex()}
    ${renderHelpTroubleshoot()}
    ${renderHelpNavmap()}
    ${renderHelpGlossary()}
    ${renderHelpAdvanced()}
    ${renderHelpSupport()}
  `;
}

function renderHelpHeader() {
  return `

  <div class="page-header help-guide-header">
    <h2>使用帮助</h2>
    <p>给第一次使用 DEX AI + Codex 桌面的用户看的图文教程。使用帮助只负责教你；真正配置入口在「服务概览」里的「Codex 桌面」快捷配置。</p>
  </div>
  `;
}

function renderHelpToc() {
  return `
  <div class="help-toc help-guide-toc">
        <a onclick="document.getElementById('h-visual').scrollIntoView({behavior:'smooth'})">动画演示</a>
        <a onclick="document.getElementById('h-before').scrollIntoView({behavior:'smooth'})">配置前检查</a>
        <a onclick="document.getElementById('h-modal').scrollIntoView({behavior:'smooth'})">弹窗怎么填</a>
        <a onclick="document.getElementById('h-after').scrollIntoView({behavior:'smooth'})">怎么验证</a>
        <a onclick="document.getElementById('h-pick').scrollIntoView({behavior:'smooth'})">选模式</a>
        <a onclick="document.getElementById('h-relay').scrollIntoView({behavior:'smooth'})">中转模式</a>
        <a onclick="document.getElementById('h-api').scrollIntoView({behavior:'smooth'})">API 模式</a>
        <a onclick="document.getElementById('h-smart').scrollIntoView({behavior:'smooth'})">智能路由</a>
        <a onclick="document.getElementById('h-codex').scrollIntoView({behavior:'smooth'})">Codex 里怎么选</a>
        <a onclick="document.getElementById('h-accounts').scrollIntoView({behavior:'smooth'})">账号/渠道</a>
        <a onclick="document.getElementById('h-troubleshoot').scrollIntoView({behavior:'smooth'})">出错怎么办</a>
        <a onclick="document.getElementById('h-navmap').scrollIntoView({behavior:'smooth'})">左侧导航</a>
        <a onclick="document.getElementById('h-glossary').scrollIntoView({behavior:'smooth'})">名词解释</a>
        <a onclick="document.getElementById('h-advanced').scrollIntoView({behavior:'smooth'})">高级说明</a>
        <a onclick="document.getElementById('h-support').scrollIntoView({behavior:'smooth'})">支持项目</a>
      </div>
  `;
}

function renderHelpHero() {
  return `
  <div class="help-guide-hero">
        <div class="help-guide-hero-copy">
          <span class="help-guide-kicker">新手结论</span>
          <h3>不知道选哪个？先选「中转模式」</h3>
          <p>你只要有中转 API Key，就能在「服务概览 → 右键 Codex 桌面图标 → 快捷配置」里填进去开始用。不要在使用帮助页里找配置按钮。</p>
          <div class="help-guide-hero-actions">
            <span>第一步：进入服务概览</span>
            <span>第二步：右键 Codex 桌面图标，点「快捷配置」</span>
            <span>第三步：填 Key 后去 Codex 测试</span>
          </div>
        </div>
        <div class="help-guide-mode-result">
          <div class="help-hero-picture" aria-hidden="true">
            <div class="hero-codex-orb"><img src="assets/provider-logos/codex.png" alt=""></div>
            <div class="hero-token-card"><span>Token</span><b>余额</b></div>
            <div class="hero-flow-line"></div>
            <div class="hero-key-chip">API Key</div>
          </div>
          <small>默认推荐</small>
          <strong>中转模式</strong>
          <span>最少配置 · 最适合小白</span>
        </div>
      </div>
  `;
}

function renderHelpVisual() {
  return `
  <div class="help-section" id="h-visual">
        <h3>一张动图看懂整个流程</h3>
        <div class="help-guide-stage" aria-label="DEX AI 快速配置到 Codex 桌面的动画示意">
          <div class="guide-screen guide-screen-dex">
            <div class="guide-window-dots"><i></i><i></i><i></i></div>
            <aside class="guide-sidebar">
              <b>DEX AI</b>
              <span class="active">服务概览</span>
              <span>账号管理</span>
              <span>高级设置</span>
              <span>执行诊断</span>
              <span>使用帮助</span>
              <em>服务运行中</em>
            </aside>
            <main class="guide-main-panel">
              <div class="guide-panel-title">客户端启动</div>
              <div class="guide-client-dock-demo">
                <div class="guide-client-dock-icon">
                  <img src="assets/provider-logos/codex.png" alt="Codex 桌面图标">
                  <span class="guide-surface-badge">▭</span>
                  <span class="guide-right-click-hint">右键</span>
                </div>
                <div class="guide-client-dock-dots"><i></i><i></i><i></i></div>
                <div class="guide-client-dock-label">Codex 桌面</div>
              </div>
              <div class="guide-inline-tip">真实入口长这样：找「Codex 桌面」图标，右键图标弹出「快捷配置」菜单。</div>
              <div class="guide-panel-title small">开始使用 Codex Desktop</div>
              <div class="guide-mode-row">
                <div class="guide-mode-pill">API 模式</div>
                <div class="guide-mode-pill selected">中转模式</div>
                <div class="guide-mode-pill muted">智能路由</div>
              </div>
              <label>中转 Base URL</label>
              <div class="guide-input">https://api.dexai.vip/v1</div>
              <label>中转 API Key</label>
              <div class="guide-input secret">••••••••••••••••</div>
              <button>完成配置</button>
            </main>
            <div class="guide-cursor"></div>
            <div class="guide-number n1">1</div>
            <div class="guide-number n2">2</div>
            <div class="guide-number n3">3</div>
          </div>
          <div class="guide-flow-arrow">→</div>
          <div class="guide-screen guide-screen-codex">
            <div class="guide-window-dots"><i></i><i></i><i></i></div>
            <div class="guide-codex-title">Codex 桌面</div>
            <div class="guide-model-select">
              <span>模型</span>
              <strong>gpt-5.5</strong>
            </div>
            <div class="guide-chat-line user">你好，帮我写一个测试函数</div>
            <div class="guide-chat-line assistant">DEX AI 已转发到可用渠道并返回结果。</div>
            <div class="guide-number n4">4</div>
          </div>
        </div>
        <div class="help-guide-caption-grid">
          <div><b>1</b><span>在 DEX AI 左侧进入「服务概览」。</span></div>
          <div><b>2</b><span>找到「Codex 桌面」图标，右键图标弹出「快捷配置」菜单。</span></div>
          <div><b>3</b><span>弹窗里小白默认选「中转模式」，填 URL 和 API Key。</span></div>
          <div><b>4</b><span>去 Codex 桌面选 GPT 模型，发一句测试。</span></div>
        </div>
        <div class="guide-entry-map" aria-label="快速配置入口定位图">
          <div class="entry-map-card nav"><small>左侧导航</small><b>服务概览</b><span>不是「使用帮助」</span></div>
          <i>→</i>
          <div class="entry-map-card dock"><small>页面区域</small><b>客户端启动</b><span>找蓝紫色 Codex 图标</span></div>
          <i>→</i>
          <div class="entry-map-card action"><small>触发方式</small><b>右键图标菜单</b><span class="entry-mini-action">右键</span></div>
        </div>
      </div>
  `;
}

function renderHelpBefore() {
  return `
  <div class="help-section" id="h-before">
        <h3>配置前先检查这 3 件事</h3>
        <div class="guide-check-grid">
          <article>
            <b>1. 服务状态</b>
            <span>看左下角：绿色「服务运行中」才说明 DEX AI 网关正在工作。红色「服务已停止」时，Codex 连接会失败。</span>
          </article>
          <article>
            <b>2. 配置入口</b>
            <span>配置入口不是使用帮助页。去「服务概览」→「客户端启动」→ 右键「Codex 桌面」图标 → 点「快捷配置」。</span>
          </article>
          <article>
            <b>3. 不要手改配置</b>
            <span>新手不要先改 <code>~/.codex/config.toml</code>。快速配置会自动写 Codex 配置，手改容易把模式弄乱。</span>
          </article>
        </div>
        <div class="guide-warning-line">切换 API / 中转 / 智能路由后，网关可能会停止以应用新配置；看到提示后重新启动 DEX AI，再重新打开 Codex 桌面。</div>
      </div>
  `;
}

function renderHelpModal() {
  return `
  <div class="help-section" id="h-modal">
        <h3>快速配置弹窗每一项怎么填？</h3>
        <div class="guide-field-map">
          <div><b>账号名称</b><span>只是给你自己看的名字。新手可以不改，例如「DEX AI 中转」或「DeepSeek 桌面版」。</span></div>
          <div><b>默认模型</b><span>新手不用改。中转模式会固定使用 Codex 内置 GPT 模型；API 模式会用你账号能访问的模型。</span></div>
          <div><b>中转 Base URL</b><span>填中转服务地址，例如 <code>https://api.dexai.vip/v1</code>。不要填 Codex 官网地址。</span></div>
          <div><b>DeepSeek Base URL</b><span>API 模式下通常保持默认。除非你明确知道自己的兼容接口地址。</span></div>
          <div><b>API Key</b><span>填你购买 Token 或模型服务后拿到的 Key。它不是 Codex 登录密码，也不是邮箱验证码。</span></div>
          <div><b>粘贴中转</b><span>如果你复制的是 newapi_channel_conn 这类连接信息，用它自动识别 URL 和 Key。</span></div>
          <div><b>获取模型</b><span>API 模式用于读取账号可用模型。中转模式通常不用点，因为走 Codex 内置 GPT 模型名。</span></div>
          <div><b>完成配置</b><span>保存账号/渠道，写入 Codex 配置，并尝试启动服务。完成后去 Codex 桌面测试。</span></div>
        </div>
      </div>
  `;
}

function renderHelpAfter() {
  return `
  <div class="help-section" id="h-after">
        <h3>完成配置后，怎么知道成功了？</h3>
        <div class="guide-success-flow">
          <div><b>看 DEX AI</b><span>左下角显示「服务运行中」。如果刚切换模式，先重启服务。</span></div>
          <span>→</span>
          <div><b>打开 Codex 桌面</b><span>中转模式选 <code>gpt-5.5</code> 这类 GPT 模型；API 模式选「账号名 / 模型名」。</span></div>
          <span>→</span>
          <div><b>发测试消息</b><span>例如：「你好，用一句话回复我」。能正常返回就是链路通了。</span></div>
          <span>→</span>
          <div><b>看请求历史</b><span>请求历史里能看到模式、模型、状态码和上游返回，便于确认是否真的走 DEX AI。</span></div>
        </div>
      </div>
  `;
}

function renderHelpPick() {
  return `
  <div class="help-section" id="h-pick">
        <h3>先看你手里有什么，再选模式</h3>
        <div class="help-guide-picker">
          <article class="guide-pick-card primary">
            <div class="guide-pick-icon">🧾</div>
            <h4>我买了 DEX AI Token / 有中转 API Key</h4>
            <p>选 <strong>中转模式</strong>。这是给新手准备的最短路径。</p>
            <ul>
              <li>你只需要填中转 URL 和 Key。</li>
              <li>Codex 里继续选 <code>gpt-5.5</code> 这类模型名。</li>
              <li>不用管账号池、登录态、路由这些高级概念。</li>
            </ul>
          </article>
          <article class="guide-pick-card">
            <div class="guide-pick-icon">🔑</div>
            <h4>我自己有 DeepSeek / MiniMax / OpenAI 兼容 Key</h4>
            <p>选 <strong>API 模式</strong>。DEX AI 会把 Codex 请求转给你自己的模型账号。</p>
            <ul>
              <li>你需要知道上游 Base URL。</li>
              <li>需要在账号管理里刷新模型列表。</li>
              <li>Codex 里会看到「账号名 / 模型名」。</li>
            </ul>
          </article>
          <article class="guide-pick-card advanced">
            <div class="guide-pick-icon">🧠</div>
            <h4>我已经登录 Codex 桌面，还要保留官方工具能力</h4>
            <p>选 <strong>智能路由</strong>。这不是入门模式，是给知道自己要做账号池和故障切换的人。</p>
            <ul>
              <li>需要官方 Codex 登录态。</li>
              <li>需要配置可执行账号和 Pool。</li>
              <li>Computer Use 等原生工具会影响路由选择。</li>
            </ul>
          </article>
        </div>
      </div>
  `;
}

function renderHelpRelay() {
  return `
  <div class="help-section" id="h-relay">
        <h3>中转模式：新手照着填</h3>
        <div class="guide-mode-tutorial relay">
          <div class="guide-mode-intro">
            <span class="guide-mode-badge">推荐新手</span>
            <h4>把 Codex 接到一个中转渠道</h4>
            <p>你可以把它理解成：Codex 负责界面，DEX AI 负责把请求送到你买的 Token / 中转服务。</p>
          </div>
          <div class="guide-steps-list">
            <div><b>01</b><strong>进入「服务概览」</strong><span>左侧导航点服务概览，不是在使用帮助页里配置。</span></div>
            <div><b>02</b><strong>认准「Codex 桌面」图标并右键打开菜单</strong><span>在客户端启动区域找蓝紫色 Codex 图标，下面写着「Codex 桌面」。右键图标弹出菜单，点「快捷配置」即可。</span></div>
            <div><b>03</b><strong>弹窗里选择「中转模式」</strong><span>看到 API / 中转 / 智能路由三张卡，点中转模式。</span></div>
            <div><b>04</b><strong>填写 URL 和 API Key</strong><span>Base URL 填中转地址；API Key 填你购买 Token 后拿到的 Key，不要填 Codex 账号密码。</span></div>
            <div><b>05</b><strong>点「完成配置」并测试</strong><span>打开 Codex 桌面，模型选 <code>gpt-5.5</code> 或同类 GPT 模型名，发一句「你好」。</span></div>
          </div>
          <div class="guide-do-dont">
            <div class="do"><b>该做</b><span>填中转 URL / Key；Codex 里选 GPT 模型名。</span></div>
            <div class="dont"><b>不要做</b><span>不要去手改 <code>~/.codex/config.toml</code>；不要在中转模式里找第三方账号模型。</span></div>
          </div>
        </div>
      </div>
  `;
}

function renderHelpApi() {
  return `
  <div class="help-section" id="h-api">
        <h3>API 模式：我有自己的模型 Key</h3>
        <div class="guide-mode-tutorial api">
          <div class="guide-mode-intro">
            <span class="guide-mode-badge">自带 Key</span>
            <h4>把 Codex 接到你自己的 DeepSeek / 兼容 API</h4>
            <p>适合你已经有模型服务账号，并希望明确使用某个账号、某个模型。</p>
          </div>
          <div class="guide-steps-list compact">
            <div><b>01</b><strong>进入服务概览并点「快速配置」</strong><span>在「Codex 桌面」图标右上角打开快速配置弹窗，再选择 API 模式。</span></div>
            <div><b>02</b><strong>填写 API Key</strong><span>DeepSeek Base URL 新手保持默认，只填 Key。</span></div>
            <div><b>03</b><strong>点击「获取模型」</strong><span>让 DEX AI 读取你这个账号能用的模型。</span></div>
            <div><b>04</b><strong>Codex 里选择账号模型</strong><span>你会看到类似「DeepSeek 桌面版 / deepseek-v4-pro」这样的模型项。</span></div>
          </div>
        </div>
      </div>
  `;
}

function renderHelpSmart() {
  return `
  <div class="help-section" id="h-smart">
        <h3>智能路由：高级用户再开</h3>
        <div class="guide-mode-tutorial smart">
          <div class="guide-mode-intro">
            <span class="guide-mode-badge">高级</span>
            <h4>让官方登录态做入口，模型执行交给账号池</h4>
            <p>这个模式不是“更强的新手模式”。它适合你明确需要官方 Codex 登录态、工具能力、账号池、优先级和故障切换。</p>
          </div>
          <div class="guide-smart-diagram">
            <div>Codex 官方登录态<br><small>保持入口 / 工具能力</small></div>
            <span>→</span>
            <div>DEX AI Router<br><small>按规则选择账号</small></div>
            <span>→</span>
            <div>执行账号池<br><small>DeepSeek / MiniMax / 其他</small></div>
          </div>
          <div class="guide-do-dont">
            <div class="do"><b>打开前确认</b><span>Codex 桌面已登录；账号管理里有可执行账号；Pool / 权重 / 优先级你知道怎么配。</span></div>
            <div class="dont"><b>小白先别开</b><span>如果你只是想“能用”，智能路由会增加排障复杂度。</span></div>
          </div>
        </div>
      </div>
  `;
}

function renderHelpAccounts() {
  return `
  <div class="help-section" id="h-accounts">
        <h3>账号和中转渠道在哪维护？</h3>
        <div class="guide-account-grid">
          <article>
            <h4>中转模式用户</h4>
            <p>去「账号管理」里看「中转渠道」。这里可以添加、测试、切换、删除中转 URL/API Key。</p>
            <ul>
              <li>看不到普通第三方账号是正常的；中转模式把它们隔离成渠道。</li>
              <li>渠道失败时，先点「测试中转渠道」，再看请求历史状态码。</li>
            </ul>
          </article>
          <article>
            <h4>API 模式用户</h4>
            <p>去「账号管理」→「添加账号」，填你的 DeepSeek/MiniMax/OpenAI 兼容 Key。</p>
            <ul>
              <li>添加后先测试上游连接。</li>
              <li>再刷新模型，最后应用这个账号。</li>
            </ul>
          </article>
          <article>
            <h4>智能路由用户</h4>
            <p>需要一个 Codex 官方登录态作为入口，还需要至少一个可执行账号参与模型执行。</p>
            <ul>
              <li>官方登录态默认只保持入口，不一定参与模型执行。</li>
              <li>Pool、优先级、权重用于高级调度；小白不用碰。</li>
            </ul>
          </article>
        </div>
      </div>
  `;
}

function renderHelpCodex() {
  return `
  <div class="help-section" id="h-codex">
        <h3>配置好以后，Codex 桌面里怎么选模型？</h3>
        <div class="guide-codex-table">
          <div class="head"><span>当前模式</span><span>Codex 里通常看到</span><span>小白怎么选</span></div>
          <div><span>中转模式</span><span><code>gpt-5.5</code> / <code>gpt-5.4-mini</code></span><span>直接选 GPT 模型名。</span></div>
          <div><span>API 模式</span><span>账号名 / 模型名</span><span>选你刚配置的账号模型。</span></div>
          <div><span>智能路由</span><span>GPT 模型名 + 账号模型</span><span>普通对话选 GPT；需要指定账号时选账号模型。</span></div>
        </div>
        <p class="guide-note">中转模式看不到第三方账号模型是正常的；它本来就是把 Codex 的 GPT 模型名映射到中转渠道。</p>
      </div>
  `;
}

function renderHelpTroubleshoot() {
  return `
  <div class="help-section" id="h-troubleshoot">
        <h3>出错先看这里</h3>
        <div class="guide-trouble-grid guide-trouble-grid-wide">
          <div><b>连接不上 DEX AI</b><span>先确认左下角是「服务运行中」。红点或服务已停止时，先回服务概览启动服务。</span></div>
          <div><b>model not found</b><span>中转模式选 GPT 模型名；API 模式先刷新模型；智能路由检查账号池有没有可执行账号。</span></div>
          <div><b>一直转圈</b><span>多半是 API Key、余额、Base URL 或中转渠道不可用。打开请求历史看状态码。</span></div>
          <div><b>401 / 403</b><span>通常是 API Key 错、Key 没权限或余额服务拒绝。重新复制 Key，不要带空格。</span></div>
          <div><b>404</b><span>通常是 Base URL 或模型名不对。中转地址要带 <code>/v1</code>，模型名按当前模式选择。</span></div>
          <div><b>429</b><span>请求太多或额度限制。等一会儿，或换一个可用渠道/账号。</span></div>
          <div><b>5xx / timeout</b><span>上游或中转服务异常。先测试中转渠道/账号上游连接。</span></div>
          <div><b>诊断颜色</b><span>绿色正常；黄色需要注意；红色必须处理。先修红色项，再回 Codex 测试。</span></div>
        </div>
      </div>
  `;
}

function renderHelpNavmap() {
  return `
  <div class="help-section" id="h-navmap">
        <h3>左侧导航速查</h3>
        <div class="guide-nav-map">
          <div><b>服务概览</b><span>启动/停止服务，客户端启动，右键「Codex 桌面」图标快捷配置。</span></div>
          <div><b>账号管理</b><span>添加 API 账号、中转渠道、测试上游、看余额和模型。</span></div>
          <div><b>高级设置</b><span>路由模式、自动注入、持久注入、智能守护等高级选项。</span></div>
          <div><b>执行诊断</b><span>配置失败或连不上时先跑诊断。</span></div>
          <div><b>请求历史</b><span>看每次请求的模型、状态码、错误和路由过程。</span></div>
          <div><b>线程聚合</b><span>整理 Codex/客户端线程，普通新手可以先跳过。</span></div>
          <div><b>插件市场</b><span>安装和管理插件，和首次接入 Codex 无关。</span></div>
          <div><b>DEX助手</b><span>用于排查问题、查询历史、辅助诊断，不是快速配置入口。</span></div>
        </div>
        <div class="guide-warning-line">如果你同时使用 CC Switch 等工具，请不要让两个工具同时改 Codex 配置；否则配置会互相覆盖。</div>
      </div>
  `;
}

function renderHelpGlossary() {
  return `
  <div class="help-section" id="h-glossary">
        <h3>小白名词解释</h3>
        <div class="guide-glossary guide-glossary-full">
          <div><b>服务概览</b><span>左侧第一个页面。真正的快捷配置入口在这里，不在使用帮助页。</span></div>
          <div><b>客户端启动</b><span>服务概览里的客户端图标区。Codex 桌面、Claude 等入口都在这里。</span></div>
          <div><b>Codex 桌面</b><span>蓝紫色 Codex 图标，下面写着「Codex 桌面」。右键图标菜单里的「快捷配置」即可配置。</span></div>
          <div><b>快捷配置</b><span>右键客户端图标弹出的菜单项。点击后弹出 API / 中转 / 智能路由三种模式。</span></div>
          <div><b>中转模式</b><span>最适合新手。你填中转 URL 和 API Key，Codex 里继续选 GPT 模型名。</span></div>
          <div><b>API 模式</b><span>适合自己有 DeepSeek、MiniMax 或其他兼容 API Key 的用户。</span></div>
          <div><b>智能路由</b><span>高级模式。需要 Codex 官方登录态和可执行账号池；小白先不要开。</span></div>
          <div><b>Token / 余额</b><span>你购买的可调用额度。余额不足时，Codex 可能一直转圈或返回上游错误。</span></div>
          <div><b>API Key</b><span>一串密钥，像门票。DEX AI 用它访问中转服务或模型服务。</span></div>
          <div><b>Base URL</b><span>服务地址。中转模式填中转地址，例如 <code>https://api.dexai.vip/v1</code>。</span></div>
          <div><b>模型名</b><span>Codex 下拉框里选的模型。中转模式常选 <code>gpt-5.5</code> 这类 GPT 名称。</span></div>
          <div><b>账号模型</b><span>API 模式里出现的「账号名 / 模型名」，表示指定某个账号去调用某个真实模型。</span></div>
          <div><b>登录态</b><span>Codex 桌面已经登录官方账号的状态。主要给智能路由使用。</span></div>
          <div><b>账号池 / Pool</b><span>多个执行账号放在一起，智能路由按优先级、权重、可用状态挑一个。</span></div>
          <div><b>执行诊断</b><span>出错时先点这里。它会检查服务、配置、账号、模型等常见问题。</span></div>
          <div><b>请求历史</b><span>查看每次请求走了哪个模式、哪个账号、返回了什么状态码。</span></div>
        </div>
      </div>
  `;
}

function renderHelpAdvanced() {
  return `
  <div class="help-section" id="h-advanced">
        <h3>高级说明：看不懂可以跳过</h3>
        <p>DEX AI 会自动维护 Codex 配置。当前活动 provider 统一叫 <code>dex_ai</code>，不同模式只改变内部地址：</p>
        <div class="guide-codex-table compact">
          <div class="head"><span>模式</span><span>内部地址</span><span>含义</span></div>
          <div><span>API 模式</span><span><code>/codex-desktop/v1</code></span><span>账号模型直选</span></div>
          <div><span>中转模式</span><span><code>/codex-relay/desktop/v1</code></span><span>中转渠道执行</span></div>
          <div><span>智能路由</span><span><code>/codex-router/v1</code></span><span>登录态 + 账号池路由</span></div>
        </div>
        <p class="guide-note">不要手动把 Codex 桌面指到旧的通用 <code>/v1</code>。如果你使用 CC Switch 等工具接管配置，请关闭 DEX AI 的自动注入和持久注入。</p>
      </div>
  `;
}

function renderHelpSupport() {
  return `
  <div class="help-section support-help-section" id="h-support">
        <h3>支持 DEX AI</h3>
        <p>DEX AI 会继续围绕 Codex 桌面、账号路由、客户端接入和本地工具链做维护。你可以自愿支持项目，帮助它保持更新。</p>
        <div class="support-help-panel">
          <div class="support-help-copy">
            <strong>请开发者喝杯咖啡</strong>
            <span>点击右侧按钮查看支付宝和微信收款码。</span>
          </div>
          <button type="button" class="btn btn-primary" onclick="showSupportProjectModal('help')">
            <span class="support-project-icon" aria-hidden="true"></span>
            <span>打开打赏码</span>
          </button>
        </div>
      </div>

  `;
}


// ═══════════════════════════════════════════════════════════════
// 表单工具
// ═══════════════════════════════════════════════════════════════
function togglePass(fieldId, btn) {
  const input = document.getElementById(fieldId);
  if (!input) return;
  if (input.type === 'password') {
    input.type = 'text';
    if (btn) {
      btn.title = '隐藏';
      btn.setAttribute('aria-label', '隐藏');
      btn.setAttribute('aria-pressed', 'true');
    }
  } else {
    input.type = 'password';
    if (btn) {
      btn.title = '显示';
      btn.setAttribute('aria-label', '显示');
      btn.setAttribute('aria-pressed', 'false');
    }
  }
}

function toggleContextWindowFields() {
  const cb = document.getElementById('edit_cw_enabled');
  const sf = document.getElementById('cwSizeField');
  if (cb && sf) {
    sf.style.display = cb.checked ? '' : 'none';
  }
}

function toggleVisionFields() {
  const mode = document.getElementById('edit_vision_mode');
  const vf = document.getElementById('visionFields');
  if (vf) {
    vf.style.display = mode && mode.value === 'glue' ? '' : 'none';
  }
}

function toggleReasoningFields() {
  const cb = document.getElementById('edit_reasoning_enabled');
  const rf = document.getElementById('reasoningFields');
  if (cb && rf) {
    rf.style.display = cb.checked ? '' : 'none';
    if (!cb.checked) {
      // 取消勾选时清空值
      const sel = document.getElementById('edit_reasoning_effort');
      if (sel) sel.value = '';
      const num = document.getElementById('edit_thinking_tokens');
      if (num) num.value = '';
    }
  }
}

function toggleFastFields() {
  const cb = document.getElementById('edit_fast_enabled');
  const ff = document.getElementById('fastFields');
  if (cb && ff) {
    ff.style.display = cb.checked ? '' : 'none';
    if (!cb.checked) {
      const tier = document.getElementById('edit_fast_service_tier');
      if (tier) tier.value = 'priority';
    }
  }
}

function collectFormData() {
  const data = currentConfig ? { ...currentConfig } : {};
  for (const sec of CONFIG_SECTIONS) {
    for (const f of sec.fields) {
      const el = document.getElementById('field_' + f.key);
      if (!el) continue;
      if (f.type === 'checkbox') {
        data[f.key] = el.checked;
      } else if (f.type === 'number') {
        const v = el.value.trim();
        if (v === '') {
          data[f.key] = null;
        } else if (f.key.includes('ratio') || f.step < 1) {
          data[f.key] = parseFloat(v);
        } else {
          data[f.key] = parseInt(v, 10);
        }
      } else if (f.type === 'json') {
        data[f.key] = el.value.trim() || '{}';
      } else {
        data[f.key] = el.value;
      }
    }
  }
  return data;
}

// ═══════════════════════════════════════════════════════════════
// Tauri IPC 调用
// ═══════════════════════════════════════════════════════════════
async function loadConfig() {
  try {
    currentConfig = await invoke('get_config');
    if (currentPanel === 'config') renderPanel('config');
  } catch (err) {
    showToast('加载配置失败: ' + err, 'error');
  }
}

async function loadStatus() {
  try {
    const todayStart = (() => {
      const now = new Date();
      return Math.floor(new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime() / 1000);
    })();
    const [status, cfg, tokenStats] = await Promise.all([
      invoke('get_service_status').catch(() => null),
      invoke('get_config').catch(() => null),
      invoke('get_request_stats_since', { since: todayStart }).catch(() => null),
    ]);

    window._statusData = {
      running: status?.running ?? false,
      host: status?.host || cfg?.host || '127.0.0.1',
      port: status?.port ?? '—',
      preferred_port: status?.preferred_port ?? status?.port ?? cfg?.port ?? '—',
      port_auto: status?.port_auto ?? cfg?.port_auto ?? true,
      auto_selected: status?.auto_selected ?? false,
      port_selection_reason: status?.port_selection_reason || null,
      uptime_secs: status?.running ? status.uptime_secs : 0,
      version: status?.version || '—',
      upstream: cfg ? cfg.upstream : '—',
      vision_enabled: cfg ? !!(cfg.vision_upstream && cfg.vision_api_key) : false,
      computer_executor: cfg ? cfg.computer_executor : 'disabled',
      chinese_thinking: cfg ? cfg.chinese_thinking : false,
      cdp_port: cfg ? cfg.cdp_port : 9222,
      codex_launch_with_cdp: cfg ? cfg.codex_launch_with_cdp : false,
      token_stats: tokenStats || {},
    };

    // 更新侧边栏连接指示器
    const dot = document.getElementById('connDot');
    const label = document.getElementById('connLabel');
    if (status?.running) {
      dot.className = 'indicator ok'; label.textContent = '服务运行中';
    } else {
      dot.className = 'indicator err'; label.textContent = '服务已停止';
    }

    if (currentPanel === 'status') {
      renderPanel('status');
      refreshStatusClientDock();
    }
  } catch (err) {
    window._statusData = { running: false, port: '—', uptime_secs: 0 };
    document.getElementById('connDot').className = 'indicator err';
    document.getElementById('connLabel').textContent = '服务不可达';
    if (currentPanel === 'status') {
      renderPanel('status');
      refreshStatusClientDock();
    }
  }
}

async function saveConfig() {
  const sidebarBtn = document.getElementById('sidebarSaveBtn');
  const mainBtn = document.getElementById('btnSave');
  const sidebarMsg = document.getElementById('sidebarMsg');
  const configMsg = document.getElementById('configMsg');

  const setLoading = (loading) => {
    [sidebarBtn, mainBtn].forEach(b => { if (b) b.disabled = loading; });
    const msg = loading ? '保存中...' : '';
    if (sidebarMsg) { sidebarMsg.textContent = msg; sidebarMsg.className = 'sidebar-status loading'; }
    if (configMsg) { configMsg.textContent = msg; configMsg.style.color = 'var(--amber)'; }
  };

  setLoading(true);

  try {
    if (!currentConfig || Object.keys(currentConfig).length === 0) {
      currentConfig = await invoke('get_config');
    }
    const data = collectFormData();
    const status = window._statusData || {};
    const endpointChanged = Boolean(status.running) && (
      normalizeServiceHost(data.host) !== normalizeServiceHost(status.host || currentConfig.host) ||
      String(data.port || '') !== String(status.preferred_port || status.port || currentConfig.port || '') ||
      Boolean(data.port_auto) !== (status.port_auto !== false)
    );
    const previousMode = currentConfig.codex_router_mode || 'api';
    const nextMode = data.codex_router_mode || 'api';
    const modeChanged = nextMode !== previousMode;
    const wasRunning = Boolean(status.running);
    await invoke('save_config', { config: data });
    if (modeChanged) {
      window.DexTelemetry?.track?.('account_mode_switch', {
        previous_mode: previousMode,
        next_mode: nextMode,
        service_running: wasRunning,
      });
    }

    // 切换路由模式时：网关在跑就自动停止（让用户重新启动以应用新模式），关闭状态不动。
    if (modeChanged && wasRunning) {
      try {
        await invoke('stop_service');
        showToast('路由模式已切换，网关已停止，请重新启动', 'info');
      } catch (err) {
        showToast('停止网关失败: ' + err, 'error');
      }
    } else if (modeChanged) {
      showToast('路由模式已切换', 'success');
    }

    const msg = endpointChanged ? '配置已保存，服务地址/端口重启后生效' : '配置已保存';
    if (sidebarMsg) { sidebarMsg.textContent = msg; sidebarMsg.className = 'sidebar-status success'; }
    if (configMsg) { configMsg.textContent = msg; configMsg.style.color = endpointChanged ? 'var(--amber)' : 'var(--green)'; }
    showToast(endpointChanged ? '配置保存成功，重启网关后切换服务入口' : '配置保存成功', 'success');

    await loadConfig();
  } catch (err) {
    const errMsg = String(err);
    if (sidebarMsg) { sidebarMsg.textContent = errMsg; sidebarMsg.className = 'sidebar-status error'; }
    if (configMsg) { configMsg.textContent = errMsg; configMsg.style.color = 'var(--red)'; }
    showToast(errMsg, 'error');
  } finally {
    setLoading(false);
  }
}

async function validateConfig() {
  const sidebarMsg = document.getElementById('sidebarMsg');
  const configMsg = document.getElementById('configMsg');
  const mainBtn = document.getElementById('btnValidate') || document.getElementById('btnValidateDiag');

  if (mainBtn) { mainBtn.disabled = true; mainBtn.textContent = '诊断中...'; }
  if (sidebarMsg) { sidebarMsg.textContent = '诊断中...'; sidebarMsg.className = 'sidebar-status loading'; }

  try {
    // 配置面板未渲染时（如从诊断面板调用），使用已加载的配置
    const data = document.getElementById('field_port')
      ? collectFormData()
      : currentConfig;
    const result = await invoke('run_full_diagnostics', { config: data });
    window._diagData = result;

    if (currentPanel !== 'diagnostics') {
      switchPanel('diagnostics');
    } else {
      renderPanel('diagnostics');
    }

    const s = result.summary;
    const hlabels = { healthy: '正常', degraded: '降级', broken: '异常' };
    if (s.fail > 0) {
      if (sidebarMsg) { sidebarMsg.textContent = s.fail + ' 失败 · ' + s.warn + ' 警告'; sidebarMsg.className = 'sidebar-status error'; }
      showToast(s.fail + ' 项失败，' + s.warn + ' 项警告 — 健康状态: ' + (hlabels[s.health] || s.health), 'error');
    } else if (s.warn > 0) {
      if (sidebarMsg) { sidebarMsg.textContent = s.warn + ' 个警告'; sidebarMsg.className = 'sidebar-status loading'; }
      showToast(s.warn + ' 项警告 — 健康状态: ' + (hlabels[s.health] || s.health), 'info');
    } else {
      if (sidebarMsg) { sidebarMsg.textContent = '全部通过'; sidebarMsg.className = 'sidebar-status success'; }
      showToast('诊断完成，所有检查项通过', 'success');
    }
  } catch (err) {
    if (sidebarMsg) { sidebarMsg.textContent = '诊断失败'; sidebarMsg.className = 'sidebar-status error'; }
    showToast('诊断请求失败: ' + err, 'error');
  } finally {
    if (mainBtn) { mainBtn.disabled = false; mainBtn.textContent = '运行诊断'; }
    if (sidebarMsg && sidebarMsg.className === 'sidebar-status loading') {
      sidebarMsg.textContent = ''; sidebarMsg.className = 'sidebar-status';
    }
  }
}

// ═══════════════════════════════════════════════════════════════
// 配置引导（首次安装/更新后显示，顶栏按顺序跟随页面）
// ═══════════════════════════════════════════════════════════════
