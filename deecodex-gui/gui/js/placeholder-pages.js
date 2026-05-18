// DEX助手
// ═══════════════════════════════════════════════════════════════

// ── CSS 注入：DEX 运行态补丁样式（一次性）──
(function () {
  if (!document.getElementById('dex-inline-style')) {
    var style = document.createElement('style');
    style.id = 'dex-inline-style';
    style.textContent = [
      '.dex-spinner{display:inline-block;width:14px;height:14px;border:2px solid var(--border-color,#334155);border-top-color:var(--accent-color,#00c8e8);border-radius:50%;animation:dex-spin 0.8s linear infinite;vertical-align:middle;margin-right:6px;flex-shrink:0}',
      '@keyframes dex-spin{to{transform:rotate(360deg)}}',
      '.dex-chat-header{min-width:0}',
      '.dex-header-title{min-width:0;flex:1 1 auto}',
      '.dex-header-title h3{overflow:hidden;text-overflow:ellipsis}',
      '.dex-header-actions{flex:0 1 auto;justify-content:flex-end}',
      '.dex-header-actions .dex-icon-btn{width:28px;height:28px;padding:0;display:inline-flex;align-items:center;justify-content:center;font-size:13px;line-height:1}',
      '.dex-tool-msg{flex-wrap:wrap;align-items:flex-start}',
      '.dex-tool-name{min-width:0;max-width:100%;overflow:hidden;text-overflow:ellipsis}',
      '.dex-tool-args{flex:1 1 160px;min-width:0;max-width:none}',
      '.dex-tool-summary{margin-left:auto;min-width:0;max-width:45%;overflow:hidden;text-overflow:ellipsis}',
      '.dex-tool-details{flex:1 0 100%;margin-top:4px;font-size:12px}',
      '.dex-tool-details summary{cursor:pointer;color:var(--text-secondary,#6b7fa8)}',
      '.dex-tool-details pre{max-height:300px;overflow:auto;font-size:11px;margin-top:4px;padding:6px 8px;white-space:pre-wrap;word-break:break-word}',
      '.dex-tool-error-details summary{color:var(--error-color,#ef4444)}',
      '.dex-confirm-command{margin-top:8px;padding:8px 10px;border:1px solid rgba(240,144,64,0.28);border-radius:4px;background:rgba(0,0,0,0.24);font-family:var(--font-mono);font-size:11px;line-height:1.5;color:var(--text-primary);white-space:pre-wrap;word-break:break-word}',
      '.dex-msg-highlight .dex-bubble{outline:2px solid var(--accent-color,#00c8e8);outline-offset:2px;border-radius:8px}',
      '.dex-msg-highlight.dex-msg-search-current .dex-bubble{outline-color:#f59e0b;outline-width:3px}',
      '.dex-token-count{font-size:11px;color:var(--text-secondary,#6b7fa8);white-space:nowrap;margin-right:6px;align-self:center}',
      '.dex-cap-chips{display:flex;flex-wrap:wrap;gap:6px;padding:6px 16px 8px;border-bottom:1px solid rgba(148,163,184,0.12)}',
      '.dex-cap-chip{border:1px solid var(--border-default,#26364d);background:rgba(0,0,0,0.12);color:var(--text-secondary,#6b7fa8);border-radius:5px;padding:4px 8px;font-size:11px;line-height:1.35;cursor:pointer}',
      '.dex-cap-chip.on{color:var(--text-primary,#c4d0e4);border-color:rgba(0,200,232,0.32);background:rgba(0,200,232,0.08)}',
      '.dex-cap-chip.off{opacity:0.55}',
      '.dex-mode-tabs{display:flex;flex-wrap:wrap;gap:6px;padding:8px 16px 6px;border-bottom:1px solid rgba(148,163,184,0.10)}',
      '.dex-mode-tab{border:1px solid var(--border-default,#26364d);background:rgba(0,0,0,0.10);color:var(--text-secondary,#6b7fa8);border-radius:5px;padding:5px 10px;font-size:11px;line-height:1.35;cursor:pointer}',
      '.dex-mode-tab.active{color:var(--text-primary,#c4d0e4);border-color:rgba(0,200,232,0.4);background:rgba(0,200,232,0.1)}',
      '.dex-tool-catalog{padding:5px 16px;border-bottom:1px solid rgba(148,163,184,0.10);font-size:11px;color:var(--text-secondary,#6b7fa8)}',
      '.dex-tool-catalog summary{cursor:pointer;color:var(--text-secondary,#6b7fa8)}',
      '.dex-tool-catalog-list{display:grid;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));gap:5px;margin-top:5px;max-height:180px;overflow:auto}',
      '.dex-tool-catalog-item{border:1px solid rgba(148,163,184,0.16);border-radius:4px;padding:4px 6px;background:rgba(0,0,0,0.12);min-width:0}',
      '.dex-tool-catalog-name{display:block;color:var(--text-primary,#c4d0e4);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}',
      '.dex-tool-catalog-meta{display:block;margin-top:2px;color:var(--text-secondary,#6b7fa8)}',
      '.dex-tool-preview{font-size:11px;color:var(--accent-color,#00c8e8);padding:0 0 6px;font-style:italic}',
      '.dex-tool-badge{display:inline-flex;align-items:center;gap:4px;border:1px solid rgba(148,163,184,0.22);border-radius:4px;padding:1px 5px;font-size:10px;color:var(--text-secondary,#6b7fa8);white-space:nowrap}',
      '.dex-tool-actions{flex:1 0 100%;display:flex;gap:6px;margin-top:6px}',
      '.dex-sr-only{position:absolute;width:1px;height:1px;padding:0;margin:-1px;overflow:hidden;clip:rect(0,0,0,0);white-space:nowrap;border:0}',
      '@media (max-width:720px){.dex-chat-header{align-items:flex-start;flex-direction:column}.dex-header-actions{width:100%;justify-content:flex-start}.dex-model-drop{max-width:100%}.dex-model-btn{max-width:220px}.dex-tool-summary{max-width:100%;margin-left:0}.dex-input-row{grid-template-columns:1fr}.dex-input-row .btn{width:100%;min-height:40px}}'
    ].join('\n');
    document.head.appendChild(style);
  }
})();

