var DEX_TOOLS = [];
var DEX_CAPABILITIES = [];
var DEX_WORKSPACE_CONTEXT = null;
var DEX_MODE = (window.deeStorage && window.deeStorage.getItem('dex_mode')) || 'diag';
var DEX_LAST_ATTACHMENT_KEY = 'dex_last_attachment_path';

var DEX_MODES = {
  diag: { label: '诊断', prompt: '优先使用只读诊断工具定位问题，先给风险概览，再建议修复路径。' },
  fix: { label: '修复', prompt: '先诊断，再执行 L2 修复；L3 必须通过前端内联确认。每次修复后复核结果。' },
  workspace: { label: '工作区', prompt: '聚焦当前项目上下文、Git 状态、配置文件、CLI 版本、端口和受限文件读取。' },
  plugins: { label: '插件', prompt: '聚焦插件状态、插件账号、插件暴露给 DEX 的工具和插件 JSON-RPC 错误。' },
  cost: { label: '成本', prompt: '聚焦请求历史、Token 消耗、缓存命中、模型分布、失败率和成本估算。' },
  logs: { label: '日志', prompt: '聚焦日志异常、最近错误、关键栈信息和可复现线索。' }
};

var DEX_SYSTEM_PROMPT = [
  '你是 DEX助手 3.0，一个 AI工具链工作台 Agent，运行在 deecodex GUI 内置的 DEX助手 面板中。',
  '你的范围聚焦 Codex、Claude Code、OpenClaw、Hermes、MCP、模型账号、日志、请求历史、线程、插件和项目环境诊断。',
  'deecodex 是你的默认能力包之一，但你不只服务于 deecodex。',
  '',
  '## 核心原则',
  '1. 工具优先：始终优先调用工具获取实时数据，不要猜测系统状态。',
  '2. 安全分级：L0/L1 可直接执行；L2 执行后报告结果；L3 由前端统一弹出内联确认。',
  '3. 验证结果：每次操作后检查返回结果，必要时用只读工具复核。',
  '4. 失败不重复：同一工具同一参数失败后，换思路分析原因。',
  '5. 回复精简：中文回复，先说结论，再给必要细节。',
  '',
  '## 安全边界',
  '- 文件读取限制在 ~/.deecodex、~/.codex、/tmp 和当前工作区。',
  '- Shell 属于 L3；用户明确要求执行时，直接发起 execute_shell 工具调用，由前端内联确认，不要用普通文本确认代替。',
  '- 用户消息包含“附件:”路径时，优先使用 read_file 工具读取附件内容，再基于真实文件内容回答。',
  '- 不泄露 api_key、authorization、token、secret、password 等敏感信息。',
  '- 插件工具通过统一代理执行，遵循同一安全分级。'
].join('\n');

var DEX_CAP_LABELS = {
  'core.system': '系统',
  'core.workspace': '工作区',
  'ai.codex': 'Codex',
  'ai.claude': 'Claude',
  'ai.openclaw': 'OpenClaw',
  'ai.hermes': 'Hermes',
  'ai.mcp': 'MCP',
  'deecodex.ops': 'deecodex',
  'plugins.dynamic': '插件'
};

function dexCapabilityLabel(id) {
  if (!id) return '工具';
  return DEX_CAP_LABELS[id] || id;
}

function dexToolSourceLabel(toolDef) {
  if (!toolDef) return '工具';
  if (toolDef.source === 'plugin') return '插件';
  return dexCapabilityLabel(toolDef.capability || toolDef.source);
}

