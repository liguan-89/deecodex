function dexToolMetaHtml(meta) {
  if (!meta) return '';
  var parts = [];
  if (meta.toolDef) {
    parts.push('L' + (meta.toolDef.level || 0));
    parts.push(dexToolSourceLabel(meta.toolDef));
    parts.push(meta.toolDef.capability || 'core');
  }
  if (meta.elapsedMs !== undefined) parts.push(meta.elapsedMs + 'ms');
  if (!parts.length) return '';
  return '<span class="dex-tool-badge">' + esc(parts.join(' · ')) + '</span>';
}

function dexToolActionHtml(fnName, data) {
  if (fnName === 'list_request_history' || fnName === 'analyze_requests' || fnName === 'token_cost') {
    return '<div class="dex-tool-actions"><button class="btn btn-ghost btn-sm" onclick="switchPanel(\'sessions\')">打开请求历史</button></div>';
  }
  if (fnName === 'list_plugins' || fnName.indexOf('plugin') >= 0) {
    return '<div class="dex-tool-actions"><button class="btn btn-ghost btn-sm" onclick="switchPanel(\'plugins\')">打开插件市场</button></div>';
  }
  if (fnName === 'get_threads_status' || fnName === 'list_threads') {
    return '<div class="dex-tool-actions"><button class="btn btn-ghost btn-sm" onclick="switchPanel(\'threads\')">打开线程面板</button></div>';
  }
  return '';
}

function dexAppendMessage(type, content, meta) {
  var container = document.getElementById('dexMessages');
  if (!container) return null;
  if (type === 'assistant' || type === 'user' || type === 'tool-start') {
    var details = container.querySelectorAll('details[open]');
    for (var d = 0; d < details.length; d++) details[d].open = false;
  }
  var el = document.createElement('div');
  el.className = 'dex-msg dex-msg-' + type;
  var isHistory = meta && meta.history;

  switch (type) {
    case 'user':
      el.innerHTML = '<div class="dex-bubble dex-bubble-user"><div class="dex-bubble-text">' + esc(content) + '</div></div>';
      break;
    case 'assistant':
      var modelTag = (meta && meta.model && meta.model !== 'auto') ? '<span class="dex-model-tag">' + esc(meta.model) + '</span>' : '';
      el.innerHTML = '<div class="dex-bubble dex-bubble-assistant">'
        + modelTag
        + '<div class="dex-reasoning-wrap" style="display:none"><details class="dex-reasoning"><summary>思考过程</summary><div class="dex-reasoning-content"></div></details></div>'
        + '<div class="dex-bubble-text">' + dexRenderMarkdown(content) + '</div></div>';
      break;
    case 'system':
      el.innerHTML = '<div class="dex-system-msg">' + esc(content) + '</div>';
      break;
    case 'tool-start':
      var toolMeta = dexToolMetaHtml(meta);
      el.innerHTML = '<div class="dex-tool-msg dex-tool-start">'
        + (isHistory ? '<span class="dex-tool-icon dex-tool-icon-history" aria-hidden="true"></span>' : '<span class="dex-spinner"></span>')
        + '<span class="dex-tool-name">' + esc(content) + '</span>'
        + toolMeta
        + (meta && meta.args && Object.keys(meta.args).length > 0 ? ' <span class="dex-tool-args">' + esc(JSON.stringify(meta.args)) + '</span>' : '')
        + '</div>';
      break;
    case 'tool-result':
      var summary = dexToolSummary(content, meta && meta.result ? meta.result : null);
      var rawData = meta && meta.result ? (meta.result.data !== undefined ? meta.result.data : meta.result) : null;
      var detailText = rawData ? dexFormatResultText(content, rawData) : '';
      var actionHtml = dexToolActionHtml(content, rawData);
      el.innerHTML = '<div class="dex-tool-msg dex-tool-result">'
        + '<span class="dex-tool-icon dex-tool-icon-result" aria-hidden="true"></span>'
        + '<span class="dex-tool-name">' + esc(content) + '</span>'
        + dexToolMetaHtml(meta)
        + '<span class="dex-tool-summary">' + esc(summary) + '</span>'
        + (detailText ? '<details class="dex-tool-details"><summary>详情</summary><pre>' + esc(detailText) + '</pre></details>' : '')
        + actionHtml
        + '</div>';
      break;
    case 'tool-error':
      var errMsg = content || '未知错误';
      var errDetail = (meta && meta.error) ? String(meta.error) : '';
      el.innerHTML = '<div class="dex-tool-msg dex-tool-error">'
        + '<span class="dex-tool-icon dex-tool-icon-error" aria-hidden="true"></span>'
        + '<span class="dex-tool-name">' + esc(errMsg) + '</span>'
        + (errDetail ? '<details class="dex-tool-details dex-tool-error-details"><summary>错误详情</summary><pre>' + esc(errDetail) + '</pre></details>' : '')
        + '</div>';
      break;
    default:
      el.textContent = content;
  }
  container.appendChild(el);
  dexScrollToBottom();
  return el;
}

