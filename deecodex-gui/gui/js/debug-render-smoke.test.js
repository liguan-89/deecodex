const assert = require('assert');
const fs = require('fs');
const path = require('path');
const vm = require('vm');

const html = fs.readFileSync(path.join(__dirname, '..', 'index.html'), 'utf8');

// index.html 必须加载 xterm（内置终端）与 debug.js
assert.ok(html.includes('vendor/xterm/xterm.min.js'), 'index.html 应加载 xterm');
assert.ok(/<script src="js\/debug\.js\?/.test(html), 'index.html 应加载 debug.js');

const elements = new Map();
const context = {
  console,
  setInterval: () => 1,
  clearInterval() {},
  setTimeout,
  clearTimeout,
  window: {},
  DeeCodexTauri: {
    invoke: async () => ({}),
  },
  Terminal: function Terminal() {},
  document: {
    createElement() {
      return { className: '', id: '', innerHTML: '', addEventListener() {}, remove() {} };
    },
    getElementById(id) {
      if (!elements.has(id)) {
        elements.set(id, {
          id,
          value: '',
          innerHTML: '',
          style: {},
          classList: { toggle() {}, add() {}, remove() {} },
          setAttribute() {},
          focus() {},
          remove() {},
          textContent: '',
          disabled: false,
        });
      }
      return elements.get(id);
    },
    querySelector() { return null; },
    querySelectorAll() { return []; },
    addEventListener() {},
    removeEventListener() {},
  },
  showToast() {},
};
context.window = context;

vm.createContext(context);
const source = fs.readFileSync(path.join(__dirname, 'debug.js'), 'utf8');
vm.runInContext(source, context, { filename: 'debug.js' });

// 1) 关键函数必须存在（含之前缺失的 debugRemoteReset）
for (const fn of [
  'renderDebugPanel',
  'renderDebugLocal',
  'renderDebugRemote',
  'renderDebugRemoteResult',
  'renderDebugTunnelBlock',
  'renderDebugTerminalBlock',
  'renderDebugExecBlock',
  'debugRemoteConnect',
  'debugRemoteReset',
  'debugRemoteTunnelStart',
  'debugRemoteTunnelCloseNow',
  'debugTerminalOpenNow',
  'debugTerminalCloseNow',
  'debugRemoteExec',
  'debugPasteConnection',
]) {
  assert.strictEqual(typeof context[fn], 'function', `缺少函数 ${fn}`);
}

// 2) 未连接视图：连接目标卡片 + 空态
const unconnected = context.renderDebugRemote();
assert.ok(unconnected.includes('debugRemoteHost'), '未连接应包含目标地址输入框');
assert.ok(unconnected.includes('debugRemoteCode'), '未连接应包含验证码输入框');
assert.ok(unconnected.includes('粘贴'), '未连接应有粘贴按钮');
assert.ok(unconnected.includes('连接'), '未连接应有连接按钮');
assert.ok(unconnected.includes('debug-card'), '未连接应使用 debug-card 卡片');
assert.ok(!unconnected.includes('短码'), '「一次性短码」文案应统一为验证码');

// 3) 已连接视图：目标信息 + 远程操作（隧道/终端/命令）
context._debugRemote.token = 'tok-smoke';
const result = context.renderDebugRemoteResult({
  'system.info': { hostname: 'smoke-host', platform: 'darwin', arch: 'arm64' },
  'service.status': { configSummary: { port: 8899, upstream: 'https://u' } },
  'network.check': { tailnet: { backendState: 'Running' } },
  'config.peek': { '.deecodex/config.json': '{}' },
  'logs.tail': { '.deecodex/deecodex.log': [] },
});
assert.ok(result.includes('目标信息'), '应显示「目标信息」');
assert.ok(result.includes('远程操作'), '应显示「远程操作」');
assert.ok(result.includes('smoke-host'), '应显示主机名');
assert.ok(result.includes('网关端口'), '目标信息应显示网关端口');
assert.ok(!result.includes('Tailnet'), '目标信息不再显示 Tailnet');
assert.ok(!result.includes('上游'), '目标信息不再显示上游');
assert.ok(result.includes('SSH 隧道'), '应有 SSH 隧道操作项');
assert.ok(result.includes('内置终端'), '应有内置终端操作项');
assert.ok(result.includes('执行命令'), '应有命令执行操作项');
assert.ok(result.includes('debug-action-badge'), '操作项应使用统一 badge');

// 4) 隧道/终端已建立态
context._debugTunnel.localPort = 2222;
const tunnelOn = context.renderDebugTunnelBlock();
assert.ok(tunnelOn.includes('ssh -p 2222'), '隧道已建立应显示 ssh 命令');
context._debugTerm.sessionId = 1;
const termOn = context.renderDebugTerminalBlock();
assert.ok(termOn.includes('重新打开窗口'), '终端已打开应提供重新打开按钮');
assert.ok(termOn.includes('关闭终端'), '终端已打开应提供关闭按钮');

// 4b) 所有支持端输入框保留占位文案（位置缩进由 CSS 承担）
const allHtml = unconnected + result + tunnelOn + termOn;
assert.ok(allHtml.includes('placeholder="目标地址"'), '目标地址输入框应保留占位文案');
assert.ok(allHtml.includes('placeholder="验证码"'), '验证码输入框应保留占位文案');
assert.ok(allHtml.includes('placeholder="端口"'), '端口输入框应保留占位文案');
assert.ok(allHtml.includes('placeholder="本地端口"'), '本地端口输入框应保留占位文案');
assert.ok(allHtml.includes('placeholder="如 uname -a / sc query sshd"'), '命令输入框应保留占位文案');

// 5) debugRemoteReset 清理状态
await_debugRemoteReset(context);
assert.strictEqual(context._debugRemote.token, '');
assert.strictEqual(context._debugRemote.result, null);
assert.strictEqual(context._debugTunnel.localPort, null);
assert.strictEqual(context._debugTerm.sessionId, null);

function await_debugRemoteReset(ctx) {
  return Promise.resolve(ctx.debugRemoteReset());
}

console.log('debug-render-smoke: all assertions passed');