function dexBuildSystemPrompt() {
  var caps = (DEX_CAPABILITIES || []).filter(function(c) { return c.enabled; }).map(function(c) {
    return '- ' + c.id + '：' + c.label + '（' + (c.tool_count || 0) + ' 个工具）';
  }).join('\n') || '- 使用默认能力包';
  var ctx = DEX_WORKSPACE_CONTEXT || {};
  var mode = DEX_MODES[DEX_MODE] || DEX_MODES.diag;
  var ctxLines = [];
  if (ctx.cwd) ctxLines.push('- 当前工作区: ' + ctx.cwd);
  if (ctx.project_types && ctx.project_types.length) ctxLines.push('- 项目类型: ' + ctx.project_types.join(', '));
  if (ctx.config_files && ctx.config_files.length) ctxLines.push('- 关键文件: ' + ctx.config_files.join(', '));
  if (ctx.git && ctx.git.is_repo) ctxLines.push('- Git: ' + (ctx.git.branch || '未知分支') + '，未提交变更 ' + (ctx.git.dirty_count || 0) + ' 项');
  if (ctx.active_account) ctxLines.push('- 活跃账号: ' + (ctx.active_account.name || ctx.active_account.provider || '未知') + ' / ' + (ctx.active_account.provider || 'custom'));
  var toolLines = (DEX_TOOLS || []).slice(0, 80).map(function(t) {
    return '- ' + t.name + ' [L' + (t.level || 0) + ' / ' + dexToolSourceLabel(t) + ' / ' + (t.capability || 'core') + ']：' + (t.description || '');
  });
  if ((DEX_TOOLS || []).length > toolLines.length) {
    toolLines.push('- 另有 ' + ((DEX_TOOLS || []).length - toolLines.length) + ' 个工具，按需从工具清单调用。');
  }
  return [
    DEX_SYSTEM_PROMPT,
    '',
    '## 当前启用能力包',
    caps,
    '',
    '## 当前模式',
    '- ' + mode.label + '模式：' + mode.prompt,
    '',
    '## 工作区上下文',
    ctxLines.join('\n') || '- 暂无工作区上下文',
    '',
    '## 当前可用工具',
    toolLines.join('\n') || '- 工具清单尚未加载；请等待后端动态注册表返回。',
    '',
    '## DEX 2.0 工具调用规则',
    '- 工具定义来自后端动态注册表，工具可能来自内置能力包或插件。',
    '- 你必须优先调用工具获取实时状态，尤其是诊断、配置、日志、线程、插件和工作区问题。',
    '- L3 操作必须发起对应工具调用，由前端显示内联确认；不要先用普通文本询问“是否确认”。',
    '- 不要泄露 api_key、authorization、token、secret、password 等敏感信息。',
  ].join('\n');
}

function dexNormalizePluginArgs(args) {
  var normalized = Object.assign({}, args || {});
  if (normalized.plugin_id !== undefined && normalized.pluginId === undefined) {
    normalized.pluginId = normalized.plugin_id;
    delete normalized.plugin_id;
  }
  if (normalized.account_id !== undefined && normalized.accountId === undefined) {
    normalized.accountId = normalized.account_id;
    delete normalized.account_id;
  }
  if (normalized.plugin_path !== undefined && normalized.path === undefined) {
    normalized.path = normalized.plugin_path;
    delete normalized.plugin_path;
  }
  if (normalized.archivePath !== undefined && normalized.path === undefined) {
    normalized.path = normalized.archivePath;
    delete normalized.archivePath;
  }
  if (normalized.config_json !== undefined && normalized.config === undefined) {
    if (typeof normalized.config_json === 'string') {
      try {
        normalized.config = JSON.parse(normalized.config_json);
      } catch (e) {
        normalized.config = normalized.config_json;
      }
    } else {
      normalized.config = normalized.config_json;
    }
    delete normalized.config_json;
  }
  return normalized;
}

async function dexLoadDynamicContext() {
  try {
    var result = await Promise.all([
      DeeCodexTauri.invoke('dex_list_capabilities', {}),
      DeeCodexTauri.invoke('dex_list_tools', {}),
      DeeCodexTauri.invoke('dex_get_workspace_context', {})
    ]);
    DEX_CAPABILITIES = result[0] || [];
    if (Array.isArray(result[1])) {
      DEX_TOOLS = result[1];
    }
    DEX_WORKSPACE_CONTEXT = result[2] || null;
    window.dexAgent.messages[0] = { role: 'system', content: dexBuildSystemPrompt() };
  } catch (e) {
    DEX_TOOLS = [];
    console.warn('[dexAgent] 动态工具加载失败，当前不暴露工具清单:', e);
  }
}

