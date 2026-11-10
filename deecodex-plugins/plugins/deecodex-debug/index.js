// DEX AI DEXAI Debug 插件（第一方插件）
// 基于 DEX AI tailnet（headscale.dex.liguan.me）提供远程环境诊断：
// - 独立 netd 实例（与 mobile-codex 的 tailnet 节点隔离）
// - 短码授权（一次性、5 分钟过期）
// - 诊断命令集（系统/服务/配置/日志/网络/作用域命令执行）
// - 全程审计 + 敏感脱敏
// 支持端入口：CLI（dexai-diagnose）与 GUI（DEX AI 面板）双通道
const { createPlugin } = require('./deecodex-plugin');
const { spawn, execFile, execFileSync } = require('child_process');
const net = require('net');
const crypto = require('crypto');
const os = require('os');
const fs = require('fs');
const path = require('path');

// tailnet 对外端口 = netd --grpc-target 端口（入站转发到 localhost 对应端口）
const DIAGNOSE_PORT = 17443;
const SHORTCODE_TTL_MS = 5 * 60 * 1000;
const SESSION_TTL_MS = 30 * 60 * 1000;
// netd 二进制按平台选择：macOS(Apple Silicon) 用 deecodex-netd，Windows 用 .exe
const NETD_BIN = path.join(
  __dirname,
  process.platform === 'win32' ? 'deecodex-netd.exe' : 'deecodex-netd'
);
const HEADSCALE_URL =
  process.env.DEXAI_DIAGNOSE_HEADSCALE || 'https://headscale.dex.liguan.me';
const AUTH_SECRET_RE =
  /(password|token|secret|authorization|api[_-]?key|private[_-]?key|invitation[_-]?code|credential|auth[_-]?key)/i;

const state = {
  enabled: false,
  shortCode: null,
  shortCodeExpiry: 0,
  sessions: new Map(), // token -> { createdAt, expiresAt, source }
  netd: null,
  netdStatus: null,
  server: null,
  assetsRoot: null,
  config: {},
};

// ============ 工具 ============
function maskSecrets(value, key, seen) {
  if (key && AUTH_SECRET_RE.test(String(key))) return '[REDACTED]';
  if (typeof value === 'string') {
    const trimmed = value.trim();
    if (
      (trimmed[0] === '{' && trimmed[trimmed.length - 1] === '}') ||
      (trimmed[0] === '[' && trimmed[trimmed.length - 1] === ']')
    ) {
      try {
        return JSON.stringify(maskSecrets(JSON.parse(value), '', seen));
      } catch (_) {}
    }
    return value;
  }
  if (value === null || value === undefined || typeof value !== 'object') return value;
  if (seen.has(value)) return '[CIRCULAR]';
  seen.add(value);
  if (Array.isArray(value)) return value.map((item) => maskSecrets(item, '', seen));
  const out = {};
  Object.keys(value).forEach((k) => {
    out[k] = maskSecrets(value[k], k, seen);
  });
  return out;
}

function nowIso() {
  return new Date().toISOString();
}

function assetsDir() {
  if (state.assetsRoot) return state.assetsRoot;
  // 宿主注入的资产目录（host.assets 根）；退化到插件目录相邻 .data
  return state.assetsRoot || path.join(__dirname, '.diagnose-data');
}

function audit(action, source, detail) {
  const entry = { ts: nowIso(), action, source, detail };
  try {
    const dir = assetsDir();
    fs.mkdirSync(path.join(dir, 'audit'), { recursive: true });
    const file = path.join(dir, 'audit', 'audit.jsonl');
    fs.appendFileSync(file, JSON.stringify(maskSecrets(entry, '', new Set())) + '\n');
    // 轮转：超过 1MB 时保留最近 1000 条，防止审计日志无限增长
    try {
      const stat = fs.statSync(file);
      if (stat.size > 1024 * 1024) {
        const lines = fs.readFileSync(file, 'utf8').split('\n').filter(Boolean);
        if (lines.length > 1000) {
          fs.writeFileSync(file, lines.slice(-1000).join('\n') + '\n');
        }
      }
    } catch (_) {}
  } catch (e) {
    console.error('[dexai-diagnose] audit write failed:', e.message);
  }
  return entry;
}