function dexUpdateMessage(el, type, content, meta) {
  if (!el) return;
  switch (type) {
    case 'tool-result':
      var summary = dexToolSummary(content, meta && meta.result ? meta.result : null);
      var rawData = meta && meta.result ? (meta.result.data !== undefined ? meta.result.data : meta.result) : null;
      var detailText = rawData ? dexFormatResultText(content, rawData) : '';
      var actionHtml = dexToolActionHtml(content, rawData);
      el.className = 'dex-msg dex-msg-tool-result';
      el.innerHTML = '<div class="dex-tool-msg dex-tool-result">'
        + '<span class="dex-tool-icon dex-tool-icon-result" aria-hidden="true"></span>'
        + '<span class="dex-tool-name">' + esc(content) + '</span>'
        + dexToolMetaHtml(meta)
        + '<span class="dex-tool-summary">' + esc(summary) + '</span>'
        + (detailText ? '<details class="dex-tool-details"><summary>详情</summary><pre>' + esc(detailText) + '</pre></details>' : '')
        + actionHtml
        + '</div>';
      break;
    case 'tool-error':
      var errMsg = content || '未知错误';
      var errDetail = (meta && meta.error) ? String(meta.error) : '';
      el.className = 'dex-msg dex-msg-tool-error';
      el.innerHTML = '<div class="dex-tool-msg dex-tool-error">'
        + '<span class="dex-tool-icon dex-tool-icon-error" aria-hidden="true"></span>'
        + '<span class="dex-tool-name">' + esc(errMsg) + '</span>'
        + dexToolMetaHtml(meta)
        + (errDetail ? '<details class="dex-tool-details dex-tool-error-details"><summary>错误详情</summary><pre>' + esc(errDetail) + '</pre></details>' : '')
        + '</div>';
      break;
    case 'tool-start':
      var toolMeta = dexToolMetaHtml(meta);
      el.className = 'dex-msg dex-msg-tool-start';
      el.innerHTML = '<div class="dex-tool-msg dex-tool-start">'
        + '<span class="dex-spinner"></span>'
        + '<span class="dex-tool-name">' + esc(content) + '</span>'
        + toolMeta
        + (meta && meta.args && Object.keys(meta.args).length > 0 ? ' <span class="dex-tool-args">' + esc(JSON.stringify(meta.args)) + '</span>' : '')
        + '</div>';
      break;
  }
}

function dexFormatResultText(fnName, data) {
  if (fnName === 'get_logs' && Array.isArray(data)) {
    return data.map(function(l) { return dexStripAnsi(l); }).join('\n');
  }
  if (fnName === 'search_logs' && data && Array.isArray(data.results)) {
    return data.results.map(function(r) {
      return 'L' + r.line_number + ': ' + dexStripAnsi(r.line || r);
    }).join('\n');
  }
  try {
    var str = JSON.stringify(dexMaskApiKey(dexStripAnsi(data)), null, 2);
    return str.length > 2000 ? str.substring(0, 2000) + '\n…(已截断)' : str;
  } catch (e) {
    return String(data || '');
  }
}

