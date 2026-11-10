#!/usr/bin/env node
// dexai-diagnose CLI —— DEX AI DEXAI Debug 支持端入口
// 用法：
//   node cli.js <host> --code <短码> [子命令] [--output file]
//   node cli.js <host> --token <会话token> [子命令]
// 子命令：status | system | service | config | logs | network | exec "<cmd>" | audit | sessions
//   ssh [--local <本地端口>] [--target-port <目标端口>]  建立 SSH/TCP 隧道（默认目标 22）
//   shell                                       进入内置交互式终端（不依赖目标 sshd）
// 默认（无子命令）：拉取完整诊断包（system/service/config/logs/network）输出 JSON
const net = require('net');
const fs = require('fs');
const path = require('path');

const DEFAULT_PORT = 17443;
// 本机 dexai-debug netd 的 tailnet 出站代理（与 index.js 保持一致）
const OUTBOUND_PROXY = '127.0.0.1:17445';

function isTailnetAddress(host) {
  const ip = host.replace(/^\[|\]$/g, '');
  const v4 = ip.split('.').map(Number);
  if (v4.length === 4 && v4.every((n) => Number.isInteger(n) && n >= 0 && n <= 255)) {
    return v4[0] === 100 && v4[1] === 64;
  }
  return /^fd7a:115c:a1e0:/i.test(ip);
}

function parseArgs(argv) {
  const out = { host: null, code: null, token: null, command: null, output: null, port: DEFAULT_PORT, verbose: false, localPort: 0, targetPort: 22 };
  const positional = [];
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--code') out.code = argv[++i];
    else if (a === '--token') out.token = argv[++i];
    else if (a === '--output') out.output = argv[++i];
    else if (a === '--port') out.port = Number(argv[++i]) || DEFAULT_PORT;
    else if (a === '--local') out.localPort = Number(argv[++i]) || 0;
    else if (a === '--target-port') out.targetPort = Number(argv[++i]) || 22;
    else if (a === '--verbose' || a === '-v') out.verbose = true;
    else if (a === 'exec') { out.command = argv[i + 1] || ''; i++; out.action = 'exec'; }
    else if (a === 'ssh') { out.action = 'ssh'; }
    else if (a === 'shell') { out.action = 'shell'; }
    else if (a === '--help' || a === '-h') out.help = true;
    else if (a.startsWith('-')) { /* ignore unknown */ }
    else positional.push(a);
  }
  out.host = positional[0] || null;
  out.action = out.action || positional[1] || null;
  return out;
}

function rpc(host, port, reqs, { verbose }) {
  return new Promise((resolve, reject) => {
    // tailnet 目标经本机 netd 出站代理 CONNECT 转发（本机无 tailnet 路由时也能连通）
    const viaProxy = isTailnetAddress(host);
    const [phost, pport] = OUTBOUND_PROXY.split(':');
    const connectHost = viaProxy ? phost : host;
    const connectPort = viaProxy ? Number(pport) : port;
    const socket = net.connect(connectPort, connectHost, () => {
      if (verbose) console.error('[dexai-diagnose] connected', viaProxy ? 'outbound-proxy' : host + ':' + port);
      if (!viaProxy) { sendNext(); return; }
      const target = host.includes(':') && !host.startsWith('[')
        ? '[' + host + ']:' + port
        : host + ':' + port;
      socket.write('CONNECT ' + target + '\n', () => {
        if (verbose) console.error('[dexai-diagnose] CONNECT', target);
        sendNext();
      });
    });
    let buffer = '';
    let idx = 0;
    const results = [];
    let pending = null;
    let timer = null;
    const fail = (err) => { if (timer) clearTimeout(timer); socket.destroy(); reject(err); };
    function sendNext() {
      if (idx >= reqs.length) return;
      const req = reqs[idx];
      pending = req;
      timer = setTimeout(() => fail(new Error('RPC timeout: ' + req.method)), 20000);
      socket.write(JSON.stringify(req) + '\n');
    }
    socket.on('data', (chunk) => {
      buffer += chunk.toString('utf8');
      let nl;
      while ((nl = buffer.indexOf('\n')) >= 0) {
        const line = buffer.slice(0, nl).trim();
        buffer = buffer.slice(nl + 1);
        if (!line) continue;
        let msg;
        try { msg = JSON.parse(line); } catch (_) { continue; }
        if (pending && msg.id === pending.id) {
          if (timer) clearTimeout(timer);
          results.push(msg);
          idx++;
          pending = null;
          if (msg.error) {
            socket.end();
            resolve({ success: false, error: msg.error, results });
            return;
          }
          if (idx >= reqs.length) {
            socket.end();
            resolve({ success: true, results });
          } else {
            sendNext();
          }
        }
      }
    });
    socket.on('error', (e) => fail(e));
  });
}

