// WorkBuddy 资源页：展示从 WorkBuddy 提取的 MCP 连接器 / 技能 / Custom Agents。
// 数据来自后端命令 list_wb_resources（扫描 ~/.deecodex/wb-resources）。
var _wbData = null;
var _wbLoading = false;
var _wbError = '';
var _wbSearch = '';
var _wbSection = 'all';
var _wbZoneCategory = 'all';
var _wbVisibleCount = 60;
var _wbExpanded = {};
var _wbTargets = ['codex', 'claude', 'opencode'];
var _wbInstalled = { targets: {} };
var _wbActing = {};
// Mobile Codex 授权缓存（套餐有效校验，避免每次点击都打服务器）
var _wbEntitlementCache = { at: 0, known: false, authorized: false, error: '', planName: '', planCode: '', serviceStatus: '' };

var WB_SKILL_CATEGORY_LABEL = {
  'content-creation': '内容创作',
  'productivity-tools': '效率工具',
  'investment-finance': '投资理财',
  'development-tools': '开发工具',
  'business-operations': '商业运营',
  'education-learning': '学习教育',
  'data-analysis': '数据分析',
  'news-information': '信息资讯',
  'lifestyle-services': '生活服务',
  'opc-solo-business': 'OPC 一人公司',
  'knowledge-learning': '知识学习',
  'collaboration': '协同办公',
  'website-deployment': '网站部署'
};
var WB_AGENT_CATEGORY_LABEL = {
  '01-ProductDesign': '产品设计',
  '02-Engineering': '技术工程',
  '03-GameSpatial': '游戏空间',
  '04-DataAI': '数据智能',
  '05-MarketingGrowth': '营销增长',
  '06-ContentCreative': '内容创作',
  '07-SalesCommerce': '销售商务',
  '08-FinanceInvestment': '金融投资',
  '09-OperationsHR': '运营人力',
  '10-ProjectQuality': '项目质量',
  '11-SecurityCompliance': '法务安全',
  '12-IndustryConsultant': '行业顾问',
  '13-TencentZone': '腾讯专区',
  '14-GlobalDevelopment': '全球发展'
};
var WB_CONNECTOR_KIND_LABEL = { mcp: 'MCP', cli: 'CLI', mixed: 'MCP+CLI' };

function wbStats() {
  return (_wbData && _wbData.stats) || { connectors: 0, skills: 0, agents: 0 };
}

async function loadWbResources() {
  if (_wbLoading) return;
  _wbLoading = true;
  _wbError = '';
  try {
    try {
      await invoke('sync_wb_resources');
    } catch (e) {
      console.warn('WorkBuddy 清单同步失败', e);
    }
    _wbData = await invoke('list_wb_resources');
    try {
      _wbInstalled = await invoke('list_wb_installed') || { targets: {} };
    } catch (e) {
      _wbInstalled = { targets: {} };
    }
  } catch (e) {
    _wbError = String(e || '加载失败');
  } finally {
    _wbLoading = false;
    renderWbResourceList();
    const tabs = document.querySelector('.plugin-mode-tabs');
    if (tabs) tabs.outerHTML = renderPluginModeTabs();
    const nav = document.querySelector('.wb-market-nav-group');
    if (nav && typeof renderMarketNav === 'function') nav.outerHTML = renderMarketNav();
    const zoneCats = document.getElementById('wbZoneCats');
    if (zoneCats && _pluginZone && _pluginZone !== 'dex') {
      zoneCats.outerHTML = renderWbZoneCategories(_wbSection === 'connectors' ? 'connectors' : _wbSection === 'agents' ? 'agents' : 'skills');
    }
  }
}

function refreshWbResources() {
  _wbData = null;
  _wbError = '';
  _wbInstalled = { targets: {} };
  loadWbResources();
}

function wbTargetLabel(t) {
  return t === 'codex' ? 'Codex' : t === 'claude' ? 'Claude Code' : 'opencode';
}

function wbAssetUrl(rel) {
  if (!rel) return '';
  const root = (_wbData && _wbData.root) || '';
  if (!root || typeof DeeCodexTauri.convertFileSrc !== 'function') return '';
  return DeeCodexTauri.convertFileSrc(root + '/' + rel);
}

function wbFindResource(kind, id) {
  const data = _wbData || {};
  const list = kind === 'skill' ? (data.skills || [])
    : kind === 'connector' ? (data.connectors || []) : (data.agents || []);
  return list.find(x => (x.id === id) || (kind !== 'connector' && x.name === id));
}

