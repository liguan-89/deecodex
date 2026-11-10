// DEX 助手 opencode 引擎客户端（v1）
// ═══════════════════════════════════════════════════════════════
// 职责：把 opencode serve 的会话/事件/权限回环映射到现有 DEX 助手 UI。
// 所有 HTTP 请求都通过 Tauri 命令走 Rust 侧（不直连 opencode）。
// 引擎不可用或关闭时由 dex-assistant-agent.js 回退到旧 dex_chat 循环。
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