// ── 工具定义由后端能力注册中心动态提供 ──
var DEX_TOOLS = [];
var DEX_CAPABILITIES = [];
var DEX_WORKSPACE_CONTEXT = null;
var DEX_MODE = (window.deeStorage && window.deeStorage.getItem('dex_mode')) || 'diag';

var DEX_MODES = {
  diag: { label: '诊断', prompt: '优先使用只读诊断工具定位问题，先给风险概览，再建议修复路径。' },
  fix: { label: '修复', prompt: '先诊断，再执行 L2 修复；L3 必须通过前端内联确认。每次修复后复核结果。' },
  workspace: { label: '工作区', prompt: '聚焦当前项目上下文、Git 状态、配置文件、CLI 版本、端口和受限文件读取。' },
  plugins: { label: '插件', prompt: '聚焦插件状态、插件账号、插件暴露给 DEX 的工具和插件 JSON-RPC 错误。' },
  cost: { label: '成本', prompt: '聚焦请求历史、Token 消耗、缓存命中、模型分布、失败率和成本估算。' },
  logs: { label: '日志', prompt: '聚焦日志异常、最近错误、关键栈信息和可复现线索。' }
};

// ── System Prompt ──
var DEX_SYSTEM_PROMPT = [
  '你是 DEX助手 2.0，一个 AI工具工作台 Agent，运行在 deecodex GUI 内置的 DEX助手 面板中。',
  '你的首版范围聚焦 Codex、Claude、MCP、模型账号、日志、请求历史、线程、插件和项目环境诊断。',
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
  '- 不泄露 api_key、authorization、token、secret、password 等敏感信息。',
  '- 插件工具通过统一代理执行，遵循同一安全分级。'
].join('\n');

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
    return '- ' + t.name + ' [L' + (t.level || 0) + ' / ' + (t.source || 'builtin') + ' / ' + (t.capability || 'core') + ']：' + (t.description || '');
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

