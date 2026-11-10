const assert = require('assert');
const helpers = require('./dex-assistant-opencode');

assert.strictEqual(helpers.stripGroupPrefix('dex_ops_get_config'), 'get_config');
assert.strictEqual(helpers.stripGroupPrefix('dex_workspace_execute_shell'), 'execute_shell');
assert.strictEqual(helpers.stripGroupPrefix('get_config'), 'get_config');
assert.strictEqual(helpers.stripGroupPrefix('plugin__a__b__123'), 'plugin__a__b__123');

assert.strictEqual(helpers.eventKind({ type: 'permission.asked', properties: { id: 'per_1' } }), 'permission');
assert.strictEqual(
  helpers.eventKind({ type: 'message.part.updated', properties: { part: { type: 'text', text: 'x' } } }),
  'text'
);
assert.strictEqual(
  helpers.eventKind({ type: 'message.part.updated', properties: { part: { type: 'tool', tool: 'dex_ops_get_config' } } }),
  'tool'
);
assert.strictEqual(helpers.eventKind({ type: 'session.idle' }), 'idle');
assert.strictEqual(helpers.eventKind({ type: 'server.heartbeat' }), null);
assert.strictEqual(helpers.eventKind(null), null);

console.log('dex-assistant-opencode tests ok');
