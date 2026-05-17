const assert = require('assert');
const state = require('./dex-agent-state');

assert.strictEqual(state.shouldStopAfterToolResult({ cancelled: true }), true);
assert.strictEqual(state.shouldStopAfterToolResult({ error: 'x' }), false);

const messages = [
  { role: 'system', content: 's' },
  { role: 'user', content: '执行 shell 命令 pwd' },
  { role: 'assistant', tool_calls: [{ id: 'call_1', function: { name: 'execute_shell' } }] },
];
state.removePendingAssistantToolCall(messages);
assert.deepStrictEqual(messages.map((m) => m.role), ['system', 'user']);

const stable = [{ role: 'system' }, { role: 'assistant', content: 'ok' }];
state.removePendingAssistantToolCall(stable);
assert.strictEqual(stable.length, 2);