function wbResourceUsageText(kind, item) {
  if (!item) return '';
  const root = (_wbData && _wbData.root) || '~/.deecodex/wb-resources';
  if (kind === 'skill') {
    const name = item.name || item.id;
    return [
      `【Skill】${item.display_name || name}（${name}）`,
      `来源目录：${root}/skills/${item.id}`,
      `说明：${item.description || ''}`,
      '',
      '使用方式（复制到其他工具）：',
      `1. Codex：cp -r ${root}/skills/${item.id} ~/.codex/skills/${name}`,
      `2. Claude Code：cp -r ${root}/skills/${item.id} ~/.claude/skills/${name}`,
      `3. opencode：cp -r ${root}/skills/${item.id} ~/.config/opencode/skills/${name}`,
      '4. 或在 DEX AI 插件市场勾选目标客户端后点「安装」',
      '',
      '结构：SKILL.md（frontmatter+指令）+ scripts/ + references/ + assets/'
    ].join('\n');
  }
  if (kind === 'connector') {
    const servers = (item.mcp_servers || []).map(s =>
      `   ${s.name}  ${s.type}  ${s.url || s.command}`
    ).join('\n');
    return [
      `【MCP 连接器】${item.name}（${item.id}）`,
      `来源目录：${root}/connectors/${item.id}`,
      `说明：${item.description || ''}`,
      `MCP Servers：\n${servers || '   （CLI 型，需安装命令行工具）'}`,
      '',
      '使用方式：',
      `1. DEX AI：勾选目标客户端（Codex/Claude Code/opencode）后点「启用 MCP」`,
      `2. Codex 手动：把 mcp.json 的 mcpServers 合并进 ~/.codex/config.toml 的 [mcp_servers]`,
      `3. Claude Code：写入 ~/.claude.json 的 mcpServers`,
      `4. opencode：写入 ~/.config/opencode/opencode.json 的 mcp`,
      '',
      '结构：mcp.json（标准 MCP 定义）+ 可选 cli.json / skills/'
    ].join('\n');
  }
  // agent
  return [
    `【深度研究/专家】${item.profession || item.display_name || item.name}（${item.display_name || ''}）`,
    `prompt 位置：${root}/agents/${item.id}/agents/`,
    `说明：${item.description || ''}`,
    '',
    '使用方式（转换为 Custom Agent）：',
    '1. DEX AI：勾选目标客户端（Codex/Claude Code/opencode）后点「生成 Agent」',
    `2. Codex：生成 ~/.codex/agents/<name>.toml（developer_instructions = prompt 正文）`,
    `3. Claude Code：生成 ~/.claude/agents/wb-<name>.md（subagent）`,
    `4. opencode：写入 ~/.config/opencode/opencode.json 的 agent（wb- 前缀）`,
    '',
    '结构：agents/*.md（人设+方法论 prompt）+ avatars/（头像）'
  ].join('\n');
}

async function wbCopyResource(kind, id) {
  if (!(await wbEnsureAuthorized())) return;
  const item = wbFindResource(kind, id);
  const text = wbResourceUsageText(kind, item);
  if (!text) { showToast('未找到资源', 'error'); return; }
  // 先下载资源本体到本地，确保复制出的命令/路径可用
  try {
    await invoke('fetch_wb_resource', { kind: kind, id: id });
  } catch (e) {
    console.warn('资源本体下载失败（说明仍可复制）', e);
  }
  // 复制到剪贴板
  const done = function () {
    showToast('使用说明已复制到剪贴板', 'success');
    showWbUsageModal(item, text);
  };
  if (navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard.writeText(text).then(done).catch(function () {
      fallbackCopy(text); done();
    });
  } else {
    fallbackCopy(text); done();
  }
}

// 当前 Mobile Codex 授权（套餐）状态
function wbEntitlement() {
  const user = (_mobileCodexState && _mobileCodexState.user) || {};
  return (user && user.entitlement) || {};
}

// DEX AI 主账号（个人中心）登录状态：主账号退出后插件市场应立即锁定。
async function wbMasterLoggedIn() {
  try {
    if (typeof profileMasterState !== 'undefined' && profileMasterState.loaded) {
      return !!profileMasterState.loggedIn;
    }
    const status = await invoke('dexai_master_status', {});
    if (typeof profileMasterState !== 'undefined') {
      profileMasterState.loggedIn = !!(status && status.loggedIn);
      profileMasterState.loaded = true;
    }
    return !!(status && status.loggedIn);
  } catch (e) {
    return false;
  }
}

// 正向操作前置门禁：套餐有效才放行，否则弹授权引导。带 30s 缓存，避免频繁请求服务器。
async function wbEnsureAuthorized() {
  // 前置：DEX AI 主账号必须已登录（个人中心），退出后立即锁定。
  if (!(await wbMasterLoggedIn())) {
    _wbEntitlementCache = { at: 0, known: false, authorized: false, error: '', planName: '', planCode: '', serviceStatus: '' };
    showWbAuthRequiredModal({ known: false, authorized: false });
    return false;
  }
  const now = Date.now();
  let ent = _wbEntitlementCache;
  if (now - ent.at > 30000 || !ent.known) {
    let fresh = null;
    try {
      if (typeof refreshMobileCodexEntitlement === 'function') {
        const user = await refreshMobileCodexEntitlement();
        fresh = (user && user.entitlement) || { known: false, authorized: false };
      } else {
        fresh = { known: false, authorized: false, error: 'Mobile Codex 授权查询不可用' };
      }
    } catch (e) {
      fresh = { known: false, authorized: false, error: String(e) };
    }
    if (!fresh) fresh = { known: false, authorized: false, error: 'Mobile Codex 授权查询不可用' };
    ent = _wbEntitlementCache = {
      at: now,
      known: !!fresh.known,
      authorized: !!fresh.authorized,
      error: fresh.error || '',
      planName: fresh.planName || '',
      planCode: fresh.planCode || '',
      serviceStatus: fresh.serviceStatus || ''
    };
  }
  if (ent.known && ent.authorized) return true;
  showWbAuthRequiredModal(ent);
  return false;
}