function dexToolSummary(fnName, result) {
  if (!result) return '完成';
  var data = (result.data !== undefined) ? result.data : result;
  switch (fnName) {
    case 'run_full_diagnostics': case 'run_diagnostics':
      if (data && data.summary) return (data.summary.pass||0)+'通过 '+(data.summary.warn||0)+'警告 '+(data.summary.fail||0)+'失败'; break;
    case 'list_accounts': if (Array.isArray(data)) return data.length + ' 个账号'; break;
    case 'get_active_account': if (data && data.provider) return data.provider; break;
    case 'get_service_status': if (data && data.running) return '运行中 :' + data.port; return '已停止';
    case 'fetch_balance': if (data) { var b = data.balance || data.total_balance || data.remaining; if (b) return b; } return '已查询';
    case 'get_config': return '已获取';
    case 'get_logs': if (Array.isArray(data)) return data.length + ' 行'; if (typeof data === 'string') return data.split('\n').length + ' 行'; break;
    case 'search_logs': if (data && data.matches !== undefined) return data.matches + ' 处匹配'; break;
    case 'list_sessions': if (Array.isArray(data)) return data.length + ' 个会话'; break;
    case 'list_threads': if (Array.isArray(data)) return data.length + ' 个线程'; break;
    case 'list_plugins': if (Array.isArray(data)) return data.length + ' 个插件'; break;
    case 'list_request_history': if (Array.isArray(data)) return data.length + ' 条'; break;
    case 'test_upstream_connectivity': if (data && data.ok !== undefined) return data.ok ? '连通 ' + (data.latency_ms || '') + 'ms' : '失败'; break;
    case 'check_upgrade': if (data && data.latest) return data.latest; break;
    case 'get_threads_status': if (data && data.total !== undefined) return data.total + ' 个线程'; break;
    case 'get_env_info': if (data && data.os) return data.os + ' · deecodex ' + data.deecodex_version; break;
    case 'health_summary': if (data) return (data.service.running ? 'svc 运行' : 'svc 停止') + ' · ' + (data.account.ok ? 'acct 正常' : 'acct 异常') + ' · ' + data.recent_errors + ' err'; break;
    case 'dex_self_check': if (data) return (data.ok ? '正常' : '需关注') + ' · ' + (data.tool_count || 0) + ' 工具 · ' + ((data.plugin_tools || []).length) + ' 插件工具'; break;
    case 'analyze_requests': if (data && data.total) return data.total + '请求 · ' + data.success_rate + '%成功 · ' + data.avg_latency_ms + 'ms均值'; return '无数据';
    case 'detect_processes': if (data && Array.isArray(data.processes)) { var r = data.processes.filter(function(p){return p.running;}).length; return r + ' 个进程运行中'; } break;
    case 'detect_ports': if (data && Array.isArray(data.ports)) { var u = data.ports.filter(function(p){return p.in_use;}).length; return u + ' 个端口占用'; } break;
    case 'execute_shell': if (data && data.success) return '成功 (exit ' + (data.exit_code||0) + ')'; return '失败'; break;
    case 'start_service': case 'stop_service': return data && data.running !== undefined ? (data.running ? '已启动' : '已停止') : '完成'; break;
    case 'config_backup': if (data && data.action) return '备份: ' + (data.count || '完成'); return '完成';
    case 'config_diff': if (data && data.changes !== undefined) return data.changes + ' 处变更'; return '无差异';
    case 'token_cost': if (data && data.total_cost) return data.total_cost; return '已分析';
    case 'speed_test': if (data && data.avg_latency_ms) return data.avg_latency_ms + 'ms'; return '已测速';
    case 'thread_cleanup': if (data && data.removed !== undefined) return '清理 ' + data.removed + ' 条'; return '完成';
    case 'auto_tune': if (data && data.applied) return '已应用 ' + data.applied + ' 项优化'; return '已分析';
    case 'claude_mcp_check': if (data && data.ok !== undefined) return data.ok ? 'MCP正常' : 'MCP异常'; return '已检查';
    case 'claude_env_overview': return data && data.installed ? 'Claude 已安装' : 'Claude 未检测到';
    case 'openclaw_env_overview': return data && data.installed ? 'OpenClaw 已安装' : 'OpenClaw 未检测到';
    case 'openclaw_health_check': return data && data.ok ? 'OpenClaw 正常' : 'OpenClaw 需关注';
    case 'openclaw_mcp_check': if (data && data.mcp_servers) return data.mcp_servers.length + ' 个 MCP server'; return '已检查';
    case 'openclaw_gateway_overview': return data && data.ok ? 'Gateway 正常' : 'Gateway 需关注';
    case 'openclaw_agents_overview': return data && data.ok ? 'Agents 已读取' : 'Agents 需关注';
    case 'openclaw_models_overview': return data && data.ok ? 'Models 已读取' : 'Models 需关注';
    case 'openclaw_approvals_overview': return data && data.ok ? 'Approvals 已读取' : 'Approvals 需关注';
    case 'hermes_env_overview': return data && data.installed ? 'Hermes 已安装' : 'Hermes 未检测到';
    case 'hermes_doctor_check': return data && data.ok ? 'Hermes 正常' : 'Hermes 需关注';
    case 'hermes_skills_overview': return data && data.ok ? 'Skills 已读取' : 'Skills 需关注';
    case 'hermes_config_overview': return data && data.ok ? 'Config 已读取' : 'Config 需关注';
    case 'hermes_gateway_overview': return data && data.ok ? 'Gateway 正常' : 'Gateway 需关注';
    case 'ai_toolchain_overview': if (data && data.blockers) return data.ok ? 'AI链正常' : data.blockers.length + ' 个阻断项'; return '已汇总';
    case 'network_topology': if (data && data.nodes) return data.nodes + ' 节点'; return '已分析';
    case 'ssl_check': if (data && data.valid !== undefined) return data.valid ? '证书有效' : '证书异常'; return '已检查';
    case 'export_report': if (data && data.path) return '已导出: ' + data.path; return '已导出';
    default:
      if (typeof data === 'string' && data.length <= 60) return data;
      if (typeof data === 'number' || typeof data === 'boolean') return String(data);
      var s = JSON.stringify(data);
      return s.length > 80 ? s.substring(0, 80) + '…' : s;
  }
  return '完成';
}

