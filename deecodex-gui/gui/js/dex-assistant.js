// DEX助手
// ═══════════════════════════════════════════════════════════════

// ── Chat UI 渲染 ──
function renderDexAssistant() {
  window._dexInitialized = false;
  setTimeout(function () {
    if (window._dexInitialized) return;
    window._dexInitialized = true;
    if (window.dexAgent.messages.length === 0) window.dexAgent.init();

    dexLoadDynamicContext().then(function () {
      dexUpdateTokenCount();
    });
    dexLoadModels();

    var history = window.dexAgent.loadHistory();
    if (history && history.length > 0) {
      var container = document.getElementById('dexMessages');
      if (!container) return;
      container.innerHTML = '';
      var lastToolNames = {};
      for (var i = 0; i < history.length; i++) {
        var msg = history[i];
        if (msg.role === 'user') {
          dexAppendMessage('user', msg.content);
        } else if (msg.role === 'assistant') {
          if (msg.tool_calls) {
            for (var j = 0; j < msg.tool_calls.length; j++) lastToolNames[msg.tool_calls[j].id] = msg.tool_calls[j].function.name;
            if (msg.content) dexAppendMessage('assistant', msg.content);
            for (var k = 0; k < msg.tool_calls.length; k++)
              dexAppendMessage('tool-start', msg.tool_calls[k].function.name, { args: msg.tool_calls[k].function.arguments, history: true });
          } else {
            dexAppendMessage('assistant', msg.content);
          }
        } else if (msg.role === 'tool') {
          var toolName = lastToolNames[msg.tool_call_id] || '工具结果';
          try {
            var parsed = JSON.parse(msg.content);
            if (parsed.error) dexAppendMessage('tool-error', toolName + ': ' + parsed.error, { error: parsed.error, history: true });
            else dexAppendMessage('tool-result', toolName, { result: parsed, history: true });
          } catch (e) { dexAppendMessage('tool-result', toolName, { result: msg.content, history: true }); }
        }
      }
      dexRenderPendingConfirm();
      dexScrollToBottom();
    }

    // 输入框事件：Enter 发送、Tab 补全斜杠指令
    var input = document.getElementById('dexInput');
    if (input) input.addEventListener('keydown', function (e) {
      if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); dexSendMessage(); return; }
      if (e.key === 'Tab') {
        var val = input.value.trim();
        var expanded = dexExpandSlashCommand(val);
        if (expanded !== val) { e.preventDefault(); input.value = expanded; dexUpdateTokenCount(); }
      }
    });

    // 搜索输入事件
    var searchInput = document.getElementById('dexSearchInput');
    if (searchInput) {
      searchInput.addEventListener('input', function () { dexPerformSearch(); });
      searchInput.addEventListener('keydown', function (e) {
        if (e.key === 'Enter') { e.preventDefault(); dexNavigateSearch(1); }
        if (e.key === 'Escape') { dexCloseSearch(); }
      });
    }

    // Token 计数初始
    dexSyncAttachmentButton();
    dexUpdateTokenCount();

    // 快捷键
    dexBindShortcuts();
  }, 0);

  return '<section class="primary-page-shell primary-page-shell-dex-assistant" data-primary-panel="dex-assistant">'
    + '<div class="dex-chat-panel"><div class="dex-chat-header"><div class="dex-header-title">'
    + '<div class="dex-title-row"><h3 title="DEX助手">DEX助手</h3></div>'
    + '</div><div class="dex-header-actions">'
    + '<div class="dex-model-drop" id="dexModelDrop"><button class="dex-model-btn" id="dexModelBtn" onclick="dexToggleModelMenu(event)">模型<span class="dex-model-caret"></span></button><div class="dex-model-menu" id="dexModelMenu" style="display:none"></div></div>'
    + '<button class="btn btn-ghost btn-sm dex-icon-btn dex-icon-search" id="dexSearchBtn" onclick="dexToggleSearch()" title="搜索对话" aria-label="搜索对话"><span class="dex-sr-only">搜索对话</span></button>'
    + '<button class="btn btn-ghost btn-sm dex-icon-btn dex-icon-new" onclick="dexNewChat()" title="新对话" aria-label="新对话"><span class="dex-sr-only">新对话</span></button>'
    + '<button class="btn btn-ghost btn-sm dex-icon-btn dex-icon-clear" onclick="dexClearChat()" title="清空对话" aria-label="清空对话"><span class="dex-sr-only">清空对话</span></button></div></div>'
    + '<div class="dex-search-bar" id="dexSearchBar" style="display:none"><input id="dexSearchInput" placeholder="搜索对话..." /><span class="dex-search-count" id="dexSearchCount"></span><button class="btn btn-ghost btn-sm" onclick="dexCloseSearch()">关闭</button></div>'
    + '<div class="dex-chat-messages" id="dexMessages">' + dexWelcomeHTML() + '</div>'
    + '<div class="dex-input-area" id="dexInputAreaWrap">'
    + '<div class="dex-input-row">'
    + '<textarea id="dexInput" placeholder="向 DEX 助手提问" rows="2"></textarea>'
    + '<div class="dex-input-toolbar">'
    + '<div class="dex-input-hints"><button type="button" class="dex-input-plus" onclick="dexAttachLastFile(event)" title="添加附件；有上次附件时直接加入，按住 Shift 重新选择" aria-label="添加附件">＋</button><span>/diag</span><span>/fix</span><span>Shift+Enter 换行</span></div>'
    + '<div class="dex-input-actions"><span class="dex-token-count" id="dexTokenCount">~0 tokens</span>'
    + '<button class="btn btn-primary dex-send-btn" id="dexSendBtn" onclick="dexSendMessage()" aria-label="发送">发送</button>'
    + '<button class="btn btn-danger dex-stop-btn" id="dexStopBtn" onclick="dexStopAgent()" style="display:none" aria-label="停止生成">停止</button></div>'
    + '</div>'
    + '</div>'
    + '</div></div></section>';
}

