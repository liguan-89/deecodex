// 状态面板
// ═══════════════════════════════════════════════════════════════
function renderStatus() {
  const s = window._statusData || {};
  const v = (val, fb) => (val !== undefined && val !== null) ? val : fb;
  const tokenStats = s.token_stats || {};
  const fmtStatusTokens = (n) => {
    const value = Number(n || 0);
    if (value >= 1000000000) return compactStatusNumber(value / 1000000000) + 'B';
    if (value >= 1000000) return compactStatusNumber(value / 1000000) + 'M';
    if (value >= 1000) return compactStatusNumber(value / 1000) + 'k';
    return String(value);
  };
  const compactStatusNumber = (value) => {
    const digits = value >= 10 ? 1 : 2;
    return value.toFixed(digits).replace(/\.0$/, '');
  };
  const clientLabel = (kind) => CLIENT_KIND_LABELS?.[kind] || kind || '未知工具';
  const statusPill = (state, text) => `<span class="gateway-pill ${state}">${esc(text)}</span>`;
  const statusDot = (state) => `<span class="gateway-dot ${state}"></span>`;
  const kindOf = (account) => typeof accountClientKind === 'function' ? accountClientKind(account) : (account?.client_kind || account?.target || 'codex');
  const accounts = accountsData.accounts || [];

  const port = v(s.port, '—');
  const running = s.running;
  const clientKinds = (() => {
    const defaults = [
      { slug: 'codex', label: 'Codex' },
      { slug: 'claude_code', label: 'Claude Code' },
      { slug: 'openclaw', label: 'OpenClaw' },
      { slug: 'hermes', label: 'Hermes' },
      { slug: 'generic_client', label: '通用客户端' },
    ];
    const profiles = Array.isArray(clientProfiles) && clientProfiles.length
      ? clientProfiles.map(p => ({ slug: normalizeClientKind?.(p.slug || p.kind), label: p.label || clientLabel(normalizeClientKind?.(p.slug || p.kind)) }))
      : defaults;
    const seen = new Set();
    return profiles.concat(defaults).filter(item => {
      const slug = normalizeClientKind?.(item.slug) || item.slug;
      if (!slug || seen.has(slug)) return false;
      seen.add(slug);
      return true;
    });
  })();
  const activeAccountForKind = (kind, list) => {
    if (!list.length) return null;
    const activeId = accountsData.active_account_id || accountsData.active_id;
    if (kind === 'codex') return list.find(a => a.id === activeId) || list[0];
    return list
      .filter(a => Number(a.last_applied_at || 0) > 0)
      .sort((a, b) => Number(b.last_applied_at || 0) - Number(a.last_applied_at || 0))[0] || list[0];
  };
  const clientSummaries = clientKinds.map(kind => {
    const list = accounts.filter(a => kindOf(a) === kind.slug);
    const hasIssue = list.some(a => typeof clientAccountHasIssue === 'function' && clientAccountHasIssue(a));
    const state = list.length ? (hasIssue ? 'warn' : 'ok') : 'idle';
    const stateText = list.length ? (hasIssue ? '需处理' : '正常') : '未接入';
    const active = activeAccountForKind(kind.slug, list);
    return { ...kind, count: list.length, hasIssue, state, stateText, activeName: active?.name || '—' };
  });
  const connectedKinds = clientSummaries.filter(item => item.count > 0).length;
  const disconnectedKinds = clientKinds.length - connectedKinds;
  const clientIssueCount = clientSummaries.filter(item => item.hasIssue).length;
  const todayRequests = Number(tokenStats.total || 0);
  const problemCount = (running ? 0 : 1) + clientIssueCount + (connectedKinds === 0 ? 1 : 0);
  const gatewayTitle = '接入状态';
  const version = s.version && s.version !== '—' ? `v${s.version}` : '版本 —';
  const hasUpdate = typeof deeStorage !== 'undefined' && deeStorage?.getItem?.('updateAvailable') === '1';
  const serviceAddress = port === '—' ? '—' : `127.0.0.1:${port}`;
  const gatewaySummaryItems = [
    `已接入 ${connectedKinds}/${clientKinds.length} 个工具`,
    `${problemCount} 个问题`,
    `今日 ${todayRequests} 次请求`,
    `服务地址 ${serviceAddress}`,
  ];
  const clientRows = clientSummaries.map(kind => {
    return `
      <button class="tool-health-row ${kind.state}" type="button" onclick="selectedClientKind='${escAttr(kind.slug)}';switchPanel('accounts')">
        <span class="tool-health-name">${statusDot(kind.state)}${esc(kind.label || clientLabel(kind.slug))}</span>
        <span class="tool-health-status">${statusPill(kind.state, kind.stateText)}</span>
        <span class="tool-health-active">${esc(kind.activeName)}</span>
        <span class="tool-health-count">${esc(kind.count)} 个配置</span>
        <span class="tool-health-action">${kind.count ? '查看' : '配置'}</span>
      </button>`;
  }).join('');

  return `
    <div class="page-header status-page-header">
      <p>统一管理 Codex、Claude Code、OpenClaw、Hermes 等 AI 工具的本地接入、运行健康与使用概览</p>
    </div>

    <div class="gateway-hero">
      <div class="gateway-hero-main">
        <div class="gateway-title-row">
          <span>${esc(gatewayTitle)}</span>
          <span class="gateway-version" id="dashboardVersion">${hasUpdate ? '<span class="update-dot"></span>' : ''}${esc(version)}</span>
        </div>
        <div class="gateway-summary-line">
          ${gatewaySummaryItems.map(item => `<span>${esc(item)}</span>`).join('')}
        </div>
      </div>
    </div>

    <div class="gateway-metric-grid">
      <div class="gateway-metric-card ${running ? 'ok' : 'fail'}" onclick="mgmtToggle()">
        <span class="metric-label">网关状态</span>
        <strong>${running ? '运行中' : '已停止'}</strong>
        <span class="metric-foot">${running ? `已运行 ${esc(fmtUptime(s.uptime_secs))}` : '当前未运行'}</span>
      </div>
      <div class="gateway-metric-card" onclick="switchPanel('accounts')">
        <span class="metric-label">接入工具</span>
        <strong>${esc(connectedKinds)} / ${esc(clientKinds.length)}</strong>
        <span class="metric-foot">${disconnectedKinds ? `${disconnectedKinds} 个未接入` : '全部已接入'}</span>
      </div>
      <div class="gateway-metric-card ${problemCount ? 'warn' : 'ok'}" onclick="validateConfig()">
        <span class="metric-label">待处理问题</span>
        <strong>${esc(problemCount)}</strong>
        <span class="metric-foot">${problemCount ? '运行诊断查看详情' : '暂无待处理项'}</span>
      </div>
      <div class="gateway-metric-card" onclick="switchPanel('sessions')">
        <span class="metric-label">今日流量</span>
        <strong>${esc(todayRequests)}</strong>
        <span class="metric-foot">${esc(fmtStatusTokens(tokenStats.total_tokens))} tokens 今日累计</span>
      </div>
    </div>

    <section class="gateway-panel operations-panel">
      <div class="gateway-section-title">服务操作</div>
      <div class="mgmt-actions gateway-actions">
        <button class="btn btn-primary" onclick="mgmtToggle()" id="btnToggle">${running ? '◼ 停止网关' : '▶ 启动网关'}</button>
        <button class="btn btn-ghost" onclick="validateConfig()">◇ 运行诊断</button>
        <button class="btn btn-ghost" onclick="mgmtLaunchCodex()" id="btnLaunchCodex" style="border-color:rgba(0,200,232,0.35);color:var(--accent)">${window._cdpLaunched ? '◼ 停止 CDP' : '⬢ 启动 CDP'}</button>
        <button class="btn btn-ghost" onclick="mgmtRestart()" id="btnRestart">⟳ 重启服务</button>
        <button class="btn btn-ghost" onclick="mgmtLogs()">☰ 查看日志</button>
        <button class="btn btn-ghost" onclick="mgmtUpdate()" id="btnUpdate">⇡ 一键升级</button>
      </div>
    </section>

    <section class="gateway-panel tools-panel">
      <div class="gateway-section-title">接入工具</div>
      <div class="tool-health-list">
        <div class="tool-health-head" aria-hidden="true">
          <span>工具</span>
          <span>状态</span>
          <span>活跃账号</span>
          <span>配置</span>
          <span></span>
        </div>
        ${clientRows}
      </div>
    </section>
  `;
}