function showWbAuthRequiredModal(ent) {
  const existing = document.getElementById('wbAuthRequiredModal');
  if (existing) existing.remove();
  // 场景化文案：未登录 / 套餐失效 / 技术错误，分别引导到个人中心处理。
  let main = '插件市场的「复制 / 安装 / 生成」需要 <b>DEX AI 账号授权</b>。请先登录 DEX AI 主账号，并在个人中心确认授权后重试。';
  let detail = '';
  if (ent.error) {
    main = '授权状态暂时无法确认，请检查网络后重试。';
    detail = esc(ent.error);
  } else if (ent.known && !ent.authorized) {
    main = `你的 DEX AI 账号套餐<b>未生效或已过期</b>（当前状态：${esc(ent.serviceStatus || '未知')}）。请前往个人中心查看套餐 / 续费后重试。`;
  }
  const overlay = document.createElement('div');
  overlay.id = 'wbAuthRequiredModal';
  overlay.className = 'wb-auth-overlay';
  // 关键布局用内联样式兜底：不依赖全局 modal 样式，任何环境下都保持居中紧凑的对话框
  overlay.style.cssText = 'position:fixed;inset:0;z-index:500;display:flex;align-items:center;justify-content:center;background:rgba(12,14,18,0.52);';
  overlay.innerHTML = `<div class="wb-auth-box" role="alertdialog" aria-modal="true" aria-labelledby="wbAuthTitle" style="width:min(92vw,440px);max-width:440px;">
    <div class="wb-auth-head">
      <span class="wb-auth-icon" aria-hidden="true">🔒</span>
      <div class="wb-auth-title">
        <strong id="wbAuthTitle">需要 DEX AI 授权</strong>
        <span>登录 DEX AI 主账号并保持套餐有效后可用</span>
      </div>
      <button class="wb-auth-close" id="wbAuthX" type="button" aria-label="关闭">×</button>
    </div>
    <div class="wb-auth-body">
      <p>${main}</p>
      ${detail ? `<p class="wb-auth-reason">${detail}</p>` : ''}
    </div>
    <div class="wb-auth-foot">
      <button class="btn btn-ghost" id="wbAuthCancel" type="button">取消</button>
      <button class="btn btn-primary" id="wbAuthGo" type="button">去个人中心</button>
    </div>
  </div>`;
  document.body.appendChild(overlay);
  document.getElementById('wbAuthX').onclick = function () { overlay.remove(); };
  document.getElementById('wbAuthCancel').onclick = function () { overlay.remove(); };
  document.getElementById('wbAuthGo').onclick = function () {
    overlay.remove();
    wbGoAuthorize();
  };
  overlay.addEventListener('click', function (e) { if (e.target === overlay) overlay.remove(); });
  document.addEventListener('keydown', function onKey(e) {
    if (e.key === 'Escape') {
      overlay.remove();
      document.removeEventListener('keydown', onKey);
    }
  });
}

function wbGoAuthorize() {
  // 重置缓存，授权回来后再点会重新校验
  _wbEntitlementCache = { at: 0, known: false, authorized: false, error: '', planName: '', planCode: '', serviceStatus: '' };
  // DEX AI 统一账号与授权中心：个人中心是唯一登录与授权管理入口
  if (typeof switchPanel === 'function') switchPanel('profile');
}

function fallbackCopy(text) {
  try {
    const ta = document.createElement('textarea');
    ta.value = text;
    ta.style.position = 'fixed';
    ta.style.opacity = '0';
    document.body.appendChild(ta);
    ta.select();
    document.execCommand('copy');
    ta.remove();
  } catch (e) {}
}

function showWbUsageModal(item, text) {
  const existing = document.getElementById('wbUsageModal');
  if (existing) existing.remove();
  const title = item.display_name || item.name || '使用说明';
  const overlay = document.createElement('div');
  overlay.className = 'modal-overlay';
  overlay.id = 'wbUsageModal';
  overlay.innerHTML = `<div class="modal-box wb-usage-box">
    <div class="modal-header"><h3>${esc(title)} — 使用说明</h3></div>
    <div class="wb-usage-body">
      <div class="wb-usage-copied">✓ 已复制到剪贴板，可粘贴到任意编辑器/终端</div>
      <pre class="wb-usage-pre">${esc(text)}</pre>
    </div>
    <div style="display:flex;gap:12px;padding:0 20px 16px;justify-content:flex-end;">
      <button class="btn btn-ghost" id="wbUsageClose" type="button">关闭</button>
    </div>
  </div>`;
  document.body.appendChild(overlay);
  document.getElementById('wbUsageClose').onclick = function () { overlay.remove(); };
  overlay.addEventListener('click', function (e) { if (e.target === overlay) overlay.remove(); });
}

function wbIconHtml(rel, fallbackText) {
  const url = wbAssetUrl(rel);
  if (url && rel && rel !== 'letter') {
    return `<img class="wb-icon" src="${url}" data-wb-rel="${escAttr(rel)}" alt="" onerror="wbAssetError(this)">`;
  }
  return `<span class="wb-letter">${esc((fallbackText || '?').slice(0, 1))}</span>`;
}

function wbCaretHtml(expanded) {
  return `<span class="wb-caret${expanded ? ' open' : ''}">
    <svg viewBox="0 0 16 16" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 4l4 4-4 4"></path></svg>
  </span>`;
}