function dexRenderPendingConfirm() {
  var pending = window.dexAgent && window.dexAgent._pendingConfirm;
  var container = document.getElementById('dexMessages');
  if (!pending || !container) return;
  var id = 'dexConfirm-' + pending.id;
  var existing = document.getElementById(id);
  if (existing) existing.remove();
  var commandPreview = '';
  if (pending.toolName === 'execute_shell' && pending.fnArgs && pending.fnArgs.command) {
    commandPreview = '<pre class="dex-confirm-command">' + esc(pending.fnArgs.command) + '</pre>';
  }
  var el = document.createElement('div');
  el.className = 'dex-msg dex-msg-confirm-inline';
  el.id = id;
  el.innerHTML = '<div class="dex-confirm-card"><div class="dex-confirm-header">⚠ 需要确认操作</div>'
    + '<div class="dex-confirm-body"><div class="dex-confirm-tool">' + esc(pending.toolName) + '</div>'
    + '<div class="dex-confirm-msg">' + esc(pending.message) + '</div>' + commandPreview + '</div>'
    + '<div class="dex-confirm-actions">'
    + '<button class="btn btn-primary btn-sm dex-confirm-ok">确认执行</button>'
    + '<button class="btn btn-ghost btn-sm dex-confirm-cancel">取消</button></div></div>';
  container.appendChild(el);
  el.querySelector('.dex-confirm-ok').onclick = function () { dexResolveInlineConfirm(pending.id, true); };
  el.querySelector('.dex-confirm-cancel').onclick = function () { dexResolveInlineConfirm(pending.id, false); };
  dexScrollToBottom();
}

function dexResolveInlineConfirm(id, confirmed) {
  var pending = window.dexAgent && window.dexAgent._pendingConfirm;
  if (!pending) return;
  if (id && pending.id !== id) return;
  var el = document.getElementById('dexConfirm-' + pending.id);
  if (el) el.remove();
  window.dexAgent._pendingConfirm = null;
  if (typeof pending.resolve === 'function') pending.resolve(confirmed);
}

function dexShowInlineConfirm(toolName, message, toolCallId, fnArgs) {
  return new Promise(function (resolve) {
    var id = String(toolCallId || Date.now());
    if (window.dexAgent && window.dexAgent._pendingConfirm) {
      dexResolveInlineConfirm(null, false);
    }
    window.dexAgent._pendingConfirm = {
      id: id,
      toolName: toolName,
      message: message,
      fnArgs: fnArgs || {},
      resolve: resolve
    };
    dexRenderPendingConfirm();
  });
}

function dexShowThinking() {
  dexHideThinking();
  var c = document.getElementById('dexMessages');
  if (!c) return;
  var el = document.createElement('div');
  el.className = 'dex-msg dex-msg-thinking';
  el.id = 'dexThinking';
  el.innerHTML = '<div class="dex-thinking-inline"><div class="dex-thinking-dots" aria-label="DEX 正在思考"><span></span><span></span><span></span></div></div>';
  c.appendChild(el);
  dexScrollToBottom();
}

function dexHideThinking() {
  var el = document.getElementById('dexThinking');
  if (el) el.remove();
}

var _dexLastAssistantEl = null;
function dexUpdateLastAssistant(text, reasoning) {
  if (!_dexLastAssistantEl) {
    _dexLastAssistantEl = dexAppendMessage('assistant', text, { model: window.dexAgent.selectedModel });
    if (reasoning) {
      var rEl = _dexLastAssistantEl.querySelector('.dex-reasoning-content');
      if (rEl) rEl.textContent = reasoning;
      var rWrap = _dexLastAssistantEl.querySelector('.dex-reasoning-wrap');
      if (rWrap) rWrap.style.display = '';
    }
    return;
  }
  var bubble = _dexLastAssistantEl.querySelector('.dex-bubble-text');
  if (bubble) bubble.innerHTML = dexRenderMarkdown(text);
  if (reasoning) {
    var rWrap2 = _dexLastAssistantEl.querySelector('.dex-reasoning-wrap');
    if (rWrap2) rWrap2.style.display = '';
    var rEl2 = _dexLastAssistantEl.querySelector('.dex-reasoning-content');
    if (rEl2) rEl2.textContent = reasoning;
  }
  dexScrollToBottom();
}

function dexScrollToBottom() {
  var c = document.getElementById('dexMessages');
  if (c) c.scrollTop = c.scrollHeight;
}