function makeRequest(method, params, id) {
  return { jsonrpc: '2.0', id, method, params };
}

// 建立到目标本地端口的透传隧道：session.open（如有需要）→ tunnel.open → 原始字节透传。
// 解析用：host 为 tailnet 地址，port 固定走诊断通道 17443。
function openTunnel(host, port, { code, token, targetPort, verbose }) {
  return new Promise((resolve, reject) => {
    const viaProxy = isTailnetAddress(host);
    const [phost, pport] = OUTBOUND_PROXY.split(':');
    const socket = net.connect(viaProxy ? Number(pport) : port, viaProxy ? phost : host, () => {
      if (verbose) console.error('[dexai-diagnose] connected', viaProxy ? 'outbound-proxy' : host + ':' + port);
      if (viaProxy) {
        const target = host.includes(':') && !host.startsWith('[')
          ? '[' + host + ']:' + port
          : host + ':' + port;
        socket.write('CONNECT ' + target + '\n');
      }
      if (token) {
        socket.write(JSON.stringify(makeRequest('tunnel.open', { token, port: targetPort }, 99)) + '\n');
        return;
      }
      socket.write(JSON.stringify(makeRequest('session.open', { code, source: 'cli-ssh' }, 1)) + '\n');
    });
    let buf = '';
    let stage = token ? 'tunnel' : 'session';
    let settled = false;
    socket.on('data', (d) => {
      buf += d.toString('utf8');
      let i;
      while ((i = buf.indexOf('\n')) >= 0) {
        const line = buf.slice(0, i).trim(); buf = buf.slice(i + 1);
        if (!line) continue;
        let m; try { m = JSON.parse(line); } catch (_) { continue; }
        if (stage === 'session' && m.id === 1) {
          if (m.error) { if (!settled) { settled = true; reject(new Error('session.open: ' + JSON.stringify(m.error))); } return; }
          stage = 'tunnel';
          socket.write(JSON.stringify(makeRequest('tunnel.open', { token: m.result.token, port: targetPort }, 99)) + '\n');
        } else if (stage === 'tunnel' && m.id === 99) {
          if (m.error) { if (!settled) { settled = true; reject(new Error('tunnel.open: ' + JSON.stringify(m.error))); } return; }
          if (!settled) { settled = true; resolve(socket); }
        }
      }
    });
    socket.on('error', (e) => { if (!settled) { settled = true; reject(e); } });
  });
}

