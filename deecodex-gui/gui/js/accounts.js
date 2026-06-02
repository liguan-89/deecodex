const CODEX_MODEL_LIST = ['gpt-5.5', 'gpt-5.4', 'gpt-5.4-mini', 'gpt-5.3-codex', 'gpt-5.3-codex-spark', 'gpt-5.2', 'codex-auto-review'];
const MIMO_CODING_MODEL = 'mimo-v2.5-pro';
const MIMO_VISION_MODEL = 'mimo-v2-omni';
const MIMO_CHAT_MODELS = [MIMO_VISION_MODEL, 'mimo-v2-pro', 'mimo-v2.5', MIMO_CODING_MODEL];
const CLIENT_KIND_LABELS = {
  codex: 'Codex',
  claude_code: 'Claude',
  openclaw: 'OpenClaw',
  hermes: 'Hermes',
  generic_client: '通用客户端',
};
const CLIENT_SURFACE_LABELS = {
  cli: 'CLI',
  desktop: '桌面版',
};
var oauthLoginState = null;
var oauthLoginPollTimer = null;
var selectedClientSurface = typeof selectedClientSurface === 'string' ? selectedClientSurface : 'cli';

function providerBadgeClass(p) {
  return 'badge-provider badge-' + (p || 'custom');
}

function providerLogoSlug(p) {
  return ['openrouter', 'deepseek', 'kimi', 'minimax', 'mimo', 'longcat', 'qwen', 'glm', 'openai', 'anthropic', 'google-ai', 'openclaw', 'hermes', 'claude', 'codex', 'claude-code'].includes(p)
    ? p
    : 'custom';
}

function providerLogoSrc(p) {
  const files = {
    openrouter: 'openrouter.webp',
    deepseek: 'deepseek.png',
    kimi: 'kimi.png',
    minimax: 'minimax.webp',
    mimo: 'mimo.webp',
    longcat: 'longcat.png',
    qwen: 'qwen.svg',
    anthropic: 'anthropic.png',
    claude: 'anthropic.png',
    glm: 'glm-wordmark.png',
    openai: 'openai.png',
    codex: 'codex.png',
    'claude-code': 'claude-code.png',
    openclaw: 'openclaw.png',
    hermes: 'hermes.png',
  };
  const slug = providerLogoSlug(p);
  return `assets/provider-logos/${files[slug] || slug + '.svg'}`;
}

function providerIcon(p, label) {
  const slug = providerLogoSlug(p);
  return `<img class="provider-logo-img provider-logo-${escAttr(slug)}" src="${providerLogoSrc(p)}" alt="${escAttr(label || p || '自定义')}">`;
}

function renderProviderBadge(p) {
  const slug = p || 'custom';
  return `<span class="${providerBadgeClass(slug)}"><img src="${providerLogoSrc(slug)}" alt="" aria-hidden="true">${esc(slug)}</span>`;
}

function normalizeClientKind(kind) {
  const raw = String(kind || 'codex').trim();
  const value = raw.replace(/[-\s]+/g, '_').toLowerCase();
  if (value === 'claudecode' || value === 'claude_cli' || value === 'claude_desktop') return 'claude_code';
  if (value === 'open_claw') return 'openclaw';
  if (value === 'genericclient') return 'generic_client';
  return ['codex', 'claude_code', 'openclaw', 'hermes', 'generic_client'].includes(value) ? value : 'codex';
}

function accountClientKind(a) {
  const candidates = [
    a?.client_kind,
    a?.clientKind,
    a?.client_type,
    a?.kind,
    a?.client_options?.client_kind,
    a?.client_options?.kind,
    a?.target,
  ].filter(value => value && value !== 'client_config' && value !== 'codex_proxy');
  for (const candidate of candidates) {
    const normalized = normalizeClientKind(candidate);
    if (normalized !== 'codex' || String(candidate || '').toLowerCase() === 'codex') return normalized;
  }
  const provider = String(a?.provider || '').toLowerCase();
  if (provider === 'openclaw') return 'openclaw';
  if (provider === 'hermes') return 'hermes';
  const name = String(a?.name || '').toLowerCase();
  if (name.includes('openclaw')) return 'openclaw';
  if (name.includes('hermes')) return 'hermes';
  if (name.includes('claude')) return 'claude_code';
  return 'codex';
}

function isCodexAccount(a) {
  return accountClientKind(a) === 'codex';
}

function clientKindSupportsSurface(kind) {
  const normalized = normalizeClientKind(kind);
  return normalized === 'codex' || normalized === 'claude_code';
}

function normalizeClientSurface(surface) {
  const value = String(surface || 'cli').toLowerCase();
  return value === 'desktop' ? 'desktop' : 'cli';
}

function selectedSurfaceForKind(kind) {
  return clientKindSupportsSurface(kind) ? normalizeClientSurface(selectedClientSurface) : 'cli';
}

function accountClientSurface(a) {
  const kind = accountClientKind(a);
  if (!clientKindSupportsSurface(kind)) return 'cli';
  return normalizeClientSurface(a?.client_surface || a?.client_options?.client_surface || 'cli');
}

function clientSurfaceLabel(surface) {
  return CLIENT_SURFACE_LABELS[normalizeClientSurface(surface)] || 'CLI';
}

function clientSurfaceTitle(kind, surface) {
  const normalized = normalizeClientKind(kind);
  const base = CLIENT_KIND_LABELS[normalized] || normalized;
  if (!clientKindSupportsSurface(normalized)) return base;
  return `${base} ${normalizeClientSurface(surface) === 'desktop' ? '桌面版' : 'CLI'}`;
}

function accountDisplayTitle(a) {
  const provider = getProviderPreset(a?.provider);
  const providerLabel = provider?.label || '';
  const name = String(a?.name || providerLabel || '').trim();
  const suffix = /\s*账号\s*$/;
  if (providerLabel && suffix.test(name)) {
    const base = name.replace(suffix, '').trim();
    if (base === providerLabel || base.toLowerCase() === String(a?.provider || '').toLowerCase()) return providerLabel;
  }
  return name.replace(suffix, '').trim() || providerLabel || name;
}

function officialAccountEmail(a) {
  const options = a?.client_options || {};
  return options?.oauth?.email
    || options?.oauth_quota?.usage_email
    || options?.oauth_quota?.email
    || a?.email
    || '';
}

function accountRuntimeBadge(a) {
  const status = a?.runtime_state?.status || 'active';
  const labels = {
    active: '',
    error: '错误',
    cooling_down: '冷却',
    quota_exceeded: '配额',
  };
  const label = labels[status] || '';
  if (!label) return '';
  const retry = a?.runtime_state?.next_retry_after;
  const title = retry ? `下一次重试：${new Date(Number(retry) * 1000).toLocaleString()}` : label;
  return `<span class="runtime-badge runtime-${escAttr(status)}" title="${escAttr(title)}">${esc(label)}</span>`;
}

function endpointIsCodexOfficial(endpoint) {
  const kind = endpoint?.kind || '';
  return kind === 'codex_official' || kind === 'CodexOfficial';
}

function isCodexOfficialAccount(a) {
  if (!isCodexAccount(a)) return false;
  return Array.isArray(a?.endpoints) && a.endpoints.some(endpointIsCodexOfficial);
}

function accountRouting(a) {
  const raw = a?.routing || a?.client_options?.routing || {};
  const enabled = raw.enabled !== false && raw.disabled !== true;
  const weight = Math.max(1, Math.min(100, Number(raw.weight || 1)));
  const official = isCodexOfficialAccount(a);
  const anchorEnabled = typeof raw.anchor_enabled === 'boolean' ? raw.anchor_enabled : official;
  const executionEnabled = typeof raw.execution_enabled === 'boolean' ? raw.execution_enabled : !official;
  return {
    enabled,
    anchor_enabled: enabled && anchorEnabled,
    execution_enabled: enabled && executionEnabled,
    pool: String(raw.pool || 'codex-official'),
    priority: Number(raw.priority || 0),
    weight,
  };
}

function renderCodexOfficialRouteLine(a) {
  if (!isCodexOfficialAccount(a)) return '';
  const routing = accountRouting(a);
  const quota = officialQuotaSnapshot(a) || {};
  const plan = planDisplayName(quota.plan_type || quota.plan || '');
  const accountId = escAttr(a.id || '');
  const toggle = `<label class="account-pool-switch toggle-label" title="${routing.anchor_enabled ? '登录态锚点已启用' : '登录态锚点已停用'}">
    <input type="checkbox" aria-label="作为 Router 登录态锚点" ${routing.anchor_enabled ? 'checked' : ''}${accountId ? ` onchange="toggleAccountRouting('${accountId}', 'anchor')"` : ' disabled'}>
  </label>`;
  const role = routing.execution_enabled ? '<span class="account-plan-pill">可执行</span>' : '<span class="account-plan-pill">仅锚点</span>';
  return `${toggle}${plan !== '未知' ? `<span class="account-plan-pill">${esc(plan)}</span>` : ''}${role}`;
}

function runtimeStatusLabel(status) {
  const labels = {
    active: '正常',
    error: '错误',
    cooling_down: '冷却',
    quota_exceeded: '配额耗尽',
    Active: '正常',
    Error: '错误',
    CoolingDown: '冷却',
    QuotaExceeded: '配额耗尽',
  };
  return labels[status] || status || '正常';
}

function formatRuntimeDate(ts) {
  const n = Number(ts || 0);
  if (!n) return '—';
  return new Date(n * 1000).toLocaleString();
}

function renderCodexRouterPanel(a) {
  if (!isCodexAccount(a)) return '';
  const routing = accountRouting(a);
  const official = isCodexOfficialAccount(a);
  const accountId = escAttr(a.id || '');
  return `<section class="account-edit-section codex-router-section">
    <div class="account-section-head">
      <div class="section-sub-label">Router 路由</div>
    </div>
    <div class="config-fields">
      ${official ? `<div class="config-field">
        <label class="toggle-label">
          <input type="checkbox" id="edit_routing_anchor_enabled" ${routing.anchor_enabled ? 'checked' : ''}>
          作为登录态锚点
        </label>
        <span class="hint">只负责让 Codex Desktop 保留官方登录态和能力入口，不代表参与模型响应。</span>
      </div>` : ''}
      <div class="config-field">
        <label class="toggle-label">
          <input type="checkbox" id="edit_routing_execution_enabled" ${routing.execution_enabled ? 'checked' : ''}>
          参与模型执行
        </label>
        <span class="hint">${official ? '官方号默认关闭；只有明确开启后才会作为执行候选。' : '开启后作为同池执行账号，由 Router 按能力和健康度选择。'}</span>
      </div>
      <div class="config-field">
        <label>Pool</label>
        <input type="text" id="edit_routing_pool" value="${escAttr(routing.pool)}" placeholder="codex-official">
      </div>
      <div class="config-field">
        <label>优先级</label>
        <input type="number" id="edit_routing_priority" value="${Number(routing.priority || 0)}" min="-1000" max="1000" step="1">
      </div>
      <div class="config-field">
        <label>权重</label>
        <input type="number" id="edit_routing_weight" value="${Number(routing.weight || 1)}" min="1" max="100" step="1">
      </div>
    </div>
    <div class="official-runtime-actions">
      ${a.id ? `<button type="button" class="btn btn-ghost" onclick="applyAccountRoutingFromDetail('${accountId}')">应用路由</button>` : ''}
    </div>
  </section>`;
}

function renderCodexOfficialRuntimePanel(a) {
  if (!endpointIsCodexOfficial(currentEndpoint(a)) && !isCodexOfficialAccount(a)) return '';
  const runtime = a?.runtime_state || {};
  const modelStates = runtime.model_states && typeof runtime.model_states === 'object' ? runtime.model_states : {};
  const rows = Object.entries(modelStates);
  const accountId = escAttr(a.id || '');
  return `<section class="account-edit-section official-runtime-section">
    <div class="account-section-head">
      <div class="section-sub-label">官方登录态</div>
    </div>
    <div class="official-runtime-summary">
      <div>${esc(officialAccountEligibility(a))}</div>
      <span>${esc(officialRuntimeSummaryText(a))}</span>
    </div>
    <div class="official-runtime-actions">
      ${a.id ? `<button type="button" class="btn btn-ghost" onclick="clearAccountCooldown('${accountId}')">清除冷却</button>` : ''}
      ${a.id ? `<button type="button" class="btn btn-ghost" onclick="resetAccountRuntime('${accountId}')">重置运行态</button>` : ''}
    </div>
    <div class="official-quota-panel" id="official-quota-${accountId}">
      ${renderOfficialQuotaSnapshot(a)}
    </div>
    <div class="runtime-state-grid">
      <div><span>状态</span><strong>${esc(runtimeStatusLabel(runtime.status || 'active'))}</strong></div>
      <div><span>下一恢复</span><strong>${esc(formatRuntimeDate(runtime.next_retry_after))}</strong></div>
      <div><span>成功</span><strong>${Number(runtime.success || 0)}</strong></div>
      <div><span>失败</span><strong>${Number(runtime.failed || 0)}</strong></div>
    </div>
    <div class="runtime-model-table">
      <div class="runtime-model-head">
        <span>模型</span>
        <span>状态</span>
        <span>下一恢复</span>
        <span>说明</span>
      </div>
      ${rows.length ? rows.map(([model, state]) => `<div class="runtime-model-row">
        <span title="${escAttr(model)}">${esc(model)}</span>
        <span>${esc(runtimeStatusLabel(state?.status || 'active'))}</span>
        <span>${esc(formatRuntimeDate(state?.next_retry_after))}</span>
        <span title="${escAttr(state?.status_message || '')}">${esc(state?.status_message || '—')}</span>
      </div>`).join('') : `<div class="runtime-model-empty">${esc(runtimeModelEmptyText(a))}</div>`}
    </div>
  </section>`;
}

function runtimeModelEmptyText(a) {
  const runtime = a?.runtime_state || {};
  const success = Number(runtime.success || 0);
  const failed = Number(runtime.failed || 0);
  const quota = officialQuotaSnapshot(a);
  if (success || failed || quota) {
    return '暂无模型级冷却；账号级请求和额度状态见上方统计，当前没有单模型限制。';
  }
  return '暂无模型级冷却；尚无请求记录，当前按可用处理。';
}

function officialAccountEligibility(a) {
  const routing = accountRouting(a);
  const runtime = a?.runtime_state || {};
  const retry = Number(runtime.next_retry_after || 0);
  const now = Math.floor(Date.now() / 1000);
  if (!routing.anchor_enabled) return '锚点停用';
  if (retry && retry > now) return `${runtimeStatusLabel(runtime.status)}中`;
  if ((runtime.status || 'active') === 'error') return '最近错误';
  return routing.execution_enabled ? '锚点+执行' : '登录态锚点';
}

function officialRuntimeSummaryText(a) {
  const runtime = a?.runtime_state || {};
  const success = Number(runtime.success || 0);
  const failed = Number(runtime.failed || 0);
  const retry = runtime.next_retry_after ? `，恢复 ${formatRuntimeDate(runtime.next_retry_after)}` : '';
  if (!success && !failed) return `尚无请求记录${retry}`;
  return `成功 ${success} / 失败 ${failed}${retry}`;
}

function officialQuotaSnapshot(a) {
  const quota = a?.client_options?.oauth_quota;
  return quota && typeof quota === 'object' && !Array.isArray(quota) ? quota : null;
}

function compactNumber(value) {
  const n = Number(value || 0);
  if (!Number.isFinite(n)) return '0';
  return n.toLocaleString();
}

function quotaPercent(value) {
  if (value === null || value === undefined || value === '') return null;
  const n = Number(value);
  if (!Number.isFinite(n)) return null;
  return Math.max(0, Math.min(100, Math.round(n)));
}

