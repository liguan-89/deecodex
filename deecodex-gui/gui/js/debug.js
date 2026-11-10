// DEXAI Debug —— 远程调试（插件详情页内嵌）
// 本机视图：开关 + 短码 + 状态摘要
// 支持端视图：连接目标 → 核心摘要 + 折叠详情 + 命令执行
var _debugTab = 'local';
var _debugLocalState = {
  enabled: false,
  tailnetAddresses: [],
  netdStatus: null,
  port: 17443,
  sessions: 0,
  shortCodeActive: false,
};
var _debugRemote = { host: '', port: 17443, code: '', token: '', busy: false, result: null };
var _debugTunnel = { localPort: null, busy: false };
var _debugTerm = { sessionId: null, timer: null, term: null };
var _debugLocalTimer = null;

function escDebug(v) {
  return String(v == null ? '' : v).replace(/[&<>"']/g, function (c) {
    return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c];
  });
}

function debugLocalCall(method, params) {
  return DeeCodexTauri.invoke('debug_local_call', { method: method, params: params || {} });
}

function debugRemoteCall(payload) {
  return DeeCodexTauri.invoke('debug_remote_call', payload);
}

function debugRemoteTunnel(payload) {
  return DeeCodexTauri.invoke('debug_remote_tunnel', payload);
}

function debugRemoteTunnelClose(localPort) {
  return DeeCodexTauri.invoke('debug_remote_tunnel_close', { localPort: localPort });
}

function debugTerminalOpen(payload) {
  return DeeCodexTauri.invoke('debug_terminal_open', payload);
}

function debugTerminalPoll(sessionId) {
  return DeeCodexTauri.invoke('debug_terminal_poll', { sessionId: sessionId });
}

function debugTerminalWrite(sessionId, data) {
  return DeeCodexTauri.invoke('debug_terminal_write', { sessionId: sessionId, data: data });
}

function debugTerminalClose(sessionId) {
  return DeeCodexTauri.invoke('debug_terminal_close', { sessionId: sessionId });
}

async function debugClipboardWrite(text) {
  try { await DeeCodexTauri.invoke('clipboard_write', { text: text }); return true; }
  catch (_) { return false; }
}

async function debugClipboardRead() {
  try { return await DeeCodexTauri.invoke('clipboard_read'); }
  catch (_) { return ''; }
}

function renderDebugPanel() {
  return `<div class="plugin-detail-section debug-section">
    <h3>远程调试</h3>
    <div class="debug-tabs">
      <button type="button" class="debug-tab ${_debugTab === 'local' ? 'active' : ''}" data-debug-tab="local" onclick="debugSwitchTab('local')">本机（被诊断端）</button>
      <button type="button" class="debug-tab ${_debugTab === 'remote' ? 'active' : ''}" data-debug-tab="remote" onclick="debugSwitchTab('remote')">支持端（连接对方）</button>
    </div>
    <div id="debugContent">${_debugTab === 'local' ? renderDebugLocal() : renderDebugRemote()}</div>
  </div>`;
}

function debugSwitchTab(tab) {
  _debugTab = tab === 'remote' ? 'remote' : 'local';
  document.querySelectorAll('.debug-tab').forEach(function (t) {
    t.classList.toggle('active', t.getAttribute('data-debug-tab') === _debugTab);
  });
  const c = document.getElementById('debugContent');
  if (c) c.innerHTML = _debugTab === 'local' ? renderDebugLocal() : renderDebugRemote();
  if (tab === 'local') refreshDebugLocal();
}

// ============ 本机（被诊断端） ============
async function refreshDebugLocal() {
  try {
    const st = (await debugLocalCall('debug.status', {})) || {};
    _debugLocalState = Object.assign({}, _debugLocalState, st);
  } catch (_) {}
  const el = document.getElementById('debugContent');
  if (el && _debugTab === 'local') el.innerHTML = renderDebugLocal();
}