function tailFile(file, lines) {
  try {
    if (!fs.existsSync(file)) return [];
    const raw = fs.readFileSync(file, 'utf8');
    // 兼容 CRLF 行尾（Windows 编辑器可能把审计文件转成 \r\n）：
    // 逐行剥掉行尾 \r，避免 JSON.parse 前残留回车。
    const arr = raw.split('\n').map((l) => l.replace(/\r$/, '')).filter(Boolean);
    return arr.slice(-lines);
  } catch (_) {
    return [];
  }
}

/// 解码子进程输出：优先 UTF-8（fatal 校验），失败时在 Windows 上回退 GBK/ANSI
/// 代码页（中文 Windows 的 netstat/sc 等输出是 GBK，直接 toString 会乱码）。
function decodeOutput(buf) {
  if (typeof buf === 'string') return buf;
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(buf);
  } catch (_) {
    if (process.platform === 'win32') {
      try {
        return new TextDecoder('gbk').decode(buf);
      } catch (_2) {}
    }
    return buf.toString('utf8');
  }
}

function runCmd(cmd, args, timeoutMs) {
  return new Promise((resolve) => {
    execFile(cmd, args, { timeout: timeoutMs || 8000, maxBuffer: 4 * 1024 * 1024 }, (err, stdout, stderr) => {
      resolve({
        ok: !err,
        code: err ? err.code || -1 : 0,
        stdout: decodeOutput(stdout || ''),
        stderr: decodeOutput(stderr || ''),
        error: err ? err.message : '',
      });
    });
  });
}

// ============ 独立 netd（tailnet 客户端） ============
function netdStateDir() {
  return path.join(assetsDir(), 'netd-state');
}

function netdStatusFile() {
  return path.join(assetsDir(), 'netd-status.json');
}

/// 读取 headscale auth key：优先环境变量，其次数据目录 authkey 文件
function readAuthKey() {
  const fromEnv =
    process.env.DEXAI_DIAGNOSE_AUTHKEY || process.env.TS_AUTHKEY || '';
  if (fromEnv) return fromEnv;
  try {
    const f = path.join(assetsDir(), 'authkey');
    if (fs.existsSync(f)) return fs.readFileSync(f, 'utf8').trim();
  } catch (_) {}
  return '';
}

/// 读取系统 HTTP 代理（macOS scutil / Windows 环境变量），
/// netd（Go/Tailscale）默认不读 macOS 系统代理；DNS 被代理 fake-ip 劫持时
/// 直连会失败，显式注入 HTTPS_PROXY 让控制连接走代理到公网 headscale。
function systemProxy() {
  if (process.platform === 'darwin') {
    try {
      const out = execFileSync('scutil', ['--proxy'], { encoding: 'utf8', timeout: 3000 });
      const get = (key) => {
        const re = new RegExp('\\s*' + key + '\\s*:\\s*(\\S+)');
        const m = out.match(re);
        return m ? m[1] : '';
      };
      if (get('HTTPEnable') === '1' && get('HTTPProxy') && get('HTTPPort')) {
        return 'http://' + get('HTTPProxy') + ':' + get('HTTPPort');
      }
    } catch (_) {}
  }
  if (process.env.HTTPS_PROXY || process.env.HTTP_PROXY) {
    return process.env.HTTPS_PROXY || process.env.HTTP_PROXY;
  }
  // clash 等 TUN 模式可能未设系统代理，但 HTTP 代理端口在监听（mixed-port）。
  // 探测常见本地端口，能连通 headscale 即采用，让 netd 控制连接走代理出网。
  const candidates = [7890, 7897, 1080, 8888, 10809, 7891, 10808];
  for (const port of candidates) {
    try {
      const code = execFileSync(
        'curl',
        ['-x', 'http://127.0.0.1:' + port, '-sS', '-o', '/dev/null', '-w', '%{http_code}', '--max-time', '4', HEADSCALE_URL],
        { encoding: 'utf8', timeout: 6000 }
      );
      if (code.trim() === '200') return 'http://127.0.0.1:' + port;
    } catch (_) {}
  }
  return '';
}

