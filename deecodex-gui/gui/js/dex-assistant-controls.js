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
window.dexAgent.selectedAccountId = null;

function dexToggleAccountMenu(e) {
  e.stopPropagation();
  var m = document.getElementById('dexAccountMenu');
  if (m) m.style.display = m.style.display === 'none' ? '' : 'none';
}

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

function dexAccountSurfaceLabel(account) {
  var surface = String(account && account.client_surface || 'cli').toLowerCase();
  return surface === 'desktop' ? '桌面' : 'CLI';
}

function dexAccountLabel(account) {
  if (!account) return '账号';
  var name = account.name || account.provider || account.id || '账号';
  return dexAccountSurfaceLabel(account) + ' · ' + name;
}

function dexSetAccountButton(account) {
  var btn = document.getElementById('dexAccountBtn');
  if (!btn) return;
  btn.innerHTML = esc(dexAccountLabel(account)) + '<span class="dex-model-caret"></span>';
  window.dexAgent.selectedAccountId = account && account.id ? account.id : null;
}

function dexLoadAccounts() {
  var menu = document.getElementById('dexAccountMenu');
  if (!menu) return;
  DeeCodexTauri.invoke('list_accounts', {}).then(function(data) {
    var accounts = (data && data.accounts || []).filter(function(account) {
      return String(account.client_kind || account.target || '').toLowerCase() === 'codex';
    });
    var selection = data && data.active_by_surface && data.active_by_surface['dex:assistant'];
    var activeId = selection && selection.account_id || data && (data.active_account_id || data.active_id) || '';
    var active = accounts.find(function(account) { return account.id === activeId; }) || accounts[0] || null;
    dexSetAccountButton(active);
    if (!accounts.length) {
      menu.innerHTML = '<div class="dex-model-item dex-model-empty">暂无账号</div>';
      return;
    }
    menu.innerHTML = accounts.map(function(account) {
      var activeClass = account.id === (active && active.id) ? ' active' : '';
      return '<div class="dex-model-item dex-account-item' + activeClass + '" onclick="dexSelectAccount(\'' + escAttr(account.id) + '\')">'
        + esc(dexAccountLabel(account)) + '</div>';
    }).join('');
  }).catch(function(e) {
    console.warn('[dexAgent] 加载账号列表失败:', e);
  });
}

function dexSelectAccount(id) {
  DeeCodexTauri.invoke('set_dex_assistant_account', { accountId: id }).then(function(account) {
    dexSetAccountButton(account);
    var menu = document.getElementById('dexAccountMenu');
    if (menu) menu.style.display = 'none';
    dexLoadAccounts();
    dexLoadModels();
    dexRefreshStatus();
    showToast('DEX 助手账号已切换', 'success');
  }).catch(function(e) {
    showToast('切换 DEX 助手账号失败: ' + (e.message || e), 'error');
  });
}

function dexAccountModelValues(account) {
  var vals = [];
  if (!account) return vals;
  if (Array.isArray(account.endpoints)) {
    for (var e = 0; e < account.endpoints.length; e++) {
      if (Array.isArray(account.endpoints[e].known_models)) vals = vals.concat(account.endpoints[e].known_models);
    }
  }
  if (account.default_model) vals.push(account.default_model);
  var mm = account.model_map;
  if (typeof mm === 'string') { try { mm = JSON.parse(mm); } catch(e) { mm = {}; } }
  if (mm && typeof mm === 'object') vals = vals.concat(Object.values(mm));
  return vals;
}

function dexUniqueModels(vals) {
  var seen = {}, models = [];
  for (var i = 0; i < vals.length; i++) {
    var v = String(vals[i] || '').trim();
    if (!v || seen[v]) continue;
    seen[v] = true;
    models.push(v);
  }
  return models;
}

function dexRenderModelMenu(models, loading) {
  var menu = document.getElementById('dexModelMenu');
  if (!menu) return;
  var html = '<div class="dex-model-item" onclick="dexSelectModel(\'auto\',\'模型\')">自动</div>';
  if (loading) html += '<div class="dex-model-item dex-model-empty">正在刷新模型...</div>';
  for (var i = 0; i < models.length; i++) {
    var v = models[i];
    html += '<div class="dex-model-item" onclick="dexSelectModel(\'' + escAttr(v) + '\',\'' + escAttr(v) + '\')">' + esc(v) + '</div>';
  }
  if (!loading && !models.length) html += '<div class="dex-model-item dex-model-empty">暂无可用模型</div>';
  menu.innerHTML = html;
}

function dexLoadModels() {
  var menu = document.getElementById('dexModelMenu');
  if (!menu) return;
  window.dexAgent.modelLoadToken = (window.dexAgent.modelLoadToken || 0) + 1;
  var token = window.dexAgent.modelLoadToken;
  dexRenderModelMenu([], true);
  DeeCodexTauri.invoke('get_dex_assistant_account', {}).then(function(account) {
    if (!account) return;
    var fallbackModels = dexUniqueModels(dexAccountModelValues(account));
    if (token !== window.dexAgent.modelLoadToken) return;
    dexRenderModelMenu(fallbackModels, true);
    if (!account.id) {
      dexRenderModelMenu(fallbackModels, false);
      return;
    }
    DeeCodexTauri.invoke('fetch_upstream_models', { accountId: account.id }).then(function(models) {
      if (token !== window.dexAgent.modelLoadToken) return;
      var liveModels = Array.isArray(models) ? models : [];
      dexRenderModelMenu(dexUniqueModels(liveModels.concat(fallbackModels)), false);
    }).catch(function(e) {
      if (token !== window.dexAgent.modelLoadToken) return;
      console.warn('[dexAgent] 刷新上游模型失败，使用账号缓存:', e);
      dexRenderModelMenu(fallbackModels, false);
    });
  }).catch(function(e) { console.warn('[dexAgent] 加载模型列表失败:', e); });
}

document.addEventListener('click', function() {
  var m = document.getElementById('dexModelMenu');
  if (m) m.style.display = 'none';
  var a = document.getElementById('dexAccountMenu');
  if (a) a.style.display = 'none';
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
  'migrate_threads': '线程已归一',
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
      if (typeof dexLoadAccounts === 'function') dexLoadAccounts();
      if (typeof dexLoadModels === 'function') dexLoadModels();
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