// 建立内置交互式终端连接：session.open（如有需要）→ shell.open → 原始字节透传。
function openShell(host, port, { code, token, verbose }) {
  return new Promise((resolve, reject) => {
    const viaProxy = isTailnetAddress(host);
    const [phost, pport] = OUTBOUND_PROXY.split(':');
    const socket = net.connect(viaProxy ? Number(pport) : port, viaProxy ? phost : host, () => {
      if (viaProxy) {
        const target = host.includes(':') && !host.startsWith('[')
          ? '[' + host + ']:' + port
          : host + ':' + port;
        socket.write('CONNECT ' + target + '\n');
      }
      if (token) {
        socket.write(JSON.stringify(makeRequest('shell.open', { token }, 99)) + '\n');
        return;
      }
      socket.write(JSON.stringify(makeRequest('session.open', { code, source: 'cli-shell' }, 1)) + '\n');
    });
    let buf = '';
    let stage = token ? 'shell' : 'session';
    let settled = false;
    socket.on('data', (d) => {
      buf += d.toString('utf8');
      let i;
      while ((i = buf.indexOf('\n')) >= 0) {
        const line = buf.slice(0, i).trim(); buf = buf.slice(i + 1);
        if (!line) continue;
        let m; try { m = JSON.parse(line); } catch (_) { continue; }
        if (stage === 'session' && m.id === 1) {
          if (m.error) { if (!settled) { settled = true; reject(new Error('session.open: ' + JSON.stringify(m.error))); } return; }
          stage = 'shell';
          socket.write(JSON.stringify(makeRequest('shell.open', { token: m.result.token }, 99)) + '\n');
        } else if (stage === 'shell' && m.id === 99) {
          if (m.error) { if (!settled) { settled = true; reject(new Error('shell.open: ' + JSON.stringify(m.error))); } return; }
          if (!settled) { settled = true; resolve(socket); }
        }
      }
    });
    socket.on('error', (e) => { if (!settled) { settled = true; reject(e); } });
  });
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help || !args.host) {
    console.log(`用法:
  node cli.js <host> --code <短码> [子命令] [--output file]
  node cli.js <host> --token <会话token> [子命令]

子命令:
  status        查看诊断服务状态（默认）
  system        系统/环境信息
  service       网关/服务状态
  config        配置（脱敏）
  logs          最近日志
  network       网络连通性
  exec "<cmd>"  作用域命令执行（需授权会话）
  audit         审计记录
  sessions      活跃会话
  ssh           建立 SSH 隧道（--local 本地端口，--target-port 目标端口，默认 22）
  shell         进入内置交互式终端（不依赖目标 sshd）
  无子命令       拉取完整诊断包`);
    process.exit(args.help ? 0 : 1);
  }

  const host = args.host;
  const reqs = [];
  let token = args.token;
  if (!token) {
    if (!args.code) {
      console.error('需要 --code <短码> 或 --token <会话token>');
      process.exit(1);
    }
    reqs.push(makeRequest('session.open', { code: args.code, source: 'cli' }, 1));
  }

  // SSH/TCP 隧道模式：本地监听端口，每连接经 tailnet 17443 通道透传到目标本地端口。
  if (args.action === 'ssh') {
    const targetPort = args.targetPort || 22;
    const server = net.createServer((client) => {
      openTunnel(host, args.port, { code: args.code, token: args.token, targetPort, verbose: args.verbose })
        .then((up) => {
          client.pipe(up);
          up.pipe(client);
          const closeBoth = () => { try { client.destroy(); } catch (_) {} try { up.destroy(); } catch (_) {} };
          client.on('close', closeBoth);
          up.on('close', closeBoth);
          client.on('error', closeBoth);
          up.on('error', closeBoth);
        })
        .catch((e) => {
          console.error('[dexai-diagnose] 隧道连接失败:', e.message);
          try { client.destroy(); } catch (_) {}
        });
    });
    server.listen(args.localPort || 0, '127.0.0.1', () => {
      const port = server.address().port;
      console.error('[dexai-diagnose] SSH 隧道已就绪（目标 ' + host + ':' + targetPort + '），本地端口 ' + port);
      console.log('  ssh -p ' + port + ' <user>@127.0.0.1');
      console.log('  scp -P ' + port + ' <文件> <user>@127.0.0.1:<路径>');
      console.error('按 Ctrl+C 关闭隧道');
    });
    return;
  }

  // 内置交互式终端：把本地终端 stdin/stdout 直接透传到远端 PTY shell。
  if (args.action === 'shell') {
    const up = await openShell(host, args.port, { code: args.code, token: args.token, verbose: args.verbose });
    console.error('[dexai-diagnose] 已进入远程终端（目标 ' + host + '），输入 exit 退出');
    if (process.stdin.isTTY && process.stdin.setRawMode) {
      process.stdin.setRawMode(true);
    }
    process.stdin.pipe(up);
    up.pipe(process.stdout);
    const exit = () => {
      try { if (process.stdin.isTTY && process.stdin.setRawMode) process.stdin.setRawMode(false); } catch (_) {}
      process.exit(0);
    };
    up.on('close', exit);
    up.on('error', exit);
    process.stdin.on('error', () => {});
    return;
  }

  let action = args.action || 'default';
  const alias = {
    status: 'status',
    system: 'system.info',
    service: 'service.status',
    config: 'config.peek',
    logs: 'logs.tail',
    network: 'network.check',
    audit: 'audit',
    sessions: 'sessions',
    exec: 'exec',
  };
  const methodList = action === 'default'
    ? ['status', 'system.info', 'service.status', 'config.peek', 'network.check', 'logs.tail']
    : [alias[action] || action];

  let openResult = null;
  if (!token) {
    const openResp = await rpc(host, args.port, [makeRequest('session.open', { code: args.code, source: 'cli' }, 1)], args);
    if (!openResp.success) {
      console.error('会话打开失败:', JSON.stringify(openResp.error));
      process.exit(1);
    }
    token = openResp.results[0].result.token;
    openResult = { sessionId: openResp.results[0].result.sessionId, token };
  }

  const targetReqs = methodList.map((m, i) => {
    const params = { token };
    if (m === 'exec') params.command = args.command;
    if (m === 'logs.tail') params.lines = 100;
    return makeRequest(m, params, i + 2);
  });
  const resp = await rpc(host, args.port, targetReqs, args);

  if (!resp.success) {
    console.error('调用失败:', JSON.stringify(resp.error));
    process.exit(1);
  }

  const merged = {};
  if (openResult) merged.session = openResult;
  for (const r of resp.results) {
    if (r.result && r.id !== undefined) {
      const name = methodList[r.id - 2] || ('call-' + r.id);
      merged[name] = r.result;
    }
  }
  const payload = Object.keys(merged).length ? merged : (resp.results[resp.results.length - 1] || {}).result;
  const text = JSON.stringify(payload, null, 2);
  if (args.output) {
    fs.writeFileSync(args.output, text);
    console.error('已写入', args.output);
  } else {
    process.stdout.write(text + '\n');
  }
  if (args.verbose && token) console.error('[session token]', token);
}

main().catch((e) => {
  console.error('dexai-diagnose 错误:', e.message);
  process.exit(1);
});