// ═══════════════════════════════════════════════════════════════
// 配置面板
// ═══════════════════════════════════════════════════════════════
function renderConfig() {
  if (!currentConfig) {
    return `
      <div class="page-header">
        <h2>配置</h2>
        <p>正在加载配置...</p>
      </div>`;
  }

  let html = `
    <div class="page-header">
      <h2>配置</h2>
      <p>修改配置后点击「保存配置」使其生效，部分变更需重启服务</p>
    </div>`;

  for (const sec of CONFIG_SECTIONS) {
    html += `
    <div class="config-section" id="cfg-${sec.id}">
      <div class="config-section-header">
        <span class="section-icon">${sec.icon}</span>
        <h3>${sec.label}</h3>
        <span class="section-desc">${sec.fields.length} 项</span>
      </div>
      <div class="config-fields">`;
    for (const f of sec.fields) {
      html += renderField(f);
    }
    html += `</div></div>`;
  }

  html += `
    <div class="config-actions">
      <button class="btn btn-primary" id="btnSave" onclick="saveConfig()">保存配置</button>
      <button class="btn btn-ghost" id="btnValidate" onclick="validateConfig()">验证配置</button>
      <span id="configMsg" style="font-family:var(--font-mono);font-size:11px;color:var(--text-muted);align-self:center;margin-left:8px;"></span>
    </div>`;

  return html;
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
        const gicons = { healthy: '&#x2705;', degraded: '&#x26A0;', broken: '&#x274C;' };
        const sitems = { pass: '&#x2705;', warn: '&#x26A0;', fail: '&#x274C;', info: '&#x2139;' };
        return `
        <div class="diag-group">
          <div class="diag-group-header ${g.health}">
            <span class="group-icon">${gicons[g.health] || ''}</span> ${esc(g.name)}
          </div>
          ${g.items.map(it => `
            <div class="diag-item">
              <span class="item-icon ${it.status}">${sitems[it.status] || ''}</span>
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
      <p>全链路运行时诊断，涵盖服务状态、账号连通、Codex 路由、注入状态、运行环境</p>
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
      <p>安装后的配置指南、常见问题与故障排查</p>
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
      <p>安装完成后，<strong>deecodex 已自动启动</strong>。你需要配置 Codex 将请求发送到 deecodex：</p>
      <ul>
        <li>打开 Codex 设置 → 找到「模型提供商」或「自定义 Provider」</li>
        <li>将 API 地址设为 <strong>http://127.0.0.1:4446/v1</strong></li>
        <li>API Key 可填任意值（如果 deecodex 未开启客户端认证）</li>
        <li>模型名填写 deecodex 模型映射中的任一 Codex 侧名称，如 <strong>gpt-5.5</strong></li>
      </ul>
      <p>配置完成后发送一条测试消息，观察 deecodex 日志应有 ← codex 和 → upstream 输出。</p>
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
      <p style="font-size:11px; color:var(--text-muted);">⚠ base_url 末尾不要加 /，端口须与 deecodex 监听端口一致。</p>

      <p style="margin-top:16px;"><strong>CC Switch (CLI)</strong> — 在设置中填写：</p>
      <ul>
        <li>API 请求地址：<strong>http://127.0.0.1:4446/v1</strong></li>
        <li>API Key：任意值（若 deecodex 未开启客户端认证）。如需使用 CC Switch，请关闭高级设置中的「自动注入」和「持久注入」，避免两个工具同时修改配置文件路由导致冲突。</li>
      </ul>
    </div>

    <div class="help-section" id="h-model-map">
      <h3>模型映射</h3>
      <p>模型映射定义了 <strong>Codex 使用的模型名 → DeepSeek 实际模型名</strong> 的对应关系。</p>
      <p>默认映射：</p>
      <div class="code-block"><pre><span class="key">"gpt-5.5"</span>: <span class="str">"deepseek-v4-pro"</span>
<span class="key">"gpt-5"</span>: <span class="str">"deepseek-v4-pro"</span>
<span class="key">"gpt-4o"</span>: <span class="str">"deepseek-v4-pro"</span>
<span class="key">"gpt-4o-mini"</span>: <span class="str">"deepseek-v4-pro"</span>
<span class="key">"gpt-4.1"</span>: <span class="str">"deepseek-v4-pro"</span>
<span class="key">"o3-model"</span>: <span class="str">"deepseek-v4-pro"</span>
<span class="key">"o4-model"</span>: <span class="str">"deepseek-v4-pro"</span></pre></div>
      <p>键名<strong>大小写敏感</strong>。新模型发布后需更新此映射表。</p>
    </div>

    <div class="help-section" id="h-commands">
      <h3>管理命令</h3>
      <p style="font-size:12px; color:var(--text-muted);">先进入数据目录：<code>cd ~/.deecodex</code>（Windows 为 <code>cd %LOCALAPPDATA%\\Programs\\deecodex</code>）</p>
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
      <p style="font-size:12px; color:var(--text-muted); margin-top:8px;">如果 <code>~/.local/bin</code> 已在 PATH 中，也可用二进制命令：<code>deecodex start</code> / <code>deecodex stop</code> 等。</p>
    </div>

    <div class="help-section" id="h-faq">
      <h3>常见问题</h3>
      <div class="faq-list">
        <div class="faq-item">
          <button class="faq-q" onclick="toggleFaq(this)"><span class="faq-arrow">▸</span> Codex 连接不上 deecodex (connection refused)</button>
          <div class="faq-a">deecodex 可能未启动。在此 GUI 中点击「启动服务」，或终端执行<code>./deecodex.sh start</code>（Windows 用<code>deecodex.bat start</code>）确认服务是否运行。</div>
        </div>
        <div class="faq-item">
          <button class="faq-q" onclick="toggleFaq(this)"><span class="faq-arrow">▸</span> 提示 model not found</button>
          <div class="faq-a">Codex 请求的模型名未在 deecodex 模型映射中找到。在配置面板的「配置 → 模型映射」中添加对应条目，或检查 Codex 中填写的模型名大小写是否与映射键名一致。</div>
        </div>
        <div class="faq-item">
          <button class="faq-q" onclick="toggleFaq(this)"><span class="faq-arrow">▸</span> 对话一直转圈不响应</button>
          <div class="faq-a">通常是 DeepSeek 上游不可达或 API Key 无效。查看日志观察是否有 <code>→ upstream</code> 输出以及对应的 HTTP 状态码。401/403 说明 API Key 问题。</div>
        </div>
        <div class="faq-item">
          <button class="faq-q" onclick="toggleFaq(this)"><span class="faq-arrow">▸</span> 思维链 (reasoning_content) 输出异常</button>
          <div class="faq-a">DeepSeek 流式响应中思维链可能跨 chunk 分片。deecodex 内置三级恢复策略（call_id 匹配 / turn 指纹 / 历史扫描）并自动重试最多 3 次。若仍出现错误，尝试缩短对话上下文。</div>
        </div>
        <div class="faq-item">
          <button class="faq-q" onclick="toggleFaq(this)"><span class="faq-arrow">▸</span> 413 Payload Too Large</button>
          <div class="faq-a">请求体超过大小限制。在配置面板中将「最大请求体 (MB)」调大。</div>
        </div>
        <div class="faq-item">
          <button class="faq-q" onclick="toggleFaq(this)"><span class="faq-arrow">▸</span> 保存配置后什么时候生效？</button>
          <div class="faq-a">多数配置保存后即时生效（如模型映射、Token 检测参数）。端口、数据目录等核心配置需要重启 deecodex 才会生效。</div>
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
    input.type = 'text'; btn.textContent = '⊘'; btn.title = '隐藏';
  } else {
    input.type = 'password'; btn.textContent = '⊙'; btn.title = '显示';
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

    if (currentPanel === 'status') renderPanel('status');
  } catch (err) {
    window._statusData = { running: false, port: '—', uptime_secs: 0 };
    document.getElementById('connDot').className = 'indicator err';
    document.getElementById('connLabel').textContent = '服务不可达';
    if (currentPanel === 'status') renderPanel('status');
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
    await invoke('save_config', { config: data });

    const msg = '配置已保存';
    if (sidebarMsg) { sidebarMsg.textContent = msg; sidebarMsg.className = 'sidebar-status success'; }
    if (configMsg) { configMsg.textContent = msg; configMsg.style.color = 'var(--green)'; }
    showToast('配置保存成功', 'success');

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