async function wbAssetError(img) {
  const rel = img && img.getAttribute('data-wb-rel');
  if (!rel) { if (img) img.style.display = 'none'; return; }
  try {
    const r = await invoke('fetch_wb_asset', { rel: rel });
    if (r && r.path) {
      img.src = DeeCodexTauri.convertFileSrc(r.path);
      img.removeAttribute('data-wb-rel');
      return;
    }
  } catch (e) {}
  img.style.display = 'none';
}

async function syncAllWbResources() {
  try {
    const r = await invoke('sync_wb_resources');
    showToast(`清单已同步（新增 ${(r.downloaded || []).length}，已存在 ${(r.existing || []).length}）`, 'success');
    await loadWbResources();
  } catch (e) {
    showToast('同步失败: ' + esc(String(e)), 'error');
  }
}

function wbInstalledTargets(kind, id) {
  const out = [];
  const targets = _wbInstalled.targets || {};
  for (const t of Object.keys(targets)) {
    const list = targets[t] || {};
    if (kind === 'connector' && (list.connectors || []).some(c => c.id === id)) out.push(t);
    if (kind === 'skill' && (list.skills || []).includes(id)) out.push(t);
    if (kind === 'agent' && (list.agents || []).includes(id)) out.push(t);
  }
  return out;
}

function wbConnectorInstalled(id) {
  return wbInstalledTargets('connector', id);
}
function wbSkillInstalled(id) {
  return wbInstalledTargets('skill', id);
}
function wbAgentInstalled(id) {
  return wbInstalledTargets('agent', id);
}

async function wbInstallConnector(id) {
  if (!(await wbEnsureAuthorized())) return;
  if (_wbActing['conn-' + id]) return;
  _wbActing['conn-' + id] = true;
  try {
    const r = await invoke('install_wb_connectors', { ids: [id], targets: _wbTargets });
    const total = (r.results || []).reduce((n, x) => n + (x.installed || []).length, 0);
    if (total > 0) showToast(`已启用 ${id} 的 ${total} 个 MCP server`, 'success');
    else if ((r.results || []).some(x => (x.conflicts || []).length)) showToast(`${id}：目标已有同名 MCP server，未覆盖`, 'error');
    else showToast(`${id}：无可安装的 MCP 定义`, 'error');
  } catch (e) {
    showToast('启用失败: ' + esc(String(e)), 'error');
  } finally {
    _wbActing['conn-' + id] = false;
    await loadWbResources();
  }
}

async function wbUninstallConnector(id) {
  if (_wbActing['conn-' + id]) return;
  if (!showConfirm(`从所选客户端移除 ${id} 的 MCP server？`)) return;
  _wbActing['conn-' + id] = true;
  try {
    const r = await invoke('uninstall_wb_connectors', { ids: [id], targets: _wbTargets });
    const total = (r.results || []).reduce((n, x) => n + (x.removed || []).length, 0);
    showToast(`已移除 ${total} 个 MCP server`, 'success');
  } catch (e) {
    showToast('移除失败: ' + esc(String(e)), 'error');
  } finally {
    _wbActing['conn-' + id] = false;
    await loadWbResources();
  }
}

async function wbInstallSkill(id) {
  if (!(await wbEnsureAuthorized())) return;
  if (_wbActing['skill-' + id]) return;
  _wbActing['skill-' + id] = true;
  try {
    const r = await invoke('install_wb_skills', { ids: [id], targets: _wbTargets });
    const total = (r.results || []).reduce((n, x) => n + (x.installed || []).length, 0);
    if (total > 0) showToast(`技能已安装到 ${total} 个客户端`, 'success');
    else showToast('技能安装失败', 'error');
  } catch (e) {
    showToast('安装失败: ' + esc(String(e)), 'error');
  } finally {
    _wbActing['skill-' + id] = false;
    await loadWbResources();
  }
}

async function wbUninstallSkill(id) {
  if (_wbActing['skill-' + id]) return;
  if (!showConfirm(`从所选客户端移除技能 ${id}？`)) return;
  _wbActing['skill-' + id] = true;
  try {
    await invoke('uninstall_wb_skills', { ids: [id], targets: _wbTargets });
    showToast('技能已移除', 'success');
  } catch (e) {
    showToast('移除失败: ' + esc(String(e)), 'error');
  } finally {
    _wbActing['skill-' + id] = false;
    await loadWbResources();
  }
}

async function wbInstallAgentsAll() {
  if (!(await wbEnsureAuthorized())) return;
  if (_wbActing['agents-all']) return;
  const stats = wbStats();
  if (!showConfirm(`生成全部 ${stats.agents || 0} 个 Custom Agents 到 ~/.codex/agents/？`)) return;
  _wbActing['agents-all'] = true;
  try {
    const r = await invoke('install_wb_agents', { ids: null, targets: _wbTargets });
    const total = (r.results || []).reduce((n, x) => n + (x.installed || []).length, 0);
    showToast(`已生成 ${total} 个 Custom Agents`, 'success');
  } catch (e) {
    showToast('生成失败: ' + esc(String(e)), 'error');
  } finally {
    _wbActing['agents-all'] = false;
    await loadWbResources();
  }
}

