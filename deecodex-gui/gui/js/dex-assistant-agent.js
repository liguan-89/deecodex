// 内联 opencode 客户端（强制覆盖，保证使用最新实现）
(function (root) {
  'use strict';

  var DEX_GROUPS = ['dex_diag', 'dex_workspace', 'dex_ops', 'dex_plugins'];

  function stripGroupPrefix(name) {
    var s = String(name || '');
    for (var i = 0; i < DEX_GROUPS.length; i++) {
      var prefix = DEX_GROUPS[i] + '_';
      if (s.indexOf(prefix) === 0) return s.slice(prefix.length);
    }
    return s;
  }

  // 纯函数：把 opencode 原始事件归类为 UI 事件类型，便于 Node 回归测试。
  // 返回 null 表示应忽略。
  function eventKind(raw) {
    if (!raw || typeof raw !== 'object') return null;
    var type = String(raw.type || '');
    var props = raw.properties || {};
    var part = props.part || {};
    if (type === 'permission.asked') return 'permission';
    if (type === 'message.part.updated') {
      if (part.type === 'text') return 'text';
      if (part.type === 'reasoning') return 'reasoning';
      if (part.type === 'tool') return 'tool';
      return null;
    }
    if (type === 'session.idle') return 'idle';
    if (type === 'session.status') {
      var status = props.status;
      if (status === 'idle') return 'idle';
      if (status === 'busy') return 'busy';
      return null;
    }
    if (type === 'session.error') return 'error';
    return null;
  }

  var state = {
    enabled: false,
    sessionId: null,
    unlisten: null,
    busy: false,
    waiters: [],
    lastError: ''
  };
  var _startPromise = null;

  function waitIdle() {
    return new Promise(function (resolve) {
      state.waiters.push(resolve);
    });
  }

  function resolveIdle() {
    var list = state.waiters;
    state.waiters = [];
    for (var i = 0; i < list.length; i++) list[i]();
  }

  function engineAllowed() {
    try {
      return window.deeStorage.getItem('dex_opencode_engine') !== 'off';
    } catch (e) {
      return true;
    }
  }

  async function ensureEngine() {
    var status = await DeeCodexTauri.invoke('dex_opencode_status', {});
    if (!status.running) {
      await DeeCodexTauri.invoke('dex_opencode_start', {});
      status = await DeeCodexTauri.invoke('dex_opencode_status', {});
    }
    state.enabled = !!(status && status.running);
    return state.enabled;
  }

  async function subscribe() {
    if (state.unlisten) return;
    await DeeCodexTauri.invoke('dex_opencode_events_subscribe', {});
    state.unlisten = await DeeCodexTauri.listen('dex-opencode-event', function (evt) {
      var payload = evt && evt.payload;
      var raw = payload && payload.event ? payload.event : payload;
      handleEvent(raw);
    });
  }

  async function start() {
    // 并发调用共享同一个进行中的启动，避免重复拉起引擎/竞态回退
    if (_startPromise) return _startPromise;
    _startPromise = (async function () {
      if (!engineAllowed()) {
        state.lastError = '引擎开关已关闭 (dex_opencode_engine=off)';
        state.enabled = false;
        return false;
      }
      try {
        state.enabled = await ensureEngine();
        if (!state.enabled) state.lastError = '引擎状态未就绪（status.running=false）';
        if (state.enabled) await subscribe();
      } catch (e) {
        state.lastError = String((e && e.message) || e);
        console.warn('[DexOpencode] 引擎启动失败，回退旧循环:', e);
        state.enabled = false;
      }
      return state.enabled;
    })();
    try { return await _startPromise; } finally { _startPromise = null; }
  }

  async function newSession(title) {
    var r = await DeeCodexTauri.invoke('dex_opencode_session_create', {
      title: title || 'DEX 助手'
    });
    state.sessionId = r && r.id ? r.id : null;
    return r;
  }

  async function send(text) {
    if (!state.sessionId) await newSession();
    state.busy = true;
    setProcessing(true);
    var done = waitIdle();
    try {
      await DeeCodexTauri.invoke('dex_opencode_prompt', {
        sessionId: state.sessionId,
        text: text
      });
    } catch (e) {
      state.busy = false;
      resolveIdle();
      throw e;
    }
    // 兜底：引擎崩溃/事件丢失时不让 UI 永远卡在“处理中”
    var timeoutId = null;
    var timeout = new Promise(function (resolve) { timeoutId = setTimeout(resolve, 300000); });
    try {
      await Promise.race([done, timeout]);
    } finally {
      clearTimeout(timeoutId);
    }
    state.busy = false;
  }

  async function abort() {
    state.busy = false;
    resolveIdle();
    setProcessing(false);
    if (state.sessionId) {
      try { await DeeCodexTauri.invoke('dex_opencode_abort', { sessionId: state.sessionId }); }
      catch (e) { console.warn('[DexOpencode] abort 失败:', e); }
    }
  }

  // 账号/模型切换后调用：停引擎，下次 run() 会用新账号重建配置
  async function refresh() {
    if (!state.enabled) return;
    state.enabled = false;
    state.sessionId = null;
    resolveIdle();
    setProcessing(false);
    try {
      await DeeCodexTauri.invoke('dex_opencode_stop', {});
    } catch (e) {
      console.warn('[DexOpencode] 停止引擎失败:', e);
    }
  }

  function setProcessing(processing) {
    if (!window.dexAgent) return;
    window.dexAgent.isProcessing = processing;
    if (processing) {
      dexShowStopButton();
    } else {
      dexHideStopButton();
      dexHideThinking();
    }
  }

  function handleToolPart(part) {
    var toolId = String(part.tool || '');
    var toolName = stripGroupPrefix(toolId) || toolId;
    var st = part.state || {};
    var status = String(st.status || '');
    if (status === 'running') {
      var args = st.input && typeof st.input === 'object' ? st.input : {};
      var toolDef = null;
      if (window.DEX_TOOLS && Array.isArray(window.DEX_TOOLS)) {
        for (var i = 0; i < window.DEX_TOOLS.length; i++) {
          if (window.DEX_TOOLS[i].name === toolName) { toolDef = window.DEX_TOOLS[i]; break; }
        }
      }
      dexAppendMessage('tool-start', toolName, { args: args, toolDef: toolDef });
      return;
    }
    var els = document.querySelectorAll('.dex-msg-tool-start');
    var el = els.length ? els[els.length - 1] : null;
    if (status === 'completed' || status === 'success') {
      var output = st.output;
      var parsed = null;
      if (typeof output === 'string') { try { parsed = JSON.parse(output); } catch (e) {} }
      if (parsed === null) parsed = output;
      var masked = parsed ? dexMaskApiKey(parsed) : parsed;
      dexUpdateMessage(el, 'tool-result', toolName, { result: masked, toolDef: toolDefOf(toolName) });
    } else if (status === 'error' || status === 'failed') {
      dexUpdateMessage(el, 'tool-error', toolName + ': ' + (st.error || st.output || '失败'), {
        error: st.error || st.output || '',
        toolDef: toolDefOf(toolName)
      });
    }
  }

  function toolDefOf(toolName) {
    if (!window.DEX_TOOLS || !Array.isArray(window.DEX_TOOLS)) return null;
    for (var i = 0; i < window.DEX_TOOLS.length; i++) {
      if (window.DEX_TOOLS[i].name === toolName) return window.DEX_TOOLS[i];
    }
    return null;
  }

  function handleTextPart(part) {
    var text = part.text || '';
    var reasoning = part.reasoning || '';
    // 推理型模型（如 MiniMax-M3）的 think 内容会内联在 text 里，单独折进思考区
    var thinkRe = /<think>[\s\S]*?<\/think>/g;
    var thinkParts = String(text).match(thinkRe) || [];
    var display = String(text).replace(thinkRe, '').trim();
    if (thinkParts.length) {
      reasoning = (reasoning ? reasoning + '\n' : '') + thinkParts.join('\n');
    }
    if (display) {
      dexUpdateLastAssistant(display, reasoning);
    } else if (reasoning) {
      dexUpdateLastAssistant('', reasoning);
    }
  }

  async function handlePermission(props) {
    var permId = String(props.id || '');
    var sessionId = String(props.sessionID || state.sessionId || '');
    var toolId = String(props.permission || '');
    var toolName = stripGroupPrefix(toolId) || toolId;
    var callId = props.tool && props.tool.callID ? props.tool.callID : permId;
    var confirmed = await dexShowInlineConfirm(toolName, '确认执行高风险操作「' + toolName + '」吗？', callId, {});
    var response = confirmed ? 'once' : 'reject';
    try {
      await DeeCodexTauri.invoke('dex_opencode_permission_reply', {
        sessionId: sessionId,
        permissionId: permId,
        response: response
      });
    } catch (e) {
      console.warn('[DexOpencode] 权限回复失败:', e);
    }
  }

  function handleEvent(raw) {
    var kind = eventKind(raw);
    if (!kind) return;
    var props = (raw && raw.properties) || {};
    var part = props.part || {};
    switch (kind) {
      case 'text':
      case 'reasoning':
        handleTextPart(part);
        break;
      case 'tool':
        handleToolPart(part);
        break;
      case 'permission':
        handlePermission(props);
        break;
      case 'idle':
        state.busy = false;
        setProcessing(false);
        resolveIdle();
        break;
      case 'busy':
        setProcessing(true);
        break;
      case 'error':
        dexAppendMessage('system', 'opencode 会话错误: ' + (props.message || ''));
        state.busy = false;
        setProcessing(false);
        resolveIdle();
        break;
    }
  }

  var api = {
    get enabled() { return state.enabled; },
    stripGroupPrefix: stripGroupPrefix,
    eventKind: eventKind,
    start: start,
    send: send,
    abort: abort,
    refresh: refresh,
    newSession: newSession,
    get sessionId() { return state.sessionId; }
  };

  if (typeof module !== 'undefined' && module.exports) {
    module.exports = { stripGroupPrefix: stripGroupPrefix, eventKind: eventKind };
  }
  root.DexOpencode = api;
})(typeof window !== 'undefined' ? window : globalThis);