function renderDebugLocal() {
  const s = _debugLocalState;
  const online = (s.tailnetAddresses || []).length > 0;
  return `<div class="debug-local">
    <div class="debug-toggle-row">
      ${s.enabled
        ? `<span class="debug-state-ok">✓ 诊断服务已启用${online ? ' · 在线' : ''}</span><button class="btn btn-ghost" onclick="debugToggleLocal(false)">停用</button>`
        : `<span class="debug-state-off">诊断服务已停用</span><button class="btn btn-primary" onclick="debugToggleLocal(true)">启用</button>`}
    </div>
    ${s.enabled ? `
      <div class="debug-shortcode-box">
        <div class="debug-shortcode-label">目标地址 + 一次性短码（5 分钟有效）</div>
        <div class="debug-shortcode-addr" id="debugShortCodeAddr">${escDebug(_debugLocalState._addr || '')}</div>
        <div class="debug-shortcode" id="debugShortCode">${s.shortCodeActive ? escDebug(_debugLocalState._code || '······') : '点击生成'}</div>
        <div class="debug-shortcode-actions">
          <button class="btn btn-primary btn-sm" onclick="debugGenShortCode()">生成短码</button>
          <button class="btn btn-ghost btn-sm" onclick="debugCopyShortCode()" ${_debugLocalState._code ? '' : 'disabled'}>复制短码</button>
        </div>
      </div>` : ''}
  </div>`;
}

async function debugToggleLocal(on) {
  try {
    if (on) {
      // 自动向 control-plane 领取 headscale auth key 并写入插件数据目录（netd 启动时读取注册）
      try {
        await DeeCodexTauri.invoke('debug_obtain_network_credentials');
      } catch (authErr) {
        showToast('领取网络凭证失败（需登录 DEX AI 主账号）: ' + escDebug(authErr), 'error');
      }
      await debugLocalCall('debug.enable', {});
      await refreshDebugLocal();
      showToast('诊断服务已启用', 'success');
    } else {
      await debugLocalCall('debug.disable', {});
      await refreshDebugLocal();
      showToast('诊断服务已停用', 'success');
    }
  } catch (e) {
    showToast('操作失败: ' + escDebug(e), 'error');
    await refreshDebugLocal();
  }
}

async function debugGenShortCode() {
  try {
    const r = await debugLocalCall('debug.shortcode', {});
    _debugLocalState._code = r.code;
    const addrs = _debugLocalState.tailnetAddresses || [];
    _debugLocalState._addr = addrs.find(function (a) { return a.indexOf('.') >= 0; }) || addrs[0] || '';
    const el = document.getElementById('debugShortCode');
    if (el) el.textContent = r.code;
    const addrEl = document.getElementById('debugShortCodeAddr');
    if (addrEl) addrEl.textContent = _debugLocalState._addr;
    const copyBtn = document.querySelector('.debug-shortcode-actions .btn-ghost');
    if (copyBtn) copyBtn.disabled = false;
    showToast('短码已生成' + (_debugLocalState._addr ? '（含目标地址）' : ''), 'success');
  } catch (e) {
    showToast('生成短码失败: ' + escDebug(e), 'error');
  }
}

async function debugCopyShortCode() {
  const code = _debugLocalState._code;
  if (!code) { showToast('请先生成短码', 'error'); return; }
  const addrs = _debugLocalState.tailnetAddresses || [];
  const addr = addrs.find(function (a) { return a.indexOf('.') >= 0; }) || addrs[0] || '';
  const text = (addr ? addr + ' ' : '') + code;
  if (await debugClipboardWrite(text)) {
    const read = await debugClipboardRead();
    showToast(read === text ? '已复制（含目标地址）' : '已复制，请确认', read === text ? 'success' : 'error');
  } else {
    showToast('复制失败，请重试', 'error');
  }
}