function dexRenderCapabilityChips() {
  var el = document.getElementById('dexCapabilityChips');
  if (!el) return;
  if (!DEX_CAPABILITIES || !DEX_CAPABILITIES.length) {
    el.innerHTML = '<div class="dex-cap-summary"><span class="dex-cap-label">工具包</span><span>默认能力</span></div>';
    return;
  }
  var enabledCount = DEX_CAPABILITIES.filter(function(c) { return c.enabled; }).length;
  var toolCount = DEX_CAPABILITIES.reduce(function(sum, c) { return sum + (c.tool_count || 0); }, 0);
  var chips = DEX_CAPABILITIES.map(function(c) {
    var cls = c.enabled ? 'dex-cap-chip on' : 'dex-cap-chip off';
    var state = c.enabled ? '已启用' : '已停用';
    return '<button class="' + cls + '" onclick="dexToggleCapability(\'' + escAttr(c.id) + '\',' + (!c.enabled) + ')" title="' + escAttr(c.description || '') + '">'
      + '<span class="dex-cap-chip-name">' + esc(c.label || dexCapabilityLabel(c.id)) + '</span>'
      + '<span class="dex-cap-chip-meta">' + state + ' · ' + (c.tool_count || 0) + ' 工具</span>'
      + '</button>';
  }).join('');
  el.innerHTML = '<div class="dex-cap-summary"><span class="dex-cap-label">工具包</span><span>' + enabledCount + '/' + DEX_CAPABILITIES.length + ' 已启用 · ' + toolCount + ' 个工具</span></div>'
    + '<details class="dex-cap-panel"><summary>管理</summary><div class="dex-cap-grid">' + chips + '</div></details>';
}

function dexRenderModeTabs() {
  var el = document.getElementById('dexModeTabs');
  if (!el) return;
  el.innerHTML = Object.keys(DEX_MODES).map(function(id) {
    var mode = DEX_MODES[id];
    var cls = id === DEX_MODE ? 'dex-mode-tab active' : 'dex-mode-tab';
    return '<button class="' + cls + '" onclick="dexSetMode(\'' + escAttr(id) + '\')" title="' + escAttr(mode.prompt) + '">' + esc(mode.label) + '</button>';
  }).join('');
}

function dexSetMode(mode) {
  if (!DEX_MODES[mode]) mode = 'diag';
  DEX_MODE = mode;
  if (window.deeStorage) window.deeStorage.setItem('dex_mode', mode);
  if (window.dexAgent && window.dexAgent.messages.length) {
    window.dexAgent.messages[0] = { role: 'system', content: dexBuildSystemPrompt() };
  }
  dexRenderModeTabs();
  showToast('DEX模式：' + DEX_MODES[mode].label, 'success');
}

function dexRenderToolCatalog() {
  var el = document.getElementById('dexToolCatalog');
  if (!el) return;
  if (!DEX_TOOLS || !DEX_TOOLS.length) {
    el.innerHTML = '<details class="dex-tool-catalog"><summary>工具清单：0 个</summary><div class="dex-tool-catalog-list"></div></details>';
    return;
  }
  var html = '<details class="dex-tool-catalog"><summary>工具清单：' + DEX_TOOLS.length + ' 个（按需展开）</summary><div class="dex-tool-catalog-list">';
  for (var i = 0; i < DEX_TOOLS.length; i++) {
    var t = DEX_TOOLS[i];
    html += '<div class="dex-tool-catalog-item">'
      + '<span class="dex-tool-catalog-name">' + esc(t.name) + '</span>'
      + '<span class="dex-tool-catalog-meta">L' + (t.level || 0) + ' · ' + esc(dexToolSourceLabel(t)) + ' · ' + esc(t.capability || 'core') + '</span>'
      + '<span class="dex-tool-catalog-meta">' + esc(t.description || '') + '</span>'
      + '</div>';
  }
  html += '</div></details>';
  el.innerHTML = html;
}

async function dexToggleCapability(id, enabled) {
  try {
    await DeeCodexTauri.invoke('dex_update_capability_state', { capabilityId: id, capability_id: id, enabled: enabled });
    await dexLoadDynamicContext();
    window.dexAgent.messages[0] = { role: 'system', content: dexBuildSystemPrompt() };
    showToast((enabled ? '已启用 ' : '已停用 ') + id, 'success');
  } catch (e) {
    showToast('能力包切换失败: ' + (e.message || e), 'error');
  }
}
