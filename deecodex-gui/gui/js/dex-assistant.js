// DEX助手
// ═══════════════════════════════════════════════════════════════

// ── 工具定义由后端能力注册中心动态提供 ──
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

// ── System Prompt ──
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
  if (btn) btn.innerHTML = esc(label) + '<span class="dex-model-caret"></span>';
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