// ============ 支持端（连接对方） ============
function renderDebugRemote() {
  const connected = Boolean(_debugRemote.token);
  return `<div class="debug-remote">
    <div class="debug-card">
      <div class="debug-connect-line">
        <input id="debugRemoteHost" class="history-input" placeholder="目标地址" value="${escDebug(_debugRemote.host)}">
        <input id="debugRemoteCode" class="history-input" placeholder="验证码" value="${escDebug(_debugRemote.code)}">
        <button class="btn btn-ghost" onclick="debugPasteConnection()" title="从剪贴板粘贴 地址+验证码">粘贴</button>
        <button class="btn btn-primary" onclick="debugRemoteConnect()" ${_debugRemote.busy ? 'disabled' : ''}>${_debugRemote.busy ? '连接中…' : (connected ? '重新连接' : '连接')}</button>
        ${connected ? '<button class="btn btn-danger" onclick="debugRemoteReset()">断开</button>' : ''}
      </div>
    </div>
    <div id="debugRemoteResult">${_debugRemote.result ? renderDebugRemoteResult(_debugRemote.result) : ''}</div>
  </div>`;
}

async function debugPasteConnection() {
  try {
    let text = await debugClipboardRead();
    text = (text || '').trim();
    // 去掉可能的 HTML 源码片段（<span...> 等），只保留地址 + 短码
    text = text.replace(/<[^>]*>/g, ' ').replace(/\s+/g, ' ').trim();
    const parts = text.split(/\s+/);
    const host = parts[0] || '';
    const code = parts.slice(1).join('') || '';
    const h = document.getElementById('debugRemoteHost');
    const c = document.getElementById('debugRemoteCode');
    _debugRemote.host = host;
    _debugRemote.code = code;
    if (h) h.value = host;
    if (c) c.value = code;
    showToast(host && code ? '已粘贴' : '剪贴板内容无法解析（需要 地址+验证码）', host && code ? 'success' : 'error');
  } catch (e) {
    showToast('剪贴板读取被拒绝，请点击输入框后用系统粘贴（Cmd/Ctrl+V）', 'error');
    const h = document.getElementById('debugRemoteHost');
    if (h) h.focus();
  }
}

async function debugRemoteConnect() {
  const host = document.getElementById('debugRemoteHost').value.trim();
  const code = document.getElementById('debugRemoteCode').value.trim();
  if (!host || !code) { showToast('请输入目标地址和短码', 'error'); return; }
  _debugRemote.host = host; _debugRemote.code = code; _debugRemote.token = '';
  _debugRemote.busy = true;
  const box = document.getElementById('debugRemoteResult');
  if (box) box.innerHTML = '<div class="debug-empty">正在连接目标…</div>';
  try {
    const methods = ['status', 'system.info', 'service.status', 'network.check', 'config.peek', 'logs.tail'];
    const merged = {};
    for (const m of methods) {
      const r = await debugRemoteCall({
        host, port: 17443, code: _debugRemote.token ? null : code, token: _debugRemote.token || null,
        method: m, params: m === 'logs.tail' ? { lines: 40 } : {},
      });
      if (r.session_token) _debugRemote.token = r.session_token;
      merged[m] = r.result;
    }
    _debugRemote.result = merged;
    if (box) box.innerHTML = renderDebugRemoteResult(merged);
  } catch (e) {
    if (box) box.innerHTML = '<div class="debug-empty">连接失败：' + escDebug(e) + '</div>';
  } finally {
    _debugRemote.busy = false;
    const b = document.querySelector('.debug-connect-line .btn-primary');
    if (b) { b.disabled = false; b.textContent = '连接'; }
  }
}

// 断开连接：清理会话、隧道、终端，回到未连接状态。
function debugRemoteReset() {
  const tid = _debugTunnel.localPort;
  const sid = _debugTerm.sessionId;
  if (tid) {
    debugRemoteTunnelClose(tid).catch(function () {});
    _debugTunnel.localPort = null;
  }
  if (sid) {
    debugTerminalClose(sid).catch(function () {});
    _debugTerm.sessionId = null;
  }
  if (_debugTerm.timer) { clearInterval(_debugTerm.timer); _debugTerm.timer = null; }
  if (_debugTerm.term) { try { _debugTerm.term.dispose(); } catch (_) {} _debugTerm.term = null; }
  _debugRemote.token = '';
  _debugRemote.result = null;
  _debugRemote.host = '';
  _debugRemote.code = '';
  const c = document.getElementById('debugContent');
  if (c) c.innerHTML = renderDebugRemote();
  showToast('已断开连接', 'success');
}

