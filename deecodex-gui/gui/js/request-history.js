// 请求历史
		// ═══════════════════════════════════════════════════════════════

		// ── 请求历史 ──

		let _historyEntries = [];
		let _historyRefreshTimer = null;
		let _historyRefreshMs = 0;
		let _historyChartPeriod = 'hourly';
		let _historyStatusFilter = 'all';
		let _historyClientKindFilter = 'all';
		let _historyAccountFilter = '';
		let _historyDisplayLimit = 50;
const _historyDisplayStep = 50;
let _historyMonthlyStats = [];
let _historyAccounts = [];
let _historyAllEntries = [];
let _historyOffline = false;
let _historyReconnectTimer = null;
const HISTORY_CACHE_KEY = 'deecodex.history.cache';

		function syncHistoryAutoRefreshUi() {
		  const toggle = document.getElementById('historyAutoToggle');
		  const intervalSel = document.getElementById('historyIntervalSel');
		  if (toggle) toggle.classList.toggle('on', Boolean(_historyRefreshTimer));
		  if (intervalSel) {
		    intervalSel.style.display = _historyRefreshTimer ? '' : 'none';
		    if (_historyRefreshMs) intervalSel.value = String(_historyRefreshMs);
		  }
		}

		function historyClientLabel(kind) {
		  if (kind === 'all') return '全部';
		  const labels = (typeof CLIENT_KIND_LABELS !== 'undefined' && CLIENT_KIND_LABELS) || {
		    codex: 'Codex',
		    claude_code: 'Claude',
		    openclaw: 'OpenClaw',
		    hermes: 'Hermes',
		    generic_client: '通用客户端',
		  };
		  return labels[kind] || kind || '未知客户端';
		}

		function historyClientProfiles() {
		  const fallback = [
		    { slug: 'codex', label: 'Codex' },
		    { slug: 'claude_code', label: 'Claude' },
		    { slug: 'openclaw', label: 'OpenClaw' },
		    { slug: 'hermes', label: 'Hermes' },
		    { slug: 'generic_client', label: '通用客户端' },
		  ];
		  if (typeof clientProfiles !== 'undefined' && Array.isArray(clientProfiles) && clientProfiles.length) {
		    return clientProfiles.map(p => {
		      const raw = p.slug || p.kind;
		      const slug = typeof normalizeClientKind === 'function' ? normalizeClientKind(raw) : raw;
		      return { slug, label: historyClientLabel(slug) };
		    });
		  }
		  return fallback;
		}

		function renderHistoryClientSwitcher() {
		  const entries = Array.isArray(_historyAllEntries) && _historyAllEntries.length ? _historyAllEntries : (_historyEntries || []);
		  const profiles = [{ slug: 'all', label: '全部' }].concat(historyClientProfiles());
		  return profiles.map(profile => {
		    const kind = profile.slug;
		    const count = kind === 'all' ? entries.length : entries.filter(e => (e.client_kind || 'codex') === kind).length;
		    const active = kind === _historyClientKindFilter ? ' active' : '';
		    return `<button type="button" class="client-tab${active}" onclick="setHistoryClientKind('${escAttr(kind)}')">
		      <span>${esc(profile.label || historyClientLabel(kind))}</span><em>${count}</em>
		    </button>`;
		  }).join('');
		}

		function renderHistoryAccountOptions() {
		  const kindOf = typeof accountClientKind === 'function' ? accountClientKind : (a => a?.client_kind || a?.target || 'codex');
		  const accounts = (_historyAccounts || []).filter(a => _historyClientKindFilter === 'all' || kindOf(a) === _historyClientKindFilter);
		  const options = ['<option value="">全部账号</option>'];
		  for (const a of accounts) {
		    options.push(`<option value="${escAttr(a.id)}" ${_historyAccountFilter === a.id ? 'selected' : ''}>${esc(a.name || a.id)}</option>`);
		  }
		  return options.join('');
		}

		function historyEntryMatchesActiveFilters(entry) {
		  if (!entry) return false;
		  const clientKind = entry.client_kind || 'codex';
		  if (_historyClientKindFilter !== 'all' && clientKind !== _historyClientKindFilter) return false;
		  if (_historyAccountFilter && (entry.account_id || '') !== _historyAccountFilter) return false;
		  return true;
		}

		function visibleHistoryEntries(entries) {
		  return (entries || []).filter(historyEntryMatchesActiveFilters);
		}

		function visibleMonthlyStats(stats) {
		  return (stats || []).filter(s => {
		    const clientKind = s.client_kind || 'codex';
		    if (_historyClientKindFilter !== 'all' && clientKind !== _historyClientKindFilter) return false;
		    if (_historyAccountFilter && (s.account_id || '') !== _historyAccountFilter) return false;
		    return true;
		  });
		}

		function historyFilterArgs(extra) {
		  return Object.assign({}, extra || {}, {
		    clientKind: _historyClientKindFilter === 'all' ? null : _historyClientKindFilter,
		    accountId: _historyAccountFilter || null,
		  });
		}

		function renderHistory() {
		  return `<div class="page-header">
		    <h2>请求历史</h2>
		  </div>
		  <div id="historyClientSwitcher" class="client-switcher history-client-switcher">${renderHistoryClientSwitcher()}</div>
		  <div id="historyStats" class="history-stats">
		    <div class="history-stat"><div class="stat-value">—</div><div class="stat-label">今日请求数</div></div>
		    <div class="history-stat green"><div class="stat-value">—</div><div class="stat-label">成功率</div></div>
		    <div class="history-stat accent"><div class="stat-value">—</div><div class="stat-label">Token 消耗</div></div>
		    <div class="history-stat"><div class="stat-value">—</div><div class="stat-label">平均耗时</div></div>
		    <div class="history-stat cache"><div class="stat-value">—</div><div class="stat-label">命中缓存</div></div>
		  </div>
		  <div class="history-chart">
		    <div class="history-chart-header">
		      <h4>Token 用量趋势</h4>
		      <div class="chart-tabs">
		        <button class="chart-tab active" onclick="setChartPeriod('hourly')">1h</button>
		        <button class="chart-tab" onclick="setChartPeriod('daily')">1d</button>
		        <button class="chart-tab" onclick="setChartPeriod('monthly')">1m</button>
		      </div>
		    </div>
		    <div id="historyChartBars" class="history-chart-bars"></div>
		  </div>
		  <div class="history-controls">
		    <select class="history-select" id="historyAccountFilter" onchange="setHistoryAccountFilter(this.value)">
		      ${renderHistoryAccountOptions()}
		    </select>
		    <select class="history-select" onchange="setStatusFilter(this.value)">
		      <option value="all">全部状态</option>
		      <option value="completed">仅成功</option>
		      <option value="failed">仅失败</option>
		    </select>
		    <label class="history-toggle${_historyRefreshTimer ? ' on' : ''}" id="historyAutoToggle" onclick="toggleAutoRefresh()">
		      <div class="toggle-dot"></div> 自动刷新
		    </label>
		    <select class="history-select" id="historyIntervalSel" onchange="setRefreshInterval(this.value)" style="${_historyRefreshTimer ? '' : 'display:none;'}">
		      <option value="5000" ${_historyRefreshMs === 5000 ? 'selected' : ''}>5s</option>
		      <option value="10000" ${_historyRefreshMs === 10000 || !_historyRefreshMs ? 'selected' : ''}>10s</option>
		      <option value="30000" ${_historyRefreshMs === 30000 ? 'selected' : ''}>30s</option>
		      <option value="60000" ${_historyRefreshMs === 60000 ? 'selected' : ''}>60s</option>
		    </select>
		    <span style="flex:1"></span>
		    <button class="btn btn-primary" onclick="refreshHistory()">⟳ 刷新</button>
		    <button class="btn btn-ghost" onclick="clearHistory()">✕ 清空历史</button>
		  </div>
		  <div id="historyCardsContainer">
		    <div class="history-loading">加载中...</div>
		  </div>`;
		}

		function computeStats(entries) {
		  const total = entries.length;
		  const completed = entries.filter(e => e.status === 'completed').length;
		  const cacheHits = entries.filter(e => e.cache_hit).length;
		  const totalTokens = entries.reduce((s, e) => s + (e.total_tokens || 0), 0);
		  const totalMs = entries.reduce((s, e) => s + (e.duration_ms || 0), 0);
		  return {
		    total,
		    successRate: total > 0 ? Math.round(completed / total * 100) : 0,
		    totalTokens,
		    avgMs: total > 0 ? Math.round(totalMs / total) : 0,
		    cacheHitRate: total > 0 ? Math.round(cacheHits / total * 100) : 0
		  };
		}

		function todayStartUnixSecs() {
		  const now = new Date();
		  return Math.floor(new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime() / 1000);
		}

		function filterToday(entries) {
		  const start = todayStartUnixSecs();
		  return (entries || []).filter(e => (e.created_at || 0) >= start);
		}

		function statsFromAggregate(raw) {
		  const total = Number(raw?.total || 0);
		  const success = Number(raw?.success_count || 0);
		  const cacheHits = Number(raw?.cache_hit_count || 0);
		  return {
		    total,
		    successRate: total > 0 ? Math.round(success / total * 100) : 0,
		    totalTokens: Number(raw?.total_tokens || 0),
		    avgMs: Number(raw?.avg_duration_ms || 0),
		    cacheHitRate: total > 0 ? Math.round(cacheHits / total * 100) : 0
		  };
		}

		function updateStats(stats) {
		  document.querySelector('#historyStats .history-stat:nth-child(1) .stat-value').textContent = stats.total;
		  document.querySelector('#historyStats .history-stat:nth-child(2) .stat-value').textContent = stats.successRate + '%';
		  document.querySelector('#historyStats .history-stat:nth-child(3) .stat-value').textContent = fmtTokens(stats.totalTokens);
		  document.querySelector('#historyStats .history-stat:nth-child(4) .stat-value').textContent = fmtDuration(stats.avgMs);
		  document.querySelector('#historyStats .history-stat:nth-child(5) .stat-value').textContent = stats.cacheHitRate + '%';
		}

		function renderHistoryCards(entries) {
		  const scoped = visibleHistoryEntries(entries);
		  const filtered = _historyStatusFilter === 'all' ? scoped : scoped.filter(e => e.status === _historyStatusFilter);
		  if (!filtered.length) return '<div class="session-empty">无匹配的请求记录</div>';
		  const limit = _historyDisplayLimit || 50;
		  const show = filtered.slice(0, limit);
		  let html = '';
		  for (const e of show) {
		    const inputRatio = e.total_tokens > 0 ? Math.round((e.input_tokens || 0) / e.total_tokens * 100) : 50;
		    const providerLabel = e.provider_profile || e.provider || '';
		    const clientLabel = historyClientLabel(e.client_kind || 'codex');
		    const accountLabel = e.account_name || e.account_id || '';
		    const endpointLabel = e.endpoint_kind || '';
		    html += `<div class="history-card${e.status === 'failed' ? ' failed' : ''}">
		      <div class="hc-row">
		        <span class="hc-time">${fmtTime(e.created_at)}</span>
		        <span class="hc-model">${esc(e.model)}</span>
		        <span class="hc-chip">${esc(clientLabel)}</span>
		        ${accountLabel ? `<span class="hc-chip" title="${escAttr(e.account_id || '')}">${esc(accountLabel)}</span>` : ''}
		        ${providerLabel ? `<span class="hc-chip" title="Provider profile">${esc(providerLabel)}</span>` : ''}
		        ${statusBadge(e.status)}
		        <span class="hc-dur">${fmtDuration(e.duration_ms)}</span>
		      </div>
		      <div class="hc-tokens">
		        <span>入:${fmtTokens(e.input_tokens)}</span>
		        <span>出:${fmtTokens(e.output_tokens)}</span>
		        <span>总计:${fmtTokens(e.total_tokens)}</span>
		      </div>
		      <div class="hc-token-bar"><div class="hc-token-in" style="width:${inputRatio}%"></div><div class="hc-token-out" style="width:${100 - inputRatio}%"></div></div>
		      <div class="hc-url" title="${escAttr(e.upstream_url)}">${endpointLabel ? esc(endpointLabel + ' · ') : ''}${esc(trunc(e.upstream_url, 50))}</div>
		      ${e.error_msg ? `<div class="hc-error" onclick="this.nextElementSibling?.classList.toggle('hidden')">▸ 错误详情</div><div class="hc-error hidden" style="margin-top:2px;">${esc(e.error_msg)}</div>` : ''}
		    </div>`;
		  }
		  if (filtered.length > limit) {
		    html += `<div class="history-load-more" onclick="loadMoreHistory(${limit + _historyDisplayStep})">
		      显示 ${Math.min(filtered.length, limit + _historyDisplayStep)} / ${filtered.length} 条（点击加载更多）
		    </div>`;
		  }
		  return html;
		}

		function loadMoreHistory(newLimit) {
		  _historyDisplayLimit = newLimit;
		  const container = document.getElementById('historyCardsContainer');
		  if (container && _historyEntries.length) container.innerHTML = renderHistoryCards(_historyEntries);
		}

		function renderTrendChart(entries) {
		  const now = Date.now() / 1000;
		  let buckets = [];
		  if (_historyChartPeriod === 'hourly') {
		    for (let i = 23; i >= 0; i--) {
		      const slot = Math.floor(now / 3600) - i;
		      const d = new Date(slot * 3600 * 1000);
		      buckets.push({ label: d.getHours() + '时', start: slot * 3600, end: (slot + 1) * 3600, tokens: 0 });
		    }
		  } else if (_historyChartPeriod === 'daily') {
		    for (let i = 6; i >= 0; i--) {
		      const dayStart = new Date(new Date().getFullYear(), new Date().getMonth(), new Date().getDate() - i);
		      const start = Math.floor(dayStart.getTime() / 1000);
		      const end = start + 86400;
		      buckets.push({ label: (dayStart.getMonth() + 1) + '/' + dayStart.getDate(), start, end, tokens: 0 });
		    }
		  } else {
		    // 月度：从归档统计 + 当前月实时数据合并
		    const statMap = {};
		    for (const s of visibleMonthlyStats(_historyMonthlyStats)) {
		      statMap[s.year_month] = (statMap[s.year_month] || 0) + (s.total_tokens || 0);
		    }
		    // 覆盖当前月（实时数据优先）
		    const nowDate = new Date();
		    const thisMonth = nowDate.getFullYear() + '-' + String(nowDate.getMonth() + 1).padStart(2, '0');
		    let thisMonthTokens = 0;
		    const thisMonthStart = Math.floor(new Date(nowDate.getFullYear(), nowDate.getMonth(), 1).getTime() / 1000);
		    const nextMonthStart = Math.floor(new Date(nowDate.getFullYear(), nowDate.getMonth() + 1, 1).getTime() / 1000);
		    for (const e of entries) {
		      if (e.created_at >= thisMonthStart && e.created_at < nextMonthStart) {
		        thisMonthTokens += e.total_tokens || 0;
		      }
		    }
		    if (thisMonthTokens > 0 || !statMap[thisMonth]) statMap[thisMonth] = thisMonthTokens;
		    // 生成最近 6 个月桶
		    for (let i = 5; i >= 0; i--) {
		      const d = new Date(nowDate.getFullYear(), nowDate.getMonth() - i, 1);
		      const ym = d.getFullYear() + '-' + String(d.getMonth() + 1).padStart(2, '0');
		      buckets.push({ label: (d.getMonth() + 1) + '月', tokens: statMap[ym] || 0 });
		    }
		  }
		  if (_historyChartPeriod !== 'monthly') {
		    for (const e of entries) {
		      for (const b of buckets) {
		        if (e.created_at >= b.start && e.created_at < b.end) { b.tokens += e.total_tokens || 0; break; }
		      }
		    }
		  }
		  const maxTokens = Math.max(...buckets.map(b => b.tokens), 1);
		  let html = '';
		  for (const b of buckets) {
		    const pct = Math.round(b.tokens / maxTokens * 100);
		    html += '<div class="history-chart-col">';
		    html += '<span class="history-chart-val">' + fmtTokens(b.tokens) + '</span>';
		    html += '<div class="history-chart-bar-wrap"><div class="history-chart-bar" style="height:' + pct + '%"></div></div>';
		    html += '<span class="history-chart-label">' + b.label + '</span>';
		    html += '</div>';
		  }
		  return html;
		}

		function setChartPeriod(period) {
		  _historyChartPeriod = period;
		  document.querySelectorAll('.chart-tab').forEach(t => {
		    const txt = t.textContent.trim();
		    t.classList.toggle('active', (period === 'hourly' && txt === '1h') || (period === 'daily' && txt === '1d') || (period === 'monthly' && txt === '1m'));
		  });
		  const bars = document.getElementById('historyChartBars');
		  if (bars && _historyEntries.length) bars.innerHTML = renderTrendChart(_historyEntries);
		}

		function toggleAutoRefresh() {
		  if (_historyRefreshTimer) {
		    stopHistoryAutoRefresh();
		  } else {
		    _historyRefreshMs = parseInt(document.getElementById('historyIntervalSel').value) || 10000;
		    _historyRefreshTimer = setInterval(refreshHistory, _historyRefreshMs);
		    syncHistoryAutoRefreshUi();
		  }
		}

		function stopHistoryAutoRefresh() {
		  if (_historyRefreshTimer) {
		    clearInterval(_historyRefreshTimer);
		    _historyRefreshTimer = null;
		  }
		  _historyRefreshMs = 0;
		  syncHistoryAutoRefreshUi();
		}

		function setRefreshInterval(val) {
		  _historyRefreshMs = parseInt(val);
		  if (_historyRefreshTimer) { clearInterval(_historyRefreshTimer); _historyRefreshTimer = setInterval(refreshHistory, _historyRefreshMs); }
		  syncHistoryAutoRefreshUi();
		}

		function setStatusFilter(filter) {
		  _historyStatusFilter = filter;
		  const container = document.getElementById('historyCardsContainer');
		  if (container && _historyEntries.length) container.innerHTML = renderHistoryCards(_historyEntries);
		}

		function setHistoryClientKind(kind) {
		  _historyClientKindFilter = kind || 'all';
		  _historyAccountFilter = '';
		  _historyDisplayLimit = 50;
		  refreshHistory();
		}

		function setHistoryAccountFilter(accountId) {
		  _historyAccountFilter = accountId || '';
		  _historyDisplayLimit = 50;
		  refreshHistory();
		}

		async function refreshHistory() {
		  const statsEl = document.getElementById('historyStats');
		  const barsEl = document.getElementById('historyChartBars');
		  const cardsEl = document.getElementById('historyCardsContainer');
		  try {
		    const listArgs = historyFilterArgs({ limit: 3000 });
		    const shouldLoadAllEntries = Boolean(listArgs.clientKind || listArgs.accountId);
		    const [entries, allEntries, monthlyStats, todayStats, accountsPayload] = await Promise.all([
		      invoke('list_request_history', listArgs),
		      shouldLoadAllEntries ? invoke('list_request_history', { limit: 3000 }).catch(() => null) : Promise.resolve(null),
		      invoke('get_monthly_stats', historyFilterArgs({ limit: 60 })),
		      invoke('get_request_stats_since', historyFilterArgs({ since: todayStartUnixSecs() })),
		      invoke('list_accounts', {}),
		    ]);
		    _historyEntries = entries || [];
		    _historyAllEntries = Array.isArray(allEntries) ? allEntries : _historyEntries;
		    _historyMonthlyStats = monthlyStats || [];
		    _historyAccounts = accountsPayload?.accounts || [];
		    saveHistoryCache(_historyAllEntries, _historyMonthlyStats, _historyAccounts);
		    _historyOffline = false;
		    stopReconnectPolling();
		    hideHistoryOfflineBanner();;
		    const switcher = document.getElementById('historyClientSwitcher');
		    if (switcher) switcher.innerHTML = renderHistoryClientSwitcher();
		    const accountSel = document.getElementById('historyAccountFilter');
		    if (accountSel) accountSel.innerHTML = renderHistoryAccountOptions();
		    const visibleEntries = visibleHistoryEntries(_historyEntries);
		    if (visibleEntries.length) {
		      if (statsEl) updateStats(statsFromAggregate(todayStats));
		      if (barsEl) barsEl.innerHTML = renderTrendChart(visibleEntries);
		      if (cardsEl) cardsEl.innerHTML = renderHistoryCards(visibleEntries);
		    } else {
		      if (statsEl) statsEl.innerHTML = '<div class="history-stat"><div class="stat-value">0</div><div class="stat-label">今日请求数</div></div><div class="history-stat green"><div class="stat-value">—</div><div class="stat-label">成功率</div></div><div class="history-stat accent"><div class="stat-value">0</div><div class="stat-label">Token 消耗</div></div><div class="history-stat"><div class="stat-value">—</div><div class="stat-label">平均耗时</div></div><div class="history-stat cache"><div class="stat-value">—</div><div class="stat-label">命中缓存</div></div>';
		      if (barsEl) barsEl.innerHTML = '<div class="session-empty" style="font-size:11px;padding:10px;">暂无数据</div>';
		      if (cardsEl) cardsEl.innerHTML = '<div class="session-empty">暂无请求记录，发送一次 API 请求后会自动出现</div>';
		    }
		  } catch (e) {
		    const cached = loadHistoryCache();
		    if (cached) {
		      _historyAllEntries = cached.entries || [];
		      _historyEntries = visibleHistoryEntries(_historyAllEntries);
		      _historyMonthlyStats = cached.monthlyStats || [];
		      _historyAccounts = cached.accounts || [];
		      _historyOffline = true;
		      showHistoryOfflineBanner();
		      startReconnectPolling();
		      const switcher = document.getElementById('historyClientSwitcher');
		      if (switcher) switcher.innerHTML = renderHistoryClientSwitcher();
		      const accountSel = document.getElementById('historyAccountFilter');
		      if (accountSel) accountSel.innerHTML = renderHistoryAccountOptions();
		      const visibleEntries = visibleHistoryEntries(_historyEntries);
		      if (visibleEntries.length) {
		        if (statsEl) updateStats(computeStats(filterToday(visibleEntries)));
		        if (barsEl) barsEl.innerHTML = renderTrendChart(visibleEntries);
		        if (cardsEl) cardsEl.innerHTML = renderHistoryCards(_historyEntries);
		      } else {
		        if (statsEl) updateStats({ total: 0, successRate: 0, totalTokens: 0, avgMs: 0, cacheHitRate: 0 });
		        if (barsEl) barsEl.innerHTML = '<div class="session-empty" style="font-size:11px;padding:10px;">暂无数据</div>';
		        const emptyText = _historyAllEntries.length ? '无匹配的请求记录' : '暂无缓存数据，服务启动后将自动刷新';
		        if (cardsEl) cardsEl.innerHTML = '<div class="session-empty">' + emptyText + '</div>';
		      }
		    } else {
		      if (cardsEl) cardsEl.innerHTML = '<div class="session-empty" style="color:var(--red);">加载失败: ' + esc(e.message || String(e)) + '</div>';
		    }
		  }
		}

		function loadHistoryCache() {
		  try {
		    const raw = deeStorage.getItem(HISTORY_CACHE_KEY);
		    if (!raw) return null;
		    const data = JSON.parse(raw);
		    if (!data || !data.entries) return null;
		    return data;
		  } catch (_) { return null; }
		}

		function saveHistoryCache(entries, monthlyStats, accounts) {
		  try {
		    deeStorage.setItem(HISTORY_CACHE_KEY, JSON.stringify({
		      entries: entries || [],
		      monthlyStats: monthlyStats || [],
		      accounts: accounts || [],
		      savedAt: Date.now()
		    }));
		  } catch (_) {}
		}

		function clearHistoryCache() {
		  try {
		    deeStorage.removeItem(HISTORY_CACHE_KEY);
		  } catch (_) {}
		}

		function showHistoryOfflineBanner() {
		  const existing = document.getElementById('historyOfflineBanner');
		  if (existing) { existing.style.display = ''; return; }
		  const statsEl = document.getElementById('historyStats');
		  if (!statsEl) return;
		  const div = document.createElement('div');
		  div.id = 'historyOfflineBanner';
		  div.style.cssText = 'display:flex;align-items:center;justify-content:space-between;padding:8px 12px;margin-bottom:12px;background:rgba(251,191,36,0.1);border:1px solid rgba(251,191,36,0.3);border-radius:8px;font-size:12px;color:var(--yellow,#b45309);';
		  div.innerHTML = '<span>⚠ 服务未启动，当前显示的是本地缓存数据</span><button class="btn btn-sm btn-primary" onclick="refreshHistory()" style="font-size:11px;padding:4px 10px;">⟳ 尝试重连</button>';
		  statsEl.parentNode.insertBefore(div, statsEl);
		}

		function hideHistoryOfflineBanner() {
		  const banner = document.getElementById('historyOfflineBanner');
		  if (banner) banner.style.display = 'none';
		}

		function startReconnectPolling() {
				  if (_historyReconnectTimer) return;
		  _historyReconnectTimer = setInterval(async () => {
		    try {
		      const status = await invoke('get_service_status');
		      if (status && status.running) {
		        stopReconnectPolling();
		        refreshHistory();
		      }
		    } catch (_) {}
		  }, 3000);
		}

		function stopReconnectPolling() {
		  if (_historyReconnectTimer) { clearInterval(_historyReconnectTimer); _historyReconnectTimer = null; }
		}

		function fmtTime(unixSecs) {
		  if (!unixSecs) return '—';
		  const d = new Date(unixSecs * 1000);
		  const pad = n => String(n).padStart(2, '0');
		  return `${d.getMonth() + 1}/${d.getDate()} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
		}

		function fmtTokens(n) {
		  if (!n) return '0';
		  if (n >= 1000000000) return compactNumber(n / 1000000000) + 'B';
		  if (n >= 1000000) return compactNumber(n / 1000000) + 'M';
		  if (n >= 1000) return compactNumber(n / 1000) + 'k';
		  return n.toString();
		}

		function compactNumber(value) {
		  const digits = value >= 100 ? 1 : value >= 10 ? 1 : 2;
		  return value.toFixed(digits).replace(/\.0$/, '');
		}

		function fmtDuration(ms) {
		  if (!ms) return '—';
		  if (ms < 1000) return ms + 'ms';
		  if (ms < 60000) return (ms / 1000).toFixed(1) + 's';
		  return (ms / 60000).toFixed(1) + 'min';
		}

		function statusBadge(status) {
		  const map = { completed: ['#22c55e', '✓ 成功'], failed: ['#ef4444', '✗ 失败'] };
		  const [color, label] = map[status] || ['#9ca3af', status];
		  return `<span style="color:${color};font-weight:500;">${label}</span>`;
		}

		async function clearHistory() {
		  const scope = _historyAccountFilter
		    ? '当前账号'
		    : (_historyClientKindFilter === 'all' ? '所有请求历史' : historyClientLabel(_historyClientKindFilter) + ' 请求历史');
				  var ok = await showConfirm('确定要清空' + scope + '吗？此操作不可恢复。');
		  if (!ok) return;
		  try {
		    await invoke('clear_request_history', historyFilterArgs());
		    showToast('请求历史已清空', 'success');
		    _historyEntries = [];
		    _historyMonthlyStats = [];
		    clearHistoryCache();
		    refreshHistory();
		  } catch (e) {
		    showToast('清空失败: ' + e, 'error');
		  }
		}

		window.stopHistoryAutoRefresh = stopHistoryAutoRefresh;
		window.stopHistoryReconnectPolling = stopReconnectPolling;

// 线程聚合
// ═══════════════════════════════════════════════════════════════