// ── Agent 核心对象 ──
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
      this.compressContext(); // 每轮检查，避免 tool_calls 累积撑爆上下文
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
            // 发回 LLM 时精简：诊断/配置/日志等大结果用摘要代替
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

    // 工具缓存检查：同一轮中相同工具+相同参数只执行一次
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

    // 诊断/校验命令自动注入当前配置
    if (['run_full_diagnostics','run_diagnostics','validate_config'].indexOf(toolDef.tauriCmd) >= 0) {
      if (!fnArgs.config) {
        try { var cfg = await DeeCodexTauri.invoke('get_config'); fnArgs.config = cfg; }
        catch (e) { /* 降级：无 config 也能跑部分检查 */ }
      }
    }

    if (toolDef.tauriCmd.indexOf('plugin') >= 0) {
      fnArgs = dexNormalizePluginArgs(fnArgs);
    }

    // 错误去重：同一工具同参数连续失败不重试
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
        // 影响全局状态的工具执行后刷新状态栏 + 通知其他面板
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
    dexRenderCapabilityChips();
    dexRenderModeTabs();
    dexRenderToolCatalog();
  } catch (e) {
    DEX_TOOLS = [];
    console.warn('[dexAgent] 动态工具加载失败，当前不暴露工具清单:', e);
  }
}