function renderDebugRemoteResult(r) {
  const sys = r['system.info'] || {};
  const svc = r['service.status'] || {};
  const cfg = r['config.peek'] || {};
  const logs = r['logs.tail'] || {};
  const sum = svc.configSummary || {};
  return `<div class="debug-remote-result">
    <div class="debug-card">
      <div class="debug-card-head">
        <span class="debug-card-title">目标信息</span>
        <span class="debug-pill ok">已连接</span>
      </div>
      <div class="debug-summary">
        <div class="debug-summary-item"><span class="debug-s-label">主机</span><strong>${escDebug(sys.hostname || '-')}</strong></div>
        <div class="debug-summary-item"><span class="debug-s-label">平台</span><strong>${escDebug((sys.platform || '') + (sys.arch ? ' ' + sys.arch : '') || '-')}</strong></div>
        <div class="debug-summary-item"><span class="debug-s-label">网关端口</span><strong>${escDebug(sum.port != null ? sum.port : '-')}</strong></div>
      </div>
      <details class="debug-details"><summary>详细诊断（配置 / 日志）</summary>
        <pre class="debug-pre">${escDebug(JSON.stringify(cfg, null, 2))}</pre>
        <pre class="debug-pre">${escDebug(JSON.stringify(logs, null, 2))}</pre>
      </details>
    </div>
    <div class="debug-card">
      <div class="debug-card-head">
        <span class="debug-card-title">远程操作</span>
      </div>
      <div class="debug-actions">
        ${renderDebugTunnelBlock()}
        ${renderDebugTerminalBlock()}
        ${renderDebugExecBlock()}
      </div>
    </div>
  </div>`;
}

function renderDebugTerminalBlock() {
  const t = _debugTerm;
  if (t.sessionId) {
    return `<div class="debug-action on">
      <div class="debug-action-badge">term</div>
      <div class="debug-action-main">
        <div class="debug-action-title">内置终端 <span class="debug-pill ok">运行中</span></div>
        <div class="debug-action-control">
          <button class="btn btn-ghost btn-sm" onclick="debugTerminalOpenNow()">重新打开窗口</button>
          <button class="btn btn-danger btn-sm" onclick="debugTerminalCloseNow()">关闭终端</button>
        </div>
      </div>
    </div>`;
  }
  return `<div class="debug-action">
    <div class="debug-action-badge">term</div>
    <div class="debug-action-main">
      <div class="debug-action-title">内置终端</div>
      <div class="debug-action-control">
        <button class="btn btn-ghost btn-sm" onclick="debugTerminalOpenNow()">打开终端</button>
      </div>
    </div>
  </div>`;
}

async function debugTerminalOpenNow() {
  if (!_debugRemote.host) { showToast('请先连接目标获取地址', 'error'); return; }
  if (!_debugRemote.token) { showToast('请先连接目标（需要有效短码）', 'error'); return; }
  try {
    const r = await debugTerminalOpen({
      host: _debugRemote.host,
      token: _debugRemote.token,
    });
    _debugTerm.sessionId = r.session_id;
    openTerminalWindow();
    initDebugTerminal();
    refreshDebugRemoteOps();
    showToast('终端已打开', 'success');
  } catch (e) {
    showToast('打开终端失败: ' + escDebug(e), 'error');
  }
}

// 弹出终端窗口（大尺寸覆盖层，终端容器自适应窗口大小）
function openTerminalWindow() {
  closeTerminalWindow();
  const overlay = document.createElement('div');
  overlay.id = 'debugTermOverlay';
  overlay.className = 'debug-term-overlay';
  overlay.innerHTML = `<div class="debug-term-window">
    <div class="debug-term-window-head">
      <span>内置终端 · ${escDebug(_debugRemote.host || '')}</span>
      <button class="btn btn-danger btn-sm" onclick="debugTerminalCloseNow()">关闭终端</button>
    </div>
    <div id="debugTermEl" class="debug-terminal-el"></div>
  </div>`;
  document.body.appendChild(overlay);
}

function closeTerminalWindow() {
  const el = document.getElementById('debugTermOverlay');
  if (el && el.parentNode) el.parentNode.removeChild(el);
}

