// Node 级前端逻辑 E2E：用真实 dex-assistant-opencode.js + 桩 Tauri/DOM，
// 走通「引擎启动 → 发消息 → 文本/工具事件 → L3 权限回环 → idle」全链路，
// 断言每次 IPC 调用与 UI 回调，验证 webview 实际运行的代码路径。
const assert = require('assert');
const path = require('path');

const calls = [];
let eventListener = null;
let ui = { append: [], update: [], lastAssistant: [], confirm: 0, stop: 0, hideStop: 0, hideThink: 0 };

global.window = {
  deeStorage: { getItem: () => null },
  dexAgent: { isProcessing: false },
  DEX_TOOLS: [
    { name: 'get_config', level: 0 },
    { name: 'stop_service', level: 3 }
  ]
};
global.DeeCodexTauri = {
  invoke: async function (cmd, args) {
    calls.push({ cmd: cmd, args: args || {} });
    switch (cmd) {
      case 'dex_opencode_status': return calls.filter(c => c.cmd === 'dex_opencode_start').length ? { running: true } : { running: false, available: true };
      case 'dex_opencode_start': return { ok: true };
      case 'dex_opencode_events_subscribe': return true;
      case 'dex_opencode_session_create': return { id: 'ses_test' };
      case 'dex_opencode_prompt': return { ok: true };
      case 'dex_opencode_abort': return { ok: true };
      case 'dex_opencode_permission_reply': return true;
      default: return {};
    }
  },
  listen: async function (name, handler) { eventListener = handler; return { unlisten: true }; }
};
global.dexShowStopButton = function () { ui.stop++; };
global.dexHideStopButton = function () { ui.hideStop++; };
global.dexHideThinking = function () { ui.hideThink++; };
global.dexAppendMessage = function (type, content, meta) { ui.append.push({ type, content, meta }); return { className: 'dex-msg-tool-start' }; };
global.dexUpdateMessage = function (el, type, content, meta) { ui.update.push({ type, content, meta }); };
global.dexUpdateLastAssistant = function (text, reasoning) { ui.lastAssistant.push({ text, reasoning }); };
global.dexMaskApiKey = function (v) { return v; };
global.dexShowInlineConfirm = function (toolName, message, callId, args) { ui.confirm++; return Promise.resolve(true); };
global.document = { querySelectorAll: function () { return [{ className: 'dex-msg-tool-start' }]; } };

require('./dex-assistant-opencode');
const DexOpencode = global.window.DexOpencode;
assert(DexOpencode, 'DexOpencode 应挂到 window');

function tick(ms) { return new Promise(r => setTimeout(r, ms || 5)); }
function fire(raw) { eventListener({ payload: { event: raw } }); }

(async () => {
  // 1) 启动引擎：先 status(未运行) -> start -> status(运行) -> subscribe
  const started = await DexOpencode.start();
  assert.strictEqual(started, true, 'start 应返回 true');
  const startCmds = calls.filter(c => c.cmd.startsWith('dex_opencode_')).map(c => c.cmd);
  assert(startCmds.includes('dex_opencode_start'), '应调用 dex_opencode_start');
  assert(startCmds.includes('dex_opencode_events_subscribe'), '应订阅事件');
  assert(eventListener, '应注册 dex-opencode-event 监听');

  // 2) 发消息：prompt_async + 等待 idle
  let sent = DexOpencode.send('请检查服务状态');
  await tick();
  assert(calls.some(c => c.cmd === 'dex_opencode_session_create'), '应先建会话');
  assert(calls.some(c => c.cmd === 'dex_opencode_prompt' && c.args.sessionId === 'ses_test'), '应发 prompt');
  // 文本事件（含 think 剥离）
  fire({ type: 'message.part.updated', properties: { part: { type: 'text', text: '<think>推理</think>\n\n服务运行中' } } });
  assert.strictEqual(ui.lastAssistant.length, 1, '文本应渲染一次');
  assert(!ui.lastAssistant[0].text.includes('<think>'), 'think 应从正文剥离');
  assert(ui.lastAssistant[0].reasoning.includes('推理'), 'think 应进思考区');
  // 工具事件
  fire({ type: 'message.part.updated', properties: { part: { type: 'tool', tool: 'dex_ops_get_config', state: { status: 'running', input: {} } } } });
  assert(ui.append.some(a => a.type === 'tool-start' && a.content === 'get_config'), '工具应渲染 tool-start');
  fire({ type: 'message.part.updated', properties: { part: { type: 'tool', tool: 'dex_ops_get_config', state: { status: 'completed', output: '{"host":"127.0.0.1"}' } } } });
  assert(ui.update.some(u => u.type === 'tool-result' && u.content === 'get_config'), '工具应渲染 tool-result');
  // L3 权限回环
  fire({ type: 'permission.asked', properties: { id: 'per_1', sessionID: 'ses_test', permission: 'dex_ops_stop_service', tool: { callID: 'call_9' } } });
  await tick();
  assert.strictEqual(ui.confirm, 1, 'L3 应弹内联确认');
  assert(calls.some(c => c.cmd === 'dex_opencode_permission_reply' && c.args.permissionId === 'per_1' && c.args.response === 'once'), '确认后应回 once');
  // idle 结束处理中
  fire({ type: 'session.idle', properties: {} });
  await sent;
  assert.strictEqual(global.window.dexAgent.isProcessing, false, 'idle 后应结束处理中');
  assert(ui.hideStop > 0, '应隐藏停止按钮');

  // 3) abort
  await DexOpencode.abort();
  assert(calls.some(c => c.cmd === 'dex_opencode_abort' && c.args.sessionId === 'ses_test'), '应调用 abort');

  // 4) 账号切换 refresh -> stop 引擎
  await DexOpencode.refresh();
  assert(calls.some(c => c.cmd === 'dex_opencode_stop'), 'refresh 应停引擎');

  console.log('dex-assistant-opencode harness E2E ok');
  process.exit(0);
})().catch(e => { console.error('HARNESS FAIL', e); process.exit(1); });