async function wbInstallAgent(id) {
  if (!(await wbEnsureAuthorized())) return;
  if (_wbActing['agent-' + id]) return;
  _wbActing['agent-' + id] = true;
  try {
    const r = await invoke('install_wb_agents', { ids: [id], targets: _wbTargets });
    const total = (r.results || []).reduce((n, x) => n + (x.installed || []).length, 0);
    if (total > 0) showToast(`Custom Agent 已生成到 ${total} 个客户端`, 'success');
    else showToast('生成失败', 'error');
  } catch (e) {
    showToast('生成失败: ' + esc(String(e)), 'error');
  } finally {
    _wbActing['agent-' + id] = false;
    await loadWbResources();
  }
}

async function wbUninstallAgentsAll() {
  if (_wbActing['agents-all']) return;
  if (!showConfirm('移除全部 WorkBuddy 生成的 Custom Agents？')) return;
  _wbActing['agents-all'] = true;
  try {
    const r = await invoke('uninstall_wb_agents', { ids: null, targets: _wbTargets });
    const total = (r.results || []).reduce((n, x) => n + (x.removed || []).length, 0);
    showToast(`已移除 ${total} 个 Custom Agents`, 'success');
  } catch (e) {
    showToast('移除失败: ' + esc(String(e)), 'error');
  } finally {
    _wbActing['agents-all'] = false;
    await loadWbResources();
  }
}

async function wbUninstallAgent(id) {
  if (_wbActing['agent-' + id]) return;
  if (!showConfirm(`移除 Custom Agent ${id}？`)) return;
  _wbActing['agent-' + id] = true;
  try {
    const r = await invoke('uninstall_wb_agents', { ids: [id], targets: _wbTargets });
    const total = (r.results || []).reduce((n, x) => n + (x.removed || []).length, 0);
    if (total > 0) showToast('Custom Agent 已移除', 'success');
    else showToast('未找到该 Agent 文件', 'error');
  } catch (e) {
    showToast('移除失败: ' + esc(String(e)), 'error');
  } finally {
    _wbActing['agent-' + id] = false;
    await loadWbResources();
  }
}

function wbConnectorKindLabel(kind) {
  return kind === 'mcp' ? 'MCP' : kind === 'cli' ? 'CLI' : kind === 'mixed' ? 'MCP+CLI' : '其他';
}

function wbSkillSourceLabel(source) {
  if (source === 'builtin') return '内置';
  if (source === 'market') return '官方市场';
  if (source === 'ext') return '外部市场';
  if (source === 'teams') return 'Teams';
  return '其他';
}

function wbMatchText(items) {
  const search = String(_wbSearch || '').trim().toLowerCase();
  if (!search) return items;
  return items.filter(item => {
    const haystack = [
      item.id,
      item.name,
      item.description,
      item.profession,
      item.display_name,
      item.expert_type,
      ...(item.tags || []),
      ...(item.quick_prompts || [])
    ].join(' ').toLowerCase();
    return haystack.includes(search);
  });
}

function renderWbConnectorCard(connector) {
  const kind = wbConnectorKindLabel(connector.kind);
  const servers = connector.mcp_servers || [];
  const skills = connector.skills || [];
  const expanded = _wbExpanded['conn-' + connector.id];
  const serverText = servers.length
    ? servers.map(s => {
        return `<div class="wb-detail-line"><code>${esc(s.name)}</code> <span class="wb-muted">${esc(s.type)}</span></div>`;
      }).join('')
    : '<div class="wb-muted">无 MCP server 定义</div>';
  const skillText = skills.length
    ? skills.map(s => `<span class="wb-chip">${esc(s.name || s.id)}</span>`).join('')
    : '<span class="wb-muted">无配套技能</span>';
  const installed = wbConnectorInstalled(connector.id);
  const iconHtml = wbIconHtml('connectors-icons/' + (connector.icon && connector.icon !== 'letter' ? connector.icon : ''), connector.name);
  const installedBadge = installed.length
    ? `<span class="wb-chip wb-chip-ok">✓ 已装：${installed.map(wbTargetLabel).join(' / ')}</span>`
    : '';
  const actionBtn = installed.length
    ? `<button class="btn btn-ghost btn-sm" onclick="wbUninstallConnector('${escAttr(connector.id)}')">移除 MCP</button>`
    : `<button class="btn btn-primary btn-sm" onclick="wbInstallConnector('${escAttr(connector.id)}')">启用 MCP</button>`;
  return `<div class="wb-card" id="wb-card-conn-${escAttr(connector.id)}">
    <div class="wb-card-head" onclick="toggleWbDetail('conn-${escAttr(connector.id)}')">
      ${iconHtml}
      <div class="wb-card-title"><strong>${esc(connector.name)}</strong></div>
      <div class="wb-card-meta">
        <button type="button" class="wb-plus" title="复制使用说明" onclick="event.stopPropagation();wbCopyResource('connector','${escAttr(connector.id)}')">+</button>
        ${wbCaretHtml(expanded)}
      </div>
    </div>
    <div class="wb-card-desc">${esc(connector.description || '')}</div>
    ${expanded ? `<div class="wb-card-body">
      <div class="wb-section-label">类型</div>
      <div class="wb-chips"><span class="wb-chip">${esc(kind)}</span></div>
      <div class="wb-section-label">MCP Servers</div>
      ${serverText}
      <div class="wb-section-label">配套技能</div>
      <div class="wb-chips">${skillText}</div>
      ${installedBadge}
      <div class="wb-actions">${actionBtn}</div>
    </div>` : ''}
  </div>`;
}