function startNetd(forceAuth, opts) {
  if (state.netd && !state.netd.killed) return;
  if (!fs.existsSync(NETD_BIN)) {
    console.error('[dexai-diagnose] deecodex-netd missing:', NETD_BIN);
    return;
  }
  // 直连模式：控制面/DERP 走 NO_PROXY 直连（0.1.4 起默认，绕过 clash 对 DERP 的破坏）。
  // 代理模式：目标机器 clash fake-ip-filter 未放行 headscale 域名时，直连会被 fake-ip
  // 劫持导致 tsnet.Up 超时，此时回退让控制面走 HTTP 代理出网（fallback）。
  const useProxy = Boolean(opts && opts.useProxy);
  const hostname = 'dexai-diagnose-' + crypto.randomBytes(5).toString('hex').slice(0, 10);
  const grpc = 17443;
  const browser = 17444;
  const outbound = 17445;
  // 独立实例登录：已注册节点（state-dir 有持久化身份）直接用已有身份恢复，
  // 不再带一次性 auth key（一次性 key 用后即失效，复用会报 authkey expired）。
  // 未注册节点用 auth key 完成首次注册。
  // forceAuth：持久化身份失效时强制用 auth key 重新注册（生成新 tailnet 节点）
  const enrolled = !forceAuth && fs.existsSync(path.join(netdStateDir(), '.mobile-codex-enrolled'));
  const authKey = enrolled ? '' : readAuthKey();
  const proxy = systemProxy();
  // tailnet CGNAT 网段与本地地址不走代理，避免内部流量被转发
  // headscale 控制面与 DERP 必须直连：走系统代理（clash）时 DERP WebSocket
  // 会被破坏导致 tailnet 节点间数据路径建立不了（节点 online 但互连超时）。
  // 本机可直连 headscale 公网域名，因此显式加入 NO_PROXY。
  const noProxy = useProxy
    ? '100.64.0.0/10,100.100.100.100,localhost,127.0.0.1'
    : '100.64.0.0/10,100.100.100.100,localhost,127.0.0.1,headscale.dex.liguan.me';
  const netd = spawn(NETD_BIN, [
    '--state-dir', netdStateDir(),
    '--status-file', netdStatusFile(),
    '--hostname', hostname,
    '--login-server', HEADSCALE_URL,
    '--grpc-target', '127.0.0.1:' + grpc,
    '--browser-target', '127.0.0.1:' + browser,
    '--outbound-proxy', '127.0.0.1:' + outbound,
    '--tailnet-grpc-port', String(grpc),
    '--tailnet-browser-port', String(browser),
    // 出站代理允许端口：诊断目标 17443 + 兼容 companion 7443
    '--outbound-allow-ports', '17443,7443',
    '--parent-pipe',
  ], {
    stdio: ['pipe', 'ignore', 'pipe'],
    env: {
      ...process.env,
      ...(authKey ? { TS_AUTHKEY: authKey } : {}),
      ...(proxy ? { HTTPS_PROXY: proxy, HTTP_PROXY: proxy, NO_PROXY: noProxy } : {}),
    },
  });
  netd.on('error', (e) => console.error('[dexai-diagnose] netd error:', e.message));
  netd.stderr.on('data', (d) => {
    const s = String(d).trim();
    if (s) console.error('[dexai-diagnose][netd]', s.slice(0, 300));
  });
  netd.on('exit', (code, signal) => {
    console.error('[dexai-diagnose] netd exited code=' + code + ' signal=' + signal);
    if (state.netd === netd) state.netd = null;
  });
  state.netd = netd;
  console.error('[dexai-diagnose] netd started hostname=' + hostname + (useProxy ? ' (proxy fallback)' : ' (direct)') + (authKey ? ' (authkey)' : ' (persisted identity)') + (proxy ? ' (proxy)' : ''));
  watchNetdReady(useProxy);
}