function refreshDebugRemoteOps() {
  const box = document.getElementById('debugRemoteResult');
  if (box && _debugRemote.result) box.innerHTML = renderDebugRemoteResult(_debugRemote.result);
}

function initDebugTerminal() {
  const el = document.getElementById('debugTermEl');
  if (!el || !_debugTerm.sessionId) return;
  if (typeof Terminal === 'undefined') {
    el.textContent = '终端组件未加载（xterm.js 缺失）';
    return;
  }
  const term = new Terminal({
    cursorBlink: true,
    fontSize: 13,
    fontFamily: 'IBM Plex Mono, JetBrains Mono, Fira Code, "SF Mono", Menlo, Consolas, monospace',
    convertEol: true,
    theme: { background: '#10131a', foreground: '#d6dde6' },
  });
  let fitAddon = null;
  if (typeof FitAddon !== 'undefined' && FitAddon.FitAddon) {
    fitAddon = new FitAddon.FitAddon();
    term.loadAddon(fitAddon);
  }
  term.open(el);
  if (fitAddon) { try { fitAddon.fit(); } catch (_) {} }
  const doFit = function () { if (fitAddon) { try { fitAddon.fit(); } catch (_) {} } };
  window.addEventListener('resize', doFit);
  term.focus();
  _debugTerm.term = term;
  _debugTerm._fit = doFit;
  // 输入 → 远端
  term.onData(function (d) {
    const sid = _debugTerm.sessionId;
    if (sid) debugTerminalWrite(sid, d);
  });
  // 轮询远端输出 → 终端
  _debugTerm.timer = setInterval(async function () {
    const sid = _debugTerm.sessionId;
    if (!sid) return;
    try {
      const r = await debugTerminalPoll(sid);
      if (r.data && _debugTerm.term) {
        // base64 -> Uint8Array：保留原始字节，避免多字节字符在轮询分块处乱码
        const bin = atob(r.data);
        const u8 = new Uint8Array(bin.length);
        for (let i = 0; i < bin.length; i++) u8[i] = bin.charCodeAt(i);
        _debugTerm.term.write(u8);
      }
    } catch (_) {}
  }, 200);
}

async function debugTerminalCloseNow() {
  const sid = _debugTerm.sessionId;
  if (!sid) return;
  try {
    await debugTerminalClose(sid);
  } catch (e) {
    showToast('关闭终端失败: ' + escDebug(e), 'error');
  }
  if (_debugTerm._fit && window.removeEventListener) {
    try { window.removeEventListener('resize', _debugTerm._fit); } catch (_) {}
    _debugTerm._fit = null;
  }
  if (_debugTerm.timer) { clearInterval(_debugTerm.timer); _debugTerm.timer = null; }
  if (_debugTerm.term) { try { _debugTerm.term.dispose(); } catch (_) {} _debugTerm.term = null; }
  _debugTerm.sessionId = null;
  closeTerminalWindow();
  refreshDebugRemoteOps();
  showToast('终端已关闭', 'success');
}

function renderDebugTunnelBlock() {
  const t = _debugTunnel;
  if (t.localPort) {
    return `<div class="debug-action on">
      <div class="debug-action-badge">ssh</div>
      <div class="debug-action-main">
        <div class="debug-action-title">SSH 隧道 <span class="debug-pill ok">已建立</span></div>
        <div class="debug-action-result">
          <code>ssh -p ${escDebug(t.localPort)} user@127.0.0.1</code>
        </div>
        <div class="debug-action-control">
          <button class="btn btn-danger btn-sm" onclick="debugRemoteTunnelCloseNow()">关闭隧道</button>
        </div>
      </div>
    </div>`;
  }
  return `<div class="debug-action">
    <div class="debug-action-badge">ssh</div>
    <div class="debug-action-main">
      <div class="debug-action-title">SSH 隧道</div>
      <div class="debug-action-control">
        <input id="debugTunnelPort" class="history-input" placeholder="端口" value="22">
        <input id="debugTunnelLocal" class="history-input" placeholder="本地端口">
        <button class="btn btn-primary btn-sm" onclick="debugRemoteTunnelStart()" ${t.busy ? 'disabled' : ''}>${t.busy ? '建立中…' : '建立隧道'}</button>
      </div>
      <div id="debugTunnelOut" class="debug-action-hint"></div>
    </div>
  </div>`;
}