window.dexAgent = {
  messages: [],
  isProcessing: false,
  roundCount: 0,
  maxRounds: 30,
  maxHistorySize: 50,
  selectedModel: 'auto',
  init: function () {
    this.messages = [{ role: 'system', content: dexBuildSystemPrompt() }];
    this.isProcessing = false;
    this.roundCount = 0;
    this._lastErrorKey = null;
    this._toolCache = {};
    this._pendingConfirm = null;
    this.saveHistory();
  },

  clear: function () {
    this.messages = [{ role: 'system', content: dexBuildSystemPrompt() }];
    this.isProcessing = false;
    this.roundCount = 0;
    this._lastErrorKey = null;
    this._toolCache = {};
    dexResolveInlineConfirm(null, false);
    this.saveHistory();
  },

  compressContext: function () {
    if (this.messages.length <= 40) return;
    var systemMsg = this.messages[0];
    var recentMsgs = this.messages.slice(this.messages.length - 20);
    var summaryParts = [];
    for (var i = 1; i < this.messages.length - 20; i++) {
      var m = this.messages[i];
      if (m.role === 'user') summaryParts.push('用户: ' + (m.content || '').substring(0, 80));
      else if (m.role === 'assistant' && m.content) summaryParts.push('助手: ' + m.content.substring(0, 80));
      else if (m.role === 'tool') summaryParts.push('工具结果');
    }
    var summary = summaryParts.slice(0, 20).join('; ');
    this.messages = [systemMsg, { role: 'system', content: '[对话摘要] 之前讨论了: ' + summary }].concat(recentMsgs);
  },

  _canStream: function () {
    try {
      if (this.streamingSupported === false) return false;
      return typeof window.DeeCodexTauri?.listen === 'function';
    } catch (e) { return false; }
  },

  sendToLLM: async function (messages, tools) {
    try { if (this._canStream()) return await this.sendToLLMStream(messages, tools); }
    catch (e) { console.warn('[dexAgent] 流式不可用，降级为非流式:', e); }
    try {
      var result = await DeeCodexTauri.invoke('dex_chat', {
        messages: messages, tools: tools, stream: false,
        model: (this.selectedModel && this.selectedModel !== 'auto') ? this.selectedModel : null
      });
      return result;
    } catch (e) { console.error('[dexAgent] LLM 调用失败:', e); throw e; }
  },

  sendToLLMStream: async function (messages, tools) {
    var self = this;
    var fullContent = '';
    var fullReasoning = '';
    var finishReason = '';
    var toolCalls = null;
    _dexLastAssistantEl = null;
    var resolveStream, rejectStream;
    var streamPromise = new Promise(function (resolve, reject) {
      resolveStream = resolve; rejectStream = reject;
    });

    var unlisten = await window.DeeCodexTauri.listen('dex-chat-chunk', function (event) {
      try {
        if (self._aborted) { unlisten(); rejectStream(new Error('用户中止')); return; }
        var payload = event.payload;
        if (payload.done) {
          unlisten();
          dexHideThinking();
          _dexLastAssistantEl = null;
          self._streamed = true;
          var finalMsg = { role: 'assistant', content: fullContent || null };
          if (fullReasoning) finalMsg.reasoning_content = fullReasoning;
          if (toolCalls) finalMsg.tool_calls = toolCalls;
          resolveStream({ choices: [{ message: finalMsg, finish_reason: finishReason || 'stop' }] });
          return;
        }
        var chunk = payload.chunk;
        if (!chunk || !chunk.choices || !chunk.choices.length) return;
        var delta = chunk.choices[0].delta;
        if (!delta) return;
        if (delta.reasoning_content) { fullReasoning += delta.reasoning_content; dexUpdateLastAssistant(fullContent, fullReasoning); }
        if (delta.content) { fullContent += delta.content; dexUpdateLastAssistant(fullContent, fullReasoning); }
        if (delta.tool_calls) {
          if (!toolCalls) toolCalls = [];
          for (var i = 0; i < delta.tool_calls.length; i++) {
            var dtc = delta.tool_calls[i];
            var idx = dtc.index || 0;
            if (!toolCalls[idx]) toolCalls[idx] = { id: dtc.id || '', type: 'function', function: { name: '', arguments: '' } };
            if (dtc.id) toolCalls[idx].id = dtc.id;
            if (dtc.function) {
              if (dtc.function.name) toolCalls[idx].function.name += dtc.function.name;
              if (dtc.function.arguments) toolCalls[idx].function.arguments += dtc.function.arguments;
            }
          }
        }
        if (chunk.choices[0].finish_reason) finishReason = chunk.choices[0].finish_reason;
      } catch (e) { console.error('[dexAgent] 流式处理异常:', e); }
    });

    DeeCodexTauri.invoke('dex_chat', {
      messages: messages, tools: tools, stream: true,
      model: (self.selectedModel && self.selectedModel !== 'auto') ? self.selectedModel : null
    }).catch(function (e) { unlisten(); rejectStream(e); });

    return streamPromise;
  },

  listenStream: async function (messages, tools) { return await this.sendToLLMStream(messages, tools); },

  abort: function () {
    if (this._ocBusy && this._ocSessionId) {
      this._aborted = true;
      this._ocBusy = false;
      this.isProcessing = false;
      (this._ocWaiters || []).splice(0).forEach(function (r) { r(); });
      DeeCodexTauri.invoke('dex_opencode_abort', { sessionId: this._ocSessionId }).catch(function () {});
      return;
    }
    if (window.DexOpencode && window.DexOpencode.enabled) {
      this._aborted = true;
      this.isProcessing = false;
      window.DexOpencode.abort();
      return;
    }
    this._aborted = true;
    this.isProcessing = false;
  },

  // 诊断并尝试动态加载 opencode 客户端（文件可达性 + 是否可执行）
  _diagnoseOpencodeClient: async function () {
    var parts = [];
    try {
      var resp = await fetch('js/dex-assistant-opencode.js?v=' + Date.now());
      var text = await resp.text();
      parts.push('fetch ' + resp.status + ' len=' + text.length + ' head=' + text.slice(0, 40).replace(/\n/g, ''));
    } catch (e) {
      parts.push('fetch失败 ' + (e && e.message || e));
      return parts.join(' | ');
    }
    if (window.DexOpencode) { parts.push('已定义'); return parts.join(' | '); }
    // 动态注入脚本并等待执行
    try {
      var ok = await new Promise(function (resolve) {
        var s = document.createElement('script');
        s.src = 'js/dex-assistant-opencode.js?v=' + Date.now();
        s.onload = function () { resolve(!!window.DexOpencode); };
        s.onerror = function () { resolve(false); };
        document.head.appendChild(s);
      });
      parts.push(ok ? '注入后已定义' : '注入后仍 undefined');
    } catch (e) {
      parts.push('注入异常 ' + (e && e.message || e));
    }
    return parts.join(' | ');
  },

  // 自包含 opencode 驱动：直接走 Tauri 命令，不依赖 DexOpencode 前端状态机
  _opencodeRun: async function (text) {
    var self = this;
    // 1) 确保引擎在跑
    var status = await DeeCodexTauri.invoke('dex_opencode_status', {});
    if (!status || !status.running) {
      await DeeCodexTauri.invoke('dex_opencode_start', {});
      status = await DeeCodexTauri.invoke('dex_opencode_status', {});
    }
    if (!status || !status.running) {
      throw new Error('opencode 引擎未启动');
    }
    // 清理历史报错（旧 system 消息残留会破坏布局）
    try {
      var stale = document.querySelectorAll('.dex-msg-system');
      for (var si2 = 0; si2 < stale.length; si2++) stale[si2].remove();
    } catch (e) {}
    // 2) 订阅事件（渲染）
    if (!this._ocEventRegistered) {
      await DeeCodexTauri.invoke('dex_opencode_events_subscribe', {});
      this._ocUnlisten = await DeeCodexTauri.listen('dex-opencode-event', function (evt) {
        var payload = evt && evt.payload;
        self._handleOpencodeEvent((payload && payload.event) || payload);
      });
      this._ocEventRegistered = true;
    }
    // 3) 会话 + 发消息
    if (!this._ocSessionId) {
      var s = await DeeCodexTauri.invoke('dex_opencode_session_create', { title: 'DEX 助手' });
      if (!s || !s.id) throw new Error('创建 opencode 会话失败');
      this._ocSessionId = s.id;
    }
    this._ocBusy = true;
    this.isProcessing = true;
    this._aborted = false;
    dexShowStopButton();
    dexShowThinking();
    await DeeCodexTauri.invoke('dex_opencode_prompt', { sessionId: this._ocSessionId, text: text });
    // 等待 idle 事件或 300s 兜底
    var done = new Promise(function (resolve) {
      self._ocWaiters = self._ocWaiters || [];
      self._ocWaiters.push(resolve);
    });
    var timer = null;
    var timeout = new Promise(function (resolve) { timer = setTimeout(resolve, 300000); });
    try { await Promise.race([done, timeout]); } finally { clearTimeout(timer); }
    this._ocBusy = false;
  },

  _handleOpencodeEvent: function (raw) {
    var self = this;
    if (!raw || !raw.type) return;
    var type = String(raw.type);
    var props = raw.properties || {};
    var part = props.part || {};
    if (type === 'permission.asked') {
      var pid = String(props.id || '');
      var sid = String(props.sessionID || this._ocSessionId || '');
      var pname = String(props.permission || '').replace(/^(dex_diag|dex_workspace|dex_ops|dex_plugins)_/, '');
      var callId = (props.tool && props.tool.callID) || pid;
      dexShowInlineConfirm(pname, '确认执行高风险操作「' + pname + '」吗？', callId, {}).then(function (confirmed) {
        return DeeCodexTauri.invoke('dex_opencode_permission_reply', {
          sessionId: sid,
          permissionId: pid,
          response: confirmed ? 'once' : 'reject'
        });
      }).catch(function (e) { console.warn('[dexAgent] 权限回复失败:', e); });
      return;
    }
    if (type === 'session.idle' || (type === 'session.status' && props.status === 'idle')) {
      this._ocBusy = false;
      this._ocAssistantEl = null;
      this._ocToolEls = {};
      dexHideThinking();
      (this._ocWaiters || []).splice(0).forEach(function (r) { r(); });
      return;
    }
    if (type === 'session.error') {
      var errObj = props.error || {};
      var errMsg = (errObj.data && errObj.data.message) || errObj.message || props.message || '未知错误';
      var errStr = String(errMsg);
      if (/401|authentication|invalid|unauthor/i.test(errStr)) {
        errStr += '（请检查 DEX 助手账号是否有效，可切换到 MiniMax 等可用账号）';
      }
      dexAppendMessage('system', 'opencode 会话错误: ' + errStr);
      this._ocBusy = false;
      this._ocAssistantEl = null;
      (this._ocWaiters || []).splice(0).forEach(function (r) { r(); });
      return;
    }
    if (type !== 'message.part.updated') return;

    if (part.type === 'text' || part.type === 'reasoning') {
      var text = String(part.text || '');
      var thinkRe = /<(think|reasoning)>[\s\S]*?<\/\1>/g;
      var thinkParts = text.match(thinkRe) || [];
      var display = text.replace(thinkRe, '').trim();
      var reasoning = String(part.reasoning || '') + (thinkParts.length ? '\n' + thinkParts.join('\n') : '');
      var cleanReasoning = reasoning.replace(/\s*\n\s*/g, '\n').trim().slice(0, 4000);
      if (!this._ocAssistantEl) {
        this._ocAssistantEl = dexAppendMessage('assistant', display || '…', { model: window.dexAgent.selectedModel });
      } else {
        var bubble = this._ocAssistantEl.querySelector('.dex-bubble-text');
        if (bubble) bubble.innerHTML = dexRenderMarkdown(display || '…');
      }
      if (cleanReasoning) {
        var rEl = this._ocAssistantEl.querySelector('.dex-reasoning-content');
        if (rEl) rEl.textContent = cleanReasoning;
        var rWrap = this._ocAssistantEl.querySelector('.dex-reasoning-wrap');
        if (rWrap) rWrap.style.display = '';
      }
      dexScrollToBottom();
      return;
    }

    if (part.type === 'tool') {
      var toolId = String(part.tool || '');
      var name = toolId.replace(/^(dex_diag|dex_workspace|dex_ops|dex_plugins)_/, '');
      var st = part.state || {};
      var stStatus = String(st.status || '');
      var callKey = String(part.callID || part.id || name);
      if (stStatus === 'running') {
        var args = (st.input && typeof st.input === 'object') ? st.input : {};
        var argsStr = JSON.stringify(args);
        if (argsStr.length > 160) argsStr = argsStr.substring(0, 160) + '…';
        var argsShown = {};
        try { argsShown = JSON.parse(argsStr); } catch (e) { argsShown = args; }
        var el = dexAppendMessage('tool-start', name, { args: argsShown, toolDef: this._ocToolDef(name) });
        this._ocToolEls = this._ocToolEls || {};
        this._ocToolEls[callKey] = el;
      } else if (stStatus === 'completed' || stStatus === 'success') {
        var out = st.output;
        var parsed = null;
        if (typeof out === 'string') { try { parsed = JSON.parse(out); } catch (e) {} }
        if (parsed === null) parsed = out;
        var target = (this._ocToolEls && this._ocToolEls[callKey]) || this._ocLastToolStart();
        dexUpdateMessage(target, 'tool-result', name, { result: parsed ? dexMaskApiKey(parsed) : parsed, toolDef: this._ocToolDef(name) });
      } else if (stStatus === 'error' || stStatus === 'failed') {
        var target2 = (this._ocToolEls && this._ocToolEls[callKey]) || this._ocLastToolStart();
        dexUpdateMessage(target2, 'tool-error', name + ': ' + (st.error || st.output || '失败'), { error: st.error || '' });
      }
      return;
    }
  },

  _ocToolDef: function (name) {
    if (!window.DEX_TOOLS || !Array.isArray(window.DEX_TOOLS)) return null;
    for (var i = 0; i < window.DEX_TOOLS.length; i++) {
      if (window.DEX_TOOLS[i].name === name) return window.DEX_TOOLS[i];
    }
    return null;
  },

  _ocLastToolStart: function () {
    var els = document.querySelectorAll('.dex-msg-tool-start');
    return els.length ? els[els.length - 1] : null;
  },

  run: async function (userMessage) {
    if (this.isProcessing) { showToast('Agent 正在处理中，请等待', 'warn'); return; }
    // opencode 是唯一自动路径（v1）。旧 dex_chat 循环仅当显式开启
    // deeStorage.dex_opencode_engine='legacy' 时才会执行；引擎不可用直接报错，不静默兜底。
    var engineMode = '';
    try { engineMode = window.deeStorage.getItem('dex_opencode_engine') || ''; } catch (e) {}
    if (engineMode !== 'legacy') {
      try {
        await this._opencodeRun(userMessage);
      } catch (e) {
        console.error('[dexAgent] opencode 请求失败:', e);
        dexAppendMessage('system', '请求失败: ' + (e.message || e));
        showToast('请求失败: ' + (e.message || e), 'error');
      } finally {
        this.isProcessing = false;
        this._aborted = false;
        dexHideStopButton();
        dexHideThinking();
      }
      return;
    }
    // ── 显式 legacy 模式：旧 dex_chat 循环（仅 dex_opencode_engine='legacy' 时可达）──
    this.isProcessing = true;
    this._aborted = false;
    this._streamed = false;
    this._toolCache = {};
    this.roundCount = 0;
    this.messages.push({ role: 'user', content: userMessage });
    this.compressContext();
    this.saveHistory();
    dexShowStopButton();

    while (this.roundCount < this.maxRounds && !this._aborted) {
      this.roundCount++;
      this.compressContext();
      try {
        var response = await this.sendToLLM(this.messages, this.buildOpenAITools());
        var choice = response.choices && response.choices[0];
        if (!choice) { dexAppendMessage('system', 'LLM 返回了空的响应，请重试'); break; }
        var msg = choice.message;

        if (msg.tool_calls && msg.tool_calls.length > 0) {
          this.messages.push(msg);
          if (msg.content && !this._streamed) {
            dexAppendMessage('assistant', msg.content);
          }
          for (var i = 0; i < msg.tool_calls.length; i++) {
            var tc = msg.tool_calls[i];
            var toolResult = await this.executeTool(tc);
            if (window.DexAgentState && window.DexAgentState.shouldStopAfterToolResult(toolResult)) {
              window.DexAgentState.removePendingAssistantToolCall(this.messages);
              this.saveHistory();
              break;
            }
            var compact = toolResult;
            if (toolResult && toolResult.success) {
              var fn = tc.function.name;
              if (fn === 'get_config') compact = { success: true, summary: '已获取完整配置，含 ' + Object.keys(toolResult.data||{}).length + ' 项' };
              else if (fn === 'get_logs') compact = { success: true, summary: (Array.isArray(toolResult.data)?toolResult.data.length:'?') + ' 行日志' };
              else if (fn === 'list_threads' || fn === 'list_request_history' || fn === 'list_sessions' || fn === 'list_plugins' || fn === 'list_accounts')
                compact = { success: true, count: Array.isArray(toolResult.data)?toolResult.data.length:'?', summary: '列表已获取' };
            }
            compact = dexMaskApiKey(compact);
            var resultStr = JSON.stringify(compact);
            if (resultStr.length > 2000) resultStr = resultStr.substring(0, 2000) + '…';
            this.messages.push({ role: 'tool', tool_call_id: tc.id, content: resultStr });
          }
          if (window.DexAgentState && window.DexAgentState.shouldStopAfterToolResult(toolResult)) break;
          this.saveHistory();
          continue;
        }

        if (msg.content) {
          if (!this._streamed) dexAppendMessage('assistant', msg.content);
          this.messages.push(msg);
          this.saveHistory();
        }
        break;
      } catch (e) {
        dexAppendMessage('system', '请求失败: ' + (e.message || e));
        showToast('请求失败: ' + (e.message || e), 'error');
        break;
      }
    }
    if (this._aborted)
      dexAppendMessage('system', '已停止生成');
    else if (this.roundCount >= this.maxRounds && this.isProcessing)
      dexAppendMessage('system', '已达到最大对话轮数（' + this.maxRounds + '），请开启新对话继续。');
    this.isProcessing = false;
    this._aborted = false;
    this.roundCount = 0;
    dexHideStopButton();
    dexHideThinking();
    _dexLastAssistantEl = null;
  },

  executeTool: async function (toolCall) {
    var fnName = toolCall.function.name;
    var fnArgs = {};
    try { fnArgs = JSON.parse(toolCall.function.arguments || '{}'); }
    catch (e) { return { error: '参数解析失败: ' + e.message }; }

    var toolDef = null;
    for (var i = 0; i < DEX_TOOLS.length; i++) { if (DEX_TOOLS[i].name === fnName) { toolDef = DEX_TOOLS[i]; break; } }
    if (!toolDef) return { error: '未知工具: ' + fnName };

    var cacheKey = fnName + '|' + JSON.stringify(fnArgs);
    if (this._toolCache[cacheKey] !== undefined) {
      var cached = this._toolCache[cacheKey];
      if (cached.success) {
        dexAppendMessage('tool-result', fnName, { result: cached, history: false });
        return cached;
      }
      return cached;
    }

    var startedAt = Date.now();
    var statusEl = dexAppendMessage('tool-start', fnName, { args: fnArgs, toolDef: toolDef });

    if (toolDef.level >= 3) {
      var confirmText = toolDef.confirm || ('确定要执行高风险工具 ' + fnName + ' 吗？');
      var confirmed = await dexShowInlineConfirm(fnName, confirmText, toolCall.id, fnArgs);
      if (!confirmed) {
        dexUpdateMessage(statusEl, 'tool-error', fnName + ': 用户取消了操作', { error: '用户取消了 L3 操作', toolDef: toolDef, elapsedMs: Date.now() - startedAt });
        return { cancelled: true, error: '用户取消了 L3 操作: ' + fnName };
      }
      if (fnName === 'execute_shell') fnArgs.confirmed = true;
    }

    if (['run_full_diagnostics','run_diagnostics','validate_config'].indexOf(toolDef.tauriCmd) >= 0) {
      if (!fnArgs.config) {
        try { var cfg = await DeeCodexTauri.invoke('get_config'); fnArgs.config = cfg; }
        catch (e) {}
      }
    }

    if (toolDef.tauriCmd.indexOf('plugin') >= 0) {
      fnArgs = dexNormalizePluginArgs(fnArgs);
    }

    var errKey = fnName + '|' + JSON.stringify(fnArgs);
    if (this._lastErrorKey === errKey) {
      dexUpdateMessage(statusEl, 'tool-error', fnName + ': 跳过（重复失败）', { error: '与上次相同错误，不再重试' });
      return { error: '工具重复失败，已跳过: ' + fnName };
    }

    var lastError = null;
    for (var retry = 0; retry < 3; retry++) {
      try {
        var result = await DeeCodexTauri.invoke('dex_execute_tool', {
          name: fnName,
          args: fnArgs,
          confirmed: toolDef.level >= 3 ? true : undefined
        });
        this._lastErrorKey = null;
        this._toolCache[cacheKey] = { success: true, data: result };
        dexUpdateMessage(statusEl, 'tool-result', fnName, { result: result, success: true, toolDef: toolDef, elapsedMs: Date.now() - startedAt });
        dexAfterMutate(fnName);
        return { success: true, data: result };
      } catch (e) {
        lastError = e;
        var isTransient = /timeout|timed.?out|network|connection|ECONN|abort/i.test(String(e.message || e || ''));
        if (!isTransient) break;
        console.warn('[dexAgent] 瞬态错误，重试 ' + (retry + 1) + '/3:', fnName, e);
        if (retry < 2) dexUpdateMessage(statusEl, 'tool-start', fnName + ': 第' + (retry + 1) + '次尝试失败，正在重试…', { args: fnArgs, toolDef: toolDef });
      }
    }
    this._lastErrorKey = errKey;
    var finalErr = (lastError && (lastError.message || lastError)) ? String(lastError.message || lastError) : '未知错误';
    this._toolCache[cacheKey] = { error: '工具执行失败: ' + finalErr };
    dexUpdateMessage(statusEl, 'tool-error', fnName + ': 失败', { error: finalErr, toolDef: toolDef, elapsedMs: Date.now() - startedAt });
    return { error: '工具执行失败: ' + finalErr };
  },

  buildOpenAITools: function () {
    var tools = [];
    for (var i = 0; i < DEX_TOOLS.length; i++) {
      var t = DEX_TOOLS[i];
      tools.push({ type: 'function', function: { name: t.name, description: t.description, parameters: t.parameters } });
    }
    return tools;
  },

  handleStream: async function () {},

  saveHistory: function () {
    try {
      var history = [];
      for (var i = 0; i < this.messages.length; i++)
        if (this.messages[i].role !== 'system') history.push(this.messages[i]);
      if (history.length > this.maxHistorySize) history = history.slice(history.length - this.maxHistorySize);
      window.deeStorage.setItem('dex_chat_history', JSON.stringify(history));
    } catch (e) { console.warn('[dexAgent] 保存历史失败:', e); }
  },

  loadHistory: function () {
    try {
      var raw = window.deeStorage.getItem('dex_chat_history');
      if (raw) {
        var history = JSON.parse(raw);
        this.messages = [{ role: 'system', content: dexBuildSystemPrompt() }];
        for (var i = 0; i < history.length; i++) this.messages.push(history[i]);
        return history;
      }
    } catch (e) { console.warn('[dexAgent] 加载历史失败:', e); }
    return [];
  }
};