function quotaResetText(value) {
  const ts = Number(value || 0);
  if (!Number.isFinite(ts) || ts <= 0) return '—';
  const d = new Date(ts * 1000);
  const pad = n => String(n).padStart(2, '0');
  return `${pad(d.getMonth() + 1)}/${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function planDisplayName(plan) {
  const raw = String(plan || '').trim();
  if (!raw) return '未知';
  if (raw.length <= 3) return raw.toUpperCase();
  return raw.charAt(0).toUpperCase() + raw.slice(1);
}

function renderOfficialQuotaLine(label, remaining, used, resetAt) {
  const pct = quotaPercent(remaining);
  const usedPct = quotaPercent(used);
  const pctText = pct === null ? '—' : `${pct}%`;
  const usedText = usedPct === null ? '' : `已用 ${usedPct}%`;
  const resetText = quotaResetText(resetAt);
  return `<div class="official-quota-line">
    <div class="official-quota-line-head">
      <span>${esc(label)}</span>
      <strong>${esc(pctText)} <em>${esc(resetText)}</em></strong>
    </div>
    <div class="official-quota-track" title="${escAttr(`${label} ${pctText}${usedText ? `，${usedText}` : ''}`)}">
      <div class="official-quota-fill" style="width:${pct === null ? 0 : pct}%"></div>
    </div>
  </div>`;
}

function renderOfficialQuotaInfo(info) {
  const official = info?.official || info || {};
  const status = official.status_label || '可用';
  const plan = planDisplayName(official.plan_type || official.plan || '');
  const recover = Number(official.next_recover_at || 0);
  const note = official.message || '显示 ChatGPT WHAM usage 返回的 Codex 额度窗口。';
  const recoverText = recover ? `恢复 ${formatRuntimeDate(recover)}` : '未触发限制';
  const hasRealQuota = official.hours_5_remaining_percent != null || official.weekly_remaining_percent != null;
  if (hasRealQuota) {
    const statusHead = status && status !== '可用'
      ? `<div class="balance-official-head"><span>${esc(status)}</span></div>`
      : '';
    return `<div class="balance-pill balance-official ${official.quota_exceeded ? 'quota-hit' : ''}" title="${escAttr(note)}">
      ${statusHead}
      <div class="official-quota-bars">
        ${renderOfficialQuotaLine('5h', official.hours_5_remaining_percent, official.hours_5_used_percent, official.hours_5_reset_at)}
        ${renderOfficialQuotaLine('7d', official.weekly_remaining_percent, official.weekly_used_percent, official.weekly_reset_at)}
      </div>
    </div>`;
  }
  return `<div class="balance-pill balance-official ${official.quota_exceeded ? 'quota-hit' : ''}" title="${escAttr(note)}">
    <div class="balance-official-head">
      <span>${esc(status)}</span>
      <strong>${esc(recoverText)}</strong>
    </div>
    <div class="balance-official-metrics">
      <span class="balance-quota"><em>计划</em><strong>${esc(plan)}</strong></span>
      <span class="balance-quota"><em>5h</em><strong>${compactNumber(official.requests_5h)} 次</strong></span>
      <span class="balance-quota"><em>7d</em><strong>${compactNumber(official.requests_7d)} 次</strong></span>
    </div>
  </div>`;
}

function renderOfficialQuotaSnapshot(a) {
  const snapshot = officialQuotaSnapshot(a);
  if (snapshot) return renderOfficialQuotaInfo(snapshot);
  return `<div class="official-quota-empty">尚未检测额度状态；检测上游连通性后会显示 Codex 5 小时/周剩余额度、套餐和恢复时间。</div>`;
}

function renderOfficialCardStatus(a) {
  const snapshot = officialQuotaSnapshot(a);
  if (snapshot) return renderOfficialQuotaInfo(snapshot);
  const runtime = a?.runtime_state || {};
  const status = runtime.status || 'active';
  return `<div class="official-card-status runtime-${escAttr(status)}" title="${escAttr(officialRuntimeSummaryText(a))}">
    <span>${esc(officialAccountEligibility(a))}</span>
    <strong>${esc(officialRuntimeSummaryText(a))}</strong>
  </div>`;
}

function renderOfficialPoolOverview(list) {
  const official = list.filter(isCodexOfficialAccount);
  if (!official.length) return '';
  const enabled = official.filter(account => accountRouting(account).enabled).length;
  const unavailable = official.filter(account => officialAccountEligibility(account) !== '可参与').length;
  const totalSuccess = official.reduce((sum, account) => sum + Number(account?.runtime_state?.success || 0), 0);
  const totalFailed = official.reduce((sum, account) => sum + Number(account?.runtime_state?.failed || 0), 0);
  const pools = Array.from(new Set(official.map(account => accountRouting(account).pool))).join(', ');
  return `<div class="official-pool-overview">
    <div>
      <span>官方账号池</span>
      <strong>${enabled}/${official.length} 已启用</strong>
    </div>
    <div>
      <span>Pool</span>
      <strong title="${escAttr(pools)}">${esc(pools || 'codex-official')}</strong>
    </div>
    <div>
      <span>冷却/停用</span>
      <strong>${unavailable}</strong>
    </div>
    <div>
      <span>请求</span>
      <strong>${totalSuccess}/${totalFailed}</strong>
    </div>
  </div>`;
}

function routerReasonLabel(reason) {
  const labels = {
    ready: '可用',
    no_anchor: '无锚点',
    anchor_disabled: '锚点已停用',
    execution_disabled: '未参与执行',
    routing_disabled: '已停用',
    pool_mismatch: '非同池',
    no_supported_endpoint: '未映射',
    cooling_down: '冷却中',
    account_quota_cooling: '账号额度冷却',
    account_cooling_down: '账号冷却',
    account_retry_wait: '账号等待恢复',
    model_quota_cooling: '模型额度冷却',
    model_cooling_down: '模型冷却',
    model_retry_wait: '模型等待恢复',
    capability_mismatch: '能力不匹配',
    attempt_failed: '本次已失败',
    stream_preflight_risk: '流式预选避开',
    recent_failure_rate: '近期失败率高',
    low_health_score: '健康分偏低',
    recent_account_error: '账号近期错误',
    recent_model_error: '模型近期错误',
  };
  return labels[reason] || reason || '未知';
}

function routerCandidateDetail(candidate) {
  const name = candidate.account_name || candidate.account_id || '未知账号';
  const parts = [`${name}: ${routerReasonLabel(candidate.reason)}`];
  const capabilities = candidate.capabilities || {};
  const capabilityBits = [
    capabilities.protocol,
    capabilities.tool_mode && capabilities.tool_mode !== 'none' ? `tools:${capabilities.tool_mode}` : '',
    capabilities.vision && capabilities.vision !== 'off' ? `vision:${capabilities.vision}` : '',
    capabilities.web ? 'web' : '',
    capabilities.image_generation ? 'image2' : '',
  ].filter(Boolean);
  if (capabilityBits.length) parts.push(capabilityBits.join('/'));
  if (Array.isArray(candidate.capability_gaps) && candidate.capability_gaps.length) {
    parts.push(`缺口 ${candidate.capability_gaps.join('/')}`);
  }
  const modelRetry = candidate.model_runtime_next_retry_after;
  const accountRetry = candidate.runtime_next_retry_after;
  const retry = modelRetry || accountRetry;
  if (retry) parts.push(`恢复 ${formatRuntimeDate(retry)}`);
  const message = candidate.model_runtime_message || candidate.runtime_message;
  if (message) parts.push(message);
  const endpoint = candidate.endpoint_name || candidate.endpoint_kind;
  if (endpoint) parts.push(endpoint);
  if (candidate.mapped_model) parts.push(candidate.mapped_model);
  if (candidate.health_score !== undefined && candidate.health_score !== null) {
    parts.push(`健康 ${candidate.health_score}`);
  }
  if (candidate.failure_rate_percent !== undefined && candidate.failure_rate_percent !== null) {
    parts.push(`失败率 ${candidate.failure_rate_percent}%`);
  }
  if (candidate.stream_preflight_risk) {
    parts.push(`流式预选 ${routerReasonLabel(candidate.stream_preflight_risk)}`);
  }
  if (candidate.route_score !== undefined && candidate.route_score !== null) {
    parts.push(`评分 ${candidate.route_score}`);
  }
  const recentSuccess = Number(candidate.recent_success || 0);
  const recentFailed = Number(candidate.recent_failed || 0);
  if (recentSuccess || recentFailed) {
    parts.push(`近1h ${recentSuccess}/${recentFailed}`);
  }
  return parts.join(' · ');
}

function routerNodeLabel(node) {
  return node?.account_name || node?.account_id || '未知账号';
}

function routerPreflightSummary(preflight) {
  if (!preflight || typeof preflight !== 'object') return '';
  const reason = routerReasonLabel(preflight.reason);
  const from = routerNodeLabel(preflight.from);
  const to = routerNodeLabel(preflight.to);
  if (preflight.action === 'rerouted') {
    return `流式预选避开 ${from}，改走 ${to} · ${reason}`;
  }
  if (preflight.action === 'kept_no_alternative') {
    return `流式预选风险 ${from} · ${reason}，暂无替代候选`;
  }
  return `流式预选 ${reason}`;
}

function routerScenarioDetail(scenario) {
  const candidates = Array.isArray(scenario?.candidates) ? scenario.candidates : [];
  const selected = scenario?.selected;
  const preflight = routerPreflightSummary(scenario?.stream_preflight);
  const selectedLine = selected
    ? `执行 ${selected.account_name || selected.account_id || '未知账号'}`
    : '暂无可用执行账号';
  const skipped = candidates
    .filter(candidate => !candidate.eligible)
    .map(routerCandidateDetail)
    .join('\n');
  const toolDecisions = scenario?.tool_decisions || selected?.tool_decisions || {};
  const decisions = [
    Array.isArray(toolDecisions.kept) && toolDecisions.kept.length ? `保留 ${toolDecisions.kept.join('/')}` : '',
    Array.isArray(toolDecisions.translated) && toolDecisions.translated.length ? `转译 ${toolDecisions.translated.join('/')}` : '',
    Array.isArray(toolDecisions.local) && toolDecisions.local.length ? `本地 ${toolDecisions.local.join('/')}` : '',
    Array.isArray(toolDecisions.filtered) && toolDecisions.filtered.length ? `过滤 ${toolDecisions.filtered.join('/')}` : '',
  ].filter(Boolean).join('\n');
  return [selectedLine, preflight, decisions, skipped].filter(Boolean).join('\n');
}

function renderRouterScenarioStrip(scenarios) {
  if (!Array.isArray(scenarios) || !scenarios.length) return '';
  return `<div class="router-scenario-strip">
    ${scenarios.map(scenario => {
      const candidates = Array.isArray(scenario.candidates) ? scenario.candidates : [];
      const eligible = Number(scenario.eligible_count || 0);
      const total = Number(scenario.candidate_count || candidates.length || 0);
      const selected = scenario.selected;
      const ok = Boolean(selected);
      const preflight = scenario.stream_preflight || null;
      const rerouted = preflight?.action === 'rerouted';
      const selectedName = rerouted
        ? `流式→${routerNodeLabel(preflight.to)}`
        : (selected ? (selected.account_name || selected.account_id || '可用') : '不可用');
      return `<span class="router-scenario-chip ${ok ? 'ok' : 'blocked'}${rerouted ? ' preflight' : ''}" title="${escAttr(routerScenarioDetail(scenario))}">
        <em>${esc(scenario.scenario_label || scenario.scenario_id || '场景')}</em>
        <strong>${esc(selectedName)}</strong>
        <b>${rerouted ? '预选' : `${eligible}/${total}`}</b>
      </span>`;
    }).join('')}
  </div>`;
}

function renderRouterPreflightNotice(preflight) {
  const summary = routerPreflightSummary(preflight);
  if (!summary) return '';
  const kind = preflight?.action === 'rerouted' ? 'warn' : 'muted';
  return `<div class="router-preflight-notice ${kind}" title="${escAttr(summary)}">${esc(summary)}</div>`;
}

function renderRouterStatusOverview() {
  if (selectedClientKind !== 'codex' || selectedSurfaceForKind('codex') !== 'desktop') return '';
  const router = accountsData?.router_status || {};
  const scenarios = Array.isArray(accountsData?.router_status_scenarios)
    ? accountsData.router_status_scenarios
    : [];
  const candidates = Array.isArray(router.candidates) ? router.candidates : [];
  if (!router.anchor && candidates.length === 0) return '';
  const ready = candidates.filter(candidate => candidate.eligible).length;
  const skipped = Math.max(0, candidates.length - ready);
  const anchorPool = router.anchor?.pool || '—';
  const selected = router.selected;
  const selectedName = selected ? (selected.account_name || selected.account_id || '—') : '暂无';
  const mappedModel = selected?.mapped_model || router.requested_model || '—';
  const selectedHealth = selected?.health_score;
  const preflight = router.stream_preflight || null;
  const skippedTitle = candidates
    .filter(candidate => !candidate.eligible)
    .map(routerCandidateDetail)
    .join('\n');
  const selectedDetail = selected
    ? routerCandidateDetail({
      ...selected,
      reason: 'ready',
    })
    : '暂无可用执行账号';
  return `<div class="router-status-block">
  <div class="official-pool-overview router-pool-overview">
    <div>
      <span>路由池</span>
      <strong title="${escAttr(anchorPool)}">${esc(anchorPool)}</strong>
    </div>
    <div>
      <span>执行账号</span>
      <strong title="${escAttr(selectedDetail)}">${esc(selectedName)}</strong>
    </div>
    <div>
      <span>模型</span>
      <strong title="${escAttr(mappedModel)}">${esc(mappedModel)}</strong>
    </div>
    <div>
      <span>候选</span>
      <strong title="${escAttr(skippedTitle)}">${ready}/${candidates.length}${skipped ? ` 跳过${skipped}` : ''}${selectedHealth !== undefined && selectedHealth !== null ? ` · 健康${selectedHealth}` : ''}</strong>
    </div>
  </div>
  ${renderRouterPreflightNotice(preflight)}
  ${renderRouterScenarioStrip(scenarios)}
  </div>`;
}

function clientAccountHasIssue(a) {
  if (!a || isCodexAccount(a)) return false;
  const status = clientAccountStatusReport(a) || a.last_check;
  if (!status) return false;
  return status.ok === false;
}

function clientAccountStatusReport(a) {
  if (!a || isCodexAccount(a)) return null;
  return a._client_status_report || a.last_check?.details || null;
}

function clientAccountApplied(a) {
  return clientAccountActive(a);
}

function renderClientCardStatusSummary(report, applied) {
  if (!report) {
    return '<div class="client-status-summary"><span class="status-muted">未检查</span></div>';
  }
  const diagnostics = Array.isArray(report.diagnostics) ? report.diagnostics : [];
  const hasError = diagnostics.some(item => item.level === 'error');
  if (report.ok === false || hasError) {
    return '<div class="client-status-summary"><span class="status-error">需处理</span></div>';
  }
  return `<div class="client-status-summary"><span class="status-ok">${applied ? '已写入' : '可写入'}</span></div>`;
}

function getClientProfile(kind) {
  const slug = normalizeClientKind(kind);
  return clientProfiles.find(profile => normalizeClientKind(profile.slug || profile.kind) === slug) || null;
}

function clientSecretLabel(kind) {
  const slug = normalizeClientKind(kind);
  if (slug === 'claude_code') return 'Claude 鉴权变量';
  if (slug === 'openclaw') return 'SecretRef 环境变量';
  if (slug === 'hermes') return 'Hermes .env Key';
  return 'Key 环境变量名';
}

function renderClaudeAuthEnvSelect(value) {
  const current = value || 'ANTHROPIC_API_KEY';
  return `<select id="edit_client_auth_env">
    <option value="ANTHROPIC_API_KEY" ${current === 'ANTHROPIC_API_KEY' ? 'selected' : ''}>ANTHROPIC_API_KEY</option>
    <option value="ANTHROPIC_AUTH_TOKEN" ${current === 'ANTHROPIC_AUTH_TOKEN' ? 'selected' : ''}>ANTHROPIC_AUTH_TOKEN</option>
  </select>`;
}

function clientModelSlots(kind) {
  const profile = getClientProfile(kind);
  const slug = normalizeClientKind(kind);
  const claudeSlots = [
    { key: 'default', label: '主模型', target: 'ANTHROPIC_MODEL', required: true },
    { key: 'sonnet', label: 'Sonnet 模型', target: 'ANTHROPIC_DEFAULT_SONNET_MODEL' },
    { key: 'opus', label: 'Opus 模型', target: 'ANTHROPIC_DEFAULT_OPUS_MODEL' },
    { key: 'haiku', label: 'Haiku 模型', target: 'ANTHROPIC_DEFAULT_HAIKU_MODEL' },
  ];
  if (profile && Array.isArray(profile.model_slots) && profile.model_slots.length) {
    if (slug === 'claude_code') {
      const profileByKey = new Map(profile.model_slots.map(slot => [slot.key, slot]));
      const merged = claudeSlots.map(slot => ({ ...slot, ...(profileByKey.get(slot.key) || {}) }));
      profile.model_slots.forEach(slot => {
        if (!claudeSlots.some(base => base.key === slot.key)) merged.push(slot);
      });
      return merged;
    }
    return profile.model_slots;
  }
  if (slug === 'claude_code') return claudeSlots;
  if (slug === 'openclaw') return [
    { key: 'default', label: '默认 Agent 模型', target: 'agents.defaults.model', required: true },
    { key: 'image', label: '图片理解模型', target: 'agents.defaults.imageModel' },
    { key: 'image_generation', label: '图片生成模型', target: 'agents.defaults.imageGenerationModel' },
    { key: 'video_generation', label: '视频生成模型', target: 'agents.defaults.videoGenerationModel' },
  ];
  if (slug === 'hermes') return [
    { key: 'default', label: '主模型', target: 'model.default', required: true },
    { key: 'vision', label: '视觉辅助模型', target: 'auxiliary.vision.model' },
    { key: 'web_extract', label: '网页提取模型', target: 'auxiliary.web_extract.model' },
    { key: 'compression', label: '压缩模型', target: 'auxiliary.compression.model' },
    { key: 'session_search', label: '会话检索模型', target: 'auxiliary.session_search.model' },
    { key: 'title_generation', label: '标题生成模型', target: 'auxiliary.title_generation.model' },
  ];
  return [
    { key: 'default', label: '默认模型', target: 'OPENAI_MODEL', required: true },
    { key: 'fast', label: '快速模型', target: 'OPENAI_FAST_MODEL' },
    { key: 'reasoning', label: '推理模型', target: 'OPENAI_REASONING_MODEL' },
    { key: 'vision', label: '视觉模型', target: 'OPENAI_VISION_MODEL' },
  ];
}

function clientModelMap(account) {
  const map = account?.client_options?.model_map;
  return map && typeof map === 'object' && !Array.isArray(map) ? map : {};
}

const CLAUDE_ONE_M_SUFFIX = '[1m]';

function isClaudeOneMModelSlot(kind, key) {
  return normalizeClientKind(kind) === 'claude_code'
    && ['default', 'sonnet', 'opus', 'haiku'].includes(String(key || ''));
}

function stripClaudeOneMSuffix(value) {
  return String(value || '').trim().replace(/\s*\[1m\]$/i, '');
}

function hasClaudeOneMSuffix(value) {
  return /\[1m\]$/i.test(String(value || '').trim());
}

function clientIcon(kind) {
  const slug = normalizeClientKind(kind);
  const logo = slug === 'claude_code' ? 'claude-code' : (slug === 'generic_client' ? 'custom' : slug);
  return `<span class="client-logo-box client-logo-${escAttr(slug)}"><img class="client-logo-img" src="${providerLogoSrc(logo)}" alt="" aria-hidden="true"></span>`;
}

function lineActionIcon(name) {
  return `<span class="line-action-icon line-action-icon-${escAttr(name)}" aria-hidden="true"></span>`;
}

function renderAccountIconAction(label, icon, onclick, className = '', disabled = false) {
  const disabledAttr = disabled ? ' disabled' : '';
  const onclickAttr = onclick && !disabled ? ` onclick="${onclick}"` : '';
  return `<button type="button" class="account-action account-icon-btn ${className}"${onclickAttr}${disabledAttr} title="${escAttr(label)}" aria-label="${escAttr(label)}">
    ${lineActionIcon(icon)}
  </button>`;
}

function renderToolbarIconAction(label, icon, onclick, className = 'btn-ghost') {
  return `<button type="button" class="btn ${className} account-toolbar-icon" onclick="${onclick}" title="${escAttr(label)}" aria-label="${escAttr(label)}">
    ${lineActionIcon(icon)}
  </button>`;
}

function cardEndpointKind(account) {
  if (isCodexAccount(account)) return currentEndpoint(account)?.kind || 'open_ai_chat';
  const kind = accountClientKind(account);
  if (kind === 'claude_code' || account?.provider === 'anthropic') return 'anthropic_messages';
  return 'open_ai_chat';
}

function cardUpstream(account) {
  if (isCodexAccount(account)) return currentEndpoint(account)?.base_url || account?.upstream || '';
  return account?.upstream || account?.client_options?.base_url || '';
}

function renderCardActionMenu(a, isClient) {
  return '';
}

function renderClientSwitcher(list) {
  const counts = accountsData.client_counts || {};
  const profiles = clientProfiles.length ? clientProfiles : [
    { slug: 'codex', label: 'Codex', description: 'deecodex 代理账号' },
    { slug: 'claude_code', label: 'Claude Code', description: 'Claude 本地配置' },
    { slug: 'openclaw', label: 'OpenClaw', description: 'OpenClaw 配置' },
    { slug: 'hermes', label: 'Hermes', description: 'Hermes 配置' },
    { slug: 'generic_client', label: '通用客户端', description: 'OpenAI 兼容 Env' },
  ];
  const tabs = profiles.flatMap(profile => {
    const kind = normalizeClientKind(profile.slug || profile.kind);
    if (!clientKindSupportsSurface(kind)) return [{ profile, kind, surface: 'cli' }];
    return [
      { profile, kind, surface: 'cli' },
      { profile, kind, surface: 'desktop' },
    ];
  });
  return `<div class="client-switcher" role="tablist" aria-label="账号客户端分类">
    ${tabs.map(tab => {
      const { profile, kind, surface } = tab;
      const supportsSurface = clientKindSupportsSurface(kind);
      const clientAccounts = list.filter(a => accountClientKind(a) === kind && (!supportsSurface || accountClientSurface(a) === surface));
      const count = supportsSurface ? clientAccounts.length : (counts[kind] || clientAccounts.length || 0);
      const issueCount = clientAccounts.filter(clientAccountHasIssue).length;
      const active = kind === selectedClientKind && (!supportsSurface || selectedSurfaceForKind(kind) === surface) ? ' active' : '';
      const issueClass = issueCount ? ' has-issues' : '';
      const label = supportsSurface ? clientSurfaceTitle(kind, surface) : (CLIENT_KIND_LABELS[kind] || profile.label || kind);
      const icon = supportsSurface
        ? `<span class="surface-icon-stack">${clientIcon(kind)}<span class="surface-glyph surface-${surface}" aria-hidden="true"></span></span>`
        : clientIcon(kind);
      return `<button type="button" class="client-tab${supportsSurface ? ' account-surface-tab' : ''}${active}${issueClass}" onclick="selectClientKind('${escAttr(kind)}', '${escAttr(surface)}')" title="${escAttr(label)}" aria-label="${escAttr(label)}" role="tab" aria-selected="${active ? 'true' : 'false'}">
        ${icon}
        ${count > 0 ? `<em>${count}</em>` : ''}
        ${issueCount ? `<strong class="client-tab-alert" title="${escAttr(issueCount + ' 个账号最近检查异常')}">${issueCount}</strong>` : ''}
      </button>`;
    }).join('')}
  </div>`;
}

function renderClientAccountDetail() {
  const a = editingAccount;
  const kind = accountClientKind(a);
  const surface = accountClientSurface(a);
  const profile = getClientProfile(kind) || {};
  const configPath = a.client_options?.config_path || '';
  const configHint = kind === 'claude_code' && surface === 'desktop'
    ? '~/Library/Application Support/Claude-3p/configLibrary/<id>.json'
    : (profile.config_path_hint || '');
  const apiKeyEnv = a.client_options?.api_key_env || defaultApiKeyEnvForClient(a);
  const authEnv = a.client_options?.auth_env || apiKeyEnv;
  const proxyEnabled = Boolean(a.client_options?.proxy_recording_enabled);
  const proxyBaseUrl = a.client_options?.proxy_base_url || '';
  const secretHint = kind === 'openclaw'
    ? 'OpenClaw 会写入 SecretRef，不把 Key 放进命令参数。'
    : (kind === 'hermes'
      ? 'Hermes 会把非密钥配置写入 config.yaml，密钥写入 .env。'
      : '写入前会展示脱敏 diff，不显示完整密钥。');
  return `<div class="breadcrumb account-detail-breadcrumb">
    <button type="button" class="page-back-button account-back-link" onclick="navigateAccounts('list')" aria-label="返回账号列表">
      <span class="line-action-icon line-action-icon-back" aria-hidden="true"></span>
    </button>
  </div>
  <div class="page-header account-detail-header">
    <div class="account-detail-title">
      ${clientIcon(kind)}
      <div>
        <div class="account-detail-heading">
          <h2>${esc(accountDisplayTitle(a))}</h2>
        </div>
      </div>
    </div>
  </div>

  <div class="account-form client-account-form">
    <section class="account-edit-section">
      <div class="account-section-head">
        <div class="section-sub-label">${esc(clientSurfaceTitle(kind, surface))}</div>
      </div>
      <div class="config-fields">
        <div class="config-field">
          <label>账号名称</label>
          <input type="text" id="edit_name" value="${escAttr(a.name)}" placeholder="输入账号显示名">
        </div>
        <div class="config-field">
          <label>供应商</label>
          <select id="edit_client_provider" onchange="updateClientProviderDefaults()">
            ${providersForClientKind(kind).map(p => `<option value="${escAttr(p.slug)}" ${a.provider === p.slug ? 'selected' : ''}>${esc(p.label)}</option>`).join('')}
          </select>
        </div>
        <div class="config-field wide">
          <label>目标客户端 Base URL</label>
          <input type="text" id="edit_upstream" value="${escAttr(a.upstream)}" placeholder="${escAttr(profile.default_base_url || 'https://api.example.com/v1')}">
          <span class="hint">这个 URL 会写入目标客户端配置；非 Codex 账号不会走 deecodex 代理翻译。</span>
        </div>
        <div class="config-field">
          <label>默认模型</label>
          <input type="text" id="edit_default_model" value="${escAttr(a.default_model || '')}" placeholder="${escAttr(profile.default_model || 'model-name')}">
        </div>
        <div class="config-field">
          <label>API Key</label>
          <div class="pass-group ${hasStoredPrimaryApiKey(a) ? 'has-copy' : ''}">
            <input type="password" id="edit_api_key" value="${escAttr(displayStoredSecret(a.api_key, hasStoredPrimaryApiKey(a)))}" placeholder="输入 API 密钥" autocomplete="off">
            ${hasStoredPrimaryApiKey(a) ? secretCopyButton('api_key') : ''}
            <button type="button" class="pass-toggle" onclick="togglePass('edit_api_key', this)" title="显示/隐藏 API Key" aria-label="显示或隐藏 API Key">
              <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M2.5 12s3.5-6 9.5-6 9.5 6 9.5 6-3.5 6-9.5 6-9.5-6-9.5-6z"></path><circle cx="12" cy="12" r="2.5"></circle></svg>
            </button>
          </div>
        </div>
      </div>
    </section>

    <section class="account-edit-section">
      <div class="account-section-head">
        <div class="section-sub-label">客户端模型映射</div>
      </div>
      <div class="section-action-row">
        <button class="btn btn-ghost" onclick="fetchClientModels()">从上游获取模型列表</button>
        <span id="clientModelFetchStatus"></span>
      </div>
      <div class="model-map-table client-model-map-table${kind === 'claude_code' ? ' claude-one-m-table' : ''}">
        <div class="model-map-head client-model-map-head client-model-template${kind === 'claude_code' ? ' claude-one-m-head' : ''}">
          <span>客户端槽位</span>
          <span>上游模型</span>
          <span>${kind === 'claude_code' ? '1M 上下文' : ''}</span>
        </div>
        <div id="clientModelMapRows">${renderClientModelMappingRows(a)}</div>
      </div>
      <div class="model-add-row"><button onclick="addClientModelRow()">+ 添加自定义槽位</button></div>
    </section>

    <section class="account-edit-section">
      <div class="account-section-head">
        <div class="section-sub-label">配置写入</div>
      </div>
      <div class="config-fields">
        <div class="config-field">
          <label>配置路径 <span class="optional-label">可选</span></label>
          <input type="text" id="edit_client_config_path" value="${escAttr(configPath)}" placeholder="${escAttr(configHint)}">
        </div>
        <div class="config-field">
          <label>请求历史代理</label>
          <label class="history-toggle${proxyEnabled ? ' on' : ''}" id="edit_client_proxy_toggle" onclick="toggleClientProxyRecording()">
            <div class="toggle-dot"></div> 启用记录
          </label>
          <span class="hint">${proxyEnabled && proxyBaseUrl ? esc(proxyBaseUrl) : '开启后写入本地代理 URL，并用代理 token 识别账号。'}</span>
        </div>
        <div class="config-field">
          <label>Key 环境变量名</label>
          <input type="text" id="edit_client_api_key_env" value="${escAttr(apiKeyEnv)}" placeholder="OPENAI_API_KEY">
          <span class="hint">${esc(secretHint)}</span>
        </div>
        ${kind === 'claude_code' ? `<div class="config-field">
          <label>${clientSecretLabel(kind)}</label>
          ${renderClaudeAuthEnvSelect(authEnv)}
          <span class="hint">兼容 Claude Code 的 API Key 和 Auth Token 两种本地环境变量。</span>
        </div>` : `<div class="config-field">
          <label>${clientSecretLabel(kind)}</label>
          <input type="text" value="${escAttr(apiKeyEnv)}" disabled>
        </div>
        `}
      </div>
      <div class="section-action-row">
        ${a.id ? `<button class="btn btn-ghost" onclick="refreshClientAccountStatus('${escAttr(a.id)}')">刷新状态</button>` : ''}
        ${a.id ? `<button class="btn btn-ghost" onclick="editConfigFile('${escAttr(a.id)}')">编辑配置文件</button>` : ''}
        <button class="btn btn-ghost" onclick="dryRunEditingClientAccount()">预检当前表单</button>
        <span id="clientDryRunStatus"></span>
      </div>
      <div id="clientApplyPreview" class="client-apply-preview"></div>
    </section>

    <section class="account-edit-section">
      <div class="account-section-head">
        <div>
          <div class="section-sub-label">最近备份</div>
        </div>
        ${a.id ? `<button class="btn btn-ghost btn-small" onclick="fetchClientBackupsForDetail('${escAttr(a.id)}')">刷新</button>` : ''}
      </div>
      <div id="clientBackupList" class="client-backup-list">
        <span class="status-muted">${a.id ? '加载中...' : '保存账号后显示备份'}</span>
      </div>
    </section>

    <section class="account-edit-section">
      <div class="account-section-head">
        <div>
          <div class="section-sub-label">最近配置事件</div>
        </div>
        ${a.id ? `<button class="btn btn-ghost btn-small" onclick="fetchClientEventsForDetail('${escAttr(a.id)}')">刷新</button>` : ''}
      </div>
      <div id="clientEventLog" class="client-event-log">
        <span class="status-muted">${a.id ? '加载中...' : '保存账号后显示事件'}</span>
      </div>
    </section>

    <div class="accounts-actions">
      <button class="btn btn-primary" onclick="saveAccount()">保存账号</button>
      ${a.id ? `<button class="btn btn-ghost" onclick="saveAndApplyClientAccount('${escAttr(a.id)}')">保存并写入配置</button>` : ''}
      <button class="btn btn-danger" onclick="deleteAccount('${escAttr(a.id)}')">删除账号</button>
    </div>
  </div>`;
}

function selectClientKind(kind, surface) {
  selectedClientKind = normalizeClientKind(kind);
  selectedClientSurface = clientKindSupportsSurface(selectedClientKind)
    ? normalizeClientSurface(surface || selectedClientSurface)
    : 'cli';
  if (accountsView === 'add') accountsView = 'list';
  renderMainContent();
}

function selectClientSurface(surface) {
  selectedClientSurface = normalizeClientSurface(surface);
  if (accountsView === 'add') accountsView = 'list';
  renderMainContent();
}

function toggleClientProxyRecording() {
  if (!editingAccount || isCodexAccount(editingAccount)) return;
  editingAccount.client_options = editingAccount.client_options || {};
  editingAccount.client_options.proxy_recording_enabled = !Boolean(editingAccount.client_options.proxy_recording_enabled);
  const toggle = document.getElementById('edit_client_proxy_toggle');
  if (toggle) toggle.classList.toggle('on', editingAccount.client_options.proxy_recording_enabled);
}

function getProviderPreset(provider) {
  return providerPresets.find(pp => pp.slug === provider) || null;
}

function accountCapabilityLabels(a) {
  const fromAccount = a.provider_options && Array.isArray(a.provider_options.capability_labels)
    ? a.provider_options.capability_labels
    : null;
  if (fromAccount) return fromAccount;
  const preset = getProviderPreset(a.provider);
  return preset && Array.isArray(preset.capability_labels) ? preset.capability_labels : [];
}

function renderCapabilityAccountOptions(current) {
  const list = accountsData.accounts || [];
  const currentId = current?.id || '';
  const selected = current?.capability_account_id || '';
  const options = ['<option value="">请选择能力账号</option>'];
  list
    .filter(account => account.id && account.id !== currentId)
    .forEach(account => {
      const isSelected = account.id === selected ? 'selected' : '';
      options.push(`<option value="${escAttr(account.id)}" ${isSelected}>${esc(account.name)} · ${esc(account.provider)}</option>`);
    });
  return options.join('');
}

function toggleCapabilityFields() {
  const enabled = document.getElementById('edit_capability_enabled')?.checked;
  const fields = document.getElementById('capabilityFields');
  if (fields) fields.style.display = enabled ? '' : 'none';
}

function renderDevPipelineAccountOptions(current, selected) {
  const list = accountsData.accounts || [];
  const options = [`<option value="" ${!selected ? 'selected' : ''}>活跃账号（当前主账号）</option>`];
  list
    .filter(account => account.id)
    .forEach(account => {
      const isSelected = account.id === selected ? 'selected' : '';
      options.push(`<option value="${escAttr(account.id)}" ${isSelected}>${esc(account.name)} · ${esc(account.provider)}</option>`);
    });
  return options.join('');
}

function toggleDevPipelineFields() {
  const enabled = document.getElementById('edit_dev_pipeline_enabled')?.checked;
  const fields = document.getElementById('devPipelineFields');
  if (fields) fields.style.display = enabled ? '' : 'none';
}

function currentEndpoint(a) {
  const endpoints = Array.isArray(a?.endpoints) ? a.endpoints : [];
  const selectedId = selectedEndpointId(a);
  if (selectedId) return endpoints.find(ep => ep.id === selectedId) || endpoints[0] || null;
  return endpoints[0] || null;
}

function currentEndpointIndex(a) {
  const endpoints = Array.isArray(a?.endpoints) ? a.endpoints : [];
  const ep = currentEndpoint(a);
  if (!ep) return -1;
  const idx = endpoints.findIndex(item => item.id === ep.id);
  return idx >= 0 ? idx : 0;
}

function endpointKindLabel(kind) {
  const labels = {
    open_ai_chat: 'Chat 兼容',
    open_ai_responses: 'OpenAI Responses 直连',
    anthropic_messages: 'Anthropic Messages',
    codex_official: 'Codex 官方',
    custom_chat: 'Chat 兼容',
    custom_responses: 'Responses 直连',
    OpenAiChat: 'Chat 兼容',
    OpenAiResponses: 'OpenAI Responses 直连',
    AnthropicMessages: 'Anthropic Messages',
    CodexOfficial: 'Codex 官方',
    CustomChat: 'Chat 兼容',
    CustomResponses: 'Responses 直连',
  };
  return labels[kind] || kind || 'Chat 兼容';
}

function endpointKindIsResponsesDirect(kind) {
  return kind === 'open_ai_responses'
    || kind === 'OpenAiResponses'
    || kind === 'custom_responses'
    || kind === 'CustomResponses';
}

function endpointKindUsesModelMapping(kind) {
  return kind === 'open_ai_chat'
    || kind === 'OpenAiChat'
    || kind === 'custom_chat'
    || kind === 'CustomChat'
    || kind === 'anthropic_messages'
    || kind === 'AnthropicMessages';
}

function isOpenAiNativeResponsesAccount(account) {
  return isCodexAccount(account) && String(account?.provider || '').toLowerCase() === 'openai';
}

function endpointKindIsCustomResponses(kind) {
  return kind === 'custom_responses' || kind === 'CustomResponses';
}

function normalizeResponsesKind(kind) {
  if (kind === 'OpenAiResponses') return 'open_ai_responses';
  if (kind === 'CustomResponses') return 'custom_responses';
  if (kind === 'CodexOfficial') return 'codex_official';
  return kind;
}

function isResponsesDirectFormAccount(account, endpoint = currentEndpoint(account)) {
  return isCodexAccount(account)
    && (isOpenAiNativeResponsesAccount(account)
      || endpointKindIsResponsesDirect(endpoint?.kind)
      || endpointIsCodexOfficial(endpoint));
}

function endpointKindIsLockedForForm(account, endpoint = currentEndpoint(account)) {
  return isCodexAccount(account)
    && (isOpenAiNativeResponsesAccount(account) || endpointIsCodexOfficial(endpoint));
}

function normalizeResponsesDirectAccount(account, endpoint = currentEndpoint(account)) {
  if (!isCodexAccount(account)) return account;
  const forceOpenAiResponses = isOpenAiNativeResponsesAccount(account);
  if (!forceOpenAiResponses && !endpointKindIsResponsesDirect(endpoint?.kind) && !endpointIsCodexOfficial(endpoint)) return account;
  if (!Array.isArray(account.endpoints)) account.endpoints = [];
  if (account.endpoints.length === 0) {
    account.endpoints.push(createEndpointFromTemplate(providerDefaultTemplate(forceOpenAiResponses ? 'openai' : account.provider), account));
  }
  const targetEndpointId = endpoint?.id || currentEndpoint(account)?.id || account.endpoints[0]?.id;
  const endpoints = account.endpoints;
  endpoints.forEach(endpoint => {
    if (!forceOpenAiResponses && endpoint.id !== targetEndpointId) return;
    if (!forceOpenAiResponses && !endpointKindIsResponsesDirect(endpoint.kind) && !endpointIsCodexOfficial(endpoint)) return;
    endpoint.kind = forceOpenAiResponses ? 'open_ai_responses' : normalizeResponsesKind(endpoint.kind);
    endpoint.name = endpointKindLabel(endpoint.kind);
    endpoint.template_id = endpointIsCodexOfficial(endpoint)
      ? 'codex_official'
      : (endpointKindIsCustomResponses(endpoint.kind) ? (endpoint.template_id || 'custom_responses') : 'responses_direct');
    if (!endpointKindIsCustomResponses(endpoint.kind)) endpoint.path = '';
    endpoint.model_map = {};
    endpoint.model_profiles = {};
    endpoint.vision = {
      ...(endpoint.vision || {}),
      mode: 'native',
      unsupported_image_policy: 'reject',
      glue_strategy: 'final_answer',
      adapter_id: 'minimax_coding_plan_vlm',
      base_url: '',
      api_key: '',
      model: '',
      path: 'v1/coding_plan/vlm',
    };
    endpoint.context_window_override = null;
    endpoint.reasoning_effort_override = null;
    endpoint.thinking_tokens = null;
  });
  account.model_map = {};
  account.vision_enabled = false;
  account.vision_upstream = '';
  account.vision_api_key = '';
  account.vision_model = '';
  account.vision_endpoint = 'v1/coding_plan/vlm';
  account.context_window_override = null;
  account.reasoning_effort_override = null;
  account.thinking_tokens = null;
  account.capability_enabled = false;
  account.capability_account_id = null;
  account.dev_pipeline_enabled = false;
  account.dev_pipeline_trigger_mode = 'manual';
  account.dev_pipeline_command = '/dev-pipeline';
  account.dev_pipeline_architect_account_id = null;
  account.dev_pipeline_implementer_account_id = null;
  account.dev_pipeline_reviewer_account_id = null;
  account.dev_pipeline_tool_mode = 'controlled_tools';
  account.dev_pipeline_max_iterations = 3;
  account.dev_pipeline_show_trace = false;
  account.dev_pipeline_architect_instruction = '';
  account.dev_pipeline_implementer_instruction = '';
  account.dev_pipeline_reviewer_instruction = '';
  account.translate_enabled = false;
  return account;
}

function visionModeLabel(mode) {
  const labels = {
    off: '视觉关闭',
    native: '原生多模态',
    glue: '胶水多模态',
    Off: '视觉关闭',
    Native: '原生多模态',
    Glue: '胶水多模态',
  };
  return labels[mode] || '视觉关闭';
}

function formatTimeShort(ts) {
  const n = Number(ts || 0);
  if (!n) return '—';
  const d = new Date(n * 1000);
  return d.toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' });
}

function setVisionMode(mode) {
  const input = document.getElementById('edit_vision_mode');
  if (input) input.value = mode;
  document.querySelectorAll('.vision-mode-option').forEach(btn => {
    btn.classList.toggle('active', btn.dataset.mode === mode);
  });
  toggleVisionFields();
}

function handleEndpointKindChange(value) {
  if (!editingAccount || !isCodexAccount(editingAccount)) return;
  const ep = currentEndpoint(editingAccount);
  const previousKind = ep?.kind || 'open_ai_chat';
  const kindControl = document.getElementById('edit_endpoint_kind');
  if (kindControl) kindControl.value = previousKind;
  syncEditingDraftFromForm({ preserveHiddenResponses: true });
  const current = currentEndpoint(editingAccount);
  if (current && value) {
    current.kind = value;
    current.name = endpointKindLabel(value);
    if (endpointKindIsResponsesDirect(value) && !endpointKindIsCustomResponses(value)) {
      current.path = '';
      current.template_id = 'responses_direct';
    } else if (endpointKindUsesModelMapping(value) && current.template_id === 'responses_direct') {
      current.template_id = value === 'anthropic_messages' ? 'anthropic_messages' : 'chat_compatible';
    }
  }
  if (kindControl) kindControl.value = value;
  upstreamModels = [];
  renderMainContent();
}

function selectedEndpointId(a) {
  if (!a) return null;
  if (a._editing_endpoint_id) return a._editing_endpoint_id;
  const active = activeSelectionForAccount(a);
  if (active && active.account_id === a.id && active.endpoint_id) {
    return active.endpoint_id;
  }
  return Array.isArray(a.endpoints) && a.endpoints[0] ? a.endpoints[0].id : null;
}

function activeSelectionKey(kind, surface) {
  const normalizedKind = kind === 'codex' ? 'codex' : kind;
  return `${normalizedKind}:${surface || 'cli'}`;
}

function activeSelectionFor(kind, surface) {
  const bySurface = accountsData?.active_by_surface || {};
  return bySurface[activeSelectionKey(kind, surface)] || null;
}

function activeSelectionForAccount(a) {
  const kind = accountClientKind(a);
  if (!a) {
    return null;
  }
  return activeSelectionFor(kind, accountClientSurface(a));
}

function clientAccountActive(a) {
  return activeSelectionForAccount(a)?.account_id === a?.id;
}

function activeAccountIdForSelection(kind, surface) {
  const normalizedKind = normalizeClientKind(kind);
  const active = activeSelectionFor(kind, surface);
  if (active?.account_id) return active.account_id;
  if (normalizedKind === 'codex') return accountsData.active_account_id || accountsData.active_id;
  return null;
}

function providerDefaultTemplate(provider) {
  const templates = endpointTemplates || [];
  if (provider === 'codex') {
    return templates.find(t => t.id === 'codex_official')
      || templates.find(t => t.kind === 'codex_official' || t.kind === 'CodexOfficial')
      || {
        id: 'codex_official',
        label: 'Codex 官方',
        kind: 'codex_official',
        default_base_url: 'https://chatgpt.com/backend-api/codex',
        default_path: 'responses',
        default_vision_mode: 'native',
      };
  }
  if (provider === 'anthropic') {
    return templates.find(t => t.id === 'anthropic_messages')
      || templates.find(t => t.kind === 'anthropic_messages' || t.kind === 'AnthropicMessages')
      || null;
  }
  if (provider === 'openai') {
    return templates.find(t => t.id === 'responses_direct')
      || templates.find(t => t.kind === 'open_ai_responses' || t.kind === 'OpenAiResponses')
      || null;
  }
  return templates.find(t => t.id === 'chat_compatible')
    || templates.find(t => t.kind === 'open_ai_chat' || t.kind === 'OpenAiChat')
    || templates[0]
    || null;
}

function newEndpointId() {
  return 'endpoint_' + Date.now().toString(16) + '_' + Math.random().toString(16).slice(2, 8);
}

function createEndpointFromTemplate(template, account) {
  const tpl = template || providerDefaultTemplate(account?.provider) || {};
  const hasTemplateBaseUrl = Object.prototype.hasOwnProperty.call(tpl, 'default_base_url');
  const baseUrl = hasTemplateBaseUrl
    ? (tpl.default_base_url || account?.upstream || '')
    : (account?.upstream || 'https://openrouter.ai/api/v1');
  const kind = tpl.kind || 'open_ai_chat';
  return {
    id: newEndpointId(),
    name: tpl.label || endpointKindLabel(kind),
    kind,
    base_url: baseUrl,
    path: tpl.default_path || '',
    template_id: tpl.id || '',
    template_version: 1,
    model_map: { ...(account?.model_map || {}) },
    model_profiles: {},
    vision: {
      mode: 'native',
      unsupported_image_policy: 'reject',
      glue_strategy: 'final_answer',
      adapter_id: 'minimax_coding_plan_vlm',
      base_url: account?.vision_upstream || '',
      api_key: account?.vision_api_key || '',
      model: account?.vision_model || '',
      path: account?.vision_endpoint || 'v1/coding_plan/vlm',
    },
    custom_headers: { ...(account?.custom_headers || {}) },
    request_timeout_secs: account?.request_timeout_secs || null,
    max_retries: account?.max_retries ?? null,
    context_window_override: account?.context_window_override || null,
    reasoning_effort_override: account?.reasoning_effort_override || null,
    thinking_tokens: account?.thinking_tokens || null,
    fast_mode_enabled: false,
    fast_service_tier: 'priority',
    balance_url: account?.balance_url || '',
  };
}

function applyProviderSpecificEndpointDefaults(account) {
  if (!account || accountClientKind(account) !== 'codex') return account;
  if (String(account.provider || '').toLowerCase() !== 'mimo') return account;
  (account.endpoints || []).forEach(endpoint => {
    if (!endpointKindUsesModelMapping(endpoint.kind)) return;
    endpoint.vision = {
      ...(endpoint.vision || {}),
      mode: 'off',
      unsupported_image_policy: 'reject',
      glue_strategy: 'final_answer',
      adapter_id: 'minimax_coding_plan_vlm',
      base_url: '',
      api_key: '',
      model: '',
      path: 'v1/coding_plan/vlm',
    };
    endpoint.model_profiles = {
      ...(endpoint.model_profiles || {}),
      [MIMO_VISION_MODEL]: { vision_mode: 'native' },
      [MIMO_CODING_MODEL]: { vision_mode: 'off' },
      'mimo-v2.5': { vision_mode: 'off' },
      'mimo-v2-pro': { vision_mode: 'off' },
    };
  });
  return account;
}

function ensureAccountEndpoints(a) {
  if (!a) return [];
  if (!Array.isArray(a.endpoints)) a.endpoints = [];
  if (a.endpoints.length === 0) {
    a.endpoints.push(createEndpointFromTemplate(providerDefaultTemplate(a.provider), a));
  }
  if (!selectedEndpointId(a)) a._editing_endpoint_id = a.endpoints[0].id;
  return a.endpoints;
}

function navigateAccounts(view) {
  accountsView = view;
  if (view === 'list') {
    editingAccount = null;
    stopOAuthLoginPolling();
    oauthLoginState = null;
  }
  renderMainContent();
}

function renderMainContent() {
  const main = document.getElementById('mainContent');
  main.classList.toggle('accounts-main', accountsView === 'list');
  main.classList.toggle('accounts-form-main', accountsView !== 'list');
  const html = renderAccountsPanel();
  main.innerHTML = typeof wrapPrimaryPanel === 'function' ? wrapPrimaryPanel('accounts', html) : html;
  afterRenderAccountsPanel();
}

function afterRenderAccountsPanel() {
  if (accountsView === 'edit' && editingAccount && !isCodexAccount(editingAccount) && editingAccount.id) {
    fetchClientEventsForDetail(editingAccount.id);
    fetchClientBackupsForDetail(editingAccount.id);
  }
}

function renderAccountsPanel() {
  if (accountsView === 'add') return renderAccountsFormPage(renderAddAccount(), 'accounts-add-shell');
  if (accountsView === 'edit') return renderAccountsFormPage(renderAccountDetail(), 'accounts-edit-shell');
  return renderAccountList();
}

function renderAccountsFormPage(html, className = '') {
  return `<div class="accounts-page-shell accounts-form-shell ${escAttr(className)}">
    <div class="accounts-scroll-region accounts-form-scroll">
      ${html}
    </div>
  </div>`;
}

// ── Level 1: 账号列表 ──

function renderAccountList() {
  const list = accountsData.accounts || [];
  const activeSurface = selectedSurfaceForKind(selectedClientKind);
  const activeId = activeAccountIdForSelection(selectedClientKind, activeSurface);
  const filtered = list
    .filter(a => accountClientKind(a) === selectedClientKind)
    .filter(a => accountClientSurface(a) === activeSurface)
    .map((account, index) => ({ account, index }))
    .sort((left, right) => {
      const leftActive = left.account.id === activeId ? 0 : 1;
      const rightActive = right.account.id === activeId ? 0 : 1;
      return leftActive - rightActive || left.index - right.index;
    })
    .map(item => item.account);
  let cards = '';
  if (filtered.length === 0) {
    cards = `<div class="empty-state">暂无${esc(CLIENT_KIND_LABELS[selectedClientKind] || '客户端')}${clientKindSupportsSurface(selectedClientKind) ? ' ' + esc(clientSurfaceLabel(activeSurface)) : ''}账号，点击上方按钮创建</div>`;
  } else {
    cards = '<div class="accounts-grid">' + filtered.map(a => {
      const active = a.id === activeId;
      if (!isCodexAccount(a)) return renderClientAccountCard(a);
      const official = isCodexOfficialAccount(a);
      const displayName = official ? (officialAccountEmail(a) || accountDisplayTitle(a)) : accountDisplayTitle(a);
      return `<div class="account-card${active ? ' active' : ''}">
        <div class="account-card-mainline">
          <div class="account-card-primary">
            <div class="account-card-info">
              <div class="account-card-header">
                <div class="account-card-titlebar">
                  ${renderProviderBadge(a.provider)}
                  ${renderCodexOfficialRouteLine(a)}
                  ${active ? '<span class="active-badge">活跃</span>' : ''}
                  ${accountRuntimeBadge(a)}
                </div>
              </div>
            <div class="account-card-body">
              <div class="account-card-main">
                <div class="card-name ${official ? 'official-email-name' : ''}" title="${escAttr(displayName)}">${esc(displayName)}</div>
              </div>
            </div>
          </div>
          </div>
          <div class="account-card-side">
            <div class="card-balance" id="balance-${escAttr(a.id)}">
              ${official ? renderOfficialCardStatus(a) : '<span class="balance-loading">—</span>'}
            </div>
            <div class="card-actions-row">
              ${active
                ? renderAccountIconAction('已应用', 'check', '', 'account-applied', true)
                : renderAccountIconAction('应用', 'check', `applyAccount('${escAttr(a.id)}')`, 'account-apply')}
              ${renderAccountIconAction('测试上游连接', 'test-upstream', `testAccountUpstreamForCard('${escAttr(a.id)}')`, 'account-refresh')}
              ${renderAccountIconAction('编辑', 'edit', `editAccount('${escAttr(a.id)}', 'codex')`)}
              ${renderAccountIconAction('删除', 'trash', `deleteAccount('${escAttr(a.id)}')`, 'danger')}
            </div>
          </div>
        </div>
      </div>`;
    }).join('') + '</div>';
  }

  return `<div class="accounts-page-shell">
    <div class="accounts-static-header">
      <div class="page-header accounts-page-header">
        <div><h2>账号管理</h2></div>
      </div>
      <div class="accounts-client-row">
        ${renderClientSwitcher(list)}
        <div class="page-header-actions">
          ${renderToolbarIconAction('导入配置', 'import', 'importFromCodex()')}
          ${renderToolbarIconAction('扫描客户端', 'scan', 'scanClientAccounts()')}
          ${renderToolbarIconAction('添加账号', 'plus', "navigateAccounts('add')", 'btn-primary')}
        </div>
      </div>
    </div>
    <div class="accounts-scroll-region">
      ${renderRouterStatusOverview()}
      ${cards}
    </div>
  </div>`;
}

function renderClientAccountCard(a) {
  const kind = accountClientKind(a);
  const surface = accountClientSurface(a);
  const profile = getClientProfile(kind);
  const statusId = `client-status-${escAttr(a.id)}`;
  const active = activeSelectionForAccount(a)?.account_id === a.id;
  const applied = clientAccountApplied(a);
  const statusReport = clientAccountStatusReport(a);
  const statusHtml = renderClientCardStatusSummary(statusReport, applied);
  return `<div class="account-card client-account-card${active ? ' active' : ''}">
    <div class="account-card-mainline">
      <div class="account-card-primary">
        <div class="account-card-info">
          <div class="account-card-header">
            <div class="account-card-titlebar">
              <span class="client-kind-badge">${clientIcon(kind)}${esc(clientSurfaceTitle(kind, surface))}</span>
              ${renderProviderBadge(a.provider)}
              ${active ? '<span class="active-badge">活跃</span>' : (applied ? '<span class="active-badge">已写入</span>' : '')}
              ${accountRuntimeBadge(a)}
            </div>
          </div>
          <div class="account-card-body">
            <div class="account-card-main">
              <div class="card-name">${esc(a.name)}</div>
            </div>
          </div>
        </div>
      </div>
      <div class="account-card-side">
        <div class="card-balance client-status-box" id="${statusId}">${statusHtml}</div>
        <div class="card-actions-row">
          ${renderAccountIconAction(applied ? '重新写入配置' : '写入配置', 'check', `applyClientAccount('${escAttr(a.id)}')`, active ? 'account-apply account-applied' : 'account-apply')}
          ${renderAccountIconAction('刷新状态', 'refresh', `refreshClientAccountStatus('${escAttr(a.id)}')`, 'account-refresh')}
          ${renderAccountIconAction('编辑', 'edit', `editAccount('${escAttr(a.id)}', '${escAttr(kind)}')`)}
          ${renderAccountIconAction('删除', 'trash', `deleteAccount('${escAttr(a.id)}')`, 'danger')}
        </div>
      </div>
    </div>
  </div>`;
}

// ── Level 2: 添加账号 ──

function renderAddAccount() {
  if (oauthLoginState) return renderOAuthLoginPanel();
  let cards = '';
  if (providerPresets.length === 0) {
    cards = '<div class="empty-state">加载供应商列表...</div>';
  } else {
    const providers = providersForClientKind(selectedClientKind).filter(p =>
      !(normalizeClientKind(selectedClientKind) === 'codex' && p && p.slug === 'codex')
    );
    const surface = selectedSurfaceForKind(selectedClientKind);
    cards = '<div class="provider-grid">' + renderOfficialAccountCard(selectedClientKind) + providers.map(p => {
      return `<div class="provider-card" role="button" tabindex="0" aria-label="添加 ${escAttr(p.label)} 账号" onclick="addAccount('${escAttr(p.slug)}', '${escAttr(selectedClientKind)}', '${escAttr(surface)}')" onkeydown="if(event.key==='Enter'||event.key===' '){event.preventDefault();addAccount('${escAttr(p.slug)}', '${escAttr(selectedClientKind)}', '${escAttr(surface)}')}">
        <div class="provider-icon">${providerIcon(p.slug, p.label)}</div>
        <div class="provider-copy">
          <div class="provider-name">${esc(p.label)}</div>
          <div class="provider-desc">${esc(p.description)}</div>
        </div>
        <div class="provider-card-arrow" aria-hidden="true"></div>
      </div>`;
    }).join('') + '</div>';
  }

  return `<div class="breadcrumb add-account-breadcrumb">
    <button type="button" class="page-back-button add-back-link" onclick="navigateAccounts('list')" aria-label="返回账号列表">
      <span class="line-action-icon line-action-icon-back" aria-hidden="true"></span>
    </button>
  </div>
  <div class="page-header add-provider-header">
    <div class="add-provider-title-line">
      <h2>选择供应商</h2>
      <span class="add-provider-client">${esc(CLIENT_KIND_LABELS[selectedClientKind] || '客户端')}${clientKindSupportsSurface(selectedClientKind) ? ' · ' + esc(clientSurfaceLabel(selectedSurfaceForKind(selectedClientKind))) : ''}</span>
    </div>
  </div>
  <div class="provider-picker-shell">${cards}</div>`;
}

function renderOfficialAccountCard(kind) {
  const normalized = normalizeClientKind(kind);
  if (normalized === 'codex') return renderCodexOfficialAccountCard();
  return renderOfficialOAuthCards(kind);
}

function renderCodexOfficialAccountCard() {
  const surface = selectedSurfaceForKind('codex');
  const surfaceTitle = clientSurfaceTitle('codex', surface);
  return `<details class="official-add-details">
    <summary class="provider-card official-login-card official-add-master-card" aria-label="展开 Codex 官方添加方式">
      <div class="provider-icon">${providerIcon('codex', 'Codex 官方')}</div>
      <div class="provider-copy">
        <div class="provider-name">Codex 官方</div>
        <div class="provider-desc">登录、设备码、认证 JSON 或手动配置</div>
      </div>
      <div class="provider-card-arrow" aria-hidden="true"></div>
    </summary>
    <div class="official-add-submenu">
      ${renderOfficialAddSubItem('导入认证 JSON', '导入到账号池', 'importAuthJsonAccounts()')}
      ${renderOfficialAddSubItem(`官方 ${surfaceTitle} 登录`, 'OAuth 登录', "startOAuthAccountLogin('codex', 'browser')")}
      ${renderOfficialAddSubItem(`${surfaceTitle} 设备码登录`, '设备码登录', "startOAuthAccountLogin('codex', 'device')")}
      ${renderOfficialAddSubItem('Codex 官方', '手动配置', `addCodexOfficialAccount('${escAttr(surface)}')`)}
    </div>
  </details>`;
}

function renderOfficialAddSubItem(title, desc, onclick) {
  return `<div class="provider-card official-login-card official-add-subitem" role="button" tabindex="0" aria-label="${escAttr(title)}" onclick="${onclick}" onkeydown="if(event.key==='Enter'||event.key===' '){event.preventDefault();${onclick}}">
    <div class="provider-icon">${providerIcon('codex', title)}</div>
    <div class="provider-copy">
      <div class="provider-name">${esc(title)}</div>
      <div class="provider-desc">${esc(desc)}</div>
    </div>
    <div class="provider-card-arrow" aria-hidden="true"></div>
  </div>`;
}

function addCodexOfficialAccount(surface) {
  addAccount('codex', 'codex', surface || selectedSurfaceForKind('codex'));
}

function renderOfficialOAuthCards(kind) {
  const normalized = normalizeClientKind(kind);
  const surfaceTitle = clientSurfaceTitle(normalized, selectedSurfaceForKind(normalized));
  if (normalized === 'codex') {
    return `<div class="official-login-grid">
      ${renderOfficialOAuthCard('codex', 'browser', `官方 ${surfaceTitle} 登录`, '使用 OpenAI 官方 OAuth 登录 Codex 账号', 'https://chatgpt.com/backend-api/codex')}
      ${renderOfficialOAuthCard('codex', 'device', `${surfaceTitle} 设备码登录`, '无法完成本机回调时使用设备码登录', 'auth.openai.com/codex/device')}
    </div>`;
  }
  if (normalized === 'claude_code') {
    return renderOfficialOAuthCard('claude', 'browser', `官方 ${surfaceTitle} 登录`, '使用 Anthropic OAuth 登录，并由 deecodex 管理 token', 'https://api.anthropic.com');
  }
  return '';
}

function renderOfficialOAuthCard(provider, mode, title, desc, upstream) {
  const logo = provider === 'claude' ? 'anthropic' : 'codex';
  return `<div class="provider-card official-login-card" role="button" tabindex="0" aria-label="${escAttr(title)}" onclick="startOAuthAccountLogin('${escAttr(provider)}', '${escAttr(mode)}')" onkeydown="if(event.key==='Enter'||event.key===' '){event.preventDefault();startOAuthAccountLogin('${escAttr(provider)}', '${escAttr(mode)}')}">
    <div class="provider-icon">${providerIcon(logo, title)}</div>
    <div class="provider-copy">
      <div class="provider-name">${esc(title)}</div>
      <div class="provider-desc">${esc(desc)}</div>
    </div>
    <div class="provider-card-arrow" aria-hidden="true"></div>
  </div>`;
}

function renderOAuthLoginPanel() {
  const state = oauthLoginState || {};
  const status = state.status || 'pending';
  const title = state.provider === 'claude' ? 'Claude 官方登录' : 'Codex 官方登录';
  const url = state.verification_url || state.url || '';
  const userCode = state.user_code ? `<div class="oauth-device-code">${esc(state.user_code)}</div>` : '';
  const statusText = status === 'success' ? '登录完成' : (status === 'error' ? '登录失败' : (status === 'expired' ? '登录已过期' : '等待授权'));
  return `<div class="breadcrumb add-account-breadcrumb">
    <button type="button" class="page-back-button add-back-link" onclick="cancelOAuthAccountLogin()" aria-label="返回添加账号">
      <span class="line-action-icon line-action-icon-back" aria-hidden="true"></span>
    </button>
  </div>
  <div class="page-header add-provider-header oauth-login-header">
    <h2>${esc(title)}</h2>
    <p>${esc(statusText)}</p>
  </div>
  <div class="oauth-login-panel">
    <div class="oauth-login-status ${escAttr(status)}">${esc(statusText)}</div>
    ${userCode}
    <div class="oauth-login-url" title="${escAttr(url)}">${esc(url)}</div>
    <div class="oauth-login-actions">
      ${url ? `<button class="btn btn-primary" onclick="openOAuthLoginUrl()">打开登录页</button>` : ''}
      ${url ? `<button class="btn btn-ghost" onclick="copyOAuthLoginUrl()">复制链接</button>` : ''}
      <button class="btn btn-ghost" onclick="pollOAuthAccountLogin()">刷新状态</button>
      <button class="btn btn-danger" onclick="cancelOAuthAccountLogin()">取消</button>
    </div>
    <div class="oauth-login-message">${esc(state.message || '')}</div>
  </div>`;
}

function providersForClientKind(kind) {
  const normalized = normalizeClientKind(kind);
  if (normalized === 'codex') return providerPresets;
  const bySlug = slug => providerPresets.find(p => p.slug === slug);
  const custom = bySlug('custom') || {
    slug: 'custom',
    label: '自定义',
    description: 'OpenAI-compatible 或客户端原生 Base URL',
    default_upstream: '',
    known_models: [],
  };
  if (normalized === 'claude_code') return [
    bySlug('anthropic'),
    bySlug('deepseek'),
    bySlug('kimi'),
    bySlug('minimax'),
    bySlug('mimo'),
    bySlug('longcat'),
    bySlug('qwen'),
    bySlug('glm'),
    bySlug('openrouter'),
    custom,
  ].filter(Boolean);
  if (normalized === 'openclaw') return [bySlug('openrouter'), bySlug('anthropic'), bySlug('openai'), bySlug('qwen'), bySlug('minimax'), custom].filter(Boolean);
  if (normalized === 'hermes') return [bySlug('openrouter'), bySlug('anthropic'), bySlug('openai'), bySlug('qwen'), bySlug('minimax'), custom].filter(Boolean);
  return [bySlug('openai'), bySlug('openrouter'), bySlug('qwen'), custom].filter(Boolean);
}

function clientProviderDefaults(kind, provider, preset = null) {
  const normalized = normalizeClientKind(kind);
  const p = preset || getProviderPreset(provider) || {};
  if (normalized === 'claude_code' && provider === 'deepseek') {
    return {
      upstream: 'https://api.deepseek.com/anthropic',
      default_model: 'deepseek-v4-pro[1m]',
      known_models: ['deepseek-v4-pro[1m]', 'deepseek-v4-pro', 'deepseek-v4-flash'],
      api_key_env: 'ANTHROPIC_AUTH_TOKEN',
    };
  }
  if (normalized === 'claude_code' && provider === 'kimi') {
    return {
      upstream: 'https://api.moonshot.cn/anthropic',
      default_model: 'kimi-k2.5',
      known_models: ['kimi-k2.5', 'kimi-k2-turbo-preview', 'kimi-k2-0711-preview'],
      api_key_env: 'ANTHROPIC_AUTH_TOKEN',
    };
  }
  if (normalized === 'claude_code' && provider === 'minimax') {
    return {
      upstream: 'https://api.minimaxi.com/anthropic',
      default_model: 'MiniMax-M2.7',
      known_models: ['MiniMax-M2.7', 'MiniMax-M2', 'MiniMax-M1'],
      api_key_env: 'ANTHROPIC_API_KEY',
    };
  }
  if (normalized === 'claude_code' && provider === 'mimo') {
    return {
      upstream: 'https://token-plan-cn.xiaomimimo.com/anthropic',
      default_model: MIMO_CODING_MODEL,
      known_models: MIMO_CHAT_MODELS,
      api_key_env: 'ANTHROPIC_API_KEY',
    };
  }
  if (normalized === 'claude_code' && provider === 'longcat') {
    return {
      upstream: 'https://api.longcat.chat/anthropic',
      default_model: 'LongCat-Flash-Chat',
      known_models: [
        'LongCat-2.0-Preview',
        'LongCat-Flash-Lite',
        'LongCat-Flash-Chat',
        'LongCat-Flash-Thinking-2601',
        'LongCat-Flash-Omni-2603',
      ],
      api_key_env: 'ANTHROPIC_AUTH_TOKEN',
    };
  }
  if (normalized === 'claude_code' && provider === 'qwen') {
    return {
      upstream: 'https://dashscope.aliyuncs.com/apps/anthropic',
      default_model: 'qwen3.6-plus',
      known_models: ['qwen3.6-plus', 'qwen-plus', 'qwen-max', 'qwen-turbo'],
      api_key_env: 'ANTHROPIC_API_KEY',
    };
  }
  if (normalized === 'claude_code' && provider === 'glm') {
    return {
      upstream: 'https://open.bigmodel.cn/api/anthropic',
      default_model: 'glm-5.1',
      known_models: ['glm-5.1', 'glm-5', 'glm-4.7', 'glm-4.5-air'],
      api_key_env: 'ANTHROPIC_AUTH_TOKEN',
    };
  }
  return {
    upstream: p.default_upstream || '',
    default_model: (Array.isArray(p.known_models) && p.known_models[0]) || '',
    known_models: Array.isArray(p.known_models) ? p.known_models : [],
    api_key_env: defaultApiKeyEnvForClient({ provider, client_kind: normalized }),
  };
}

// ── Level 3: 编辑账号 ──

function renderAccountDetail() {
  if (!editingAccount) return '<div class="empty-state">账号数据丢失，请返回列表</div>';
  const a = editingAccount;
  if (!isCodexAccount(a)) return renderClientAccountDetail();
  ensureAccountEndpoints(a);
  const initialEndpoint = currentEndpoint(a) || {};
  const responsesDirectForm = isResponsesDirectFormAccount(a, initialEndpoint);
  if (responsesDirectForm && endpointKindIsLockedForForm(a, initialEndpoint)) {
    normalizeResponsesDirectAccount(a, initialEndpoint);
  }
  const ep = currentEndpoint(a) || {};
  const visionMode = ep.vision?.mode || (a.vision_enabled ? 'glue' : 'native');
  const contextWindow = ep.context_window_override ?? null;
  const reasoningEffort = ep.reasoning_effort_override ?? null;
  const thinkingTokens = ep.thinking_tokens ?? null;
  const fastEnabled = ep.fast_mode_enabled === true;
  const fastServiceTier = ep.fast_service_tier || 'priority';
  const customHeaders = ep.custom_headers || {};
  const requestTimeout = ep.request_timeout_secs ?? null;
  const maxRetries = ep.max_retries ?? a.max_retries;
  const knownModels = getProviderKnownModels(a.provider);
  const responsesDirect = endpointKindIsResponsesDirect(ep.kind);
  const usesModelMapping = !responsesDirectForm && endpointKindUsesModelMapping(ep.kind);
  const lockEndpointKind = endpointKindIsLockedForForm(a, ep);
  const endpointKindControl = lockEndpointKind
    ? `<input type="hidden" id="edit_endpoint_kind" value="${escAttr(normalizeResponsesKind(ep.kind) || 'open_ai_responses')}">
          <input type="text" value="${escAttr(endpointKindLabel(ep.kind))}" disabled aria-label="上游 API 类型">
          <span class="hint">原生端点固定使用对应的官方协议。</span>`
    : `<select id="edit_endpoint_kind" onchange="handleEndpointKindChange(this.value)">
            ${ep.kind === 'custom_chat' || ep.kind === 'CustomChat' ? '<option value="custom_chat" selected hidden>OpenAI Chat 兼容（自定义路径）</option>' : ''}
            ${ep.kind === 'custom_responses' || ep.kind === 'CustomResponses' ? '<option value="custom_responses" selected hidden>OpenAI Responses 直连（自定义路径）</option>' : ''}
            <option value="open_ai_chat" ${(ep.kind || 'open_ai_chat') === 'open_ai_chat' || ep.kind === 'OpenAiChat' ? 'selected' : ''}>OpenAI Chat 兼容（推荐）</option>
            <option value="open_ai_responses" ${ep.kind === 'open_ai_responses' || ep.kind === 'OpenAiResponses' ? 'selected' : ''}>OpenAI Responses 直连</option>
            <option value="anthropic_messages" ${ep.kind === 'anthropic_messages' || ep.kind === 'AnthropicMessages' ? 'selected' : ''}>Anthropic Messages</option>
            <option value="codex_official" ${ep.kind === 'codex_official' || ep.kind === 'CodexOfficial' ? 'selected' : ''}>Codex 官方</option>
          </select>
          <span class="hint">DeepSeek、OpenRouter 这类一般选 OpenAI Chat 兼容；只有上游原生支持 Responses API 时才选直连。</span>`;
  const modelSection = usesModelMapping ? `
    <section class="account-edit-section">
      <div class="account-section-head">
        <div class="section-sub-label">模型</div>
      </div>
      <div class="section-action-row">
        <button class="btn btn-ghost" onclick="fetchAndPopulateModels()">从上游获取模型列表</button>
        <span id="modelFetchStatus"></span>
      </div>
      <div class="model-map-table">
        <div class="model-map-head">
          <span>Codex 请求模型</span>
          <span>上游模型</span>
          <span>图片处理</span>
        </div>
        <div id="modelMapRows">${renderModelMappingRows(knownModels)}</div>
      </div>
      <div class="model-add-row"><button onclick="addModelRow('modelMapRows', '${escAttr(JSON.stringify(knownModels))}')">+ 添加模型映射</button></div>
    </section>` : '';
  const passthroughModel = responsesDirect || ep.kind === 'codex_official' || ep.kind === 'CodexOfficial';
  const visionSectionTitle = passthroughModel ? '图片处理' : '其他模型图片处理';
  const visionTargetLabel = passthroughModel ? '当前端点' : '其他模型';
  const visionHint = responsesDirect
    ? 'Responses 直连保留 Codex 原始模型名；这里仅设置图片输入处理策略。'
    : (ep.kind === 'codex_official' || ep.kind === 'CodexOfficial')
      ? 'Codex 官方账号使用官方模型名；这里仅设置图片输入处理策略。'
    : '模型映射行优先生效；这里处理临时模型或未列出的模型。';
  const visionSection = responsesDirectForm ? '' : `
    <section class="account-edit-section">
      <div class="account-section-head">
        <div class="section-sub-label">${visionSectionTitle}</div>
      </div>
      <div class="config-fields">
        <div class="config-field">
          <label>${visionTargetLabel}</label>
          <input type="hidden" id="edit_vision_mode" value="${escAttr(String(visionMode).toLowerCase())}">
          <div class="vision-mode-segments" role="group" aria-label="视觉能力">
            <button type="button" class="vision-mode-option ${visionMode === 'off' || visionMode === 'Off' ? 'active' : ''}" data-mode="off" onclick="setVisionMode('off')">关闭</button>
            <button type="button" class="vision-mode-option ${visionMode === 'native' || visionMode === 'Native' ? 'active' : ''}" data-mode="native" onclick="setVisionMode('native')">原生</button>
            <button type="button" class="vision-mode-option ${visionMode === 'glue' || visionMode === 'Glue' ? 'active' : ''}" data-mode="glue" onclick="setVisionMode('glue')">胶水</button>
          </div>
          <span class="hint">${visionHint}</span>
        </div>
        <div class="config-field">
          <label>不支持图片时</label>
          <select id="edit_unsupported_image_policy">
            <option value="reject" ${(ep.vision?.unsupported_image_policy || 'reject') === 'reject' ? 'selected' : ''}>拒绝请求并提示</option>
            <option value="strip_with_warning" ${ep.vision?.unsupported_image_policy === 'strip_with_warning' ? 'selected' : ''}>剥离图片并继续</option>
          </select>
          <span class="hint">关闭视觉或模型覆盖为关闭时生效</span>
        </div>
      </div>
      <div id="visionFields" style="${visionMode === 'glue' || visionMode === 'Glue' ? '' : 'display:none;'}">
        <div class="config-fields nested-fields">
          <div class="config-field">
            <label>胶水适配器</label>
            <select id="edit_vision_adapter">
              <option value="minimax_coding_plan_vlm" ${(ep.vision?.adapter_id || 'minimax_coding_plan_vlm') === 'minimax_coding_plan_vlm' ? 'selected' : ''}>MiniMax Coding Plan VLM</option>
            </select>
            <span class="hint">第一版仅实现 MiniMax 胶水适配器</span>
          </div>
          <div class="config-field">
            <label>胶水策略</label>
            <select id="edit_glue_strategy">
              <option value="final_answer" ${(ep.vision?.glue_strategy || 'final_answer') === 'final_answer' ? 'selected' : ''}>视觉模型直接回答</option>
              <option value="caption_then_main" ${ep.vision?.glue_strategy === 'caption_then_main' ? 'selected' : ''}>先识图再交给主模型</option>
            </select>
          </div>
          <div class="config-field">
            <label>视觉上游 URL</label>
            <input type="text" id="edit_vision_upstream" value="${escAttr(ep.vision?.base_url || a.vision_upstream || '')}" placeholder="https://api.minimaxi.com">
          </div>
          <div class="config-field">
            <label>视觉 API Key</label>
            <div class="pass-group ${hasStoredVisionApiKey(a, ep) ? 'has-copy' : ''}">
              <input type="password" id="edit_vision_api_key" value="${escAttr(displayStoredSecret(ep.vision?.api_key || a.vision_api_key, hasStoredVisionApiKey(a, ep)))}" placeholder="视觉模型密钥" autocomplete="off">
              ${hasStoredVisionApiKey(a, ep) ? secretCopyButton('vision_api_key') : ''}
              <button type="button" class="pass-toggle" onclick="togglePass('edit_vision_api_key', this)" title="显示/隐藏视觉 API Key" aria-label="显示或隐藏视觉 API Key">
                <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M2.5 12s3.5-6 9.5-6 9.5 6 9.5 6-3.5 6-9.5 6-9.5-6-9.5-6z"></path><circle cx="12" cy="12" r="2.5"></circle></svg>
              </button>
            </div>
          </div>
          <div class="config-field">
            <label>视觉模型名</label>
            <input type="text" id="edit_vision_model" value="${escAttr(ep.vision?.model || a.vision_model || '')}" placeholder="MiniMax-M2.7">
          </div>
          <div class="config-field">
            <label>视觉端点路径</label>
            <input type="text" id="edit_vision_endpoint" value="${escAttr(ep.vision?.path || a.vision_endpoint || 'v1/coding_plan/vlm')}" placeholder="v1/coding_plan/vlm">
            <div class="inline-test-row">
              <button class="btn btn-ghost" onclick="testVisionConnectivity()">测试视觉端点</button>
              <span id="visionConnectivityResult"></span>
            </div>
          </div>
        </div>
      </div>
    </section>`;
  const runtimeSection = responsesDirectForm ? `
    <section class="account-edit-section">
      <div class="account-section-head">
        <div class="section-sub-label">运行参数</div>
      </div>
      <div class="config-fields">
        <div class="config-field">
          <label class="toggle-label">
            <input type="checkbox" id="edit_fast_enabled" ${fastEnabled ? 'checked' : ''} onchange="toggleFastFields()">
            GPT Fast 服务层
          </label>
          <span class="hint">OpenAI Responses 直连端点生效；仅注入 service_tier。</span>
        </div>
      </div>
      <div id="fastFields" style="${fastEnabled ? '' : 'display:none;'}">
        <div class="config-fields nested-fields">
          <div class="config-field">
            <label>service_tier</label>
            <input type="text" id="edit_fast_service_tier" value="${escAttr(fastServiceTier)}" placeholder="priority">
          </div>
        </div>
      </div>
    </section>` : `
    <section class="account-edit-section">
      <div class="account-section-head">
        <div class="section-sub-label">运行参数</div>
      </div>
      <div class="config-fields">
        <div class="config-field">
          <label class="toggle-label">
            <input type="checkbox" id="edit_cw_enabled" ${contextWindow ? 'checked' : ''} onchange="toggleContextWindowFields()">
            上下文窗口覆盖
          </label>
          <span class="hint">勾选后 Codex 使用下方 token 数作为该账号上下文。</span>
        </div>
        <div class="config-field" id="cwSizeField" style="${contextWindow ? '' : 'display:none;'}">
          <label>上下文窗口大小 (token)</label>
          <input type="number" id="edit_cw_size" value="${contextWindow || 1000000}" min="1" max="10000000" step="1" placeholder="1000000">
          <span class="hint">Codex 有效上下文约为此值 × 95%。</span>
        </div>
        <div class="config-field">
          <label class="toggle-label">
            <input type="checkbox" id="edit_reasoning_enabled" ${reasoningEffort ? 'checked' : ''} onchange="toggleReasoningFields()">
            推理强度覆盖
          </label>
          <span class="hint">用于 Claude、R1 等需要固定思考预算的账号。</span>
        </div>
        <div class="config-field">
          <label class="toggle-label">
            <input type="checkbox" id="edit_fast_enabled" ${fastEnabled ? 'checked' : ''} onchange="toggleFastFields()">
            GPT Fast 服务层
          </label>
          <span class="hint">仅 OpenAI Responses 直连端点生效；保持推理预算，只注入 service_tier。</span>
        </div>
      </div>
      <div id="reasoningFields" style="${reasoningEffort ? '' : 'display:none;'}">
        <div class="config-fields nested-fields">
          <div class="config-field">
            <label>推理强度</label>
            <select id="edit_reasoning_effort">
              <option value="" ${!reasoningEffort ? 'selected' : ''}>不覆盖（跟随 Codex 请求）</option>
              <option value="low" ${reasoningEffort === 'low' ? 'selected' : ''}>low - 低推理</option>
              <option value="medium" ${reasoningEffort === 'medium' ? 'selected' : ''}>medium - 中等推理</option>
              <option value="high" ${reasoningEffort === 'high' ? 'selected' : ''}>high - 高推理</option>
              <option value="max" ${reasoningEffort === 'max' ? 'selected' : ''}>max - 最大推理</option>
            </select>
          </div>
          <div class="config-field">
            <label>思考 Token 预算 <span class="optional-label">可选</span></label>
            <input type="number" id="edit_thinking_tokens" value="${thinkingTokens || ''}" min="1024" max="128000" step="1024" placeholder="留空不设限制，如 16000">
            <span class="hint">Claude Extended Thinking 的 token 预算，留空则不限制</span>
          </div>
        </div>
      </div>
      <div id="fastFields" style="${fastEnabled ? '' : 'display:none;'}">
        <div class="config-fields nested-fields">
          <div class="config-field">
            <label>service_tier</label>
            <input type="text" id="edit_fast_service_tier" value="${escAttr(fastServiceTier)}" placeholder="priority">
            <span class="hint">默认 priority；旧配置中的 fast 会在请求时自动转为 priority。</span>
          </div>
        </div>
      </div>
    </section>`;
  const capabilitySection = responsesDirectForm ? '' : `
    <section class="account-edit-section">
      <div class="account-section-head">
        <div class="section-sub-label">能力补全</div>
      </div>
      <div class="config-fields">
        <div class="config-field">
          <label class="toggle-label">
            <input type="checkbox" id="edit_capability_enabled" ${a.capability_enabled ? 'checked' : ''} onchange="toggleCapabilityFields()">
            启用能力补全
          </label>
          <span class="hint">能力账号不能选择当前账号，建议使用支持原生工具和多模态的账号。</span>
        </div>
        <div class="config-field" id="capabilityFields" style="${a.capability_enabled ? '' : 'display:none;'}">
          <label>能力账号</label>
          <select id="edit_capability_account_id">${renderCapabilityAccountOptions(a)}</select>
        </div>
      </div>
    </section>`;
  const devPipelineSection = responsesDirectForm ? '' : `
    <section class="account-edit-section">
      <div class="account-section-head">
        <div class="section-sub-label">开发协作编排</div>
      </div>
      <div class="config-fields">
        <div class="config-field wide">
          <label class="toggle-label">
            <input type="checkbox" id="edit_dev_pipeline_enabled" ${a.dev_pipeline_enabled ? 'checked' : ''} onchange="toggleDevPipelineFields()">
            启用开发协作编排
          </label>
          <span class="hint">Codex 中输入触发命令即可进入协作流程，默认以当前活跃账号为主。</span>
        </div>
        <div class="dev-pipeline-fields" id="devPipelineFields" style="${a.dev_pipeline_enabled ? '' : 'display:none;'}">
          <div class="config-fields nested-fields">
            <div class="config-field">
              <label>触发方式</label>
              <select id="edit_dev_pipeline_trigger_mode">
                <option value="manual" ${(a.dev_pipeline_trigger_mode || 'manual') === 'manual' ? 'selected' : ''}>手动触发</option>
                <option value="always" ${a.dev_pipeline_trigger_mode === 'always' ? 'selected' : ''}>始终触发</option>
              </select>
            </div>
            <div class="config-field">
              <label>触发命令</label>
              <input type="text" id="edit_dev_pipeline_command" value="${escAttr(a.dev_pipeline_command || '/dev-pipeline')}" placeholder="/dev-pipeline">
            </div>
            <div class="config-field">
              <label>方案设计账号</label>
              <select id="edit_dev_pipeline_architect_account_id">${renderDevPipelineAccountOptions(a, a.dev_pipeline_architect_account_id || '')}</select>
            </div>
            <div class="config-field">
              <label>实现填充账号</label>
              <select id="edit_dev_pipeline_implementer_account_id">${renderDevPipelineAccountOptions(a, a.dev_pipeline_implementer_account_id || '')}</select>
            </div>
            <div class="config-field">
              <label>验收收口账号</label>
              <select id="edit_dev_pipeline_reviewer_account_id">${renderDevPipelineAccountOptions(a, a.dev_pipeline_reviewer_account_id || '')}</select>
            </div>
            <div class="config-field">
              <label>实现阶段工具能力</label>
              <select id="edit_dev_pipeline_tool_mode">
                <option value="controlled_tools" ${(a.dev_pipeline_tool_mode || 'controlled_tools') === 'controlled_tools' ? 'selected' : ''}>受控工具执行</option>
                <option value="patch_only" ${a.dev_pipeline_tool_mode === 'patch_only' ? 'selected' : ''}>仅生成补丁</option>
                <option value="full_agent" ${a.dev_pipeline_tool_mode === 'full_agent' ? 'selected' : ''}>完整 Codex 执行能力</option>
              </select>
              <span class="hint">当前版本会把能力模式注入阶段指令；工具执行仍继承 deecodex 的安全策略。</span>
            </div>
            <div class="config-field">
              <label>最大修正轮数</label>
              <input type="number" id="edit_dev_pipeline_max_iterations" value="${Number(a.dev_pipeline_max_iterations || 3)}" min="1" max="10" step="1">
            </div>
            <div class="config-field">
              <label class="toggle-label">
                <input type="checkbox" id="edit_dev_pipeline_show_trace" ${a.dev_pipeline_show_trace ? 'checked' : ''}>
                在最终回答中显示阶段摘要
              </label>
            </div>
            <div class="config-field">
              <label>方案角色附加指令</label>
              <textarea id="edit_dev_pipeline_architect_instruction" rows="2" placeholder="可选">${escAttr(a.dev_pipeline_architect_instruction || '')}</textarea>
            </div>
            <div class="config-field">
              <label>实现角色附加指令</label>
              <textarea id="edit_dev_pipeline_implementer_instruction" rows="2" placeholder="可选">${escAttr(a.dev_pipeline_implementer_instruction || '')}</textarea>
            </div>
            <div class="config-field">
              <label>验收角色附加指令</label>
              <textarea id="edit_dev_pipeline_reviewer_instruction" rows="2" placeholder="可选">${escAttr(a.dev_pipeline_reviewer_instruction || '')}</textarea>
            </div>
          </div>
        </div>
      </div>
    </section>`;
  const endpointPathField = responsesDirectForm && !endpointKindIsCustomResponses(ep.kind) ? '' : `
        <div class="config-field">
          <label>请求路径 <span class="optional-label">可选</span></label>
          <input type="text" id="edit_endpoint_path" value="${escAttr(ep.path || '')}" placeholder="留空自动使用所选 API 类型">
          <span class="hint">私有代理或非标准网关才需要填写，例如 /v1/chat/completions。</span>
        </div>`;

  return `<div class="breadcrumb account-detail-breadcrumb">
    <button type="button" class="page-back-button account-back-link" onclick="navigateAccounts('list')" aria-label="返回账号列表">
      <span class="line-action-icon line-action-icon-back" aria-hidden="true"></span>
    </button>
  </div>
  <div class="page-header account-detail-header">
    <div class="account-detail-title">
      <img src="${providerLogoSrc(a.provider)}" alt="" aria-hidden="true">
      <div>
        <div class="account-detail-heading">
          <h2>${esc(accountDisplayTitle(a))}</h2>
        </div>
      </div>
    </div>
  </div>

  <div class="account-form">
    <section class="account-edit-section">
      <div class="account-section-head">
        <div class="section-sub-label">账号凭据</div>
      </div>
      <div class="config-fields">
        <div class="config-field account-name-field">
          <label>账号名称</label>
          <input type="text" id="edit_name" value="${escAttr(a.name)}" placeholder="输入账号显示名">
        </div>
        <div class="config-field account-key-field">
          <label>API Key</label>
          <div class="pass-group ${hasStoredPrimaryApiKey(a) ? 'has-copy' : ''}">
            <input type="password" id="edit_api_key" value="${escAttr(displayStoredSecret(a.api_key, hasStoredPrimaryApiKey(a)))}" placeholder="输入 API 密钥" autocomplete="off">
            ${hasStoredPrimaryApiKey(a) ? secretCopyButton('api_key') : ''}
            <button type="button" class="pass-toggle" onclick="togglePass('edit_api_key', this)" title="显示/隐藏 API Key" aria-label="显示或隐藏 API Key">
              <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M2.5 12s3.5-6 9.5-6 9.5 6 9.5 6-3.5 6-9.5 6-9.5-6-9.5-6z"></path><circle cx="12" cy="12" r="2.5"></circle></svg>
            </button>
          </div>
        </div>
        <div class="config-field wide">
          <label>上游 URL</label>
          <div class="upstream-test-group">
            <input type="text" id="edit_upstream" value="${escAttr(ep.base_url || a.upstream)}" placeholder="https://api.example.com/v1">
            <button class="btn btn-ghost" onclick="testUpstreamConnectivity()">测试连通性</button>
          </div>
          <div class="inline-test-result">
            <span id="connectivityResult"></span>
          </div>
        </div>
      </div>
    </section>

    <section class="account-edit-section">
      <div class="account-section-head">
        <div class="section-sub-label">上游</div>
      </div>
      <div class="config-fields">
        <div class="config-field">
          <label>上游 API 类型</label>
          ${endpointKindControl}
        </div>
        <div class="config-field">
          <label>余额查询 URL <span class="optional-label">可选</span></label>
          <input type="text" id="edit_balance_url" value="${escAttr(ep.balance_url || '')}" placeholder="留空则自动探测">
        </div>
      </div>
    </section>

    ${renderCodexRouterPanel(a)}
    ${renderCodexOfficialRuntimePanel(a)}

    ${modelSection}
    ${visionSection}
    ${runtimeSection}
    ${capabilitySection}
    ${devPipelineSection}

  <div class="collapsible-section">
    <button class="collapsible-toggle" onclick="this.classList.toggle('open');this.nextElementSibling.classList.toggle('open')">
      <span class="arrow">▸</span> 高级端点
    </button>
    <div class="collapsible-content">
      <div class="config-fields nested-fields">
        ${endpointPathField}
        <div class="config-field">
          <label>自定义 HTTP 头 <span class="optional-label">可选</span></label>
          <textarea class="mono-textarea" id="edit_custom_headers" rows="3" placeholder="每行一个: Header-Name: value&#10;例: X-Org-Id: org-xxx&#10;例: X-Custom-Auth: token123">${escAttr(Object.entries(customHeaders).map(([k, v]) => k + ': ' + v).join('\n'))}</textarea>
          <span class="hint">每行一个头，格式: 头名称: 头值，将在每次上游请求时附加</span>
        </div>
        <div class="config-field">
          <label>请求超时（秒） <span class="optional-label">可选</span></label>
          <input type="number" id="edit_request_timeout" value="${requestTimeout || ''}" min="1" max="600" step="1" placeholder="留空使用默认 300s">
          <span class="hint">此账号的上游请求超时时间，Claude 扩展思考建议设 180-300</span>
        </div>
        <div class="config-field">
          <label>最大重试次数 <span class="optional-label">可选</span></label>
          <input type="number" id="edit_max_retries" value="${maxRetries ?? ''}" min="0" max="10" step="1" placeholder="留空使用默认 3 次">
          <span class="hint">上游请求失败（401/429/502/503/连接错误）时的重试次数，0 表示不重试</span>
        </div>
      </div>
    </div>
  </div>

  <div class="accounts-actions">
    <button class="btn btn-primary" onclick="saveAccount()">保存账号</button>
    ${a.id ? `<button class="btn btn-ghost" onclick="editConfigFile('${escAttr(a.id)}')">编辑配置文件</button>` : ''}
    <button class="btn btn-danger" onclick="deleteAccount('${escAttr(a.id)}')">删除账号</button>
  </div>
  </div>`;
}

function renderModelMappingRows(knownModels) {
  const a = editingAccount;
  if (!a) return '';
  const ep = currentEndpoint(a) || {};
  const map = ep.model_map || a.model_map || {};
  const rows = [];
  for (const codexModel of CODEX_MODEL_LIST) {
    const val = map[codexModel] || '';
    rows.push({ codexModel, val, readonly: true });
  }
  // 添加自定义映射（不在预定义列表中的）
  for (const [k, v] of Object.entries(map)) {
    if (!CODEX_MODEL_LIST.includes(k) && k) {
      rows.push({ codexModel: k, val: v, readonly: false });
    }
  }
  // 优先使用从上游获取的模型列表，否则用预设列表
  const modelSource = upstreamModels.length > 0 ? upstreamModels : (knownModels || []);
  const suggestionsJson = escAttr(JSON.stringify(modelSource));

  return rows.map((r, i) => {
    const labelExtra = r.readonly ? '' : ' (自定义)';
    const upstreamModel = r.val || r.codexModel;
    const profile = (ep.model_profiles || {})[upstreamModel] || {};
    const visionMode = profile.vision_mode || ep.vision?.mode || 'native';
    const removeControl = r.readonly
      ? ''
      : '<button class="model-remove" onclick="removeModelMapRow(this)" title="移除">✕</button>';
    return `<div class="model-row model-map-row${r.readonly ? '' : ' removable'}">
      <div class="model-label codex">${esc(r.codexModel)}${labelExtra}</div>
      <div class="model-autocomplete model-upstream-cell">
        <input type="text" value="${escAttr(r.val)}" placeholder="未映射 (使用原名)"
          data-codex="${escAttr(r.codexModel)}" data-readonly="${r.readonly}"
          onchange="syncModelVisionTarget(this)"
          onfocus="showSuggestions(this)" oninput="filterSuggestions(this)" onblur="hideSuggestions(this)"
          autocomplete="off">
        <div class="model-suggestions" style="display:none;" data-suggestions="${suggestionsJson}"></div>
      </div>
      <div class="model-vision-cell">
        ${renderModelVisionSegments(upstreamModel, visionMode)}
      </div>
      ${removeControl}
    </div>`;
  }).join('');
}

function renderClientModelMappingRows(account) {
  const kind = accountClientKind(account);
  const slots = clientModelSlots(kind);
  const map = clientModelMap(account);
  const knownKeys = slots.map(slot => slot.key);
  const modelSource = upstreamModels.length > 0 ? upstreamModels : getProviderKnownModels(account.provider || '');
  const suggestionsJson = escAttr(JSON.stringify(modelSource || []));
  const rows = slots.map(slot => ({
    key: slot.key,
    label: slot.label || slot.key,
    target: slot.target || '',
    description: slot.description || '',
    required: Boolean(slot.required),
    value: map[slot.key] || (slot.key === 'default' ? (account.default_model || '') : ''),
    readonly: true,
  }));
  Object.entries(map).forEach(([key, value]) => {
    if (knownKeys.includes(key)) return;
    rows.push({ key, label: key, target: '自定义槽位', required: false, value, readonly: false });
  });
  return rows.map(row => {
    const oneMEnabled = hasClaudeOneMSuffix(row.value);
    const oneMToggle = row.readonly && isClaudeOneMModelSlot(kind, row.key)
      ? `<label class="claude-one-m-toggle toggle-label${oneMEnabled ? ' on' : ''}" title="开启 1M 上下文后，模型名会追加 ${CLAUDE_ONE_M_SUFFIX}">
          <input type="checkbox" class="claude-one-m-input" aria-label="${escAttr(row.label)} 1M 上下文" ${oneMEnabled ? 'checked' : ''} onchange="toggleClaudeOneMContext(this)">
          <span>1M</span>
        </label>`
      : '';
    const slotCell = row.readonly
      ? `<div class="model-label codex client-model-slot">
          <strong>${esc(row.label)}${row.required ? ' *' : ''}</strong>
          <span title="${escAttr(row.description || row.target)}">${esc(row.target)}</span>
          <input type="hidden" class="client-model-slot-key" value="${escAttr(row.key)}">
        </div>`
      : `<div class="model-label client-model-slot custom">
          <input type="text" class="client-model-slot-key" value="${escAttr(row.key)}" placeholder="槽位名，如 rerank">
          <span>${esc(row.target)}</span>
        </div>`;
    return `<div class="model-row client-model-row client-model-template${oneMToggle ? ' claude-one-m-row' : ''}">
      ${slotCell}
      <div class="model-value client-model-value">
        <div class="model-autocomplete">
          <input type="text" class="client-model-value-input" value="${escAttr(row.value || '')}" placeholder="留空则不写入该槽位"
            onfocus="showSuggestions(this)" oninput="filterSuggestions(this); syncClaudeOneMContextToggle(this)" onblur="hideSuggestions(this)" autocomplete="off">
          <div class="model-suggestions" style="display:none;" data-suggestions="${suggestionsJson}"></div>
        </div>
        ${oneMToggle || (row.readonly ? '<span class="model-remove-placeholder"></span>' : '<button class="model-remove" onclick="removeClientModelRow(this)" title="移除">✕</button>')}
      </div>
    </div>`;
  }).join('');
}

function syncClaudeOneMContextToggle(input) {
  const row = input?.closest?.('.client-model-row');
  const toggle = row?.querySelector?.('.claude-one-m-toggle');
  const checkbox = toggle?.querySelector?.('.claude-one-m-input');
  if (!toggle || !checkbox) return;
  const enabled = hasClaudeOneMSuffix(input.value);
  checkbox.checked = enabled;
  toggle.classList.toggle('on', enabled);
}

function toggleClaudeOneMContext(checkbox) {
  const row = checkbox?.closest?.('.client-model-row');
  const input = row?.querySelector?.('.client-model-value-input');
  if (!input) return;
  const base = stripClaudeOneMSuffix(input.value);
  input.value = checkbox.checked && base ? `${base}${CLAUDE_ONE_M_SUFFIX}` : base;
  syncClaudeOneMContextToggle(input);
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

function addClientModelRow() {
  const container = document.getElementById('clientModelMapRows');
  if (!container) return;
  const modelSource = upstreamModels.length > 0 ? upstreamModels : getProviderKnownModels(editingAccount?.provider || '');
  const suggestionsJson = escAttr(JSON.stringify(modelSource || []));
  const row = document.createElement('div');
  row.className = 'model-row client-model-row client-model-template';
  row.innerHTML = `<div class="model-label client-model-slot custom">
      <input type="text" class="client-model-slot-key" placeholder="槽位名，如 rerank">
      <span>自定义槽位</span>
    </div>
    <div class="model-value client-model-value">
      <div class="model-autocomplete">
        <input type="text" class="client-model-value-input" placeholder="上游模型名"
          onfocus="showSuggestions(this)" oninput="filterSuggestions(this)" onblur="hideSuggestions(this)" autocomplete="off">
        <div class="model-suggestions" style="display:none;" data-suggestions="${suggestionsJson}"></div>
      </div>
      <button class="model-remove" onclick="removeClientModelRow(this)" title="移除">✕</button>
    </div>`;
  container.appendChild(row);
}

function removeClientModelRow(btn) {
  btn.closest('.client-model-row')?.remove();
}

function collectClientModelMap() {
  const result = {};
  document.querySelectorAll('#clientModelMapRows .client-model-row').forEach(row => {
    const key = row.querySelector('.client-model-slot-key')?.value?.trim();
    const value = row.querySelector('.client-model-value-input')?.value?.trim();
    if (key && value) result[key] = value;
  });
  return result;
}

function normalizedVisionMode(mode) {
  const value = String(mode || 'native').toLowerCase();
  return ['off', 'native', 'glue'].includes(value) ? value : 'native';
}

function parseOptionalInteger(value) {
  if (value === undefined || value === null || String(value).trim() === '') return null;
  const parsed = parseInt(value, 10);
  return Number.isFinite(parsed) ? parsed : null;
}

function renderModelVisionSegments(model, value) {
  const mode = normalizedVisionMode(value);
  const modelAttr = escAttr(model || '');
  const options = [
    ['off', '关闭'],
    ['native', '原生'],
    ['glue', '胶水'],
  ];
  return `<div class="model-vision-segments" data-model="${modelAttr}" data-mode="${mode}" role="group" aria-label="当前映射模型视觉能力">
    ${options.map(([value, label]) => `<button type="button" class="${mode === value ? 'active' : ''}" data-mode="${value}" onclick="setModelVisionMode(this, '${value}')">${label}</button>`).join('')}
  </div>`;
}

// ── 数据加载 ──

let accountsListRenderSignature = '';

function accountListRenderSignature(data) {
  const activeSurface = selectedSurfaceForKind(selectedClientKind);
  const activeSelection = (data?.active_by_surface || {})[activeSelectionKey(selectedClientKind, activeSurface)] || {};
  const activeId = activeSelection.account_id || data?.active_account_id || data?.active_id || '';
  const activeEndpointId = activeSelection.endpoint_id || data?.active_endpoint_id || '';
  const accounts = (data?.accounts || [])
    .filter(a => accountClientKind(a) === selectedClientKind)
    .filter(a => accountClientSurface(a) === activeSurface)
    .map(a => {
      const routing = accountRouting(a);
      const runtime = a?.runtime_state || {};
      return {
        id: a.id,
        name: a.name,
        provider: a.provider,
        client_kind: accountClientKind(a),
        client_surface: accountClientSurface(a),
        auth_mode: a.auth_mode || 'api_key',
        active: a.id === activeId,
        active_endpoint: a.id === activeId ? activeEndpointId : '',
        upstream: cardUpstream(a),
        endpoint_kind: cardEndpointKind(a),
        routing_enabled: routing.enabled,
        routing_anchor_enabled: routing.anchor_enabled,
        routing_execution_enabled: routing.execution_enabled,
        routing_pool: routing.pool,
        routing_priority: routing.priority,
        routing_weight: routing.weight,
        runtime_status: runtime.status || 'active',
        runtime_next_retry_after: runtime.next_retry_after || null,
        runtime_success: runtime.success || 0,
        runtime_failed: runtime.failed || 0,
      };
    });
  return JSON.stringify({
    selectedClientKind,
    selectedClientSurface: selectedSurfaceForKind(selectedClientKind),
    activeId,
    activeEndpointId,
    routerStatus: data?.router_status || null,
    routerStatusScenarios: data?.router_status_scenarios || null,
    accounts,
  });
}

async function loadAccountsData() {
  try {
    const prevSignature = accountsListRenderSignature;
    const result = await invoke('list_accounts');
    accountsData = result;
    if (!clientProfiles.length) await loadClientProfiles();
    if (!providerPresets.length) await loadProviderPresets();
    if (!endpointTemplates.length) await loadEndpointTemplates();
    if (accountsView === 'list' && currentPanel === 'accounts') {
      const nextSignature = accountListRenderSignature(accountsData);
      const main = document.getElementById('mainContent');
      if (nextSignature !== prevSignature || !main || !main.querySelector('.accounts-grid')) {
        accountsListRenderSignature = nextSignature;
        renderMainContent();
      }
    }
  } catch (e) {
    showToast('加载账号失败: ' + e, 'error');
  }
}

async function loadClientProfiles() {
  try {
    clientProfiles = await invoke('get_client_profiles');
  } catch (e) {
    showToast('加载客户端分类失败: ' + e, 'error');
  }
}

async function loadProviderPresets() {
  try {
    providerPresets = await invoke('get_provider_presets');
  } catch (e) {
    showToast('加载供应商列表失败: ' + e, 'error');
  }
}

async function loadEndpointTemplates() {
  try {
    endpointTemplates = await invoke('get_endpoint_templates');
  } catch (e) {
    showToast('加载协议模板失败: ' + e, 'error');
  }
}

function getProviderKnownModels(provider) {
  const p = providerPresets.find(pp => pp.slug === provider);
  return p ? p.known_models : [];
}

function codexProviderModelMap(provider) {
  if (provider === 'deepseek') {
    return Object.fromEntries(CODEX_MODEL_LIST.map(model => [
      model,
      model === 'gpt-5.5' ? 'deepseek-v4-pro' : 'deepseek-v4-flash',
    ]));
  }
  if (provider === 'longcat') {
    return Object.fromEntries(CODEX_MODEL_LIST.map(model => [
      model,
      'LongCat-Flash-Chat',
    ]));
  }
  if (provider === 'mimo') {
    return Object.fromEntries(CODEX_MODEL_LIST.map(model => [
      model,
      model === 'codex-auto-review' ? MIMO_VISION_MODEL : MIMO_CODING_MODEL,
    ]));
  }
  return {};
}

function defaultApiKeyEnvForClient(account) {
  const kind = accountClientKind(account);
  if (kind === 'claude_code' && account.provider === 'deepseek') return 'ANTHROPIC_AUTH_TOKEN';
  if (kind === 'claude_code' && account.provider === 'kimi') return 'ANTHROPIC_AUTH_TOKEN';
  if (kind === 'claude_code' && account.provider === 'minimax') return 'ANTHROPIC_API_KEY';
  if (kind === 'claude_code' && account.provider === 'mimo') return 'ANTHROPIC_API_KEY';
  if (kind === 'claude_code' && account.provider === 'longcat') return 'ANTHROPIC_AUTH_TOKEN';
  if (kind === 'claude_code' && account.provider === 'qwen') return 'ANTHROPIC_API_KEY';
  if (kind === 'claude_code' && account.provider === 'glm') return 'ANTHROPIC_AUTH_TOKEN';
  if (kind === 'claude_code' || account.provider === 'anthropic') return 'ANTHROPIC_API_KEY';
  if (account.provider === 'deepseek') return 'DEEPSEEK_API_KEY';
  if (account.provider === 'openrouter') return 'OPENROUTER_API_KEY';
  if (account.provider === 'minimax') return 'MINIMAX_API_KEY';
  if (account.provider === 'mimo') return 'MIMO_API_KEY';
  if (account.provider === 'longcat') return 'LONGCAT_API_KEY';
  if (account.provider === 'qwen') return 'DASHSCOPE_API_KEY';
  if (kind === 'generic_client' || account.provider === 'openai') return 'OPENAI_API_KEY';
  return 'OPENAI_API_KEY';
}

function updateClientProviderDefaults() {
  const provider = document.getElementById('edit_client_provider')?.value || '';
  const preset = getProviderPreset(provider);
  if (!preset) return;
  const defaults = clientProviderDefaults(accountClientKind(editingAccount), provider, preset);
  const upstream = document.getElementById('edit_upstream');
  const model = document.getElementById('edit_default_model');
  const env = document.getElementById('edit_client_api_key_env');
  const authEnv = document.getElementById('edit_client_auth_env');
  const hasClientPreset = accountClientKind(editingAccount) === 'claude_code'
    && ['deepseek', 'kimi', 'minimax', 'mimo', 'longcat', 'qwen', 'glm'].includes(provider);
  if (upstream && (!upstream.value.trim() || hasClientPreset)) upstream.value = defaults.upstream || '';
  if (model && (!model.value.trim() || hasClientPreset) && defaults.default_model) {
    model.value = defaults.default_model;
  }
  if (env && (!env.value.trim() || env.value === 'OPENAI_API_KEY' || hasClientPreset)) {
    env.value = defaults.api_key_env;
  }
  if (authEnv) authEnv.value = defaults.api_key_env;
  if (editingAccount && !isCodexAccount(editingAccount)) {
    upstreamModels = [];
    syncEditingDraftFromForm();
    const clientMap = clientModelMap(editingAccount);
    const defaultModel = model?.value?.trim() || editingAccount.default_model || '';
    if (defaultModel && !clientMap.default) {
      editingAccount.client_options = editingAccount.client_options || {};
      editingAccount.client_options.model_map = { ...clientMap, default: defaultModel };
      editingAccount.default_model = defaultModel;
    }
    const rows = document.getElementById('clientModelMapRows');
    if (rows) rows.innerHTML = renderClientModelMappingRows(editingAccount);
  }
}

// ── 模型映射编辑辅助 ──

function removeModelMapRow(btn) {
  const row = btn.closest('.model-row');
  const input = row.querySelector('input');
  if (input) {
    const codex = input.dataset.codex;
    if (editingAccount && editingAccount.model_map) {
      delete editingAccount.model_map[codex];
    }
  }
  row.remove();
}

function syncEditingDraftFromForm(options = {}) {
  if (!editingAccount) return;
  const a = editingAccount;
  a.client_surface = accountClientSurface(a);
  a.client_options = a.client_options || {};
  a.client_options.client_surface = a.client_surface;
  const nameInput = document.getElementById('edit_name');
  if (nameInput) a.name = nameInput.value.trim() || a.name;
  const keyInput = document.getElementById('edit_api_key');
  if (keyInput) {
    const nextKey = keyInput.value.trim();
    if (nextKey || !hasStoredPrimaryApiKey(a)) a.api_key = nextKey;
  }

  if (!isCodexAccount(a)) {
    const provider = document.getElementById('edit_client_provider');
    if (provider) a.provider = provider.value || a.provider;
    const upstream = document.getElementById('edit_upstream');
    if (upstream) a.upstream = upstream.value.trim();
    const defaultModel = document.getElementById('edit_default_model');
    if (defaultModel) a.default_model = defaultModel.value.trim();
    const clientModels = collectClientModelMap();
    if (Object.keys(clientModels).length) {
      if (!clientModels.default && a.default_model) clientModels.default = a.default_model;
      a.client_options.model_map = clientModels;
      a.default_model = clientModels.default || a.default_model;
      if (defaultModel) defaultModel.value = a.default_model;
    } else {
      delete a.client_options.model_map;
    }
    const configPath = document.getElementById('edit_client_config_path');
    if (configPath) {
      const value = configPath.value.trim();
      if (value) a.client_options.config_path = value;
      else delete a.client_options.config_path;
    }
    const apiKeyEnv = document.getElementById('edit_client_api_key_env');
    if (apiKeyEnv) {
      const value = apiKeyEnv.value.trim();
      if (value) a.client_options.api_key_env = value;
      else delete a.client_options.api_key_env;
    }
    const authEnv = document.getElementById('edit_client_auth_env');
    if (authEnv) {
      a.client_options.auth_env = authEnv.value || a.client_options.api_key_env || 'ANTHROPIC_API_KEY';
      a.client_options.api_key_env = a.client_options.auth_env;
    }
    const proxyToggle = document.getElementById('edit_client_proxy_toggle');
    if (proxyToggle) {
      a.client_options.proxy_recording_enabled = proxyToggle.classList.contains('on');
    }
    a.translate_enabled = false;
    a.endpoints = [];
    return;
  }

  const endpoints = ensureAccountEndpoints(a);
  const ep = currentEndpoint(a) || endpoints[0];
  if (!ep) return;

  const endpointName = document.getElementById('edit_endpoint_name');
  if (endpointName) ep.name = endpointName.value.trim() || ep.name || '默认配置';
  const endpointKind = document.getElementById('edit_endpoint_kind');
  if (endpointKind) ep.kind = endpointKind.value;
  const upstream = document.getElementById('edit_upstream');
  if (upstream) {
    ep.base_url = upstream.value.trim();
    a.upstream = ep.base_url || a.upstream;
  }
  const endpointPath = document.getElementById('edit_endpoint_path');
  if (endpointPath) ep.path = endpointPath.value.trim();
  const balanceUrl = document.getElementById('edit_balance_url');
  if (balanceUrl) {
    ep.balance_url = balanceUrl.value.trim();
    a.balance_url = ep.balance_url;
  }
  const routingAnchorEnabled = document.getElementById('edit_routing_anchor_enabled');
  const routingExecutionEnabled = document.getElementById('edit_routing_execution_enabled');
  const routingPool = document.getElementById('edit_routing_pool');
  const routingPriority = document.getElementById('edit_routing_priority');
  const routingWeight = document.getElementById('edit_routing_weight');
  if (routingAnchorEnabled || routingExecutionEnabled || routingPool || routingPriority || routingWeight) {
    if (!a.client_options) a.client_options = {};
    const routing = accountRouting(a);
    routing.anchor_enabled = routingAnchorEnabled ? routingAnchorEnabled.checked : routing.anchor_enabled;
    routing.execution_enabled = routingExecutionEnabled ? routingExecutionEnabled.checked : routing.execution_enabled;
    routing.enabled = Boolean(routing.anchor_enabled || routing.execution_enabled);
    routing.pool = routingPool ? (routingPool.value.trim() || 'codex-official') : routing.pool;
    routing.priority = routingPriority ? Number(routingPriority.value || 0) : routing.priority;
    routing.weight = routingWeight ? Math.max(1, Math.min(100, Number(routingWeight.value || 1))) : routing.weight;
    a.routing = routing;
    a.client_options.routing = {
      enabled: routing.enabled,
      disabled: !routing.enabled,
      anchor_enabled: routing.anchor_enabled,
      execution_enabled: routing.execution_enabled,
      pool: routing.pool,
      priority: routing.priority,
      weight: routing.weight,
    };
  }

  if (!ep.vision) ep.vision = {};
  const visionMode = document.getElementById('edit_vision_mode');
  if (visionMode) {
    ep.vision.mode = visionMode.value || 'native';
    a.vision_enabled = ep.vision.mode !== 'off';
  }
  const visionUpstream = document.getElementById('edit_vision_upstream');
  if (visionUpstream) { ep.vision.base_url = visionUpstream.value.trim(); a.vision_upstream = ep.vision.base_url; }
  const visionKey = document.getElementById('edit_vision_api_key');
  if (visionKey) {
    const nextVisionKey = visionKey.value.trim();
    if (nextVisionKey || !hasStoredVisionApiKey(a, ep)) {
      ep.vision.api_key = nextVisionKey;
      a.vision_api_key = ep.vision.api_key;
    }
  }
  const visionModel = document.getElementById('edit_vision_model');
  if (visionModel) { ep.vision.model = visionModel.value.trim(); a.vision_model = ep.vision.model; }
  const visionPath = document.getElementById('edit_vision_endpoint');
  if (visionPath) { ep.vision.path = visionPath.value.trim() || 'v1/coding_plan/vlm'; a.vision_endpoint = ep.vision.path; }
  ep.vision.adapter_id = document.getElementById('edit_vision_adapter')?.value || ep.vision.adapter_id || 'minimax_coding_plan_vlm';
  ep.vision.glue_strategy = document.getElementById('edit_glue_strategy')?.value || ep.vision.glue_strategy || 'final_answer';
  ep.vision.unsupported_image_policy = document.getElementById('edit_unsupported_image_policy')?.value || ep.vision.unsupported_image_policy || 'reject';

  if (!endpointKindUsesModelMapping(ep.kind) && !options.preserveHiddenResponses) {
    ep.model_map = {};
    ep.model_profiles = {};
  } else if (endpointKindUsesModelMapping(ep.kind)) {
    const modelMapRows = document.getElementById('modelMapRows');
    if (modelMapRows) {
      ep.model_map = collectModelMap();
      ep.model_profiles = collectModelProfiles();
    }
  }
  a.model_map = ep.model_map;

  const cwEnabled = document.getElementById('edit_cw_enabled');
  if (cwEnabled) {
    ep.context_window_override = cwEnabled.checked
      ? parseOptionalInteger(document.getElementById('edit_cw_size')?.value)
      : null;
    a.context_window_override = ep.context_window_override;
  }

  const reasoningEnabled = document.getElementById('edit_reasoning_enabled');
  if (reasoningEnabled) {
    ep.reasoning_effort_override = reasoningEnabled.checked
      ? (document.getElementById('edit_reasoning_effort')?.value || null)
      : null;
    ep.thinking_tokens = reasoningEnabled.checked
      ? parseOptionalInteger(document.getElementById('edit_thinking_tokens')?.value)
      : null;
    a.reasoning_effort_override = ep.reasoning_effort_override;
    a.thinking_tokens = ep.thinking_tokens;
  }

  const fastEnabled = document.getElementById('edit_fast_enabled');
  if (fastEnabled) {
    ep.fast_mode_enabled = fastEnabled.checked;
    ep.fast_service_tier = fastEnabled.checked
      ? (document.getElementById('edit_fast_service_tier')?.value.trim() || 'priority')
      : 'priority';
  }

  const headersText = document.getElementById('edit_custom_headers');
  if (headersText) {
    ep.custom_headers = {};
    const raw = headersText.value.trim();
    if (raw) {
      raw.split('\n').forEach(line => {
        const colonIdx = line.indexOf(':');
        if (colonIdx > 0) {
          const k = line.substring(0, colonIdx).trim();
          const v = line.substring(colonIdx + 1).trim();
          if (k && v) ep.custom_headers[k] = v;
        }
      });
    }
    a.custom_headers = ep.custom_headers;
  }

  const timeoutInput = document.getElementById('edit_request_timeout');
  if (timeoutInput) {
    ep.request_timeout_secs = parseOptionalInteger(timeoutInput.value);
    a.request_timeout_secs = ep.request_timeout_secs;
  }
  const retriesInput = document.getElementById('edit_max_retries');
  if (retriesInput) {
    ep.max_retries = parseOptionalInteger(retriesInput.value);
    a.max_retries = ep.max_retries;
  }
  if (isResponsesDirectFormAccount(a, ep) && !options.preserveHiddenResponses) {
    normalizeResponsesDirectAccount(a, ep);
  }
  a.translate_enabled = ep.kind === 'open_ai_chat' || ep.kind === 'custom_chat';
}

function showSuggestions(input) {
  const wrap = input.closest('.model-autocomplete');
  if (!wrap) return;
  const dropdown = wrap.querySelector('.model-suggestions');
  if (!dropdown) return;
  const suggestions = JSON.parse(dropdown.dataset.suggestions || '[]');
  renderSuggestionsDropdown(input, dropdown, suggestions);
  dropdown.style.display = 'block';
}

function filterSuggestions(input) {
  const wrap = input.closest('.model-autocomplete');
  if (!wrap) return;
  const dropdown = wrap.querySelector('.model-suggestions');
  if (!dropdown) return;
  const suggestions = JSON.parse(dropdown.dataset.suggestions || '[]');
  const q = input.value.toLowerCase();
  const filtered = suggestions.filter(s => s.toLowerCase().includes(q));
  renderSuggestionsDropdown(input, dropdown, filtered);
  dropdown.style.display = 'block';
}

function hideSuggestions(input) {
  setTimeout(() => {
    const wrap = input.closest('.model-autocomplete');
    if (!wrap) return;
    const dropdown = wrap.querySelector('.model-suggestions');
    if (dropdown) dropdown.style.display = 'none';
  }, 150);
}

function renderSuggestionsDropdown(input, dropdown, items) {
  if (items.length === 0) { dropdown.innerHTML = ''; return; }
  dropdown.innerHTML = items.map(s =>
    `<div class="suggestion-item" onmousedown="event.preventDefault();selectSuggestion(this)">${esc(s)}</div>`
  ).join('');
}

function selectSuggestion(item) {
  const wrap = item.closest('.model-autocomplete');
  if (!wrap) return;
  const input = wrap.querySelector('input');
  if (input) {
    input.value = item.textContent;
    syncModelVisionTarget(input);
    input.focus();
  }
  const dropdown = wrap.querySelector('.model-suggestions');
  if (dropdown) dropdown.style.display = 'none';
}

function addModelRow(containerId, knownModelsJson) {
  const knownModels = JSON.parse(knownModelsJson);
  const suggestionsJson = escAttr(JSON.stringify(knownModels));

  const container = document.getElementById(containerId);
  const row = document.createElement('div');
  row.className = 'model-row model-map-row removable';
  row.innerHTML = `<div class="model-label codex custom-codex-cell"><input type="text" class="custom-codex-model" placeholder="Codex 模型名"></div>
    <div class="model-autocomplete model-upstream-cell">
      <input type="text" placeholder="上游模型名" autocomplete="off"
        onchange="syncModelVisionTarget(this)"
        onfocus="showSuggestions(this)" oninput="filterSuggestions(this)" onblur="hideSuggestions(this)">
      <div class="model-suggestions" style="display:none;" data-suggestions="${suggestionsJson}"></div>
    </div>
    <div class="model-vision-cell">
      ${renderModelVisionSegments('', document.getElementById('edit_vision_mode')?.value || 'native')}
    </div>
    <button class="model-remove" onclick="removeModelMapRow(this)" title="移除">✕</button>`;
  container.appendChild(row);
}

function setModelVisionMode(button, mode) {
  const wrap = button.closest('.model-vision-segments');
  if (!wrap) return;
  wrap.querySelectorAll('button').forEach(item => {
    item.classList.toggle('active', item === button);
  });
  wrap.dataset.mode = normalizedVisionMode(mode);
}

function syncModelVisionTarget(input) {
  const row = input.closest('.model-row');
  const wrap = row?.querySelector('.model-vision-segments');
  if (!wrap) return;
  wrap.dataset.model = input.value.trim();
}

function collectModelMap() {
  const result = {};
  const rows = document.querySelectorAll('#modelMapRows .model-row');
  rows.forEach(row => {
    const inputs = row.querySelectorAll('input[type="text"]');
    let codex, upstream;
    if (inputs.length === 2) {
      // 自定义行：左输入框是 codex 模型名，右是上游
      codex = inputs[0].value.trim();
      upstream = inputs[1].value.trim();
    } else if (inputs.length === 1) {
      codex = inputs[0].dataset.codex;
      upstream = inputs[0].value.trim();
    }
    if (codex && upstream) result[codex] = upstream;
  });
  return result;
}

function collectModelProfiles() {
  const result = {};
  const rows = document.querySelectorAll('#modelMapRows .model-row');
  rows.forEach(row => {
    const inputs = row.querySelectorAll('input[type="text"]');
    const upstreamInput = inputs.length === 2 ? inputs[1] : inputs[0];
    const model = upstreamInput?.value?.trim() || upstreamInput?.dataset?.codex || row.querySelector('.custom-codex-model')?.value?.trim();
    const mode = row.querySelector('.model-vision-segments')?.dataset.mode || 'native';
    if (model) {
      result[model] = { vision_mode: mode };
    }
  });
  return result;
}

// ── CRUD 操作 ──

async function startOAuthAccountLogin(provider, mode = 'browser') {
  try {
    const result = await invoke('start_oauth_account_login', {
      provider,
      clientKind: selectedClientKind,
      client_kind: selectedClientKind,
      clientSurface: selectedSurfaceForKind(selectedClientKind),
      client_surface: selectedSurfaceForKind(selectedClientKind),
      mode,
    });
    oauthLoginState = { ...result, status: 'pending' };
    accountsView = 'add';
    renderMainContent();
    startOAuthLoginPolling();
  } catch (e) {
    showToast('启动官方登录失败: ' + e, 'error');
  }
}

function startOAuthLoginPolling() {
  stopOAuthLoginPolling();
  oauthLoginPollTimer = setInterval(() => {
    pollOAuthAccountLogin();
  }, Math.max(2000, Number(oauthLoginState?.poll_interval_secs || 3) * 1000));
}

function stopOAuthLoginPolling() {
  if (oauthLoginPollTimer) {
    clearInterval(oauthLoginPollTimer);
    oauthLoginPollTimer = null;
  }
}

async function pollOAuthAccountLogin() {
  if (!oauthLoginState?.state) return;
  try {
    const result = await invoke('poll_oauth_account_login', { state: oauthLoginState.state });
    oauthLoginState = { ...oauthLoginState, ...result };
    if (result.status === 'success') {
      stopOAuthLoginPolling();
      showToast('官方账号登录成功', 'success');
      await loadAccountsData();
      if (result.account) {
        selectedClientKind = accountClientKind(result.account);
        selectedClientSurface = accountClientSurface(result.account);
      }
      accountsView = 'list';
      oauthLoginState = null;
      renderMainContent();
      return;
    }
    if (result.status === 'error' || result.status === 'expired') {
      stopOAuthLoginPolling();
    }
    renderMainContent();
  } catch (e) {
    oauthLoginState = { ...(oauthLoginState || {}), status: 'error', message: String(e) };
    stopOAuthLoginPolling();
    renderMainContent();
  }
}

async function cancelOAuthAccountLogin() {
  const state = oauthLoginState?.state;
  stopOAuthLoginPolling();
  oauthLoginState = null;
  if (state) {
    try {
      await invoke('cancel_oauth_account_login', { state });
    } catch (_) {}
  }
  accountsView = 'add';
  renderMainContent();
}

function openOAuthLoginUrl() {
  const url = oauthLoginState?.verification_url || oauthLoginState?.url;
  if (!url) return;
  window.open(url, '_blank');
}

async function copyOAuthLoginUrl() {
  const url = oauthLoginState?.verification_url || oauthLoginState?.url;
  if (!url) return;
  try {
    await navigator.clipboard.writeText(url);
    showToast('登录链接已复制', 'success');
  } catch (e) {
    showToast('复制失败: ' + e, 'error');
  }
}

async function switchAccount(id) {
  try {
    await invoke('switch_account', { id });
    accountsData.active_id = id;
    showToast('已切换活跃账号', 'success');
    await loadAccountsData();
  } catch (e) {
    showToast('切换账号失败: ' + e, 'error');
  }
}

async function clearAccountCooldown(id) {
  try {
    await invoke('clear_account_cooldown', { id });
    showToast('已清除账号冷却', 'success');
    await loadAccountsData();
    refreshEditingAccountAfterManagement(id);
  } catch (e) {
    showToast('清除冷却失败: ' + e, 'error');
  }
}

async function resetAccountRuntime(id) {
  if (!await showConfirm('确定要重置此账号的运行态统计吗？')) return;
  try {
    await invoke('reset_account_runtime_state', { id });
    showToast('运行态已重置', 'success');
    await loadAccountsData();
    refreshEditingAccountAfterManagement(id);
  } catch (e) {
    showToast('重置运行态失败: ' + e, 'error');
  }
}

async function toggleAccountRouting(id, mode = 'execution') {
  const account = (accountsData.accounts || []).find(item => item.id === id);
  const routing = accountRouting(account);
  const anchorMode = mode === 'anchor';
  const nextAnchor = anchorMode ? !routing.anchor_enabled : routing.anchor_enabled;
  const nextExecution = anchorMode ? routing.execution_enabled : !routing.execution_enabled;
  try {
    await invoke('set_account_routing', {
      id,
      anchorEnabled: nextAnchor,
      executionEnabled: nextExecution,
    });
    showToast(anchorMode
      ? (nextAnchor ? '登录态锚点已启用' : '登录态锚点已停用')
      : (nextExecution ? '执行候选已启用' : '执行候选已停用'), 'success');
    await loadAccountsData();
    refreshEditingAccountAfterManagement(id);
  } catch (e) {
    showToast('更新路由设置失败: ' + e, 'error');
  }
}

function refreshEditingAccountAfterManagement(id) {
  if (accountsView !== 'edit' || !editingAccount || editingAccount.id !== id) return;
  const updated = (accountsData.accounts || []).find(account => account.id === id);
  if (!updated) return;
  editingAccount = JSON.parse(JSON.stringify(updated));
  renderMainContent();
}

async function applyAccountRoutingFromDetail(id) {
  if (!editingAccount || !id) return;
  syncEditingDraftFromForm();
  const routing = accountRouting(editingAccount);
  try {
    const result = await invoke('set_account_routing', {
      id,
      anchorEnabled: routing.anchor_enabled,
      executionEnabled: routing.execution_enabled,
      pool: routing.pool,
      priority: routing.priority,
      weight: routing.weight,
    });
    showToast('路由设置已应用', 'success');
    await loadAccountsData();
    editingAccount = accountsData.accounts.find(account => account.id === result.id) || result;
    if (editingAccount) editingAccount = JSON.parse(JSON.stringify(editingAccount));
    renderMainContent();
  } catch (e) {
    showToast('应用账号池失败: ' + e, 'error');
  }
}

function addAccount(provider, clientKind, clientSurface) {
  const kind = normalizeClientKind(clientKind || selectedClientKind || 'codex');
  const surface = clientKindSupportsSurface(kind) ? normalizeClientSurface(clientSurface || selectedSurfaceForKind(kind)) : 'cli';
  const preset = providerPresets.find(p => p.slug === provider);
  if (!preset) return;
  const clientProfile = getClientProfile(kind);
  const defaults = clientProviderDefaults(kind, provider, preset);
  const codexModelMap = kind === 'codex' ? codexProviderModelMap(provider) : {};
  editingAccount = {
    id: '',
    name: kind === 'codex' ? `${preset.label} ${clientSurfaceLabel(surface)} 账号` : `${clientSurfaceTitle(kind, surface)} · ${preset.label}`,
    provider: provider,
    client_kind: kind,
    client_surface: surface,
    target: kind,
    upstream: kind === 'codex' ? preset.default_upstream : defaults.upstream,
    api_key: '',
    default_model: kind === 'codex' ? '' : (defaults.default_model || clientProfile?.default_model || ''),
    client_options: kind === 'codex' ? { client_surface: surface } : {
      client_surface: surface,
      api_key_env: defaults.api_key_env,
      ...(kind === 'claude_code' ? { auth_env: defaults.api_key_env } : {}),
      model_map: {
        default: defaults.default_model || clientProfile?.default_model || '',
      },
    },
    last_applied_at: null,
    last_check: null,
    model_map: codexModelMap,
    vision_enabled: false,
    vision_upstream: '',
    vision_api_key: '',
    vision_model: '',
    vision_endpoint: '',
    from_codex_config: false,
    balance_url: '',
    context_window_override: null,
    reasoning_effort_override: null,
    thinking_tokens: null,
    custom_headers: {},
    provider_options: preset.provider_options || { capability_labels: preset.capability_labels || [] },
    request_timeout_secs: null,
    max_retries: null,
    translate_enabled: true,
    capability_enabled: false,
    capability_account_id: null,
    dev_pipeline_enabled: false,
    dev_pipeline_trigger_mode: 'manual',
    dev_pipeline_command: '/dev-pipeline',
    dev_pipeline_architect_account_id: null,
    dev_pipeline_implementer_account_id: null,
    dev_pipeline_reviewer_account_id: null,
    dev_pipeline_tool_mode: 'controlled_tools',
    dev_pipeline_max_iterations: 3,
    dev_pipeline_show_trace: false,
    dev_pipeline_architect_instruction: '',
    dev_pipeline_implementer_instruction: '',
    dev_pipeline_reviewer_instruction: '',
  };
  if (kind === 'codex') {
    editingAccount.endpoints = [createEndpointFromTemplate(providerDefaultTemplate(provider), editingAccount)];
    applyProviderSpecificEndpointDefaults(editingAccount);
    editingAccount._editing_endpoint_id = editingAccount.endpoints[0].id;
    normalizeResponsesDirectAccount(editingAccount);
  } else {
    editingAccount.translate_enabled = false;
    editingAccount.endpoints = [];
  }
  accountsView = 'edit';
  renderMainContent();
}

function editAccount(id, expectedKind) {
  const normalizedExpected = expectedKind ? normalizeClientKind(expectedKind) : '';
  editingAccount = accountsData.accounts.find(a =>
    a.id === id && (!normalizedExpected || accountClientKind(a) === normalizedExpected)
  );
  if (!editingAccount && normalizedExpected) {
    const visibleAccounts = (accountsData.accounts || []).filter(a =>
      accountClientKind(a) === normalizedExpected
      && accountClientSurface(a) === selectedSurfaceForKind(normalizedExpected)
    );
    if (visibleAccounts.length === 1) editingAccount = visibleAccounts[0];
  }
  if (!editingAccount) {
    showToast(normalizedExpected ? `未找到 ${CLIENT_KIND_LABELS[normalizedExpected] || normalizedExpected} 账号` : '账号不存在', 'error');
    return;
  }
  // 深拷贝避免直接修改原数据
  editingAccount = JSON.parse(JSON.stringify(editingAccount));
  accountsView = 'edit';
  renderMainContent();
}

function serializeAccountForBackend(account) {
  const payload = JSON.parse(JSON.stringify(account || {}));
  if (Object.prototype.hasOwnProperty.call(payload, 'client_kind')) {
    delete payload.target;
  }
  delete payload._editing_endpoint_id;
  return JSON.stringify(payload);
}

function addAccountInvokeArgs(account) {
  const clientKind = accountClientKind(account);
  const clientSurface = accountClientSurface(account);
  return {
    provider: account?.provider || 'custom',
    accountJson: serializeAccountForBackend(account),
    clientKind,
    client_kind: clientKind,
    clientSurface,
    client_surface: clientSurface,
  };
}

async function saveAccount(options = {}) {
  if (!editingAccount) return;
  if (options instanceof Event) options = {};
  const a = editingAccount;
  const isNewAccount = !a.id;

  syncEditingDraftFromForm();
  const preset = getProviderPreset(a.provider);
  if (!a.provider_options || Object.keys(a.provider_options).length === 0) {
    a.provider_options = { capability_labels: preset ? (preset.capability_labels || []) : [] };
  }
  const capabilityEnabled = document.getElementById('edit_capability_enabled');
  if (capabilityEnabled) a.capability_enabled = capabilityEnabled.checked;
  const capabilityAccount = document.getElementById('edit_capability_account_id');
  a.capability_account_id = capabilityEnabled && capabilityEnabled.checked && capabilityAccount
    ? (capabilityAccount.value || null)
    : null;
  if (a.capability_enabled) {
    if (!a.capability_account_id) {
      showToast('请选择能力补全账号', 'error');
      return;
    }
    if (a.id && a.capability_account_id === a.id) {
      showToast('能力补全账号不能选择当前账号', 'error');
      return;
    }
  }
  const devPipelineEnabled = document.getElementById('edit_dev_pipeline_enabled');
  if (devPipelineEnabled) a.dev_pipeline_enabled = devPipelineEnabled.checked;
  const devTriggerMode = document.getElementById('edit_dev_pipeline_trigger_mode');
  a.dev_pipeline_trigger_mode = devTriggerMode ? devTriggerMode.value : (a.dev_pipeline_trigger_mode || 'manual');
  const devCommand = document.getElementById('edit_dev_pipeline_command');
  a.dev_pipeline_command = devCommand ? (devCommand.value.trim() || '/dev-pipeline') : (a.dev_pipeline_command || '/dev-pipeline');
  a.dev_pipeline_architect_account_id = document.getElementById('edit_dev_pipeline_architect_account_id')?.value || null;
  a.dev_pipeline_implementer_account_id = document.getElementById('edit_dev_pipeline_implementer_account_id')?.value || null;
  a.dev_pipeline_reviewer_account_id = document.getElementById('edit_dev_pipeline_reviewer_account_id')?.value || null;
  a.dev_pipeline_tool_mode = document.getElementById('edit_dev_pipeline_tool_mode')?.value || (a.dev_pipeline_tool_mode || 'controlled_tools');
  a.dev_pipeline_max_iterations = Math.max(1, Math.min(10, Number(document.getElementById('edit_dev_pipeline_max_iterations')?.value || a.dev_pipeline_max_iterations || 3)));
  a.dev_pipeline_show_trace = Boolean(document.getElementById('edit_dev_pipeline_show_trace')?.checked);
  a.dev_pipeline_architect_instruction = document.getElementById('edit_dev_pipeline_architect_instruction')?.value || '';
  a.dev_pipeline_implementer_instruction = document.getElementById('edit_dev_pipeline_implementer_instruction')?.value || '';
  a.dev_pipeline_reviewer_instruction = document.getElementById('edit_dev_pipeline_reviewer_instruction')?.value || '';
  if (a.dev_pipeline_enabled && a.dev_pipeline_trigger_mode === 'manual' && !a.dev_pipeline_command.trim()) {
    showToast('开发协作编排触发命令不能为空', 'error');
    return;
  }
  const ep = currentEndpoint(a);
  if (!isCodexAccount(a)) {
    a.translate_enabled = false;
    a.endpoints = [];
    a.model_map = {};
    a.vision_enabled = false;
  } else if (ep) {
    ep.name = endpointKindLabel(ep.kind);
    a.upstream = ep.base_url || a.upstream;
    a.balance_url = ep.balance_url || '';
    a.model_map = ep.model_map || {};
    a.context_window_override = ep.context_window_override ?? null;
    a.reasoning_effort_override = ep.reasoning_effort_override ?? null;
    a.thinking_tokens = ep.thinking_tokens ?? null;
    a.custom_headers = ep.custom_headers || {};
    a.request_timeout_secs = ep.request_timeout_secs ?? null;
    a.max_retries = ep.max_retries ?? null;
    a.translate_enabled = ep.kind === 'open_ai_chat' || ep.kind === 'custom_chat';
    a._editing_endpoint_id = ep.id;
  }
  const editingEndpointId = isCodexAccount(a) ? (ep?.id || selectedEndpointId(a)) : null;
  a.updated_at = Math.floor(Date.now() / 1000);

  try {
    let result;
    if (isNewAccount) {
      // 新账号
      result = await invoke('add_account', addAccountInvokeArgs(a));
      if (!options.silent) showToast('账号已创建', 'success');
    } else {
      result = await invoke('update_account', { accountJson: serializeAccountForBackend(a) });
      if (!options.silent) showToast('账号已保存', 'success');
    }
    await loadAccountsData();
    editingAccount = accountsData.accounts.find(ac => ac.id === result.id);
    if (editingAccount) {
      editingAccount = JSON.parse(JSON.stringify(editingAccount));
      editingAccount._editing_endpoint_id = editingEndpointId;
    }
    if (isNewAccount && !options.stay) {
      accountsView = 'list';
      editingAccount = null;
    }
    if (!options.stay) renderMainContent();
    return result;
  } catch (e) {
    if (!options.silent) showToast('保存账号失败: ' + e, 'error');
    if (options.silent) throw e;
    return null;
  }
}

async function deleteAccount(id) {
  if (!await showConfirm('确定要删除此账号吗？')) return;
  try {
    await invoke('delete_account', { id });
    showToast('账号已删除', 'success');
    accountsView = 'list'; editingAccount = null;
    await loadAccountsData();
    if (currentPanel === 'accounts') {
      accountsListRenderSignature = accountListRenderSignature(accountsData);
      renderMainContent();
    }
  } catch (e) {
    showToast('删除账号失败: ' + e, 'error');
  }
}

async function importFromCodex() {
  try {
    await invoke('import_codex_config');
    showToast('Codex 配置导入成功', 'success');
    await loadAccountsData();
  } catch (e) {
    showToast('导入失败: ' + e, 'error');
  }
}

async function importAuthJsonAccounts() {
  const input = document.createElement('input');
  input.type = 'file';
  input.accept = '.json,application/json';
  input.multiple = true;
  input.style.display = 'none';
  document.body.appendChild(input);
  input.onchange = async () => {
    const files = Array.from(input.files || []);
    input.remove();
    if (!files.length) return;
    try {
      const payload = await Promise.all(files.map(async file => ({
        name: file.name,
        content: await file.text(),
      })));
      const result = await invoke('import_auth_json_accounts', {
        authFilesJson: JSON.stringify(payload),
        clientSurface: selectedSurfaceForKind('codex'),
        client_surface: selectedSurfaceForKind('codex'),
      });
      const failedCount = Array.isArray(result.failed) ? result.failed.length : 0;
      showToast((result.message || '认证 JSON 导入完成') + (failedCount ? `，失败 ${failedCount} 个` : ''), failedCount ? 'info' : 'success');
      await loadAccountsData();
      const importedAccounts = Array.isArray(result.accounts) ? result.accounts : [];
      const preferred = importedAccounts[0];
      if (preferred) {
        selectedClientKind = accountClientKind(preferred);
        selectedClientSurface = accountClientSurface(preferred);
      }
      accountsView = 'list';
      if (currentPanel === 'accounts') renderMainContent();
      else if (currentPanel === 'config') renderPanel('config');
    } catch (e) {
      showToast('认证 JSON 导入失败: ' + e, 'error');
    }
  };
  input.oncancel = () => input.remove();
  input.click();
}

async function scanClientAccounts() {
  try {
    const previousKind = selectedClientKind;
    const previousSurface = selectedSurfaceForKind(previousKind);
    const result = await invoke('import_client_accounts');
    showToast(result.message || '客户端扫描完成', 'success');
    await loadAccountsData();
    if (Number(result.imported || 0) > 0) {
      const importedAccounts = Array.isArray(result.accounts) ? result.accounts : [];
      const hasPreviousKind = (accountsData.accounts || []).some(account =>
        accountClientKind(account) === previousKind
        && accountClientSurface(account) === previousSurface
      );
      const preferred = importedAccounts.find(account =>
        accountClientKind(account) === previousKind
        && accountClientSurface(account) === previousSurface
      ) || (hasPreviousKind ? { client_kind: previousKind, client_surface: previousSurface } : importedAccounts[0]);
      if (preferred) {
        selectedClientKind = accountClientKind(preferred);
        selectedClientSurface = accountClientSurface(preferred);
      }
      accountsView = 'list';
      if (currentPanel === 'accounts') renderMainContent();
      else if (currentPanel === 'config') renderPanel('config');
    }
  } catch (e) {
    showToast('客户端扫描失败: ' + e, 'error');
  }
}

function renderClientReport(report) {
  if (!report) return '';
  const diagnostics = Array.isArray(report.diagnostics) ? report.diagnostics : [];
  const diff = Array.isArray(report.diff) ? report.diff : [];
  const changedFiles = Array.isArray(report.changed_files) ? report.changed_files : [];
  const backupPaths = Array.isArray(report.backup_paths) ? report.backup_paths : (report.backup_path ? [report.backup_path] : []);
  const levelClass = report.ok ? 'status-ok' : 'status-error';
  const envPath = report.env_path ? `<div class="client-report-path">${esc(report.env_path)}</div>` : '';
  const meta = [
    report.risk_level ? `风险 ${report.risk_level}` : '',
    report.schema_ok === false ? 'Schema 异常' : 'Schema 正常',
    report.recoverable === false ? '不可自动恢复' : '可恢复',
    report.secret_source ? `密钥: ${report.secret_source}` : '',
  ].filter(Boolean);
  return `<div class="client-report">
    <div class="${levelClass}">${esc(report.message || (report.ok ? '检查通过' : '检查失败'))}</div>
    ${meta.length ? `<div class="client-report-meta">${meta.map(item => `<span>${esc(item)}</span>`).join('')}</div>` : ''}
    ${report.config_path ? `<div class="client-report-path">${esc(report.config_path)}</div>` : ''}
    ${envPath}
    ${changedFiles.length ? `<div class="client-report-files"><strong>变更文件</strong>${changedFiles.map(path => `<span title="${escAttr(path)}">${esc(trunc(path, 72))}</span>`).join('')}</div>` : ''}
    ${backupPaths.length ? `<div class="client-report-files"><strong>备份</strong>${backupPaths.map(path => `<span title="${escAttr(path)}">${esc(trunc(path, 72))}</span>`).join('')}</div>` : ''}
    ${diagnostics.length ? `<div class="client-report-list">${diagnostics.map(d => `<div class="client-report-item ${escAttr(d.level || 'info')}">${esc(d.message || d.code || '')}</div>`).join('')}</div>` : ''}
    ${diff.length ? `<div class="client-report-diff">${diff.map(line => `<code>${esc(line)}</code>`).join('')}</div>` : ''}
  </div>`;
}

function secretCopyButton(secretKind) {
  const label = secretKind === 'vision_api_key' ? '复制已保存视觉 API Key' : '复制已保存 API Key';
  return `<button type="button" class="secret-copy-btn" onclick="copyEditingAccountSecret('${escAttr(secretKind)}')" title="${escAttr(label)}" aria-label="${escAttr(label)}">
    <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="8" y="8" width="11" height="11" rx="2"></rect><path d="M5 15H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h8a2 2 0 0 1 2 2v1"></path></svg>
  </button>`;
}

function showClientApplyConfirm(report) {
  return new Promise(resolve => {
    const existing = document.getElementById('clientApplyConfirmModal');
    if (existing) existing.remove();
    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';
    overlay.id = 'clientApplyConfirmModal';
    overlay.innerHTML = `<div class="modal-box client-apply-modal">
      <div class="modal-header">
        <h3>写入前预检</h3>
        <button class="modal-close" id="clientApplyCloseBtn" type="button">✕</button>
      </div>
      <div class="modal-body">
        <div class="client-apply-modal-intro ${report.ok ? 'status-ok' : 'status-error'}">
          ${esc(report.ok ? '预检通过，请确认脱敏 diff 后写入外部客户端配置。' : '预检未通过，已阻止写入外部客户端配置。')}
        </div>
        ${renderClientReport(report)}
      </div>
      <div class="client-apply-modal-actions">
        <button class="btn btn-primary" id="clientApplyOkBtn" type="button" ${report.ok ? '' : 'disabled'}>确认写入</button>
        <button class="btn btn-ghost" id="clientApplyCancelBtn" type="button">取消</button>
      </div>
    </div>`;
    document.body.appendChild(overlay);
    function cleanup(value) {
      overlay.remove();
      resolve(value);
    }
    document.getElementById('clientApplyCloseBtn').onclick = () => cleanup(false);
    document.getElementById('clientApplyCancelBtn').onclick = () => cleanup(false);
    document.getElementById('clientApplyOkBtn').onclick = () => cleanup(Boolean(report.ok));
    overlay.addEventListener('click', e => {
      if (e.target === overlay) cleanup(false);
    });
  });
}

async function fetchClientEventsForDetail(id) {
  const el = document.getElementById('clientEventLog');
  if (!el || !id) return;
  el.innerHTML = '<span class="status-muted">加载中...</span>';
  try {
    const events = await invoke('get_account_events', { accountId: id, limit: 12 });
    el.innerHTML = renderClientEventLog(Array.isArray(events) ? events : []);
  } catch (e) {
    el.innerHTML = `<span class="status-error">${esc(String(e))}</span>`;
  }
}

function renderClientEventLog(events) {
  if (!events.length) return '<span class="status-muted">暂无配置事件</span>';
  return events.map(event => {
    const ok = event.ok !== false;
    const action = accountEventActionLabel(event.action);
    const time = formatTimeShort(event.ts);
    const message = event.message || '';
    const details = event.details || {};
    const backupPaths = Array.isArray(details.backup_paths) ? details.backup_paths : [];
    const path = details.source_path || details.config_path || details.backup_path || backupPaths[0] || '';
    const source = path ? `<span title="${escAttr(path)}">${esc(trunc(path, 52))}</span>` : '';
    return `<div class="client-event-item ${ok ? 'ok' : 'error'}">
      <div class="client-event-dot"></div>
      <div class="client-event-main">
        <div class="client-event-line">
          <strong>${esc(action)}</strong>
          <time>${esc(time)}</time>
        </div>
        <div class="client-event-message">${esc(message)}</div>
        ${source ? `<div class="client-event-source">${source}</div>` : ''}
      </div>
    </div>`;
  }).join('');
}

function accountEventActionLabel(action) {
  const labels = {
    client_account_import: '导入账号',
    client_account_status: '状态刷新',
    client_account_dry_run: '配置预检',
    client_account_apply: '写入配置',
    client_account_restore: '恢复备份',
    client_config_open: '编辑配置',
    client_config_save: '保存配置',
    claude_desktop_developer_mode: '开发者模式',
  };
  return labels[action] || action || '配置事件';
}

async function fetchClientBackupsForDetail(id) {
  const el = document.getElementById('clientBackupList');
  if (!el || !id) return;
  el.innerHTML = '<span class="status-muted">加载中...</span>';
  try {
    const backups = await invoke('list_client_backups', { accountId: id });
    el.innerHTML = renderClientBackupList(id, Array.isArray(backups) ? backups : []);
  } catch (e) {
    el.innerHTML = `<span class="status-error">${esc(String(e))}</span>`;
  }
}

function renderClientBackupList(id, backups) {
  if (!backups.length) return '<span class="status-muted">暂无备份</span>';
  return backups.slice(0, 12).map(backup => {
    const time = formatTimeShort(backup.created_at);
    const sizeKb = Math.max(1, Math.round(Number(backup.size || 0) / 1024));
    return `<div class="client-backup-item">
      <div class="client-backup-main">
        <strong>${esc(backup.kind || 'config')}</strong>
        <span title="${escAttr(backup.path)}">${esc(trunc(backup.path || '', 68))}</span>
        <em>${esc(time)} · ${sizeKb} KB</em>
      </div>
      <button class="btn btn-ghost btn-small" onclick="restoreClientBackup('${escAttr(id)}', '${escAttr(backup.path || '')}')">恢复</button>
    </div>`;
  }).join('');
}

async function restoreClientBackup(id, backupPath) {
  if (!backupPath) return;
  if (!await showConfirm('确定恢复这个客户端配置备份吗？当前配置会先再次备份。')) return;
  try {
    const report = await invoke('restore_client_backup', { accountId: id, backupPath });
    showToast(report.ok ? '客户端备份已恢复' : '恢复后仍有诊断问题', report.ok ? 'success' : 'error');
    await loadAccountsData();
    fetchClientEventsForDetail(id);
    fetchClientBackupsForDetail(id);
  } catch (e) {
    showToast('恢复客户端备份失败: ' + e, 'error');
  }
}

function refreshClientSwitcherIssues() {
  const switcher = document.querySelector('.client-switcher');
  if (switcher) switcher.outerHTML = renderClientSwitcher(accountsData.accounts || []);
}

async function dryRunEditingClientAccount() {
  if (!editingAccount) return;
  syncEditingDraftFromForm();
  const statusEl = document.getElementById('clientDryRunStatus');
  const preview = document.getElementById('clientApplyPreview');
  if (statusEl) statusEl.innerHTML = '<span class="status-muted">预检中...</span>';
  try {
    const report = await invoke('test_client_account', { accountJson: serializeAccountForBackend(editingAccount) });
    if (statusEl) statusEl.innerHTML = report.ok ? '<span class="status-ok">预检通过</span>' : '<span class="status-error">预检有问题</span>';
    if (preview) preview.innerHTML = renderClientReport(report);
    if (editingAccount.id) fetchClientEventsForDetail(editingAccount.id);
  } catch (e) {
    if (statusEl) statusEl.innerHTML = '<span class="status-error">预检失败</span>';
    if (preview) preview.innerHTML = `<div class="status-error">${esc(String(e))}</div>`;
  }
}

async function dryRunClientAccount(id) {
  try {
    const report = await invoke('test_client_account', { accountId: id });
    showToast(report.ok ? '客户端预检通过' : '客户端预检发现问题', report.ok ? 'success' : 'error');
    const account = (accountsData.accounts || []).find(item => item.id === id);
    if (account) account._client_status_report = report;
    const el = document.getElementById('client-status-' + id);
    if (el) el.innerHTML = renderClientStatusSummary(report);
    refreshClientSwitcherIssues();
  } catch (e) {
    showToast('客户端预检失败: ' + e, 'error');
  }
}

async function refreshClientAccountStatus(id) {
  try {
    const report = await invoke('refresh_client_status', { accountId: id });
    showToast(report.ok ? '客户端状态正常' : '客户端状态有问题', report.ok ? 'success' : 'error');
    const account = (accountsData.accounts || []).find(item => item.id === id);
    if (account) {
      account._client_status_report = report;
      account.last_check = { ok: report.ok, message: report.message, details: report };
    }
    if (accountsView === 'list') {
      renderMainContent();
      refreshClientSwitcherIssues();
      return;
    }
    const el = document.getElementById('client-status-' + id);
    if (el) el.innerHTML = renderClientStatusSummary(report);
    const preview = document.getElementById('clientApplyPreview');
    if (preview) preview.innerHTML = renderClientReport(report);
    refreshClientSwitcherIssues();
    fetchClientEventsForDetail(id);
  } catch (e) {
    showToast('刷新客户端状态失败: ' + e, 'error');
  }
}

async function refreshClaudeDesktopDeveloperMode(id) {
  const input = document.getElementById('claudeDesktopDevModeSwitch');
  const label = document.getElementById('claudeDesktopDevModeLabel');
  const hint = document.getElementById('claudeDesktopDevModeHint');
  const restartBtn = document.getElementById('claudeDesktopRestartBtn');
  if (!input) return;
  input.disabled = true;
  if (label) label.textContent = '读取中...';
  if (restartBtn) restartBtn.disabled = true;
  try {
    const state = await invoke('get_claude_desktop_developer_mode');
    const enabled = Boolean(state.enabled);
    input.checked = enabled;
    input.disabled = false;
    const restartRequired = Boolean(state.restart_required);
    if (label) label.textContent = enabled
      ? (restartRequired ? '已开启（重启后确认）' : '已开启')
      : (restartRequired ? '已关闭（需重启）' : '已关闭');
    if (restartBtn) restartBtn.disabled = !restartRequired;
    if (hint) {
      const entries = Array.isArray(state.entries) ? state.entries.filter(item => item.exists) : [];
      const paths = entries.map(item => item.path).filter(Boolean);
      const runtimeDirs = Array.isArray(state.runtime?.user_data_dirs) ? state.runtime.user_data_dirs : [];
      hint.textContent = restartRequired
        ? `设置已落盘，Claude 正在运行，重启后生效${runtimeDirs.length ? ` · ${runtimeDirs.join(' · ')}` : ''}`
        : paths.length
        ? `同步 ${state.enabled_count || 0}/${paths.length} 个 Claude 设置文件`
        : '未找到现有设置文件，开启时会创建 Claude 默认配置';
      hint.title = paths.join('\n');
    }
  } catch (e) {
    input.disabled = true;
    if (label) label.textContent = '读取失败';
    if (hint) hint.textContent = String(e);
  }
}

async function toggleClaudeDesktopDeveloperMode(id, enabled) {
  const input = document.getElementById('claudeDesktopDevModeSwitch');
  const label = document.getElementById('claudeDesktopDevModeLabel');
  const hint = document.getElementById('claudeDesktopDevModeHint');
  const restartBtn = document.getElementById('claudeDesktopRestartBtn');
  if (!input) return;
  input.disabled = true;
  if (label) label.textContent = enabled ? '开启中...' : '关闭中...';
  if (restartBtn) restartBtn.disabled = true;
  try {
    const args = { enabled };
    if (id) args.accountId = id;
    const result = await invoke('set_claude_desktop_developer_mode', args);
    input.checked = Boolean(result.enabled);
    input.disabled = false;
    const restartRequired = Boolean(result.restart_required);
    if (label) label.textContent = result.enabled
      ? (restartRequired ? '已开启（重启后确认）' : '已开启')
      : (restartRequired ? '已关闭（需重启）' : '已关闭');
    if (restartBtn) restartBtn.disabled = !restartRequired;
    if (hint) {
      const paths = Array.isArray(result.changed_files) ? result.changed_files : [];
      const runtimeDirs = Array.isArray(result.runtime?.user_data_dirs) ? result.runtime.user_data_dirs : [];
      hint.textContent = restartRequired
        ? `已写入 ${paths.length || 0} 个设置文件，Claude 正在运行，重启后生效${runtimeDirs.length ? ` · ${runtimeDirs.join(' · ')}` : ''}`
        : (paths.length ? `已写入 ${paths.length} 个 Claude 设置文件` : '设置已保存');
      hint.title = paths.join('\n');
    }
    showToast(
      restartRequired ? `${result.message || 'Claude 开发者模式已更新'}，重启 Claude 后生效` : (result.message || 'Claude 开发者模式已更新'),
      restartRequired ? 'info' : 'success'
    );
    fetchClientEventsForDetail(id);
  } catch (e) {
    showToast('更新 Claude 开发者模式失败: ' + e, 'error');
    refreshClaudeDesktopDeveloperMode();
  }
}

async function restartClaudeDesktopForDevMode() {
  const restartBtn = document.getElementById('claudeDesktopRestartBtn');
  if (restartBtn) restartBtn.disabled = true;
  try {
    await invoke('dex_toggle_desktop_client', { kind: 'claude_desktop', running: true });
    await new Promise(resolve => setTimeout(resolve, 1200));
    await invoke('dex_toggle_desktop_client', { kind: 'claude_desktop', running: false });
    showToast('Claude 已重启，请稍后确认开发者模式状态', 'success');
    setTimeout(() => refreshClaudeDesktopDeveloperMode(), 1500);
  } catch (e) {
    showToast('重启 Claude 失败: ' + e, 'error');
    if (restartBtn) restartBtn.disabled = false;
  }
}

async function openClientConfig(id) {
  try {
    const result = await invoke('open_client_config', { accountId: id });
    showToast('已交给系统打开: ' + trunc(result.path || '', 64), 'success');
    fetchClientEventsForDetail(id);
  } catch (e) {
    showToast('打开配置文件失败: ' + e, 'error');
  }
}

async function editConfigFile(id) {
  if (!id) return;
  try {
    const file = await invoke('get_account_config_file', { accountId: id });
    showConfigEditorModal(id, file);
  } catch (e) {
    showToast('读取配置文件失败: ' + e, 'error');
  }
}

function showConfigEditorModal(id, file) {
  const existing = document.getElementById('configEditorModal');
  if (existing) existing.remove();
  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay';
  overlay.id = 'configEditorModal';
  const statusText = file.exists ? `已加载 · ${Number(file.size_bytes || 0)} bytes` : '文件不存在，将按当前账号生成初始内容';
  overlay.innerHTML = `<div class="modal-box config-editor-modal">
    <div class="modal-header">
      <h3>编辑配置文件</h3>
      <button class="modal-close" id="configEditorCloseBtn" type="button">✕</button>
    </div>
    <div class="modal-body">
      <div class="config-editor-meta">
        <strong>${esc(file.label || '配置文件')}</strong>
        <span title="${escAttr(file.path || '')}">${esc(file.path || '')}</span>
        <em>${esc((file.format || '').toUpperCase())} · ${esc(statusText)}</em>
      </div>
      <div id="configEditorValidation">${renderConfigValidation(file.validation)}</div>
      <textarea id="configEditorContent" class="config-editor-textarea" spellcheck="false">${esc(file.content || '')}</textarea>
    </div>
    <div class="config-editor-actions">
      <span id="configEditorSaveStatus"></span>
      <button class="btn btn-ghost" id="configEditorValidateBtn" type="button">校验/编译</button>
      <button class="btn btn-ghost" id="configEditorSystemOpenBtn" type="button">系统打开</button>
      <button class="btn btn-primary" id="configEditorSaveBtn" type="button">保存</button>
    </div>
  </div>`;
  document.body.appendChild(overlay);
  const cleanup = () => overlay.remove();
  document.getElementById('configEditorCloseBtn').onclick = cleanup;
  overlay.addEventListener('click', e => { if (e.target === overlay) cleanup(); });
  document.getElementById('configEditorValidateBtn').onclick = async () => {
    const content = document.getElementById('configEditorContent')?.value || '';
    const status = document.getElementById('configEditorSaveStatus');
    if (status) status.innerHTML = '<span class="status-muted">校验中...</span>';
    try {
      const validation = await invoke('validate_account_config_file', { accountId: id, content });
      document.getElementById('configEditorValidation').innerHTML = renderConfigValidation(validation);
      if (status) status.innerHTML = validation.ok ? '<span class="status-ok">校验通过</span>' : '<span class="status-error">校验失败</span>';
    } catch (e) {
      if (status) status.innerHTML = '<span class="status-error">校验异常</span>';
      showToast('配置校验失败: ' + e, 'error');
    }
  };
  document.getElementById('configEditorSaveBtn').onclick = async () => {
    const content = document.getElementById('configEditorContent')?.value || '';
    const status = document.getElementById('configEditorSaveStatus');
    if (status) status.innerHTML = '<span class="status-muted">保存中...</span>';
    try {
      const result = await invoke('save_account_config_file', { accountId: id, content });
      document.getElementById('configEditorValidation').innerHTML = renderConfigValidation(result.validation);
      if (!result.ok) {
        if (status) status.innerHTML = '<span class="status-error">未保存</span>';
        showToast(result.message || '配置校验未通过，未保存', 'error');
        return;
      }
      if (status) status.innerHTML = `<span class="status-ok">已保存${result.backup_path ? '，已备份' : ''}</span>`;
      showToast('配置文件已保存', 'success');
      fetchClientEventsForDetail(id);
      fetchClientBackupsForDetail(id);
    } catch (e) {
      if (status) status.innerHTML = '<span class="status-error">保存失败</span>';
      showToast('保存配置文件失败: ' + e, 'error');
    }
  };
  document.getElementById('configEditorSystemOpenBtn').onclick = () => openClientConfig(id);
}

function renderConfigValidation(validation) {
  const result = validation || {};
  const diagnostics = Array.isArray(result.diagnostics) ? result.diagnostics : [];
  const cls = result.ok ? 'status-ok' : 'status-error';
  const label = result.ok ? '语法正常' : '需要修正';
  return `<div class="config-editor-validation">
    <strong class="${cls}">${label}</strong>
    ${diagnostics.map(item => `<span class="${item.level === 'error' ? 'status-error' : (item.level === 'warning' ? 'status-warn' : 'status-muted')}">${esc(item.message || '')}</span>`).join('')}
  </div>`;
}

async function saveAndApplyClientAccount(id) {
  const saved = await saveAccount({ silent: true, stay: true });
  if (!saved) {
    showToast('写入前保存账号失败', 'error');
    return;
  }
  showToast('账号已保存，开始预检写入配置', 'success');
  await applyClientAccount(saved.id || id);
}

async function applyClientAccount(id) {
  try {
    showToast('正在执行写入前预检', 'info');
    const dryReport = await invoke('apply_client_account', { accountId: id, dryRun: true });
    const statusEl = document.getElementById('client-status-' + id);
    if (statusEl) statusEl.innerHTML = renderClientStatusSummary(dryReport);
    fetchClientEventsForDetail(id);
    if (!await showClientApplyConfirm(dryReport)) {
      if (!dryReport.ok) showToast('预检未通过，已取消写入', 'error');
      return;
    }
    const report = await invoke('apply_client_account', { accountId: id, dryRun: false });
    showToast(report.ok ? '客户端配置已写入' : '客户端配置写入后仍有问题', report.ok ? 'success' : 'error');
    await loadAccountsData();
    if (accountsView === 'list') {
      renderMainContent();
      refreshClientSwitcherIssues();
      return;
    }
    const el = document.getElementById('client-status-' + id);
    if (el) el.innerHTML = renderClientStatusSummary(report);
    fetchClientEventsForDetail(id);
    fetchClientBackupsForDetail(id);
  } catch (e) {
    showToast('写入客户端配置失败: ' + e, 'error');
  }
}

async function fetchClientStatusForCard(a) {
  const el = document.getElementById('client-status-' + a.id);
  if (!el || !a.id) return;
  el.innerHTML = '<span class="balance-loading">检查中...</span>';
  try {
    const report = await invoke('get_client_status', { accountId: a.id });
    a._client_status_report = report;
    el.innerHTML = renderClientStatusSummary(report);
    refreshClientSwitcherIssues();
  } catch (e) {
    el.innerHTML = `<span class="status-error">${esc(String(e))}</span>`;
  }
}

function renderClientStatusSummary(report) {
  const command = report.command || {};
  const version = command.version || (command.installed ? '已安装' : '未检测到');
  const diagnostics = Array.isArray(report.diagnostics) ? report.diagnostics : [];
  const hasError = diagnostics.some(item => item.level === 'error');
  const cls = hasError ? 'status-error' : (report.ok ? 'status-ok' : 'status-warn');
  const risk = report.risk_level ? `风险 ${report.risk_level}` : (report.schema_ok === false ? 'Schema 异常' : '');
  return `<div class="client-status-summary">
    <span class="${cls}">${esc(version)}</span>
    ${risk ? `<span>${esc(risk)}</span>` : ''}
  </div>`;
}

// ── 模型获取与余额查询 ──

let balanceCache = {};

async function applyAccount(id) {
  try {
    await invoke('switch_account', { id });
    accountsData.active_id = id;
    showToast('已切换活跃账号', 'success');
    await loadAccountsData();
  } catch (e) {
    showToast('切换账号失败: ' + e, 'error');
  }
}

function isRedactedSecret(value) {
  return typeof value === 'string' && value.includes('****');
}

function displayStoredSecret(value, hasStored) {
  if (!hasStored) return '';
  const text = String(value || '').trim();
  if (!text) return '****';
  return isRedactedSecret(text) ? text : maskKey(text);
}

function hasStoredPrimaryApiKey(account = editingAccount) {
  return Boolean(
    account?.id
    && (account.api_key_present || isRedactedSecret(account.api_key || ''))
  );
}

function hasStoredVisionApiKey(account = editingAccount, endpoint = currentEndpoint(account)) {
  return Boolean(
    account?.id
    && (
      account.vision_api_key_present
      || isRedactedSecret(account.vision_api_key || '')
      || isRedactedSecret(endpoint?.vision?.api_key || '')
    )
  );
}

async function copyEditingAccountSecret(secretKind) {
  if (!editingAccount?.id) {
    showToast('请先保存账号后再复制密钥', 'error');
    return;
  }
  try {
    const endpoint = currentEndpoint(editingAccount);
    await invoke('copy_account_secret', {
      accountId: editingAccount.id,
      secretKind,
      endpointId: endpoint?.id || null,
    });
    showToast(secretKind === 'vision_api_key' ? '视觉 API Key 已复制' : 'API Key 已复制', 'success');
  } catch (e) {
    showToast('复制密钥失败: ' + e, 'error');
  }
}

function buildEditingUpstreamProbeArgs(upstream, apiKey, endpointKind, options = {}) {
  const args = { upstream, endpointKind };
  if (editingAccount?.id && (options.forceStored || isRedactedSecret(apiKey) || (!apiKey && hasStoredPrimaryApiKey(editingAccount)))) {
    args.accountId = editingAccount.id;
  } else {
    args.apiKey = apiKey || '';
  }
  return args;
}

function buildEditingVisionProbeArgs(upstream, apiKey, visionPath, adapterId) {
  const args = { upstream, visionPath, adapterId };
  if (editingAccount?.id && (isRedactedSecret(apiKey) || (!apiKey && hasStoredVisionApiKey(editingAccount)))) {
    args.accountId = editingAccount.id;
    const endpointId = currentEndpoint(editingAccount)?.id;
    if (endpointId) args.endpointId = endpointId;
  } else {
    args.apiKey = apiKey || '';
  }
  return args;
}

async function fetchAndPopulateModels() {
  const statusEl = document.getElementById('modelFetchStatus');
  if (statusEl) statusEl.innerHTML = '<span class="status-muted">获取中...</span>';
  try {
    // 统一使用表单中的 upstream 和 api_key（用户可能已修改但尚未保存）
    const upstream = document.getElementById('edit_upstream')?.value?.trim();
    const keyInput = document.getElementById('edit_api_key');
    const apiKey = keyInput ? keyInput.value.trim() : '';
    if (!upstream) { showToast('请先填写上游 URL', 'error'); return; }
    const endpointKind = document.getElementById('edit_endpoint_kind')?.value || 'open_ai_chat';
    const useStoredOfficialAccount = isCodexOfficialAccount(editingAccount) && editingAccount?.id;
    upstreamModels = await invoke(
      'fetch_upstream_models',
      buildEditingUpstreamProbeArgs(upstream, apiKey, endpointKind, { forceStored: useStoredOfficialAccount })
    );
    if (upstreamModels.length > 0) {
      if (statusEl) statusEl.innerHTML = `<span class="status-ok">获取到 ${upstreamModels.length} 个模型</span>`;
      // 重新渲染模型映射行
      const knownModels = getProviderKnownModels(editingAccount?.provider || '');
      document.getElementById('modelMapRows').innerHTML = renderModelMappingRows(knownModels);
    } else {
      if (statusEl) statusEl.innerHTML = '<span class="status-muted">上游未返回模型</span>';
    }
  } catch (e) {
    if (statusEl) statusEl.innerHTML = '<span class="status-error">获取失败</span>';
    showToast('获取模型列表失败: ' + e, 'error');
  }
}

async function fetchClientModels() {
  const statusEl = document.getElementById('clientModelFetchStatus');
  if (statusEl) statusEl.innerHTML = '<span class="status-muted">获取中...</span>';
  try {
    const upstream = document.getElementById('edit_upstream')?.value?.trim();
    const apiKey = document.getElementById('edit_api_key')?.value?.trim() || '';
    if (!upstream) { showToast('请先填写目标客户端 Base URL', 'error'); return; }
    const provider = document.getElementById('edit_client_provider')?.value || editingAccount?.provider || '';
    const endpointKind = accountClientKind(editingAccount) === 'claude_code' || provider === 'anthropic'
      ? 'anthropic_messages'
      : 'open_ai_chat';
    upstreamModels = await invoke('fetch_upstream_models', buildEditingUpstreamProbeArgs(upstream, apiKey, endpointKind));
    if (upstreamModels.length > 0) {
      if (statusEl) statusEl.innerHTML = `<span class="status-ok">获取到 ${upstreamModels.length} 个模型</span>`;
      syncEditingDraftFromForm();
      const rows = document.getElementById('clientModelMapRows');
      if (rows) rows.innerHTML = renderClientModelMappingRows(editingAccount);
    } else {
      if (statusEl) statusEl.innerHTML = '<span class="status-muted">上游未返回模型</span>';
    }
  } catch (e) {
    if (statusEl) statusEl.innerHTML = '<span class="status-error">获取失败</span>';
    showToast('获取客户端模型列表失败: ' + e, 'error');
  }
}

async function testUpstreamConnectivity() {
  const upstream = document.getElementById('edit_upstream')?.value?.trim();
  if (!upstream) { showToast('请先填写上游 URL', 'error'); return; }
  const keyInput = document.getElementById('edit_api_key');
  const apiKey = keyInput ? keyInput.value.trim() : '';
  const resultEl = document.getElementById('connectivityResult');
  if (resultEl) resultEl.innerHTML = '<span class="status-muted">检测中...</span>';
  try {
    const endpointKind = document.getElementById('edit_endpoint_kind')?.value || 'open_ai_chat';
    const result = await invoke('test_upstream_connectivity', buildEditingUpstreamProbeArgs(upstream, apiKey, endpointKind));
    if (result.ok) {
      const models = result.model_count != null ? `，${result.model_count} 个模型` : '';
      if (resultEl) resultEl.innerHTML = `<span class="status-ok">连通 (${result.status}, ${result.latency_ms}ms${models})</span>`;
      showToast(`上游连通正常 (${result.latency_ms}ms${models})`, 'success');
    } else if (result.error) {
      if (resultEl) resultEl.innerHTML = `<span class="status-error">${esc(result.error)}</span>`;
      showToast('连通失败: ' + result.error, 'error');
    } else {
      if (resultEl) resultEl.innerHTML = `<span class="status-warn">HTTP ${result.status} (${result.latency_ms}ms)</span>`;
      showToast(`上游返回 HTTP ${result.status}`, 'error');
    }
  } catch (e) {
    if (resultEl) resultEl.innerHTML = `<span class="status-error">${esc(String(e))}</span>`;
    showToast('连通测试异常: ' + e, 'error');
  } finally {
    await refreshBalanceFromFormConnectivity();
  }
}

async function testVisionConnectivity() {
  const upstream = document.getElementById('edit_vision_upstream')?.value?.trim();
  if (!upstream) { showToast('请先填写视觉上游 URL', 'error'); return; }
  const keyInput = document.getElementById('edit_vision_api_key');
  const apiKey = keyInput ? keyInput.value.trim() : '';
  const resultEl = document.getElementById('visionConnectivityResult');
  if (resultEl) resultEl.innerHTML = '<span class="status-muted">检测中...</span>';
  try {
    const visionPath = document.getElementById('edit_vision_endpoint')?.value?.trim() || 'v1/coding_plan/vlm';
    const adapterId = document.getElementById('edit_vision_adapter')?.value || 'minimax_coding_plan_vlm';
    const result = await invoke('test_vision_connectivity', buildEditingVisionProbeArgs(upstream, apiKey, visionPath, adapterId));
    if (result.ok) {
      const detail = result.detail ? `，${esc(result.detail)}` : '';
      if (resultEl) resultEl.innerHTML = `<span class="status-ok">连通 (${result.status}, ${result.latency_ms}ms${detail})</span>`;
      showToast(`视觉上游连通正常 (${result.latency_ms}ms)`, 'success');
    } else if (result.error) {
      if (resultEl) resultEl.innerHTML = `<span class="status-error">${esc(result.error)}</span>`;
      showToast('视觉上游连通失败: ' + result.error, 'error');
    } else {
      if (resultEl) resultEl.innerHTML = `<span class="status-warn">HTTP ${result.status} (${result.latency_ms}ms)</span>`;
      showToast(`视觉上游返回 HTTP ${result.status}`, 'error');
    }
  } catch (e) {
    if (resultEl) resultEl.innerHTML = `<span class="status-error">${esc(String(e))}</span>`;
    showToast('视觉连通测试异常: ' + e, 'error');
  }
}

async function fetchBalanceForCard(a) {
  const el = document.getElementById('balance-' + a.id);
  if (!el) return;
  const cacheKey = a.id;
  // 5 分钟内使用缓存
  if (balanceCache[cacheKey] && balanceCache[cacheKey].ts > Date.now() - 300000) {
    el.innerHTML = renderBalanceInfo(balanceCache[cacheKey].info);
    return;
  }
  el.innerHTML = '<span class="balance-loading">...</span>';
  try {
    const info = await invoke('fetch_balance', { accountId: a.id });
    balanceCache[cacheKey] = { info, ts: Date.now() };
    el.innerHTML = renderBalanceInfo(info);
  } catch (e) {
    el.innerHTML = '<span class="balance-na">不可用</span>';
  }
}

async function refreshBalanceForCard(id) {
  const a = (accountsData.accounts || []).find(acc => acc.id === id);
  if (!a) return;
  delete balanceCache[id];
  await fetchBalanceForCard(a);
}

async function refreshOfficialQuotaFromDetail(id) {
  const box = document.getElementById('official-quota-' + id);
  if (box) box.innerHTML = '<span class="balance-loading">刷新中...</span>';
  delete balanceCache[id];
  try {
    const info = await invoke('fetch_balance', { accountId: id });
    balanceCache[id] = { info, ts: Date.now() };
    if (box) box.innerHTML = renderBalanceInfo(info);
    const official = info?.official || null;
    if (official) {
      editingAccount.client_options = editingAccount.client_options || {};
      editingAccount.client_options.oauth_quota = official;
    }
    const fresh = await invoke('list_accounts');
    accountsData = fresh;
    refreshEditingAccountAfterManagement(id);
    showToast('额度状态已刷新', 'success');
  } catch (e) {
    if (box) box.innerHTML = `<span class="balance-na">${esc(String(e))}</span>`;
    showToast('刷新额度失败: ' + e, 'error');
  }
}

async function refreshBalanceFromFormConnectivity() {
  if (!editingAccount || !editingAccount.id || !isCodexAccount(editingAccount)) return;
  await refreshOfficialQuotaFromDetail(editingAccount.id);
}

async function testAccountUpstreamForCard(id) {
  const a = (accountsData.accounts || []).find(acc => acc.id === id);
  if (!a) return;
  const isCodex = isCodexAccount(a);
  const el = document.getElementById(isCodex ? 'balance-' + id : 'client-status-' + id);
  const upstream = cardUpstream(a);
  if (!upstream) {
    showToast('未配置上游 URL', 'error');
    return;
  }
  if (el) el.innerHTML = '<span class="balance-loading">检测中...</span>';
  try {
    const result = await invoke('test_upstream_connectivity', {
      accountId: id,
      upstream,
      endpointKind: cardEndpointKind(a),
    });
    if (result.ok) {
      const models = result.model_count != null ? `，${result.model_count} 个模型` : '';
      showToast(`上游连通正常 (${result.latency_ms}ms${models})`, 'success');
    } else if (result.error) {
      showToast('连通失败: ' + result.error, 'error');
    } else {
      showToast(`上游返回 HTTP ${result.status}`, 'error');
    }
  } catch (e) {
    showToast('连通测试异常: ' + e, 'error');
  } finally {
    if (isCodex) await refreshBalanceForCard(id);
    else await refreshClientAccountStatus(id);
  }
}

function balanceQuotaText(current, total = null) {
  if (current != null && current !== '' && total != null && total !== '') return `${current}/${total}`;
  if (current != null && current !== '') return String(current);
  if (total != null && total !== '') return String(total);
  return '—';
}

function renderBalanceInfo(info) {
  if (info.mode === 'official_oauth') {
    return renderOfficialQuotaInfo(info);
  }
  if (info.mode === 'token_credit') {
    const remaining = info.credit_remaining;
    const limit = info.credit_limit;
    const pct = limit > 0 ? Math.round(remaining / limit * 100) : 0;
    const safePct = Math.max(0, Math.min(pct, 100));
    const label = info.credit_label ? ` (${info.credit_label})` : '';
    return `<div class="balance-pill balance-credit">
      <span class="balance-credit-text"><span class="balance-card-mark" aria-hidden="true"></span>$${remaining != null ? remaining.toFixed(2) : '—'}${limit != null ? ' / $' + limit.toFixed(2) : ''}${label}</span>
      <div class="bar-track"><div class="bar-fill" style="width:${safePct}%"></div></div>
    </div>`;
  }
  if (info.mode === 'subscription') {
    return `<div class="balance-pill balance-plan">
      <span class="balance-quota"><em>5h</em><strong>${balanceQuotaText(info.hours_5_remaining)}</strong></span>
      <span class="balance-quota"><em>周</em><strong>${balanceQuotaText(info.weekly_remaining, info.weekly_limit)}</strong></span>
    </div>`;
  }
  if (info.mode === 'coding_plan' && info.model_remains) {
    const coding = info.model_remains.find(m => m.model_name === 'MiniMax-M*') || info.model_remains[0];
    if (!coding) return '<div class="balance-pill balance-empty"><span class="balance-na">无模型数据</span></div>';
    const iRemain = coding.interval_total - coding.interval_used;
    const wRemain = coding.weekly_total - coding.weekly_used;
    return `<div class="balance-pill balance-plan">
      <span class="balance-quota"><em>5h</em><strong>${iRemain}/${coding.interval_total}</strong></span>
      <span class="balance-quota"><em>周</em><strong>${wRemain}/${coding.weekly_total}</strong></span>
    </div>`;
  }
  return '<div class="balance-pill balance-empty"><span class="balance-na">不支持</span></div>';
}

// ═══════════════════════════════════════════════════════════════
// 键盘快捷键