function renderDebugExecBlock() {
  return `<div class="debug-action">
    <div class="debug-action-badge">exec</div>
    <div class="debug-action-main">
      <div class="debug-action-title">执行命令</div>
      <div class="debug-action-control">
        <input id="debugRemoteCmd" class="history-input" placeholder="如 uname -a / sc query sshd">
        <button class="btn btn-primary btn-sm" onclick="debugRemoteExec()">执行</button>
      </div>
      <div id="debugRemoteExecOut"></div>
    </div>
  </div>`;
}

async function debugRemoteTunnelStart() {
  const targetPort = (document.getElementById('debugTunnelPort').value || '22').trim();
  const localPortInput = (document.getElementById('debugTunnelLocal').value || '').trim();
  const out = document.getElementById('debugTunnelOut');
  if (!_debugRemote.host) { if (out) out.textContent = '请先连接目标获取地址'; return; }
  if (!_debugRemote.token) { if (out) out.textContent = '请先连接目标（需要有效短码）'; return; }
  _debugTunnel.busy = true;
  if (out) out.textContent = '正在建立隧道…';
  try {
    const r = await debugRemoteTunnel({
      host: _debugRemote.host,
      port: Number(targetPort) || 22,
      token: _debugRemote.token,
      localPort: localPortInput ? Number(localPortInput) : 0,
    });
    _debugTunnel.localPort = r.local_port;
    if (out) out.textContent = '隧道已建立：' + r.command;
    const box = document.getElementById('debugRemoteResult');
    if (box) box.innerHTML = renderDebugRemoteResult(_debugRemote.result);
    showToast('SSH 隧道已建立，本地端口 ' + r.local_port, 'success');
  } catch (e) {
    if (out) out.textContent = '隧道建立失败：' + escDebug(e);
  } finally {
    _debugTunnel.busy = false;
  }
}

async function debugRemoteTunnelCloseNow() {
  const lp = _debugTunnel.localPort;
  if (!lp) return;
  try {
    await debugRemoteTunnelClose(lp);
  } catch (e) {
    showToast('关闭隧道失败: ' + escDebug(e), 'error');
  }
  _debugTunnel.localPort = null;
  const box = document.getElementById('debugRemoteResult');
  if (box) box.innerHTML = renderDebugRemoteResult(_debugRemote.result);
  showToast('SSH 隧道已关闭', 'success');
}

async function debugRemoteExec() {
  const cmd = document.getElementById('debugRemoteCmd').value.trim();
  if (!cmd) return;
  const out = document.getElementById('debugRemoteExecOut');
  if (out) out.innerHTML = '<div class="debug-action-hint">执行中…</div>';
  try {
    const r = await debugRemoteCall({ host: _debugRemote.host, port: 17443, token: _debugRemote.token, method: 'exec', params: { command: cmd } });
    const res = r.result || {};
    if (out) out.innerHTML = `<pre class="debug-pre">$ ${escDebug(cmd)}\nexit=${escDebug(res.code)}\n${escDebug(res.stdout || '')}${res.stderr ? '\n[stderr] ' + escDebug(res.stderr) : ''}</pre>`;
  } catch (e) {
    if (out) out.innerHTML = '<div class="debug-action-hint">执行失败：' + escDebug(e) + '</div>';
  }
}

function debugPanelEnter() {
  refreshDebugLocal();
  startDebugLocalPolling();
}

// 自动轮询本机状态：netd 注册/地址上线可能需要数秒，避免"刷新不出目标地址"
function startDebugLocalPolling() {
  stopDebugLocalPolling();
  _debugLocalTimer = setInterval(function () {
    if (_debugTab === 'local') refreshDebugLocal();
  }, 4000);
}

function stopDebugLocalPolling() {
  if (_debugLocalTimer) {
    clearInterval(_debugLocalTimer);
    _debugLocalTimer = null;
  }
}
