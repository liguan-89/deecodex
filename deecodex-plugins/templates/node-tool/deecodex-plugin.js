'use strict';

const readline = require('readline');

function createPlugin(definition) {
  const methods = definition.methods || {};
  const notifications = definition.notifications || {};
  let config = {};
  let initializeParams = {};
  let nextHostRequestId = 10000;
  const pendingHostRequests = new Map();

  const host = {
    get config() { return config; },
    get initializeParams() { return initializeParams; },
    request(method, params, options) {
      const timeoutMs = Number(options && options.timeoutMs) || 30000;
      const id = nextHostRequestId++;
      write({ jsonrpc: '2.0', id, method, params });
      return new Promise((resolve, reject) => {
        pendingHostRequests.set(id, { resolve, reject });
        setTimeout(() => {
          if (!pendingHostRequests.has(id)) return;
          pendingHostRequests.delete(id);
          reject(new Error('Host request timeout: ' + method));
        }, timeoutMs);
      });
    },
    notify(method, params) {
      write({ jsonrpc: '2.0', method, params });
    },
    log(level, message) {
      host.notify('log', { level, message });
    },
    llm: {
      call(params) { return host.request('llm.call', params); },
    },
    assets: {
      list(path = '') { return host.request('assets.list', { path }); },
      read(path) { return host.request('assets.read', { path }); },
      write(path, content, options = {}) {
        return host.request('assets.write', { path, content, append: Boolean(options.append) });
      },
      delete(path) { return host.request('assets.delete', { path }); },
    },
    cache: {
      read(path) { return host.request('cache.read', { path }); },
      write(path, content, options = {}) {
        return host.request('cache.write', { path, content, append: Boolean(options.append) });
      },
      clear() { return host.request('cache.clear', {}); },
    },
    secrets: {
      set(key, value) { return host.request('secrets.set', { key, content: value }); },
      get(key) { return host.request('secrets.get', { key }); },
      delete(key) { return host.request('secrets.delete', { key }); },
    },
  };

  function start() {
    const rl = readline.createInterface({ input: process.stdin });
    rl.on('line', (line) => {
      let message;
      try {
        message = JSON.parse(line.trim());
      } catch {
        return;
      }
      if (!message || message.jsonrpc !== '2.0') return;
      if (message.id !== undefined && !message.method) {
        resolveHostResponse(message);
        return;
      }
      if (message.id !== undefined && message.method) {
        handleRequest(message);
      } else if (message.method) {
        handleNotification(message);
      }
    });
  }

  async function handleRequest(req) {
    try {
      if (req.method === 'initialize') {
        initializeParams = req.params || {};
        config = initializeParams.config || {};
        const result = definition.initialize
          ? await definition.initialize(initializeParams, host, req)
          : { ok: true };
        respond(req.id, result || { ok: true });
        return;
      }

      const handler = methods[req.method];
      if (!handler) {
        respondError(req.id, -32601, 'Method not found: ' + req.method);
        return;
      }
      const result = await handler(req.params || {}, host, req);
      respond(req.id, result === undefined ? { ok: true } : result);
    } catch (error) {
      respondError(req.id, -32603, String(error && error.message ? error.message : error));
    }
  }

  function handleNotification(notif) {
    if (notif.method === 'config.update') {
      config = (notif.params && notif.params.config) || {};
    }
    if (notif.method === 'shutdown' && !notifications.shutdown) {
      setTimeout(() => process.exit(0), 50);
      return;
    }
    const handler = notifications[notif.method];
    if (handler) {
      Promise.resolve(handler(notif.params || {}, host, notif)).catch((error) => {
        host.log('error', String(error && error.message ? error.message : error));
      });
    }
  }

  function resolveHostResponse(message) {
    const pending = pendingHostRequests.get(message.id);
    if (!pending) return;
    pendingHostRequests.delete(message.id);
    if (message.error) pending.reject(new Error(message.error.message || 'Host request failed'));
    else pending.resolve(message.result || {});
  }

  return { start, host };
}

function respond(id, result) {
  write({ jsonrpc: '2.0', id, result });
}

function respondError(id, code, message) {
  write({ jsonrpc: '2.0', id, error: { code, message } });
}

function write(message) {
  process.stdout.write(JSON.stringify(message) + '\n');
}

module.exports = { createPlugin };