function dexWelcomeHTML() {
  return '<div class="dex-msg dex-msg-assistant dex-welcome"><div class="dex-workspace-empty">'
    + '<div class="dex-workspace-head"><div><h4>你想排查什么？</h4></div><p>直接输入问题，或从下面选择一个起点。</p></div>'
    + '<div class="dex-quick-actions">'
    + '<button class="btn btn-sm btn-primary" onclick="dexQuickAction(\'汇总 Codex、Claude、OpenClaw、Hermes、MCP、模型账号和插件状态\')">AI 链总览</button>'
    + '<button class="btn btn-sm btn-ghost" onclick="dexQuickAction(\'检查服务状态、端口、配置和最近错误，并给出修复建议\')">服务诊断</button>'
    + '<button class="btn btn-sm btn-ghost" onclick="dexQuickAction(\'跨工具对比 Claude、OpenClaw、Hermes 的 MCP 配置\')">MCP 对比</button>'
    + '<button class="btn btn-sm btn-ghost" onclick="dexQuickAction(\'查看插件状态和可用插件能力\')">插件状态</button>'
    + '<button class="btn btn-sm btn-ghost" onclick="dexQuickAction(\'读日志，检查异常\')">日志异常</button></div></div></div>';
}

function dexDisposePanel() {
}

async function dexSendMessage() {
  var input = document.getElementById('dexInput');
  if (!input) return;
  var text = input.value.trim();
  if (!text) return;
  if (window.dexAgent.isProcessing) { showToast('Agent 正在处理中，请等待', 'warn'); return; }
  input.value = '';
  dexAppendMessage('user', text);
  dexUpdateTokenCount();
  dexShowThinking();
  try { await window.dexAgent.run(text); }
  catch (e) { console.error('[dexAssistant] sendMessage 失败:', e); dexAppendMessage('system', '消息处理失败: ' + (e.message || e)); }
  dexHideThinking();
  dexUpdateTokenCount();
  input.focus();
}
