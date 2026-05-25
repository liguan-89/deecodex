// 状态面板
// ═══════════════════════════════════════════════════════════════
function clientKindUiLabel(kind, profile) {
  const normalized = typeof normalizeClientKind === 'function' ? normalizeClientKind(kind) : String(kind || '');
  const labels = (typeof CLIENT_KIND_LABELS !== 'undefined' && CLIENT_KIND_LABELS) || {
    codex: 'Codex',
    claude_code: 'Claude',
    openclaw: 'OpenClaw',
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
  const clientKinds = (() => {
    const defaults = [
      { slug: 'codex', label: 'Codex' },
      { slug: 'claude_code', label: 'Claude' },
      { slug: 'openclaw', label: 'OpenClaw' },
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

  return `
    <section class="status-overview-shell" aria-label="接入状态总览">
      <div class="gateway-hero">
      <div class="gateway-hero-main">
        <div class="gateway-version-line">
          <span class="gateway-version" id="dashboardVersion">${hasUpdate ? '<span class="update-dot"></span>' : ''}${esc(version)}</span>
        </div>
      </div>
      </div>

      <div class="gateway-metric-grid">
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
      </div>

      <section class="gateway-panel operations-panel">
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
        <button class="btn btn-ghost" onclick="mgmtLaunchCodex()" id="btnLaunchCodex">
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
          <span>一键升级</span>
        </button>
      </div>
      </section>

      ${renderStatusClientDock(running)}
    </section>
  `;
}

function statusClientKinds() {
  return [
    { slug: 'codex_cli', label: 'Codex CLI', accountKind: 'codex', iconKind: 'codex' },
    { slug: 'codex_desktop', label: 'Codex 桌面', accountKind: 'codex', iconKind: 'codex', desktopApp: 'Codex' },
    { slug: 'claude_cli', label: 'Claude CLI', accountKind: 'claude_code', iconKind: 'claude_code' },
    { slug: 'claude_desktop', label: 'Claude 桌面', accountKind: 'claude_code', iconKind: 'claude_code', desktopApp: 'Claude' },
    { slug: 'openclaw', label: 'OpenClaw', accountKind: 'openclaw', iconKind: 'openclaw' },
    { slug: 'hermes', label: 'Hermes', accountKind: 'hermes', iconKind: 'hermes' },
  ];
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
  const installedKnown = typeof status.installed === 'boolean';
  const accountKnown = typeof status.account_exists === 'boolean';
  const installed = installedKnown ? status.installed : true;
  const accountReady = accountKnown ? status.account_exists : Boolean(info.account);
  const configured = status.account_configured !== false && accountReady;
  const action = status.next_action || (installed ? (accountReady ? 'launch' : 'configure') : 'install');
  const installLabel = installedKnown ? (installed ? '已安装' : '未安装') : '检测中';
  const accountLabel = accountKnown ? (accountReady ? (configured ? '已配置' : '待写入') : '需账号') : (info.account ? '已配置' : '需账号');
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
  };
}

function statusClientIcon(item) {
  const accountKind = item.iconKind || item.accountKind || item.slug;
  const icon = typeof clientIcon === 'function' ? clientIcon(accountKind) : '';
  if (item.accountKind === 'codex' || item.accountKind === 'claude_code') {
    const surface = statusClientSurface(item.slug);
    return `<span class="surface-icon-stack">${icon}<span class="surface-glyph surface-${escAttr(surface)}" aria-hidden="true"></span></span>`;
  }
  return icon;
}

function renderStatusClientDock(gatewayRunning) {
  return `<section class="gateway-panel client-dock-panel" aria-label="客户端运行控制">
    <div class="gateway-section-title client-dock-title">客户端接入</div>
    <div class="client-dock-grid">
      ${statusClientKinds().map(item => {
        const kind = item.slug;
        const label = item.label || clientKindUiLabel(item.accountKind);
        const info = statusClientInfo(kind, gatewayRunning);
        const lifecycle = statusClientLifecycleMeta(kind, info);
        return `<button type="button" class="client-dock-item ${escAttr(info.state)} ${escAttr(lifecycle.classes)}${info.processRunning ? ' process-running' : ''}" data-client-kind="${escAttr(kind)}" data-client-label="${escAttr(label)}" data-next-action="${escAttr(lifecycle.action)}" onclick="handleClientDockClick('${escAttr(kind)}')" title="${escAttr(label)}" aria-label="${escAttr(label + ' · ' + info.text)}">
          <span class="client-dock-icon-wrap">
            <span class="client-dock-icon">${statusClientIcon(item)}</span>
            <span class="client-dock-runtime ${info.processRunning ? 'live' : 'idle'}" title="${escAttr(info.processRunning ? '客户端进程运行中' : '未检测到客户端进程')}"></span>
          </span>
          <span class="client-dock-state-row" aria-hidden="true">
            <span class="client-dock-state-dot install" title="${escAttr(lifecycle.installLabel)}"></span>
            <span class="client-dock-state-dot account" title="${escAttr(lifecycle.accountLabel)}"></span>
            <span class="client-dock-state-dot runtime" title="${escAttr(lifecycle.runLabel)}"></span>
          </span>
          <span class="client-dock-label">${esc(label)}</span>
        </button>`;
      }).join('')}
    </div>
  </section>`;
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

async function toggleStatusClientKind(kind) {
  const normalized = String(kind || '');
  if (normalized === 'codex_cli') {
    await mgmtToggle();
    return;
  }
  if (normalized === 'codex_desktop' || normalized === 'claude_desktop') {
    const running = statusClientProcessRunning(normalized);
    try {
      await invoke('dex_toggle_desktop_client', { kind: normalized, running });
      showToast(`${statusClientDockItem(normalized).label} 已${running ? '关闭' : '打开'}`, running ? 'info' : 'success');
      setTimeout(refreshStatusClientDock, 900);
    } catch (err) {
      showToast(`${statusClientDockItem(normalized).label} 操作失败: ` + err, 'error');
    }
    return;
  }
  const accountKind = statusClientAccountKind(normalized);
  const account = statusClientPrimaryAccount(normalized);
  if (!account) {
    selectedClientKind = accountKind;
    switchPanel('accounts');
    showToast('请先添加该客户端账号', 'info');
    return;
  }
  const enabled = statusClientEnabled(accountKind, account, Boolean(window._statusData?.running));
  const next = JSON.parse(JSON.stringify(account));
  next.client_options = next.client_options || {};
  next.client_options.proxy_recording_enabled = !enabled;
  if (!next.client_options.proxy_recording_enabled) {
    delete next.client_options.proxy_base_url;
  }
  delete next._client_status_report;
  try {
    await invoke('update_account', { accountJson: JSON.stringify(next) });
    const report = await invoke('apply_client_account', { accountId: next.id, dryRun: false });
    showToast(report.ok
      ? `${clientKindUiLabel(accountKind)} 已${enabled ? '关闭' : '开启'}`
      : `${clientKindUiLabel(accountKind)} 配置写入后仍有问题`,
      report.ok ? 'success' : 'error');
    await loadAccountsData();
    if (currentPanel === 'status') renderPanel('status');
    await refreshStatusClientDock();
  } catch (err) {
    showToast(`${clientKindUiLabel(accountKind)} 切换失败: ` + err, 'error');
  }
}

// ═══════════════════════════════════════════════════════════════
// 配置面板
// ═══════════════════════════════════════════════════════════════
function normalizeConfigClientKind(kind) {
  const value = String(kind || 'global');
  if (value === 'global') return 'global';
  if (typeof normalizeClientKind === 'function') return normalizeClientKind(value);
  return ['codex', 'claude_code', 'openclaw', 'hermes', 'generic_client'].includes(value) ? value : 'global';
}

function configKindForSection(sectionId) {
  const section = CONFIG_SECTIONS.find(sec => sec.id === sectionId);
  return normalizeConfigClientKind(section?.scope || 'global');
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
    { slug: 'openclaw', label: 'OpenClaw', description: 'OpenClaw 配置', config_path_hint: '~/.openclaw/openclaw.json', model_slots: [] },
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

function configTimeLabel(ts) {
  if (!ts) return '—';
  if (typeof formatTimeShort === 'function') return formatTimeShort(ts);
  return new Date(Number(ts) * 1000).toLocaleString();
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

function openAccountsForConfigClient(kind) {
  selectedClientKind = normalizeConfigClientKind(kind);
  accountsView = 'list';
  switchPanel('accounts');
}

async function scanConfigClientAccounts(kind) {
  selectedConfigClientKind = normalizeConfigClientKind(kind);
  if (typeof deeStorage !== 'undefined' && typeof CONFIG_CLIENT_STORAGE_KEY !== 'undefined') {
    deeStorage.setItem(CONFIG_CLIENT_STORAGE_KEY, selectedConfigClientKind);
  }
  if (typeof scanClientAccounts === 'function') {
    await scanClientAccounts();
    if (currentPanel === 'config') renderPanel('config');
  }
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

function configFieldByKey(key) {
  for (const section of CONFIG_SECTIONS) {
    const field = section.fields.find(item => item.key === key);
    if (field) return field;
  }
  return null;
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

function configClientStatusInfo(account) {
  if (!account) return { text: '未接入', detail: '暂无客户端账号', className: 'pending' };
  const status = account._client_status_report || account.last_check || {};
  if (status.ok === false) return { text: '需处理', detail: status.message || '最近检查发现问题', className: 'error' };
  if (status.ok === true) return { text: '正常', detail: status.message || configTimeLabel(account.last_applied_at), className: 'ok' };
  return { text: '待检查', detail: configTimeLabel(account.last_applied_at), className: 'pending' };
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
    openclaw: {
      heading: 'Agent 网关治理',
      summary: 'OpenClaw 是多通道 agent gateway，高级设置应围绕 gateway、channels、agent 路由、工具执行和 schema 校验。',
      focus: [
        { title: 'Gateway / Channels', tag: '通道', tone: 'info', body: '关注 gateway 暴露、通道账号、群组 allowlist 与 mention 规则，避免 agent 被错误渠道唤起。' },
        { title: 'Agent 路由', tag: '路由', tone: 'info', body: '检查 agents.list、bindings、workspace 与 per-sender session，确保不同入口落到正确 agent。' },
        { title: '执行审批', tag: '审批', tone: 'warn', body: 'OpenClaw 的 exec approvals 是高风险控制面，应审计本机、gateway 与 node 的有效策略。' },
        { title: 'Schema 校验', tag: '配置', tone: 'ok', body: '先用官方 schema/validate 检查配置结构；供应商、密钥和模型关系继续留在账号管理。' },
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
  if (kind === 'openclaw') return [
    { label: '配置文件', command: 'openclaw config file' },
    { label: 'Schema 校验', command: 'openclaw config validate --json' },
    { label: '审批策略', command: 'openclaw exec-policy show --json' },
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

function claudeCustomFilterState(account) {
  const opts = account?.client_options || {};
  const rules = Array.isArray(opts.claude_custom_filter_rules)
    ? opts.claude_custom_filter_rules
        .map(rule => String(rule || '').trim())
        .filter(Boolean)
    : [];
  return {
    cchEnabled: opts.claude_cch_filter_enabled !== false,
    enabled: Boolean(opts.claude_custom_filter_enabled),
    rules,
  };
}

function renderClaudeCustomFilterSection(account) {
  const state = claudeCustomFilterState(account);
  const disabled = account ? '' : ' disabled';
  const ruleText = state.rules.join('\n');
  return `<section class="config-client-section">
    <div class="config-section-header">
      <span class="section-icon">⌬</span>
      <h3>自定义过滤</h3>
      <span class="section-desc">Anthropic system 行过滤</span>
    </div>
    <div class="config-filter-panel${account ? '' : ' disabled'}">
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
      <p>仅作用于 Claude Code 通过 Anthropic Messages 端口发送的顶层 <code>system</code> 文本；命中的整行会在转发前移除。内置已处理 <code>x-anthropic-billing-header</code> + <code>cch=</code>。</p>
      ${account ? '' : '<div class="config-client-empty">暂无 Claude 账号。请先在账号管理中添加或扫描 Claude Code 账号，再保存过滤规则。</div>'}
    </div>
  </section>`;
}

async function saveClaudeCustomFilters() {
  const account = configPrimaryClientAccount('claude_code');
  if (!account) {
    showToast('请先添加 Claude 账号', 'error');
    return;
  }
  const cchEnabled = Boolean(document.getElementById('claudeCchFilterEnabled')?.checked);
  const enabled = Boolean(document.getElementById('claudeCustomFilterEnabled')?.checked);
  const rules = (document.getElementById('claudeCustomFilterRules')?.value || '')
    .split(/\r?\n/)
    .map(rule => rule.trim())
    .filter(Boolean);
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
  try {
    await invoke('update_account', { accountJson: JSON.stringify(next) });
    showToast('Claude 自定义过滤已保存', 'success');
    await loadAccountsData();
    renderPanel('config');
  } catch (err) {
    showToast('保存 Claude 过滤规则失败: ' + err, 'error');
  }
}

function renderConfigClientMeta(kind, profile, accounts, account, status) {
  const issueCount = configClientIssueCount(kind);
  const configHint = profile.config_path_hint || '原生配置';
  const accountLabel = account ? (account.name || '未命名账号') : '未接入';
  const issueText = issueCount ? `${issueCount} 个` : '无';
  return `<div class="config-client-meta-grid">
    <div>
      <span>账号覆盖</span>
      <strong>${accounts.length ? `${accounts.length} 个` : '未接入'}</strong>
    </div>
    <div>
      <span>当前账号</span>
      <strong title="${escAttr(accountLabel)}">${esc(accountLabel)}</strong>
    </div>
    <div>
      <span>待处理</span>
      <strong class="${issueCount ? 'error' : 'ok'}">${esc(issueText)}</strong>
    </div>
    <div>
      <span>原生配置</span>
      <strong title="${escAttr(configHint)}">${esc(configHint)}</strong>
    </div>
    <div>
      <span>最近检查</span>
      <strong class="${escAttr(status.className)}" title="${escAttr(status.detail || '')}">${esc(status.text)}</strong>
    </div>
  </div>`;
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
  const changed = runningHost !== configuredHost || runningPort !== configuredPort;
  const runningEndpoint = `${runningHost.includes(':') ? `[${runningHost}]` : runningHost}:${runningPort || '—'}`;
  const configuredEndpoint = `${configuredHost.includes(':') ? `[${configuredHost}]` : configuredHost}:${configuredPort || '—'}`;
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
    ${kind === 'claude_code' ? renderClaudeCustomFilterSection(account) : ''}
    ${renderConfigCommandStrip(kind)}`;
}

async function refreshConfigClientAdvanced(kind) {
  const account = configPrimaryClientAccount(kind);
  if (!account) {
    showToast('请先添加该客户端账号', 'error');
    return;
  }
  try {
    const report = await invoke('refresh_client_status', { accountId: account.id });
    showToast(report.ok ? '客户端状态正常' : '客户端状态有问题', report.ok ? 'success' : 'error');
    await loadAccountsData();
    renderPanel('config');
  } catch (err) {
    showToast('刷新客户端状态失败: ' + err, 'error');
  }
}

function editConfigClientFile(kind) {
  const account = configPrimaryClientAccount(kind);
  if (!account) {
    showToast('请先添加该客户端账号', 'error');
    return;
  }
  if (typeof editConfigFile === 'function') editConfigFile(account.id);
  else if (typeof openClientConfig === 'function') openClientConfig(account.id);
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
      const opts = (f.options || []).map(o => `<option value="${escAttr(o)}" ${String(val) === o ? 'selected' : ''}>${esc(o)}</option>`).join('');
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
    <div class="page-header">
      <h2>使用帮助</h2>
    </div>

    <div class="help-toc">
      <a onclick="document.getElementById('h-quickstart').scrollIntoView({behavior:'smooth'})">快速开始</a>
      <a onclick="document.getElementById('h-codex-config').scrollIntoView({behavior:'smooth'})">Codex 配置</a>
      <a onclick="document.getElementById('h-model-map').scrollIntoView({behavior:'smooth'})">模型映射</a>
      <a onclick="document.getElementById('h-commands').scrollIntoView({behavior:'smooth'})">管理命令</a>
      <a onclick="document.getElementById('h-faq').scrollIntoView({behavior:'smooth'})">常见问题</a>
    </div>

    <div class="help-section" id="h-quickstart">
      <h3>快速开始</h3>
      <p>安装完成后，<strong>DEX AI 已自动启动</strong>。你需要配置 Codex 将请求发送到 DEX AI：</p>
      <ul>
        <li>打开 Codex 设置 → 找到「模型提供商」或「自定义 Provider」</li>
        <li>将 API 地址设为 <strong>http://127.0.0.1:4446/v1</strong></li>
        <li>API Key 可填任意值（如果 DEX AI 未开启客户端认证）</li>
        <li>模型名填写 DEX AI 模型映射中的任一 Codex 侧名称，如 <strong>gpt-5.5</strong></li>
      </ul>
      <p>配置完成后发送一条测试消息，观察 DEX AI 日志应有 ← codex 和 → upstream 输出。</p>
    </div>

    <div class="help-section" id="h-codex-config">
      <h3>Codex 配置</h3>
      <p><strong>Codex 桌面版</strong> — 编辑 <code>~/.codex/config.toml</code>：</p>
      <div class="code-block"><pre><span class="comment"># ~/.codex/config.toml</span>
<span class="key">model</span> = <span class="str">"gpt-5.5"</span>
<span class="key">model_provider</span> = <span class="str">"custom"</span>
<span class="key">model_reasoning_effort</span> = <span class="str">"medium"</span>

<span class="key">[model_providers.custom]</span>
<span class="key">base_url</span> = <span class="str">"http://127.0.0.1:4446/v1"</span>
<span class="key">name</span> = <span class="str">"custom"</span>
<span class="key">requires_openai_auth</span> = <span class="val">false</span>
<span class="key">wire_api</span> = <span class="str">"responses"</span></pre></div>
      <p class="help-note">注意：base_url 末尾不要加 /，端口须与 DEX AI 监听端口一致。</p>

      <p class="help-subsection"><strong>CC Switch (CLI)</strong> — 在设置中填写：</p>
      <ul>
        <li>API 请求地址：<strong>http://127.0.0.1:4446/v1</strong></li>
        <li>API Key：任意值（若 DEX AI 未开启客户端认证）。如需使用 CC Switch，请关闭高级设置中的「自动注入」和「持久注入」，避免两个工具同时修改配置文件路由导致冲突。</li>
      </ul>
    </div>

    <div class="help-section" id="h-model-map">
      <h3>模型映射</h3>
      <p>模型映射定义了 <strong>Codex 使用的模型名 → DeepSeek 实际模型名</strong> 的对应关系。</p>
      <p>默认映射：</p>
      <div class="code-block"><pre><span class="key">"gpt-5.5"</span>: <span class="str">"deepseek-v4-pro"</span>
<span class="key">"gpt-5.4"</span>: <span class="str">"deepseek-v4-flash"</span>
<span class="key">"gpt-5.4-mini"</span>: <span class="str">"deepseek-v4-flash"</span>
<span class="key">"gpt-5.3-codex"</span>: <span class="str">"deepseek-v4-pro"</span>
<span class="key">"gpt-5.3-codex-spark"</span>: <span class="str">"deepseek-v4-flash"</span>
<span class="key">"gpt-5.2"</span>: <span class="str">"deepseek-v4-flash"</span>
<span class="key">"codex-auto-review"</span>: <span class="str">"deepseek-v4-flash"</span>
<span class="key">"gpt-4o"</span>: <span class="str">"deepseek-v4-pro"</span>
<span class="key">"gpt-4o-mini"</span>: <span class="str">"deepseek-v4-pro"</span>
<span class="key">"gpt-4.1"</span>: <span class="str">"deepseek-v4-pro"</span>
<span class="key">"o3-model"</span>: <span class="str">"deepseek-v4-pro"</span>
<span class="key">"o4-model"</span>: <span class="str">"deepseek-v4-pro"</span></pre></div>
      <p>键名<strong>大小写敏感</strong>。新模型发布后需更新此映射表。</p>
    </div>

    <div class="help-section" id="h-commands">
      <h3>管理命令</h3>
      <p class="help-note">先进入数据目录：<code>cd ~/.deecodex</code>（Windows 为 <code>cd %LOCALAPPDATA%\\Programs\\deecodex</code>）</p>
      <table class="cmd-table">
        <thead><tr><th>操作</th><th>macOS / Linux</th><th>Windows</th></tr></thead>
        <tbody>
          <tr><td>启动</td><td><code>./deecodex.sh start</code></td><td><code>deecodex.bat start</code></td></tr>
          <tr><td>停止</td><td><code>./deecodex.sh stop</code></td><td><code>deecodex.bat stop</code></td></tr>
          <tr><td>重启</td><td><code>./deecodex.sh restart</code></td><td><code>deecodex.bat restart</code></td></tr>
          <tr><td>状态</td><td><code>./deecodex.sh status</code></td><td><code>deecodex.bat status</code></td></tr>
          <tr><td>日志</td><td><code>./deecodex.sh logs</code></td><td><code>deecodex.bat logs</code></td></tr>
          <tr><td>修复配置</td><td><code>./deecodex.sh fix-config</code></td><td><code>deecodex.bat fix-config</code></td></tr>
          <tr><td>诊断</td><td><code>./deecodex.sh diagnose</code></td><td><code>deecodex.bat diagnose</code></td></tr>
        </tbody>
      </table>
      <p class="help-note help-note-after">如果 <code>~/.local/bin</code> 已在 PATH 中，也可用二进制命令：<code>deecodex start</code> / <code>deecodex stop</code> 等。</p>
    </div>

    <div class="help-section" id="h-faq">
      <h3>常见问题</h3>
      <div class="faq-list">
        <div class="faq-item">
          <button class="faq-q" onclick="toggleFaq(this)"><span class="faq-chevron" aria-hidden="true"></span> Codex 连接不上 DEX AI (connection refused)</button>
          <div class="faq-a">DEX AI 可能未启动。在此 GUI 中点击「启动服务」，或终端执行<code>./deecodex.sh start</code>（Windows 用<code>deecodex.bat start</code>）确认服务是否运行。</div>
        </div>
        <div class="faq-item">
          <button class="faq-q" onclick="toggleFaq(this)"><span class="faq-chevron" aria-hidden="true"></span> 提示 model not found</button>
          <div class="faq-a">Codex 请求的模型名未在 DEX AI 模型映射中找到。在配置面板的「配置 → 模型映射」中添加对应条目，或检查 Codex 中填写的模型名大小写是否与映射键名一致。</div>
        </div>
        <div class="faq-item">
          <button class="faq-q" onclick="toggleFaq(this)"><span class="faq-chevron" aria-hidden="true"></span> 对话一直转圈不响应</button>
          <div class="faq-a">通常是 DeepSeek 上游不可达或 API Key 无效。查看日志观察是否有 <code>→ upstream</code> 输出以及对应的 HTTP 状态码。401/403 说明 API Key 问题。</div>
        </div>
        <div class="faq-item">
          <button class="faq-q" onclick="toggleFaq(this)"><span class="faq-chevron" aria-hidden="true"></span> 思维链 (reasoning_content) 输出异常</button>
          <div class="faq-a">DeepSeek 流式响应中思维链可能跨 chunk 分片。DEX AI 内置三级恢复策略（call_id 匹配 / turn 指纹 / 历史扫描）并自动重试最多 3 次。若仍出现错误，尝试缩短对话上下文。</div>
        </div>
        <div class="faq-item">
          <button class="faq-q" onclick="toggleFaq(this)"><span class="faq-chevron" aria-hidden="true"></span> 413 Payload Too Large</button>
          <div class="faq-a">请求体超过大小限制。在配置面板中将「最大请求体 (MB)」调大。</div>
        </div>
        <div class="faq-item">
          <button class="faq-q" onclick="toggleFaq(this)"><span class="faq-chevron" aria-hidden="true"></span> 保存配置后什么时候生效？</button>
          <div class="faq-a">多数配置保存后即时生效（如模型映射、Token 检测参数）。端口、数据目录等核心配置需要重启 DEX AI 才会生效。</div>
        </div>
      </div>
    </div>
  `;
}

function toggleFaq(btn) {
  btn.parentElement.classList.toggle('open');
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
  const cb = document.getElementById('edit_vision_enabled');
  const mode = document.getElementById('edit_vision_mode');
  const vf = document.getElementById('visionFields');
  if (vf) {
    vf.style.display = mode ? (mode.value === 'glue' ? '' : 'none') : (cb && cb.checked ? '' : 'none');
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

function toggleCapabilityFields() {
  const cb = document.getElementById('edit_capability_enabled');
  const fields = document.getElementById('capabilityFields');
  if (cb && fields) {
    fields.style.display = cb.checked ? '' : 'none';
    if (!cb.checked) {
      const select = document.getElementById('edit_capability_account_id');
      if (select) select.value = '';
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
      String(data.port || '') !== String(status.port || currentConfig.port || '')
    );
    await invoke('save_config', { config: data });

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
