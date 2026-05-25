function dexQuickAction(prompt) {
  var i = document.getElementById('dexInput');
  if (i) i.value = prompt;
  dexSendMessage();
}

function dexNewChat() {
  if (window.dexAgent.isProcessing) { showToast('Agent 正在处理中，请等待', 'warn'); return; }
  window.dexAgent.init();
  var c = document.getElementById('dexMessages'); if (c) c.innerHTML = dexWelcomeHTML();
  dexCloseSearch();
  dexUpdateTokenCount();
  dexScrollToBottom(); showToast('已开始新对话', 'success');
}

function dexClearChat() {
  if (window.dexAgent.isProcessing) { showToast('Agent 正在处理中，请等待', 'warn'); return; }
  window.dexAgent.clear();
  var c = document.getElementById('dexMessages'); if (c) c.innerHTML = dexWelcomeHTML();
  dexCloseSearch();
  dexUpdateTokenCount();
  dexScrollToBottom(); showToast('对话已清空', 'success');
}

function dexRefreshChat() {
  var c = document.getElementById('dexMessages');
  if (c) c.innerHTML = dexWelcomeHTML();
  dexRenderPendingConfirm();
}

window.dexAgent.selectedModel = 'auto';
function dexToggleModelMenu(e) {
  e.stopPropagation();
  var m = document.getElementById('dexModelMenu');
  if (m) m.style.display = m.style.display === 'none' ? '' : 'none';
}

function dexSelectModel(v, label) {
  var btn = document.getElementById('dexModelBtn');
  if (btn) btn.innerHTML = esc(label) + '<span class="dex-model-caret"></span>';
  window.dexAgent.selectedModel = v;
  document.getElementById('dexModelMenu').style.display = 'none';
}

function dexChangeModel() {}

function dexLoadModels() {
  var menu = document.getElementById('dexModelMenu');
  if (!menu) return;
  DeeCodexTauri.invoke('get_active_account', {}).then(function(account) {
    if (!account || !account.model_map) return;
    var mm = account.model_map;
    if (typeof mm === 'string') { try { mm = JSON.parse(mm); } catch(e) { return; } }
    if (typeof mm !== 'object') return;
    var vals = Object.values(mm);
    var seen = {}, html = '';
    html += '<div class="dex-model-item" onclick="dexSelectModel(\'auto\',\'模型\')">自动</div>';
    for (var i = 0; i < vals.length; i++) {
      var v = vals[i];
      if (seen[v]) continue; seen[v] = true;
      html += '<div class="dex-model-item" onclick="dexSelectModel(\'' + esc(v) + '\',\'' + esc(v) + '\')">' + esc(v) + '</div>';
    }
    menu.innerHTML = html;
  }).catch(function(e) { console.warn('[dexAgent] 加载模型列表失败:', e); });
}

document.addEventListener('click', function() {
  var m = document.getElementById('dexModelMenu');
  if (m) m.style.display = 'none';
});

function dexShowStopButton() {
  var sendBtn = document.getElementById('dexSendBtn');
  var stopBtn = document.getElementById('dexStopBtn');
  if (sendBtn) sendBtn.style.display = 'none';
  if (stopBtn) stopBtn.style.display = '';
}

function dexHideStopButton() {
  var sendBtn = document.getElementById('dexSendBtn');
  var stopBtn = document.getElementById('dexStopBtn');
  if (sendBtn) sendBtn.style.display = '';
  if (stopBtn) stopBtn.style.display = 'none';
}

function dexStopAgent() {
  if (window.dexAgent) window.dexAgent.abort();
  dexHideStopButton();
  dexHideThinking();
}

function dexMaskApiKey(text) {
  if (!text) return text;
  if (typeof text === 'object') {
    var obj = JSON.parse(JSON.stringify(text));
    if (obj.api_key) obj.api_key = 'sk-***';
    if (obj.vision_api_key) obj.vision_api_key = '***';
    return obj;
  }
  return String(text).replace(/"api_key":"[^"]+"/g, '"api_key":"sk-***"');
}