function dexRenderCapabilityChips() {
  var el = document.getElementById('dexCapabilityChips');
  if (!el) return;
  if (!DEX_CAPABILITIES || !DEX_CAPABILITIES.length) {
    el.innerHTML = '<span class="dex-cap-chip">默认能力</span>';
    return;
  }
  el.innerHTML = DEX_CAPABILITIES.map(function(c) {
    var cls = c.enabled ? 'dex-cap-chip on' : 'dex-cap-chip off';
    return '<button class="' + cls + '" onclick="dexToggleCapability(\'' + escAttr(c.id) + '\',' + (!c.enabled) + ')" title="' + escAttr(c.description || '') + '">' + esc(c.label || c.id) + ' · ' + (c.tool_count || 0) + '</button>';
  }).join('');
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
      + '<span class="dex-tool-catalog-meta">L' + (t.level || 0) + ' · ' + esc(t.source || 'builtin') + ' · ' + esc(t.capability || 'core') + '</span>'
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
    dexRenderModeTabs();

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

    // 输入时显示工具预览
    if (input) input.addEventListener('input', function () {
      var val = input.value.trim();
      var preview = dexPreviewTools(val);
      var existing = document.getElementById('dexToolPreview');
      if (!existing) return;
      if (preview) {
        existing.textContent = preview;
        existing.style.display = '';
      } else {
        existing.style.display = 'none';
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

    // 状态栏初始刷新
    dexRefreshStatus();
    if (!window._dexStatusTimer) {
      window._dexStatusTimer = setInterval(dexRefreshStatus, 30000);
    }

    // Token 计数初始
    dexUpdateTokenCount();

    // 快捷键
    dexBindShortcuts();
  }, 0);

  return '<div class="dex-chat-panel"><div class="dex-chat-header"><div class="dex-header-title">'
    + '<div class="dex-title-row"><span class="dex-title-mark"></span><h3 title="DEX助手 2.0">DEX助手 2.0</h3><span class="dex-title-sub">AI 工具工作台</span></div>'
    + '<div class="dex-status-bar" id="dexStatusBar"><span class="dex-status-dot" id="dexStatusDot"></span><span id="dexStatusText">加载中...</span></div></div><div class="dex-header-actions">'
    + '<div class="dex-model-drop" id="dexModelDrop"><button class="dex-model-btn" id="dexModelBtn" onclick="dexToggleModelMenu(event)">模型 ▾</button><div class="dex-model-menu" id="dexModelMenu" style="display:none"></div></div>'
    + '<button class="btn btn-ghost btn-sm dex-icon-btn" onclick="dexExportChat()" title="导出对话" aria-label="导出对话">⇩<span class="dex-sr-only">导出对话</span></button>'
    + '<button class="btn btn-ghost btn-sm dex-icon-btn" id="dexSearchBtn" onclick="dexToggleSearch()" title="搜索对话" aria-label="搜索对话">⌕<span class="dex-sr-only">搜索对话</span></button>'
    + '<button class="btn btn-ghost btn-sm dex-icon-btn" onclick="dexNewChat()" title="新对话" aria-label="新对话">＋<span class="dex-sr-only">新对话</span></button>'
    + '<button class="btn btn-ghost btn-sm dex-icon-btn" onclick="dexClearChat()" title="清空对话" aria-label="清空对话">⌫<span class="dex-sr-only">清空对话</span></button></div></div>'
    + '<div class="dex-search-bar" id="dexSearchBar" style="display:none"><input id="dexSearchInput" placeholder="搜索对话..." /><span class="dex-search-count" id="dexSearchCount"></span><button class="btn btn-ghost btn-sm" onclick="dexCloseSearch()">关闭</button></div>'
    + '<div class="dex-mode-tabs" id="dexModeTabs"></div>'
    + '<div class="dex-cap-chips" id="dexCapabilityChips"></div>'
    + '<div id="dexToolCatalog"></div>'
    + '<div class="dex-chat-messages" id="dexMessages">' + dexWelcomeHTML() + '</div>'
    + '<div class="dex-input-area" id="dexInputAreaWrap">'
    + '<div id="dexToolPreview" class="dex-tool-preview" style="display:none"></div>'
    + '<div class="dex-input-row">'
    + '<textarea id="dexInput" placeholder="输入消息…（Enter 发送，Shift+Enter 换行 / /diag /fix 快捷指令）" rows="2"></textarea>'
    + '<button class="btn btn-primary dex-send-btn" id="dexSendBtn" onclick="dexSendMessage()">发送</button>'
    + '<button class="btn btn-danger" id="dexStopBtn" onclick="dexStopAgent()" style="display:none">停止</button>'
    + '</div>'
    + '<div class="dex-input-foot"><span class="dex-token-count" id="dexTokenCount">~0 tokens</span></div>'
    + '</div></div>';
}

function dexWelcomeHTML() {
  return '<div class="dex-msg dex-msg-assistant dex-welcome"><div class="dex-bubble dex-bubble-assistant dex-welcome-bubble"><div class="dex-bubble-text">'
    + '<p>DEX助手 2.0 就绪。描述问题，或选择一个快速操作：</p></div>'
    + '<div class="dex-quick-actions">'
    + '<button class="btn btn-sm btn-primary" onclick="dexQuickAction(\'运行完整诊断，自动修复所有发现的问题\')">一键修复</button>'
    + '<button class="btn btn-sm btn-ghost" onclick="dexQuickAction(\'健康概览，指出当前 AI 工具链风险\')">健康概览</button>'
    + '<button class="btn btn-sm btn-ghost" onclick="dexQuickAction(\'运行 DEX 自检，检查工具注册表、能力包、插件工具和最近错误\')">DEX自检</button>'
    + '<button class="btn btn-sm btn-ghost" onclick="dexQuickAction(\'检查 Codex、Claude、MCP 和模型账号环境\')">AI工具诊断</button>'
    + '<button class="btn btn-sm btn-ghost" onclick="dexQuickAction(\'检查 Claude Code MCP 集成状态\')">MCP检查</button>'
    + '<button class="btn btn-sm btn-ghost" onclick="dexQuickAction(\'查看插件状态和可用插件能力\')">插件状态</button>'
    + '<button class="btn btn-sm btn-ghost" onclick="dexQuickAction(\'分析当前项目工作区环境\')">项目环境</button>'
    + '<button class="btn btn-sm btn-ghost" onclick="dexQuickAction(\'读日志，检查异常\')">日志异常</button></div></div></div>';
}

function dexDisposePanel() {
  if (window._dexStatusTimer) {
    clearInterval(window._dexStatusTimer);
    window._dexStatusTimer = null;
  }
}

function dexToolMetaHtml(meta) {
  if (!meta) return '';
  var parts = [];
  if (meta.toolDef) {
    parts.push('L' + (meta.toolDef.level || 0));
    parts.push(meta.toolDef.source || 'builtin');
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
    return '<div class="dex-tool-actions"><button class="btn btn-ghost btn-sm" onclick="switchPanel(\'plugins\')">打开插件管理</button></div>';
  }
  if (fnName === 'get_threads_status' || fnName === 'list_threads') {
    return '<div class="dex-tool-actions"><button class="btn btn-ghost btn-sm" onclick="switchPanel(\'threads\')">打开线程面板</button></div>';
  }
  return '';
}

// ── UI 交互 ──
function dexAppendMessage(type, content, meta) {
  var container = document.getElementById('dexMessages');
  if (!container) return null;
  // 新消息到达时自动收起旧详情
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
        + (isHistory ? '<span class="dex-tool-icon">🔧</span>' : '<span class="dex-spinner"></span>')
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
        + '<span class="dex-tool-icon">✅</span>'
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
        + '<span class="dex-tool-icon">❌</span>'
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
        + '<span class="dex-tool-icon">✅</span>'
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
        + '<span class="dex-tool-icon">❌</span>'
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

// 格式化工具结果详情文本（日志等特殊处理）
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
    case 'get_service_status': if (data && data.running) return '✅运行中 :'+data.port; return '⏸已停止';
    case 'fetch_balance': if (data) { var b = data.balance || data.total_balance || data.remaining; if (b) return b; } return '已查询';
    case 'get_config': return '已获取';
    case 'get_logs': if (Array.isArray(data)) return data.length + ' 行'; if (typeof data === 'string') return data.split('\n').length + ' 行'; break;
    case 'search_logs': if (data && data.matches !== undefined) return data.matches + ' 处匹配'; break;
    case 'list_sessions': if (Array.isArray(data)) return data.length + ' 个会话'; break;
    case 'list_threads': if (Array.isArray(data)) return data.length + ' 个线程'; break;
    case 'list_plugins': if (Array.isArray(data)) return data.length + ' 个插件'; break;
    case 'list_request_history': if (Array.isArray(data)) return data.length + ' 条'; break;
    case 'test_upstream_connectivity': if (data && data.ok !== undefined) return data.ok ? '✅连通'+(data.latency_ms||'')+'ms' : '❌失败'; break;
    case 'check_upgrade': if (data && data.latest) return data.latest; break;
    case 'get_threads_status': if (data && data.total !== undefined) return data.total + ' 个线程'; break;
    case 'get_env_info': if (data && data.os) return data.os + ' · deecodex ' + data.deecodex_version; break;
    case 'health_summary': if (data) return (data.service.running?'🟢':'🔴') + ' svc ' + (data.account.ok?'🟢':'🔴') + ' acct · ' + data.recent_errors + ' err'; break;
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

// ── Markdown 渲染（增强版）──
function dexRenderMarkdown(text) {
  if (!text) return '';
  var codeBlocks = [];
  var safe = text.replace(/```(\w*)\n([\s\S]*?)```/g, function (m, lang, code) {
    codeBlocks.push('<pre><code class="dex-code-block">' + esc(code.trim()) + '</code></pre>');
    return '%%CODEBLOCK_' + (codeBlocks.length - 1) + '%%';
  });
  var inlineCodes = [];
  safe = safe.replace(/`([^`]+)`/g, function (m, code) {
    inlineCodes.push('<code class="dex-inline-code">' + esc(code) + '</code>');
    return '%%INLINECODE_' + (inlineCodes.length - 1) + '%%';
  });
  var html = esc(safe);
  var lines = html.split('\n');
  var result = [], i = 0;
  var inTable = false, tableRows = [];
  var inBQ = false, bqLines = [];
  var inList = false, listItems = [], listOrd = false;

  function fBQ() { if (bqLines.length) { result.push('<blockquote><p>' + bqLines.join('<br>') + '</p></blockquote>'); bqLines = []; inBQ = false; } }
  function fTbl() { if (tableRows.length) { result.push(dexRenderTable(tableRows)); tableRows = []; inTable = false; } }
  function fList() { if (listItems.length) { var h = ''; for (var li = 0; li < listItems.length; li++) h += '<li>' + listItems[li] + '</li>'; result.push(listOrd ? '<ol>' + h + '</ol>' : '<ul>' + h + '</ul>'); listItems = []; inList = false; listOrd = false; } }
  function fAll() { fBQ(); fTbl(); fList(); }

  while (i < lines.length) {
    var line = lines[i].trim();
    if (line === '') { fAll(); result.push(''); i++; continue; }
    if (/^(-{3,}|\*{3,})$/.test(line)) { fAll(); result.push('<hr>'); i++; continue; }
    if (/^&gt; /.test(line)) { fTbl(); fList(); inBQ = true; bqLines.push(line.replace(/^&gt; /, '')); i++; continue; }
    if (/^\|.+\|$/.test(line) && line.indexOf('|') > 0) {
      fBQ(); fList();
      if (i + 1 < lines.length && /^\|[-: |]+\|$/.test(lines[i + 1].trim())) { inTable = true; tableRows.push(line); i++; continue; }
      if (inTable) { tableRows.push(line); i++; continue; }
    }
    if (inTable && !/^\|.+\|$/.test(line)) fTbl();
    if (!inBQ && bqLines.length) fBQ();
    if (/^\d+\. .+/.test(line)) { if (!inList || !listOrd) { fList(); inList = true; listOrd = true; } listItems.push(line.replace(/^\d+\. /, '')); i++; continue; }
    if (/^[-*] .+/.test(line)) { if (!inList || listOrd) { fList(); inList = true; listOrd = false; } listItems.push(line.replace(/^[-*] /, '')); i++; continue; }
    if (inList) fList();
    if (/^### .+/.test(line)) { result.push('<h4 class="dex-md-h4">' + line.replace(/^### /, '') + '</h4>'); i++; continue; }
    if (/^## .+/.test(line)) { result.push('<h3 class="dex-md-h3">' + line.replace(/^## /, '') + '</h3>'); i++; continue; }
    result.push(line); i++;
  }
  fAll();
  html = result.join('\n');
  html = html.replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2" target="_blank" rel="noopener">$1</a>');
  html = html.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  html = html.replace(/\*([^*]+)\*/g, '<em>$1</em>');
  html = html.replace(/%%CODEBLOCK_(\d+)%%/g, function (m, n) { return codeBlocks[parseInt(n)]; });
  html = html.replace(/%%INLINECODE_(\d+)%%/g, function (m, n) { return inlineCodes[parseInt(n)]; });
  html = html.replace(/\n\n/g, '</p><p>');
  html = html.replace(/\n/g, '<br>');
  // 不包裹表格/代码块等块级元素
  if (/<(table|pre|blockquote|h[3-4]|ul|ol|hr)/.test(html))
    return '<div class="dex-md">' + html + '</div>';
  return '<p>' + html + '</p>';
}

function dexRenderTable(rows) {
  if (rows.length === 0) return '';
  var h = '<table class="dex-md-table"><thead><tr>';
  var hdr = rows[0].replace(/^\|/, '').replace(/\|$/, '').split('|');
  for (var ci = 0; ci < hdr.length; ci++) h += '<th>' + hdr[ci].trim() + '</th>';
  h += '</tr></thead><tbody>';
  for (var ri = 1; ri < rows.length; ri++) {
    h += '<tr>';
    var cells = rows[ri].replace(/^\|/, '').replace(/\|$/, '').split('|');
    for (var cj = 0; cj < cells.length; cj++) h += '<td>' + cells[cj].trim() + '</td>';
    h += '</tr>';
  }
  return h + '</tbody></table>';
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

function dexShowThinking() {
  dexHideThinking();
  var c = document.getElementById('dexMessages');
  if (!c) return;
  var el = document.createElement('div');
  el.className = 'dex-msg dex-msg-thinking';
  el.id = 'dexThinking';
  el.innerHTML = '<div class="dex-bubble dex-bubble-assistant"><div class="dex-thinking-dots"><span></span><span></span><span></span></div></div>';
  c.appendChild(el);
  dexScrollToBottom();
}

function dexHideThinking() { var el = document.getElementById('dexThinking'); if (el) el.remove(); }

var _dexLastAssistantEl = null;
var _dexLastReasoningEl = null;
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

function dexScrollToBottom() { var c = document.getElementById('dexMessages'); if (c) c.scrollTop = c.scrollHeight; }

function dexQuickAction(prompt) { var i = document.getElementById('dexInput'); if (i) i.value = prompt; dexSendMessage(); }

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

// ── 自定义模型下拉 ──
window.dexAgent.selectedModel = 'auto';
function dexToggleModelMenu(e) { e.stopPropagation();
  var m = document.getElementById('dexModelMenu');
  if (m) m.style.display = m.style.display === 'none' ? '' : 'none';
}
function dexSelectModel(v, label) {
  var btn = document.getElementById('dexModelBtn');
  if (btn) btn.innerHTML = label + ' ▾';
  window.dexAgent.selectedModel = v;
  document.getElementById('dexModelMenu').style.display = 'none';
}
function dexChangeModel() {} // 兼容旧引用

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
// 点击其他地方关闭下拉
document.addEventListener('click', function() {
  var m = document.getElementById('dexModelMenu');
  if (m) m.style.display = 'none';
});

// ── 停止按钮 ──
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

// ── API Key 脱敏 ──
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

// ── ANSI 转义码剥离 ──
function dexStripAnsi(text) {
  if (!text) return text;
  if (Array.isArray(text)) return text.map(dexStripAnsi);
  if (typeof text !== 'string') return text;
  return text.replace(/\x1B\[[0-9;]*[a-zA-Z]/g, '').replace(/\[[0-9;]*[a-zA-Z]/g, '');
}

// ── 工具执行后联动刷新 GUI ──
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
    // 刷新全局配置缓存，让其他面板立即看到变更
    if (fnName === 'save_config' && typeof loadConfig === 'function') {
      loadConfig().catch(function(){});
    }
    if (fnName === 'start_service' || fnName === 'stop_service') {
      // 刷新服务概览面板的 _statusData
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
        // 更新侧边栏连接指示器
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

// ── 状态栏刷新 ──
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

// ── 对话导出 ──
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

// ── 斜杠指令映射 ──
var DEX_SLASH_COMMANDS = {
  '/diag': '运行完整诊断，分析结果',
  '/fix': '自动修复所有发现的问题',
  '/health': '健康概览',
  '/self': '运行 DEX 自检，检查工具注册表、能力包、插件工具和最近错误',
  '/mcp': '检查 Claude Code MCP 集成状态',
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

// ── Token 计数器 ──
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

// ── 工具建议预览 ──
var DEX_TOOL_KEYWORDS = [
  { keywords: ['自检', 'self', 'dex状态', '工具注册表'], tools: 'dex_self_check, get_env_info' },
  { keywords: ['诊断', 'diag', '问题', '修复', 'fix', '错误', '失败', '异常'], tools: 'get_service_status, run_diagnostics, run_full_diagnostics' },
  { keywords: ['账号', '账户', 'account', '余额', '切换', '供应商', '导入'], tools: 'list_accounts, get_active_account, fetch_balance' },
  { keywords: ['配置', 'config', 'conf', '备份', 'backup', '恢复'], tools: 'get_config, save_config, config_backup' },
  { keywords: ['日志', 'log', '错误日志', '搜索日志'], tools: 'get_logs, search_logs' },
  { keywords: ['插件', 'plugin', '安装', '卸载', '扫码'], tools: 'list_plugins, query_plugin_status' },
  { keywords: ['线程', 'thread', '会话', 'session', '清理', '迁移'], tools: 'list_threads, get_threads_status, thread_cleanup' },
  { keywords: ['服务', 'service', '启动', '停止', '状态', '端口', '重启'], tools: 'get_service_status, start_service, stop_service' },
  { keywords: ['速度', 'speed', '延迟', 'latency', '测速', '连通'], tools: 'speed_test, test_upstream_connectivity' },
  { keywords: ['成本', 'cost', 'token', '花费', '消耗'], tools: 'token_cost, analyze_requests' },
  { keywords: ['安全', 'ssl', '证书', 'tls', '网络'], tools: 'ssl_check, network_topology' },
  { keywords: ['claude', 'mcp', '集成'], tools: 'claude_env_overview, claude_mcp_check' },
  { keywords: ['升级', 'upgrade', '更新', '版本'], tools: 'check_upgrade, run_upgrade' }
];

function dexPreviewTools(userMessage) {
  if (!userMessage) return null;
  var lower = userMessage.toLowerCase();
  var matched = {};
  for (var i = 0; i < DEX_TOOL_KEYWORDS.length; i++) {
    var entry = DEX_TOOL_KEYWORDS[i];
    for (var j = 0; j < entry.keywords.length; j++) {
      if (lower.indexOf(entry.keywords[j]) !== -1) {
        var tools = entry.tools.split(', ');
        for (var k = 0; k < tools.length; k++) matched[tools[k]] = true;
        break;
      }
    }
  }
  var names = Object.keys(matched);
  if (names.length === 0) return null;
  return names.slice(0, 5).join(', ');
}

// ── 搜索功能 ──
window._dexSearchIndex = -1;
window._dexSearchMatches = [];

function dexToggleSearch() {
  var bar = document.getElementById('dexSearchBar');
  if (!bar) return;
  if (bar.style.display === 'none') {
    bar.style.display = '';
    var input = document.getElementById('dexSearchInput');
    if (input) { input.value = ''; input.focus(); }
    var countEl = document.getElementById('dexSearchCount');
    if (countEl) countEl.textContent = '';
  } else {
    dexCloseSearch();
  }
}

function dexPerformSearch() {
  var input = document.getElementById('dexSearchInput');
  var countEl = document.getElementById('dexSearchCount');
  if (!input || !countEl) return;
  var query = input.value.trim().toLowerCase();
  dexClearHighlights();
  window._dexSearchMatches = [];
  window._dexSearchIndex = -1;
  if (!query) { countEl.textContent = ''; return; }
  var msgs = document.getElementById('dexMessages');
  if (!msgs) return;
  var bubbles = msgs.querySelectorAll('.dex-msg');
  for (var i = 0; i < bubbles.length; i++) {
    var msg = bubbles[i];
    var text = (msg.textContent || '').toLowerCase();
    if (text.indexOf(query) !== -1) {
      msg.classList.add('dex-msg-highlight');
      window._dexSearchMatches.push(msg);
    }
  }
  countEl.textContent = window._dexSearchMatches.length + ' 个匹配';
  if (window._dexSearchMatches.length > 0) dexNavigateSearch(1);
}

function dexNavigateSearch(direction) {
  if (window._dexSearchMatches.length === 0) return;
  for (var i = 0; i < window._dexSearchMatches.length; i++) {
    window._dexSearchMatches[i].classList.remove('dex-msg-search-current');
  }
  window._dexSearchIndex += direction;
  if (window._dexSearchIndex >= window._dexSearchMatches.length) window._dexSearchIndex = 0;
  if (window._dexSearchIndex < 0) window._dexSearchIndex = window._dexSearchMatches.length - 1;
  var current = window._dexSearchMatches[window._dexSearchIndex];
  if (current) {
    current.classList.add('dex-msg-search-current');
    current.scrollIntoView({ behavior: 'smooth', block: 'center' });
  }
  var countEl = document.getElementById('dexSearchCount');
  if (countEl) countEl.textContent = (window._dexSearchIndex + 1) + '/' + window._dexSearchMatches.length;
}

function dexCloseSearch() {
  dexClearHighlights();
  var bar = document.getElementById('dexSearchBar');
  if (bar) bar.style.display = 'none';
  window._dexSearchMatches = [];
  window._dexSearchIndex = -1;
}

function dexClearHighlights() {
  var highlighted = document.querySelectorAll('.dex-msg-highlight');
  for (var i = 0; i < highlighted.length; i++) {
    highlighted[i].classList.remove('dex-msg-highlight', 'dex-msg-search-current');
  }
}

// ── 快捷键绑定 ──
function dexBindShortcuts() {
  if (window._dexShortcutsBound) return;
  window._dexShortcutsBound = true;
  document.addEventListener('keydown', function (e) {
    // Ctrl+K / Cmd+K → 聚焦输入框
    if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
      var panel = document.getElementById('dexMessages');
      if (!panel || panel.offsetParent === null) return;
      e.preventDefault();
      var input = document.getElementById('dexInput');
      if (input) input.focus();
      return;
    }
    // Ctrl+L / Cmd+L → 清空对话
    if ((e.ctrlKey || e.metaKey) && e.key === 'l') {
      var panel2 = document.getElementById('dexMessages');
      if (!panel2 || panel2.offsetParent === null) return;
      e.preventDefault();
      dexClearChat();
      return;
    }
    // Escape → 停止 Agent / 关闭搜索
    if (e.key === 'Escape') {
      var searchBar = document.getElementById('dexSearchBar');
      if (searchBar && searchBar.style.display !== 'none') {
        dexCloseSearch();
        return;
      }
      if (window.dexAgent && window.dexAgent.isProcessing) {
        dexStopAgent();
        return;
      }
    }
  });
}

// ═══════════════════════════════════════════════════════════════
// 个人中心
// ═══════════════════════════════════════════════════════════════
function renderProfile() {
  return '<div class="page-header"><h2>个人中心</h2><p>账户信息与偏好设置</p></div>'
    + '<div class="profile-empty">'
    + '<div class="profile-empty-icon">◎</div>'
    + '<h3>个人中心正在准备中</h3>'
    + '<p>后续会集中展示账户身份、偏好设置与本机使用概览。当前版本先保留入口，避免空白页造成误判。</p>'
    + '</div>';
}


// ═══════════════════════════════════════════════════════════════