function renderWbSkillCard(skill) {
  const expanded = _wbExpanded['skill-' + skill.id];
  const displayName = skill.display_name || skill.name;
  const iconHtml = wbIconHtml('skills-icons/' + (skill.icon && skill.icon !== 'letter' ? skill.icon : ''), displayName);
  const examples = (skill.examples || []).slice(0, 3).map(e => `<div class="wb-detail-line">· ${esc(e)}</div>`).join('');
  const installed = wbSkillInstalled(skill.id);
  const installedBadge = installed.length
    ? `<span class="wb-chip wb-chip-ok">✓ 已装：${installed.map(wbTargetLabel).join(' / ')}</span>`
    : '';
  const actionBtn = installed.length
    ? `<button class="btn btn-ghost btn-sm" onclick="wbUninstallSkill('${escAttr(skill.id)}')">移除</button>`
    : `<button class="btn btn-primary btn-sm" onclick="wbInstallSkill('${escAttr(skill.id)}')">安装</button>`;
  return `<div class="wb-card" id="wb-card-skill-${escAttr(skill.id)}">
    <div class="wb-card-head" onclick="toggleWbDetail('skill-${escAttr(skill.id)}')">
      ${iconHtml}
      <div class="wb-card-title">
        <strong>${esc(displayName)}</strong>
      </div>
      <div class="wb-card-meta">
        <button type="button" class="wb-plus" title="复制使用说明" onclick="event.stopPropagation();wbCopyResource('skill','${escAttr(skill.id)}')">+</button>
        ${wbCaretHtml(expanded)}
      </div>
    </div>
    <div class="wb-card-desc">${esc(skill.description || '（无描述）')}</div>
    ${expanded ? `<div class="wb-card-body">
      ${examples ? `<div class="wb-section-label">试试这样问</div>${examples}` : ''}
      ${installedBadge}<div class="wb-actions">${actionBtn}</div>
    </div>` : ''}
  </div>`;
}

function renderWbAgentCard(agent) {
  const expanded = _wbExpanded['agent-' + agent.id];
  const typeBadge = agent.expert_type === 'team' ? `<span class="wb-badge agent">专家团</span>` : '';
  const tags = (agent.tags || []).map(t => `<span class="wb-chip">${esc(t)}</span>`).join('');
  const prompts = (agent.quick_prompts || []).map(p => `<div class="wb-detail-line">· ${esc(p)}</div>`).join('');
  const files = (agent.agent_files || []).map(f => `<div class="wb-muted">${esc(f)}</div>`).join('');
  const installed = wbAgentInstalled(agent.id);
  let avatarHtml = '';
  if (agent.avatar) {
    const name = agent.avatar.split('/').pop();
    const rel = 'agents/' + agent.id + '/avatars/' + name;
    const url = wbAssetUrl(rel);
    if (url) avatarHtml = `<img class="wb-avatar" src="${url}" data-wb-rel="${escAttr(rel)}" alt="" onerror="wbAssetError(this)">`;
  }
  const installedBadge = installed.length
    ? `<span class="wb-chip wb-chip-ok">✓ 已装：${installed.map(wbTargetLabel).join(' / ')}</span>`
    : '';
  const actionBtn = installed.length
    ? `<button class="btn btn-ghost btn-sm" onclick="wbUninstallAgent('${escAttr(agent.id)}')">移除 Agent</button>`
    : `<button class="btn btn-primary btn-sm" onclick="wbInstallAgent('${escAttr(agent.id)}')">生成 Agent</button>`;
  return `<div class="wb-card wb-agent-card" id="wb-card-agent-${escAttr(agent.id)}">
    <div class="wb-card-head" onclick="toggleWbDetail('agent-${escAttr(agent.id)}')">
      ${avatarHtml || wbIconHtml('', agent.display_name || agent.name)}
      <div class="wb-card-title">
        ${typeBadge}
        <div class="wb-agent-titles">
          <strong>${esc(agent.profession || agent.display_name || agent.name)}</strong>
          <span class="wb-muted">${esc(agent.display_name && agent.profession ? agent.display_name : '')}</span>
        </div>
      </div>
      <div class="wb-card-meta">
        ${(agent.skills && agent.skills.length > 0) ? `<span>${agent.skills.length} 技能</span>` : ''}
        <button type="button" class="wb-plus" title="复制使用说明" onclick="event.stopPropagation();wbCopyResource('agent','${escAttr(agent.id)}')">+</button>
        ${wbCaretHtml(expanded)}
      </div>
    </div>
    <div class="wb-card-desc">${esc(agent.description || '')}</div>
    ${tags ? `<div class="wb-chips">${tags}</div>` : ''}
    ${expanded ? `<div class="wb-card-body">
      <div class="wb-section-label">试试这样问</div>
      ${prompts || '<div class="wb-muted">（无推荐提示词）</div>'}
      ${installedBadge}
      <div class="wb-actions">${actionBtn}</div>
    </div>` : ''}
  </div>`;
}

