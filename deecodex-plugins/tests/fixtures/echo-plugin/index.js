// Echo 测试插件：读取 stdin JSON-RPC 消息，响应初始化、llm.call 等请求
const readline = require('readline');

const rl = readline.createInterface({ input: process.stdin });
let config = {};

rl.on('line', (line) => {
  let msg;
  try {
    msg = JSON.parse(line.trim());
  } catch {
    return;
  }

  if (!msg.jsonrpc || msg.jsonrpc !== '2.0') return;

  // 处理请求
  if (msg.id !== undefined && msg.method) {
    handleRequest(msg);
  }
  // 处理通知
  else if (msg.method) {
    handleNotification(msg);
  }
});

function handleRequest(req) {
  const id = req.id;

  switch (req.method) {
    case 'initialize': {
      config = req.params?.config || {};
      sendJson({ jsonrpc: '2.0', id, result: {
        name: 'Echo Plugin',
        version: '1.0.0',
        capabilities: ['llm.echo'],
      }});
      break;
    }
    case 'llm.call': {
      const messages = req.params?.messages || [];
      const lastMsg = messages.length > 0 ? messages[messages.length - 1].content : '';
      const prefix = config.echo_prefix || 'ECHO:';

      // 先发 stream chunks
      const responseText = prefix + ' ' + lastMsg;
      for (let i = 0; i < responseText.length; i++) {
        sendJson({
          jsonrpc: '2.0',
          method: 'llm.stream_chunk',
          params: { id, index: i, delta: responseText[i] },
        });
      }

      // 返回最终结果
      sendJson({ jsonrpc: '2.0', id, result: {
        content: responseText,
        usage: { input_tokens: 10, output_tokens: responseText.length },
      }});
      break;
    }
    case 'config.update': {
      config = req.params?.config || {};
      sendJson({ jsonrpc: '2.0', id, result: { ok: true }});
      break;
    }
    default: {
      sendJson({ jsonrpc: '2.0', id, error: {
        code: -32601, message: 'Method not found: ' + req.method,
      }});
    }
  }
}

function handleNotification(notif) {
  switch (notif.method) {
    case 'shutdown': {
      // 优雅退出
      setTimeout(() => process.exit(0), 100);
      break;
    }
    case 'initialized': {
      // 握手完成，上报状态
      sendJson({ jsonrpc: '2.0', method: 'log', params: {
        level: 'info', message: 'Echo 插件已就绪',
      }});
      sendJson({ jsonrpc: '2.0', method: 'status', params: {
        account_id: 'echo-1', status: 'connected',
      }});
      break;
    }
  }
}

function sendJson(obj) {
  process.stdout.write(JSON.stringify(obj) + '\n');
}

// 进程退出时清理
process.on('SIGTERM', () => process.exit(0));
process.on('SIGINT', () => process.exit(0));