function stopNetd() {
  if (state.netd) {
    try { state.netd.stdin.end(); } catch (_) {}
    try { state.netd.kill(); } catch (_) {}
    state.netd = null;
  }
}

// netd 注册成功后清理一次性 auth key（已消费，避免下次复用过期 key）。
// 直连模式超过 90 秒仍未就绪时，自动回退到代理模式重启一次（应对目标机器 clash
// fake-ip-filter 未放行 headscale 域名导致直连被劫持的情况）。
function watchNetdReady(useProxy) {
  let tries = 0;
  let retriedProxy = false;
  const timer = setInterval(() => {
    const st = readNetdStatus();
    if (st && st.backendState === 'Running' && Array.isArray(st.tailnetAddresses) && st.tailnetAddresses.length > 0) {
      clearInterval(timer);
      try {
        const f = path.join(assetsDir(), 'authkey');
        if (fs.existsSync(f)) {
          fs.unlinkSync(f);
          console.error('[dexai-diagnose] authkey consumed, removed');
        }
      } catch (_) {}
      return;
    }
    if (++tries > 45) {
      clearInterval(timer); // 最多等 90 秒
      if (!useProxy && !retriedProxy) {
        retriedProxy = true;
        console.error('[dexai-diagnose] 直连 headscale 超时（可能被 clash fake-ip 劫持），回退走代理重试');
        stopNetd();
        setTimeout(() => startNetd(false, { useProxy: true }), 500);
      }
    }
  }, 2000);
}

function readNetdStatus() {
  try {
    const f = netdStatusFile();
    if (fs.existsSync(f)) {
      state.netdStatus = JSON.parse(fs.readFileSync(f, 'utf8'));
    }
  } catch (_) {}
  return state.netdStatus || null;
}

function tailnetAddresses() {
  const st = readNetdStatus();
  return (st && st.tailnetAddresses) || [];
}

// ============ 短码授权 ============
function generateShortCode() {
  const alphabet = 'ABCDEFGHJKLMNPQRSTUVWXYZ23456789';
  let code = '';
  for (let i = 0; i < 10; i++) code += alphabet[crypto.randomInt(alphabet.length)];
  state.shortCode = code;
  state.shortCodeExpiry = Date.now() + SHORTCODE_TTL_MS;
  return { code, expiresIn: Math.floor(SHORTCODE_TTL_MS / 1000) };
}

function verifyShortCode(code) {
  if (!code || !state.shortCode) return false;
  if (Date.now() > state.shortCodeExpiry) return false;
  return crypto.timingSafeEqual(
    Buffer.from(String(code).toUpperCase()),
    Buffer.from(state.shortCode)
  );
}

function issueSession(source) {
  const token = crypto.randomBytes(24).toString('hex');
  state.sessions.set(token, {
    createdAt: Date.now(),
    expiresAt: Date.now() + SESSION_TTL_MS,
    source,
  });
  audit('session.open', source, { sessionId: token.slice(0, 8) });
  return token;
}

function verifySession(token) {
  const s = state.sessions.get(token);
  if (!s) return null;
  if (Date.now() > s.expiresAt) {
    state.sessions.delete(token);
    return null;
  }
  return s;
}

function sweepSessions() {
  for (const [token, s] of state.sessions.entries()) {
    if (Date.now() > s.expiresAt) state.sessions.delete(token);
  }
}

async function getStatus() {
  return {
    ok: true,
    enabled: state.enabled,
    tailnetAddresses: tailnetAddresses(),
    netdStatus: readNetdStatus(),
    port: DIAGNOSE_PORT,
    sessions: state.sessions.size,
    shortCodeActive: Boolean(state.shortCode) && Date.now() < state.shortCodeExpiry,
  };
}

// ============ 诊断命令集 ============
async function systemInfo() {
  return {
    hostname: os.hostname(),
    platform: os.platform(),
    arch: os.arch(),
    release: os.release(),
    cpus: os.cpus().length,
    totalMem: os.totalmem(),
    freeMem: os.freemem(),
    loadavg: os.loadavg ? os.loadavg() : [],
    uptime: os.uptime(),
    node: process.version,
  };
}

