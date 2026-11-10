'use strict';

const assert = require('assert');
const fs = require('fs');
const path = require('path');
const vm = require('vm');

const debugCalls = [];
const context = {
  console: {
    debug: (...args) => debugCalls.push(args),
    error: () => {},
  },
  document: {
    documentElement: { dataset: {} },
    addEventListener: () => {},
  },
  navigator: { userAgent: 'node' },
  WeakSet,
  window: {
    __TAURI__: {
      core: {
        invoke: async () => ({ ok: true }),
      },
      event: {
        listen: async () => () => {},
      },
    },
  },
};
vm.createContext(context);
vm.runInContext(
  fs.readFileSync(path.join(__dirname, 'tauri-api.js'), 'utf8'),
  context,
  { filename: 'tauri-api.js' }
);

(async () => {
  await context.window.DeeCodexTauri.invoke('invoke_plugin_account_action', {
    action: 'login',
    params: {
      email: 'owner@example.test',
      password: 'plain-password',
      invitationCode: 'invite-code',
      nested: {
        refresh_token: 'refresh-token',
        authorization: 'Bearer access-token',
      },
    },
  });

  const start = debugCalls.find(call => call[0] === '[ipc:start]');
  assert(start, '应记录 IPC 开始事件');
  const logged = start[2];
  assert.strictEqual(logged.params.email, 'owner@example.test');
  assert.strictEqual(logged.params.password, '[REDACTED]');
  assert.strictEqual(logged.params.invitationCode, '[REDACTED]');
  assert.strictEqual(logged.params.nested.refresh_token, '[REDACTED]');
  assert.strictEqual(logged.params.nested.authorization, '[REDACTED]');

  await context.window.DeeCodexTauri.invoke('update_account', {
    accountJson: JSON.stringify({
      name: 'safe-name',
      api_key: 'sk-plain',
      client_options: {
        oauth: {
          access_token: 'oauth-access',
          refresh_token: 'oauth-refresh',
        },
      },
    }),
  });
  const accountStart = debugCalls.filter(call => call[0] === '[ipc:start]').at(-1);
  const accountLogged = JSON.parse(accountStart[2].accountJson);
  assert.strictEqual(accountLogged.name, 'safe-name');
  assert.strictEqual(accountLogged.api_key, '[REDACTED]');
  assert.strictEqual(accountLogged.client_options.oauth.access_token, '[REDACTED]');
  assert.strictEqual(accountLogged.client_options.oauth.refresh_token, '[REDACTED]');
  console.log('tauri api redaction ok');
})().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
