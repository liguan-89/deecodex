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
const HISTORY_LIST_LIMIT = 20000;

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

			function historyRouteLabel(path) {
			  const value = String(path || '');
			  if (value.startsWith('/codex-router/')) return 'Router';
			  if (value.startsWith('/dex-assistant/')) return 'DEX';
			  if (value.startsWith('/codex-desktop/')) return 'Desktop';
			  if (value.startsWith('/codex-cli/')) return 'CLI';
			  if (value.startsWith('/client-proxy/')) return 'Client Proxy';
			  return '';
			}

				function historyRouteTitle(entry) {
				  const parts = [];
				  if (entry.request_path) parts.push(entry.request_path);
				  if (entry.endpoint_kind) parts.push(entry.endpoint_kind);
				  if (entry.account_id) parts.push(entry.account_id);
				  return parts.join(' · ');
				}

				function historyRouteReasonLabel(reason) {
				  const labels = {
				    ready: '可用',
				    no_anchor: '无锚点',
				    routing_disabled: '已停用',
				    pool_mismatch: '非同池',
				    no_supported_endpoint: '未映射',
				    account_quota_cooling: '账号额度冷却',
					    account_cooling_down: '旧账号运行态',
					    account_retry_wait: '旧账号运行态',
					    model_quota_cooling: '模型额度冷却',
					    model_cooling_down: '旧模型运行态',
					    model_retry_wait: '旧模型运行态',
				    capability_mismatch: '能力不匹配',
				    attempt_failed: '本次已失败',
				    recent_account_error: '账号近期错误',
				    recent_model_error: '模型近期错误',
				  };
				  return labels[reason] || reason || '未知';
				}

				function parseHistoryRouteTrace(entry) {
				  const raw = entry?.route_trace;
				  if (!raw) return null;
				  if (typeof raw === 'object') return raw;
				  if (typeof raw !== 'string') return null;
				  try {
				    const value = JSON.parse(raw);
				    return value && typeof value === 'object' ? value : null;
				  } catch (_) {
				    return null;
				  }
				}

				function historyRouterNodeLabel(node) {
				  if (!node) return '未知账号';
				  return node.account_name || node.account_id || '未知账号';
				}

				function renderHistoryRouterFlow(trace) {
				  const rows = [];
				  const attempts = Array.isArray(trace.fallback_attempts) ? trace.fallback_attempts : [];
				  attempts.forEach(attempt => {
				    const name = historyRouterNodeLabel(attempt);
				    const status = attempt.status ? `HTTP ${attempt.status}` : '';
				    const message = attempt.message ? ` · ${attempt.message}` : '';
				    const title = [
				      `第 ${attempt.attempt || '?'} 次尝试失败`,
				      `账号: ${name}`,
				      attempt.endpoint_kind ? `端点: ${attempt.endpoint_kind}` : '',
				      attempt.mapped_model ? `模型: ${attempt.mapped_model}` : '',
				      status,
				      attempt.code ? `错误码: ${attempt.code}` : '',
				      attempt.message || '',
				    ].filter(Boolean).join('\n');
				    rows.push(`<span class="hc-route-step warn" title="${escAttr(title)}">降级 ${esc(name)} · ${esc(status || '失败')}${message ? ` · ${esc(trunc(attempt.message, 36))}` : ''}</span>`);
				  });

				  if (attempts.length) {
				    const selected = trace.selected || {};
				    rows.push(`<span class="hc-route-step ok">最终 ${esc(historyRouterNodeLabel(selected))}</span>`);
				  }

				  if (!rows.length) return '';
				  return `<div class="hc-route-flow">${rows.join('')}</div>`;
				}

				function renderHistoryRouteTrace(entry) {
				  const trace = parseHistoryRouteTrace(entry);
				  if (!trace || trace.route_surface !== 'codex_router') return '';
				  const anchor = trace.anchor || {};
				  const selected = trace.selected || {};
				  const capabilities = selected.capabilities || {};
				  const anchorName = anchor.account_name || anchor.account_id || '无锚点';
				  const selectedName = selected.account_name || selected.account_id || '暂无执行账号';
				  const mappedModel = selected.mapped_model || trace.requested_model || '';
				  const toolDecisions = trace.tool_decisions || selected.tool_decisions || {};
				  const capabilityBits = [
				    capabilities.protocol,
				    capabilities.tool_mode && capabilities.tool_mode !== 'none' ? `tools:${capabilities.tool_mode}` : '',
				    capabilities.vision && capabilities.vision !== 'off' ? `vision:${capabilities.vision}` : '',
				    capabilities.web ? 'web' : '',
				    capabilities.image_generation ? 'image2' : '',
				  ].filter(Boolean);
				  const toolDecisionBits = [
				    Array.isArray(toolDecisions.kept) && toolDecisions.kept.length ? `保留 ${toolDecisions.kept.length}` : '',
				    Array.isArray(toolDecisions.translated) && toolDecisions.translated.length ? `转译 ${toolDecisions.translated.length}` : '',
				    Array.isArray(toolDecisions.local) && toolDecisions.local.length ? `本地 ${toolDecisions.local.length}` : '',
				    Array.isArray(toolDecisions.filtered) && toolDecisions.filtered.length ? `过滤 ${toolDecisions.filtered.length}` : '',
				  ].filter(Boolean);
				  const total = Number(trace.candidate_count || 0);
				  const eligible = Number(trace.eligible_count || 0);
				  const skipped = Number(trace.skipped_count || Math.max(0, total - eligible));
				  const skippedLines = Array.isArray(trace.candidates)
				    ? trace.candidates
				      .filter(candidate => !candidate.eligible)
				      .map(candidate => {
				        const gaps = Array.isArray(candidate.capability_gaps) && candidate.capability_gaps.length
				          ? ` (${candidate.capability_gaps.join('/')})`
				          : '';
				        return `${candidate.account_name || candidate.account_id || '未知账号'}: ${historyRouteReasonLabel(candidate.reason)}${gaps}`;
				      })
				    : [];
				  const titleParts = [
				    `锚点 ${anchorName}`,
				    `执行 ${selectedName}`,
				    mappedModel ? `模型 ${mappedModel}` : '',
				    capabilityBits.length ? `能力 ${capabilityBits.join('/')}` : '',
				    toolDecisionBits.length ? `工具 ${toolDecisionBits.join(' / ')}` : '',
				    Array.isArray(toolDecisions.labels) && toolDecisions.labels.length ? `请求工具 ${toolDecisions.labels.join(', ')}` : '',
				    Array.isArray(trace.fallback_attempts) && trace.fallback_attempts.length ? `降级 ${trace.fallback_attempts.length} 次` : '',
				    `候选 ${eligible}/${total}`,
				    skippedLines.join('\n'),
				  ].filter(Boolean);
				  const routeLine = `<div class="hc-route" title="${escAttr(titleParts.join('\n'))}">
				    Router ${esc(anchorName)} → ${esc(selectedName)}
				    <span>${esc(mappedModel || '原模型')}</span>
				    ${capabilityBits.length ? `<span>${esc(capabilityBits.join('/'))}</span>` : ''}
				    ${toolDecisionBits.length ? `<span>${esc(toolDecisionBits.join('/'))}</span>` : ''}
				    <span>候选 ${eligible}/${total}${skipped ? ` · 跳过 ${skipped}` : ''}</span>
				  </div>`;
				  return routeLine + renderHistoryRouterFlow(trace);
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
		    const label = kind === 'generic_client' ? '通用' : (profile.label || historyClientLabel(kind));
		    return `<button type="button" class="history-client-tab${active}" onclick="setHistoryClientKind('${escAttr(kind)}')">
		      <span>${esc(label)}</span><em>${count}</em>
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
		  <div id="historyClientSwitcher" class="history-client-switcher">${renderHistoryClientSwitcher()}</div>
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
		    <div class="history-control-group">
		      <select class="history-select" id="historyAccountFilter" onchange="setHistoryAccountFilter(this.value)">
		        ${renderHistoryAccountOptions()}
		      </select>
		      <select class="history-select" id="historyStatusFilter" onchange="setStatusFilter(this.value)">
		        <option value="all" ${_historyStatusFilter === 'all' ? 'selected' : ''}>全部状态</option>
		        <option value="completed" ${_historyStatusFilter === 'completed' ? 'selected' : ''}>仅成功</option>
		        <option value="failed" ${_historyStatusFilter === 'failed' ? 'selected' : ''}>仅失败</option>
		      </select>
		      <label class="history-toggle${_historyRefreshTimer ? ' on' : ''}" id="historyAutoToggle" onclick="toggleAutoRefresh()">
		        <div class="toggle-dot"></div><span>自动刷新</span>
		      </label>
		      <select class="history-select history-interval-select" id="historyIntervalSel" onchange="setRefreshInterval(this.value)" style="${_historyRefreshTimer ? '' : 'display:none;'}">
		        <option value="5000" ${_historyRefreshMs === 5000 ? 'selected' : ''}>5s</option>
		        <option value="10000" ${_historyRefreshMs === 10000 || !_historyRefreshMs ? 'selected' : ''}>10s</option>
		        <option value="30000" ${_historyRefreshMs === 30000 ? 'selected' : ''}>30s</option>
		        <option value="60000" ${_historyRefreshMs === 60000 ? 'selected' : ''}>60s</option>
		      </select>
		    </div>
		    <div class="history-action-group">
		      <button class="btn btn-primary" onclick="refreshHistory()">刷新</button>
		      <button class="btn btn-ghost history-clear-btn" onclick="clearHistory()">清空历史</button>
		    </div>
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
			    const routeLabel = historyRouteLabel(e.request_path);
			    html += `<div class="history-card${e.status === 'failed' ? ' failed' : ''}">
			      <div class="hc-row">
			        <span class="hc-time">${fmtTime(e.created_at)}</span>
			        <span class="hc-model">${esc(e.model)}</span>
			        <span class="hc-chip">${esc(clientLabel)}</span>
			        ${routeLabel ? `<span class="hc-chip" title="${escAttr(historyRouteTitle(e))}">${esc(routeLabel)}</span>` : ''}
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
			      ${renderHistoryRouteTrace(e)}
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
		  for (let i = 0; i < buckets.length; i++) {
		    const b = buckets[i];
		    const pct = Math.round(b.tokens / maxTokens * 100);
		    const value = fmtTokens(b.tokens);
		    const showValue = b.tokens > 0 && _historyChartPeriod !== 'hourly';
		    const label = _historyChartPeriod === 'hourly'
		      ? (i % 2 === 1 ? '' : String(b.label).replace('时', '').padStart(2, '0'))
		      : b.label;
		    const hoverLabel = (_historyChartPeriod === 'hourly' ? String(b.label).padStart(3, '0') : b.label) + ' · ' + value;
		    html += '<div class="history-chart-col' + (b.tokens > 0 ? '' : ' empty') + '" aria-label="' + escAttr(hoverLabel) + '" data-value="' + escAttr(value) + '">';
		    html += '<span class="history-chart-val" title="' + escAttr(value) + '">' + (showValue ? esc(value) : '') + '</span>';
		    html += '<div class="history-chart-bar-wrap"><div class="history-chart-bar" style="height:' + pct + '%"></div></div>';
		    html += '<span class="history-chart-label">' + esc(label) + '</span>';
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
		    const listArgs = historyFilterArgs({ limit: HISTORY_LIST_LIMIT });
		    const shouldLoadAllEntries = Boolean(listArgs.clientKind || listArgs.accountId);
		    const [entries, allEntries, monthlyStats, todayStats, accountsPayload] = await Promise.all([
		      invoke('list_request_history', listArgs),
		      shouldLoadAllEntries ? invoke('list_request_history', { limit: HISTORY_LIST_LIMIT }).catch(() => null) : Promise.resolve(null),
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
		  div.className = 'history-offline-banner';
		  div.innerHTML = '<span>服务未启动，当前显示的是本地缓存数据</span><button class="btn btn-sm btn-primary" onclick="refreshHistory()">尝试重连</button>';
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