async function serviceStatus() {
  const home = os.homedir();
  const results = {};
  const checks = [];
  // 网关端口 8899 / 常用端口监听
  const isWin = os.platform() === 'win32';
  const netstat = await runCmd(
    isWin ? 'netstat' : 'sh',
    isWin ? ['-ano'] : ['-c', 'lsof -nP -iTCP:8899 -sTCP:LISTEN || true'],
    6000
  );
  checks.push({ name: 'gateway-8899', raw: netstat.stdout.slice(0, 1200) });
  const cfgPath = path.join(home, '.deecodex', 'config.json');
  if (fs.existsSync(cfgPath)) {
    try {
      const cfg = maskSecrets(JSON.parse(fs.readFileSync(cfgPath, 'utf8')), '', new Set());
      results.configSummary = {
        port: cfg.port,
        port_auto: cfg.port_auto,
        host: cfg.host,
        upstream: cfg.upstream,
        codex_router_mode: cfg.codex_router_mode,
      };
    } catch (_) {}
  }
  results.checks = checks;
  return results;
}

async function configPeek() {
  const home = os.homedir();
  const out = {};
  const files = ['.deecodex/config.json', '.codex/config.toml', '.claude/settings.json'];
  for (const rel of files) {
    const p = path.join(home, rel);
    if (!fs.existsSync(p)) continue;
    try {
      const text = fs.readFileSync(p, 'utf8');
      out[rel] = maskSecrets(text, '', new Set());
    } catch (_) {}
  }
  return out;
}

async function logsTail(lines) {
  const home = os.homedir();
  const n = Math.min(Math.max(Number(lines) || 100, 10), 500);
  const files = ['.deecodex/deecodex.log'];
  const out = {};
  for (const rel of files) {
    const p = path.join(home, rel);
    out[rel] = tailFile(p, n);
  }
  return out;
}

async function networkCheck() {
  const targets = [HEADSCALE_URL];
  const out = {};
  for (const t of targets) {
    const r = await runCmd('curl', ['-sS', '-o', '/dev/null', '-w', '%{http_code} %{time_total}', '--max-time', '8', t], 10000);
    out[t] = { ok: r.ok, code: r.code, body: r.stdout.trim() };
  }
  return { tailnet: readNetdStatus(), targets: out };
}

async function execCommand(command, source) {
  const trimmed = String(command || '').trim();
  if (!trimmed) return { ok: false, error: 'empty command' };
  if (trimmed.length > 4000) return { ok: false, error: 'command too long' };
  const entry = audit('exec', source, { command: maskSecrets(trimmed, 'command', new Set()) });
  const isWin = os.platform() === 'win32';
  const r = await new Promise((resolve) => {
    const cp = spawn(isWin ? 'cmd.exe' : 'sh', isWin ? ['/c', trimmed] : ['-c', trimmed], {
      timeout: 15000,
      maxBuffer: 4 * 1024 * 1024,
    });
    let stdout = '';
    let stderr = '';
    cp.stdout.on('data', (d) => (stdout += d));
    cp.stderr.on('data', (d) => (stderr += d));
    cp.on('error', (e) => resolve({ ok: false, code: -1, stdout, stderr, error: e.message }));
    cp.on('close', (code) => resolve({ ok: code === 0, code, stdout, stderr }));
  });
  return {
    ok: r.ok,
    code: r.code,
    stdout: maskSecrets(r.stdout, '', new Set()).slice(0, 60000),
    stderr: maskSecrets(r.stderr, '', new Set()).slice(0, 20000),
    auditId: entry.ts,
  };
}