function renderWbSection(title, count, cards) {
  if (!cards.length) return '';
  return `<div class="wb-section">
    <div class="wb-section-head"><h3>${esc(title)}</h3><em>${count}</em>
      ${title === 'Custom Agents' ? `<span class="wb-section-actions">
        <button class="btn btn-primary btn-sm" onclick="wbInstallAgentsAll()">生成全部 Custom Agents</button>
        <button class="btn btn-ghost btn-sm" onclick="wbUninstallAgentsAll()">移除全部</button>
      </span>` : ''}
    </div>
    <div class="wb-grid">${cards.join('')}</div>
  </div>`;
}

function renderWbResourceList() {
  const el = document.getElementById('wbResourceList');
  if (!el) return;
  if (_wbLoading && !_wbData) {
    el.innerHTML = '<div class="empty-state">正在加载 WorkBuddy 资源…</div>';
    return;
  }
  if (_wbError) {
    el.innerHTML = `<div class="empty-state">加载失败：${esc(_wbError)}</div>`;
    return;
  }
  const data = _wbData || { connectors: [], skills: [], agents: [] };
  const connectors = wbMatchText(data.connectors || []).filter(wbConnectorInCategory);
  const skills = wbMatchText(data.skills || []).filter(wbSkillInCategory);
  const agents = wbMatchText(data.agents || []).filter(wbAgentInCategory);

  if (!connectors.length && !skills.length && !agents.length) {
    el.innerHTML = '<div class="empty-state">没有匹配的资源</div>';
    return;
  }

  // 专区页（Skill/MCP/深度研究）只显示卡片网格，不渲染 section 标题
  const zoneMode = _pluginZone === 'skill' || _pluginZone === 'mcp' || _pluginZone === 'agents';
  if (zoneMode) {
    let cards = [];
    let total = 0;
    if (_wbSection === 'skills') {
      cards = skills.slice(0, _wbVisibleCount).map(renderWbSkillCard);
      total = skills.length;
    } else if (_wbSection === 'connectors') {
      cards = connectors.slice(0, _wbVisibleCount).map(renderWbConnectorCard);
      total = connectors.length;
    } else if (_wbSection === 'agents') {
      cards = agents.slice(0, _wbVisibleCount).map(renderWbAgentCard);
      total = agents.length;
    }
    if (!cards.length) {
      el.innerHTML = '<div class="empty-state">该分类下没有资源</div>';
      return;
    }
    const shown = cards.slice(0, _wbVisibleCount).join('');
    const more = total > _wbVisibleCount
      ? `<div class="wb-load-more"><button class="btn btn-ghost" onclick="wbLoadMore()">加载更多（还剩 ${total - _wbVisibleCount}）</button></div>`
      : '';
    el.innerHTML = `<div class="wb-grid">${shown}</div>${more}`;
    return;
  }

  let html = '';
  if (_wbSection === 'all' || _wbSection === 'connectors') {
    html += renderWbSection('MCP 连接器', connectors.length, connectors.map(renderWbConnectorCard));
  }
  if (_wbSection === 'all' || _wbSection === 'skills') {
    html += renderWbSection('技能', skills.length, skills.map(renderWbSkillCard));
  }
  if (_wbSection === 'all' || _wbSection === 'agents') {
    html += renderWbSection('Custom Agents', agents.length, agents.map(renderWbAgentCard));
  }
  el.innerHTML = html || '<div class="empty-state">该分类下没有资源</div>';
}

function wbLoadMore() {
  _wbVisibleCount += 60;
  // 插件市场面板的滚动容器是 .primary-page-shell（.main 在该面板下 overflow:hidden）
  const scroller = document.querySelector('.primary-page-shell') || document.getElementById('mainContent');
  const scrollTop = scroller ? scroller.scrollTop : 0;
  renderWbResourceList();
  if (scroller) scroller.scrollTop = scrollTop;
}

function wbResetPagination() {
  _wbVisibleCount = 60;
}

function wbSkillInCategory(s) {
  if (_wbZoneCategory === 'all') return true;
  return (s.categories || []).includes(_wbZoneCategory);
}

function wbAgentInCategory(a) {
  if (_wbZoneCategory === 'all') return true;
  return a.category === _wbZoneCategory;
}

function wbConnectorInCategory(c) {
  if (_wbZoneCategory === 'all') return true;
  return (c.kind || 'mcp') === _wbZoneCategory;
}

function renderWbZoneCategories(kind) {
  const data = _wbData || {};
  const counts = { all: 0 };
  const labelMap = kind === 'skills' ? WB_SKILL_CATEGORY_LABEL
    : kind === 'agents' ? WB_AGENT_CATEGORY_LABEL : WB_CONNECTOR_KIND_LABEL;
  const list = kind === 'skills' ? (data.skills || [])
    : kind === 'agents' ? (data.agents || []) : (data.connectors || []);
  counts.all = list.length;
  list.forEach(item => {
    const cats = kind === 'skills' ? (item.categories || [])
      : kind === 'agents' ? [item.category] : [item.kind];
    cats.forEach(c => { if (c && labelMap[c]) counts[c] = (counts[c] || 0) + 1; });
  });
  const keys = Object.keys(counts).filter(k => k === 'all' || counts[k] > 0);
  keys.sort((a, b) => {
    if (a === 'all') return -1;
    if (b === 'all') return 1;
    return (labelMap[a] || a).localeCompare(labelMap[b] || b, 'zh-Hans-CN');
  });
  return `<div class="plugin-category-tabs" id="wbZoneCats" role="tablist" aria-label="资源分类">
    ${keys.map(key => `<button type="button" class="${_wbZoneCategory === key ? 'active' : ''}" onclick="setWbZoneCategory('${escAttr(key)}')" role="tab" aria-selected="${_wbZoneCategory === key ? 'true' : 'false'}">
      <span>${esc(labelMap[key] || key)}</span><em>${counts[key]}</em>
    </button>`).join('')}
  </div>`;
}