function dexStripAnsi(text) {
  if (!text) return text;
  if (Array.isArray(text)) return text.map(dexStripAnsi);
  if (typeof text !== 'string') return text;
  return text.replace(/\x1B\[[0-9;]*[a-zA-Z]/g, '').replace(/\[[0-9;]*[a-zA-Z]/g, '');
}

var DEX_MUTATE_TOOLS = {
  'save_config': '配置已更新，切换到「协议配置」面板查看',
  'start_service': '服务已启动',
  'stop_service': '服务已停止',
  'switch_account': '账号已切换',
  'add_account': '账号已添加',
  'update_account': '账号已更新',
  'delete_account': '账号已删除',
  'migrate_threads': '线程已迁移',
  'restore_threads': '线程已还原',
  'calibrate_threads': '线程已校准',
  'run_upgrade': '升级完成',
  'install_plugin': '插件已安装',
  'uninstall_plugin': '插件已卸载',
  'launch_codex_cdp': 'Codex CDP 已启动',
  'stop_codex_cdp': 'Codex CDP 已停止',
};

function dexAfterMutate(fnName) {
  if (DEX_MUTATE_TOOLS[fnName]) {
    dexRefreshStatus();
    showToast(DEX_MUTATE_TOOLS[fnName], 'success');
    if (fnName === 'save_config' && typeof loadConfig === 'function') {
      loadConfig().catch(function(){});
    }
    if (fnName === 'start_service' || fnName === 'stop_service') {
      DeeCodexTauri.invoke('get_service_status').then(function(s) {
        window._statusData = {
          running: s && s.running, port: s ? s.port : '—',
          uptime_secs: s && s.running ? s.uptime_secs : 0,
          version: (s && s.version) || (window._statusData && window._statusData.version) || '—',
          upstream: (window._statusData && window._statusData.upstream) || '—',
          vision_enabled: (window._statusData && window._statusData.vision_enabled) || false,
          computer_executor: (window._statusData && window._statusData.computer_executor) || 'disabled',
          chinese_thinking: (window._statusData && window._statusData.chinese_thinking) || false,
          cdp_port: (window._statusData && window._statusData.cdp_port) || 9222,
          codex_launch_with_cdp: (window._statusData && window._statusData.codex_launch_with_cdp) || false,
        };
        var dot = document.getElementById('connDot');
        var label = document.getElementById('connLabel');
        if (dot && label && s) {
          if (s.running) { dot.className = 'indicator ok'; label.textContent = '服务运行中'; }
          else { dot.className = 'indicator off'; label.textContent = '服务已停止'; }
        }
      }).catch(function(){});
    }
    if (fnName.indexOf('account') >= 0 && typeof loadAccountsData === 'function') {
      loadAccountsData();
    }
    if (fnName.indexOf('plugin') >= 0 && typeof loadPluginsData === 'function') {
      loadPluginsData();
    }
  }
}

function dexRefreshStatus() {
  var dot = document.getElementById('dexStatusDot');
  var text = document.getElementById('dexStatusText');
  if (!dot || !text) return;
  DeeCodexTauri.invoke('dex_health_summary', {}).then(function (data) {
    if (!data) { dot.className = 'dex-status-dot dex-status-warn'; text.textContent = '无数据'; return; }
    var svcOk = data.service && data.service.running;
    var acctOk = data.account && data.account.ok;
    var errCount = data.recent_errors || 0;
    var m = window.dexAgent.selectedModel;
    var accountProvider = (data.account && data.account.provider) || '';
    var accountProfile = (data.account && data.account.profile) || '';
    var modelLabel = (m && m !== 'auto') ? m : accountProvider;
    var profileLabel = accountProfile && accountProfile !== accountProvider ? accountProfile : '';
    var parts = [
      { label: svcOk ? '服务运行' : '服务停止', tone: svcOk ? 'ok' : 'error' },
      { label: acctOk ? '账号正常' : '账号异常', tone: acctOk ? 'ok' : 'error' },
    ];
    if (errCount > 0) parts.push({ label: errCount + ' err', tone: 'warn' });
    if (modelLabel) parts.push({ label: modelLabel, tone: 'muted' });
    if (profileLabel) parts.push({ label: profileLabel, tone: 'muted' });
    text.innerHTML = parts.map(function(part, index) {
      return (index ? '<span class="dex-status-sep">/</span>' : '')
        + '<span class="dex-status-item dex-status-' + part.tone + '">' + esc(part.label) + '</span>';
    }).join('');
    if (!svcOk || !acctOk) { dot.className = 'dex-status-dot dex-status-err'; }
    else if (errCount > 0) { dot.className = 'dex-status-dot dex-status-warn'; }
    else { dot.className = 'dex-status-dot dex-status-ok'; }
  }).catch(function () {
    dot.className = 'dex-status-dot dex-status-err';
    text.textContent = '状态获取失败';
  });
}

function dexExportChat() {
  var msgs = window.dexAgent.messages;
  var md = '# DEX助手 对话导出\n\n';
  md += '> 导出时间: ' + new Date().toLocaleString() + '\n\n---\n\n';
  for (var i = 0; i < msgs.length; i++) {
    var m = msgs[i];
    if (m.role === 'system' && m.content && m.content.indexOf('[对话摘要]') !== 0) continue;
    if (m.role === 'user') md += '**用户:** ' + (m.content || '') + '\n\n';
    else if (m.role === 'assistant' && m.content) md += '**DEX助手:** ' + m.content + '\n\n';
    else if (m.role === 'system' && m.content && m.content.indexOf('[对话摘要]') === 0) md += '> ' + m.content + '\n\n';
    else if (m.role === 'tool') { try { var d = JSON.parse(m.content); md += '*工具: ' + (d.error || '完成') + '*\n\n'; } catch(e) {} }
  }
  navigator.clipboard.writeText(md).then(function () {
    showToast('对话已复制到剪贴板', 'success');
  }).catch(function () {
    showToast('复制失败，请手动复制', 'error');
  });
}

var DEX_SLASH_COMMANDS = {
  '/diag': '运行完整诊断，分析结果',
  '/fix': '自动修复所有发现的问题',
  '/health': '汇总 Codex、Claude、OpenClaw、Hermes、MCP、模型账号和插件状态',
  '/self': '运行 DEX 自检，检查工具注册表、能力包、插件工具和最近错误',
  '/mcp': '跨工具对比 Claude、OpenClaw、Hermes 的 MCP 配置',
  '/claude': '检查 Claude Code 安装、配置目录和 MCP 集成状态',
  '/openclaw': '检查 OpenClaw CLI、Gateway、Agents、Models、MCP 和 Approvals 状态',
  '/hermes': '检查 Hermes CLI、doctor、skills、config 和 gateway 状态',
  '/agents': '检查 OpenClaw agents 和 Hermes skills 状态',
  '/workspace': '分析当前项目工作区环境',
  '/plugins': '查看插件状态和可用插件能力',
  '/cost': '分析请求成本',
  '/status': '服务状态',
  '/logs': '读日志，检查异常',
  '/log': '读日志，检查异常',
  '/help': '你能做什么'
};

function dexExpandSlashCommand(text) {
  if (!text || text[0] !== '/') return text;
  var cmd = text.split(' ')[0];
  if (DEX_SLASH_COMMANDS[cmd]) return DEX_SLASH_COMMANDS[cmd];
  return text;
}

function dexUpdateTokenCount() {
  var el = document.getElementById('dexTokenCount');
  if (!el) return;
  var msgs = window.dexAgent.messages;
  if (!msgs || msgs.length <= 1) { el.textContent = '~0 tokens'; return; }
  try {
    var totalLen = 0;
    for (var i = 0; i < msgs.length; i++) {
      totalLen += JSON.stringify(msgs[i]).length;
    }
    var estimated = Math.max(1, Math.round(totalLen / 4));
    el.textContent = '~' + estimated + ' tokens';
  } catch (e) { el.textContent = '~? tokens'; }
}