// ============ TCP 远程通道（支持端连接） ============
function startRemoteServer() {
  if (state.server) return;
  let attempts = 0;
  let canceled = false;
  const createServer = () => net.createServer((socket) => {
    let buffer = '';
    let sessionSource = '';
    let tunnelMode = false;
    const send = (obj) => socket.write(JSON.stringify(obj) + '\n');
    socket.on('data', async (chunk) => {
      // 隧道透传模式：TCP 字节流直接转发到目标本地端口，不再解析 JSON。
      // 数据由 tunnel.pipe 处理，这里只负责把 JSON 缓冲区的残余清掉。
      if (tunnelMode) return;
      buffer += chunk.toString('utf8');
      let idx;
      while ((idx = buffer.indexOf('\n')) >= 0) {
        const line = buffer.slice(0, idx).trim();
        buffer = buffer.slice(idx + 1);
        if (!line) continue;
        let req;
        try {
          req = JSON.parse(line);
        } catch (_) {
          send({ jsonrpc: '2.0', id: null, error: { code: -32700, message: 'parse error' } });
          continue;
        }
        const { id, method, params } = req;
        if (method === 'session.open') {
          const { code } = params || {};
          if (!verifyShortCode(code)) {
            send({ jsonrpc: '2.0', id, error: { code: -32001, message: 'invalid short code' } });
            continue;
          }
          sessionSource = String(params.source || socket.remoteAddress || 'remote');
          const token = issueSession(sessionSource);
          send({ jsonrpc: '2.0', id, result: { token, sessionId: token.slice(0, 8), ttl: SESSION_TTL_MS / 1000 } });
          continue;
        }
        // 其余方法需要会话
        const session = verifySession((params || {}).token);
        if (!session) {
          send({ jsonrpc: '2.0', id, error: { code: -32002, message: 'no valid session' } });
          continue;
        }
        sweepSessions();
        let result;
        try {
          switch (method) {
            case 'status': result = await getStatus(); break;
            case 'system.info': result = await systemInfo(); break;
            case 'service.status': result = await serviceStatus(); break;
            case 'config.peek': result = await configPeek(); break;
            case 'logs.tail': result = await logsTail((params || {}).lines); break;
            case 'network.check': result = await networkCheck(); break;
            case 'exec': result = await execCommand((params || {}).command, sessionSource); break;
            // 应用层端口转发隧道：SSH/任意 TCP 走 tailnet 通道透传到目标本地端口。
            // 支持端先发 tunnel.open，插件连上目标本地端口后回一行确认，
            // 之后该 TCP 连接的原始字节双向透传（不再走 JSON-RPC）。
            case 'tunnel.open': {
              const port = Number((params || {}).port);
              if (!port || port <= 0 || port > 65535) {
                send({ jsonrpc: '2.0', id, error: { code: -32000, message: 'invalid port' } });
                continue;
              }
              const target = net.connect(port, '127.0.0.1', () => {
                tunnelMode = true;
                audit('tunnel.open', sessionSource, { port });
                // 通知支持端切换到透传，随后双向 pipe 原始字节。
                socket.write(JSON.stringify({ jsonrpc: '2.0', id, result: { ok: true, port } }) + '\n');
                target.pipe(socket);
                socket.pipe(target);
              });
              target.on('error', (e) => {
                if (!tunnelMode) {
                  send({ jsonrpc: '2.0', id, error: { code: -32000, message: 'connect 127.0.0.1:' + port + ' failed: ' + e.message } });
                }
              });
              target.on('close', () => {
                try { socket.end(); } catch (_) {}
              });
              socket.on('close', () => {
                try { target.destroy(); } catch (_) {}
              });
              continue; // 响应由 target 连接回调发出，不走统一响应
            }
            // 内置交互式终端（不依赖目标机 sshd）：spawn 一个 PTY shell，
            // 经 tailnet 通道把原始字节双向透传。macOS/Linux 用 script 分配 pty
            // 获得终端语义（Ctrl-C、彩色输出）；Windows 用交互 cmd（管道）。
            case 'shell.open': {
              const isWin = os.platform() === 'win32';
              const shell = isWin ? 'cmd.exe' : process.env.SHELL || 'sh';
              let shellProc;
              if (isWin) {
                shellProc = spawn('cmd.exe', [], { stdio: ['pipe', 'pipe', 'pipe'] });
              } else {
                // macOS/Linux：pty-shell.py 用 pty.fork 创建 PTY 并把 erase 设为 DEL，
                // 与 xterm Backspace（\x7f）匹配，保证删除键正常（pty.spawn 的
                // 子进程 erase 默认可能是 BS，导致删除键光标乱走）。
                shellProc = spawn(
                  'python3',
                  [path.join(__dirname, 'pty-shell.py'), shell],
                  { stdio: ['pipe', 'pipe', 'pipe'] }
                );
              }
              shellProc.on('error', (e) => {
                if (!tunnelMode) {
                  send({ jsonrpc: '2.0', id, error: { code: -32000, message: '启动 shell 失败: ' + e.message } });
                }
              });
              shellProc.on('close', () => { try { socket.end(); } catch (_) {} });
              socket.on('close', () => {
                try { shellProc.stdin.end(); } catch (_) {}
                try { shellProc.kill(); } catch (_) {}
              });
              tunnelMode = true;
              audit('shell.open', sessionSource, { shell, platform: os.platform() });
              socket.write(JSON.stringify({ jsonrpc: '2.0', id, result: { ok: true, platform: os.platform(), shell } }) + '\n');
              shellProc.stdout.pipe(socket);
              shellProc.stderr.pipe(socket);
              if (isWin) {
                // cmd.exe 管道模式只在收到 \n 时才处理一行；xterm.js 回车只发 \r（0x0D），
                // 不补 \n 会导致回车无效（命令不执行）。把未被 \n 跟随的 \r 规范化为 \r\n。
                socket.on('data', (chunk) => {
                  try {
                    shellProc.stdin.write(chunk.toString('utf8').replace(/\r(?!\n)/g, '\r\n'));
                  } catch (_) {}
                });
              } else {
                socket.pipe(shellProc.stdin);
              }
              continue; // 响应已发出，后续为透传
            }
            case 'session.close': {
              state.sessions.delete((params || {}).token);
              audit('session.close', sessionSource, {});
              result = { ok: true };
              break;
            }
            default:
              send({ jsonrpc: '2.0', id, error: { code: -32601, message: 'method not found: ' + method } });
              continue;
          }
          audit('call', sessionSource, { method });
          send({ jsonrpc: '2.0', id, result });
        } catch (e) {
          send({ jsonrpc: '2.0', id, error: { code: -32000, message: String(e.message || e) } });
        }
      }
    });
    socket.on('error', () => {});
    socket.on('close', () => {});
  });
  // 绑定 127.0.0.1：tailnet 地址是 tsnet（userspace 网络）内部地址，系统接口
  // 上不存在，直接绑定会 EADDRNOTAVAIL；tsnet 会把 tailnet 入站连接转发到
  // 本机 127.0.0.1:<port>，因此监听 localhost 即可被 tailnet 内节点访问。
  const tryBind = () => {
    if (canceled || state.server) return;
    const host = '127.0.0.1';
    const server = createServer();
    server.on('error', (e) => {
      console.error('[dexai-diagnose] remote server error:', e.message);
      try { server.close(); } catch (_) {}
      if (attempts < 30) {
        attempts++;
        setTimeout(tryBind, 3000);
      }
    });
    server.listen(DIAGNOSE_PORT, host, () => {
      state.server = server;
      console.error('[dexai-diagnose] remote server listening on ' + host + ':' + DIAGNOSE_PORT);
    });
  };
  tryBind();
  state._remoteCancel = () => { canceled = true; };
}