function setWbZoneCategory(cat) {
  _wbZoneCategory = cat || 'all';
  _wbVisibleCount = 60;
  const cats = document.getElementById('wbZoneCats');
  if (cats) cats.outerHTML = renderWbZoneCategories(_wbSection === 'connectors' ? 'connectors' : _wbSection === 'agents' ? 'agents' : 'skills');
  renderWbResourceList();
}

function renderWbSectionTabs() {
  const stats = wbStats();
  const sections = [
    { key: 'all', label: '全部', count: stats.connectors + stats.skills + stats.agents },
    { key: 'connectors', label: 'MCP 连接器', count: stats.connectors },
    { key: 'skills', label: '技能', count: stats.skills },
    { key: 'agents', label: 'Custom Agents', count: stats.agents }
  ];
  return `<div class="plugin-category-tabs" role="tablist" aria-label="WorkBuddy 资源分类">
    ${sections.map(item => `<button type="button" class="${_wbSection === item.key ? 'active' : ''}" onclick="setWbSection('${item.key}')" role="tab" aria-selected="${_wbSection === item.key ? 'true' : 'false'}">
      <span>${esc(item.label)}</span><em>${item.count}</em>
    </button>`).join('')}
  </div>`;
}

function setWbSection(key) {
  _wbSection = key || 'all';
  _wbVisibleCount = 60;
  const tabs = document.querySelector('.plugin-category-tabs');
  if (tabs) tabs.outerHTML = renderWbSectionTabs();
  renderWbResourceList();
}

function setWbSearch(value) {
  _wbSearch = value || '';
  _wbVisibleCount = 60;
  renderWbResourceList();
}

function setWbTarget(value, checked) {
  if (checked) {
    if (!_wbTargets.includes(value)) _wbTargets.push(value);
  } else {
    _wbTargets = _wbTargets.filter(t => t !== value);
  }
  const bar = document.getElementById('wbTargetBar');
  if (bar) bar.outerHTML = renderWbTargetBar();
  renderWbResourceList();
}

function renderWbTargetBar() {
  const options = [
    { key: 'codex', label: 'Codex' },
    { key: 'claude', label: 'Claude Code' },
    { key: 'opencode', label: 'opencode' }
  ];
  return `<div class="wb-target-bar" id="wbTargetBar">
    <span class="wb-target-label">安装目标：</span>
    ${options.map(o => `<label class="wb-target-check"><input type="checkbox" ${_wbTargets.includes(o.key) ? 'checked' : ''} onchange="setWbTarget('${o.key}', this.checked)">${esc(o.label)}</label>`).join('')}
  </div>`;
}

function toggleWbDetail(id) {
  _wbExpanded[id] = !_wbExpanded[id];
  // 局部更新目标卡片：避免整列表重渲染导致滚动位置跳到顶部。
  const target = document.getElementById('wb-card-' + id);
  if (!target) {
    renderWbResourceList();
    return;
  }
  let html = '';
  if (id.indexOf('conn-') === 0) {
    const item = wbFindResource('connector', id.slice(5));
    if (item) html = renderWbConnectorCard(item);
  } else if (id.indexOf('skill-') === 0) {
    const item = wbFindResource('skill', id.slice(6));
    if (item) html = renderWbSkillCard(item);
  } else if (id.indexOf('agent-') === 0) {
    const item = wbFindResource('agent', id.slice(6));
    if (item) html = renderWbAgentCard(item);
  }
  if (html) {
    target.outerHTML = html;
  } else {
    renderWbResourceList();
  }
}

function renderPluginsWbPanel() {
  const stats = wbStats();
  const root = (_wbData && _wbData.root) || '';
  return `<div class="page-header">
    <h2>WorkBuddy 资源</h2>
  </div>
  <div class="plugin-console">
    ${renderWbSectionTabs()}
    <div class="plugin-filter-bar">
      <input id="wbSearchInput" value="${escAttr(_wbSearch)}" placeholder="搜索连接器 / 技能 / 专家" oninput="setWbSearch(this.value)">
    </div>
    ${renderWbTargetBar()}
    <div class="plugin-controls">
      <button class="btn btn-ghost" onclick="refreshWbResources()">刷新</button>
    </div>
  </div>
  <div id="wbResourceList" class="wb-resource-list">
    <div class="empty-state">${_wbLoading ? '正在加载 WorkBuddy 资源…' : '暂无数据，请先运行 scripts/import-workbuddy-resources.sh 提取资源'}</div>
  </div>
  <div class="wb-footnote wb-muted">数据来源：${esc(root || '~/.deecodex/wb-resources')}（由 WorkBuddy 提取，只读展示）</div>`;
}
