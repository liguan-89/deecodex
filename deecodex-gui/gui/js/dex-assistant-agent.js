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
    console.log('[dexAgent] 上下文已压缩: ' + (this.messages.length) + ' 条消息');
  },

  _canStream: function () {
    try {
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

  abort: function () { this._aborted = true; this.isProcessing = false; },

  run: async function (userMessage) {
    if (this.isProcessing) { showToast('Agent 正在处理中，请等待', 'warn'); return; }
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
      console.log('[dexAgent] 缓存命中:', fnName);
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