function stopRemoteServer() {
  if (state._remoteCancel) {
    try { state._remoteCancel(); } catch (_) {}
    state._remoteCancel = null;
  }
  if (state.server) {
    try { state.server.close(); } catch (_) {}
    state.server = null;
  }
}

// ============ 宿主插件生命周期 ============
createPlugin({
  initialize(params) {
    state.config = params.config || {};
    state.assetsRoot = (params.assets && params.assets.root) || state.assetsRoot;
    if (state.enabled === false && state.config.enabled) {
      state.enabled = true;
      startNetd();
      startRemoteServer();
    }
    return { ok: true, name: 'DEXAI Debug' };
  },
  notifications: {
    'config.update': (params) => {
      state.config = params.config || {};
      const wasEnabled = state.enabled;
      state.enabled = Boolean(state.config.enabled);
      if (state.enabled && !wasEnabled) {
        startNetd();
        startRemoteServer();
      } else if (!state.enabled && wasEnabled) {
        stopRemoteServer();
        stopNetd();
        state.sessions.clear();
      }
    },
  },
  methods: {
    async 'debug.status'(_params, host) { return await getStatus(); },
    async 'debug.enable'(params, host) {
      state.enabled = true;
      startNetd();
      startRemoteServer();
      audit('enable', 'local', {});
      return await getStatus();
    },
    async 'debug.disable'(_params, host) {
      state.enabled = false;
      stopRemoteServer();
      stopNetd();
      state.sessions.clear();
      audit('disable', 'local', {});
      return { ok: true, enabled: false };
    },
    async 'debug.shortcode'(_params, host) {
      if (!state.enabled) return { ok: false, error: '诊断服务未开启' };
      const c = generateShortCode();
      audit('shortcode', 'local', { expiresIn: c.expiresIn });
      return { ok: true, ...c };
    },
    // 支持端连接前确保 netd 已启动（仅提供 tailnet 出站代理，不开启被诊断服务）
    async 'debug.ensure_netd'(_params, host) {
      const force = Boolean(_params && _params.force);
      startNetd(force);
      // 等待 netd 注册完成（tailnet 地址出现），最多 15 秒
      const netdReady = () => {
        const st = readNetdStatus();
        return st && st.backendState === 'Running' && Array.isArray(st.tailnetAddresses) && st.tailnetAddresses.length > 0;
      };
      const waitReady = async (ms) => {
        const deadline = Date.now() + ms;
        while (Date.now() < deadline) {
          if (netdReady()) return true;
          await new Promise((r) => setTimeout(r, 1000));
        }
        return netdReady();
      };
      let ready = await waitReady(15000);
      if (!ready) {
        // 直连未就绪（目标机器 clash fake-ip 可能劫持了 headscale 域名）：回退代理模式重启。
        console.error('[dexai-diagnose] ensure_netd 直连 15s 未就绪，回退走代理重试');
        stopNetd();
        startNetd(force, { useProxy: true });
        ready = await waitReady(15000);
      }
      return {
        ok: true,
        netdRunning: Boolean(state.netd && !state.netd.killed),
        netdStatus: readNetdStatus(),
      };
    },
    async 'debug.sessions'(_params, host) {
      sweepSessions();
      return { ok: true, sessions: Array.from(state.sessions.entries()).map(([t, s]) => ({
        sessionId: t.slice(0, 8),
        source: s.source,
        expiresAt: new Date(s.expiresAt).toISOString(),
      })) };
    },
    async 'debug.audit'(_params, host) {
      const f = path.join(assetsDir(), 'audit', 'audit.jsonl');
      return { ok: true, entries: tailFile(f, 200).map((l) => {
        try { return JSON.parse(l); } catch (_) { return { raw: l }; }
      }) };
    },
    async 'debug.system.info'(_params, host) {
      return { ok: true, data: await systemInfo() };
    },
    async 'debug.service.status'(_params, host) {
      return { ok: true, data: await serviceStatus() };
    },
    async 'debug.config.peek'(_params, host) {
      return { ok: true, data: await configPeek() };
    },
    async 'debug.logs.tail'(_params, host) {
      return { ok: true, data: await logsTail(100) };
    },
    async 'debug.network.check'(_params, host) {
      return { ok: true, data: await networkCheck() };
    },
    async 'debug.exec'(_params, host) {
      // 作用域命令执行：本机联调用途（GUI 面板确认后调用），同样走审计
      return { ok: false, error: 'exec 仅允许通过已授权远程会话调用' };
    },
  },
}).start();
