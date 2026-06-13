const assert = require('assert');
const fs = require('fs');
const path = require('path');
const vm = require('vm');

const context = {
  console,
  esc: value => String(value ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;'),
  escAttr: value => String(value ?? '')
    .replace(/&/g, '&amp;')
    .replace(/"/g, '&quot;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;'),
  trunc: (value, max) => {
    const text = String(value ?? '');
    return text.length > max ? text.slice(0, max) + '…' : text;
  },
};

vm.createContext(context);
const source = fs.readFileSync(path.join(__dirname, 'threads.js'), 'utf8');
vm.runInContext(source, context, { filename: 'threads.js' });

const pageHtml = context.renderThreads();
assert(!pageHtml.includes('总线程'));
assert(!pageHtml.includes('可读源'));
assert(!pageHtml.includes('当前筛选'));

const sources = [
  { client_kind: 'codex', client_label: 'Codex', count: 2, available: true, scan_paths: ['/tmp/.codex'], diagnostics: [] },
  { client_kind: 'claude_code', client_label: 'Claude Code', count: 1, available: true, scan_paths: ['/tmp/.claude/projects'], diagnostics: [] },
  { client_kind: 'openclaw', client_label: 'OpenClaw', count: 0, available: false, scan_paths: ['/tmp/.openclaw'], diagnostics: ['暂未发现可读线程源'] },
];

const switcher = context.renderThreadClientSwitcher(sources, 3);
assert(switcher.includes('thread-source-tab active'));
assert(switcher.includes('Claude'));
assert(!switcher.includes('Claude Code'));
assert(switcher.includes('has-issues'));
assert(switcher.includes('thread-client-tabs'));
assert(!switcher.includes('client-logo-codex'));

const rows = context.renderThreadRows([
  {
    client_kind: 'codex',
    native_id: 'codex-thread-1',
    thread_key: 'codex:codex-thread-1',
    title: 'Codex 线程',
    provider: 'deecodex',
    updated_at_ms: 1800000000000,
    message_count: 0,
    delete_available: true,
    detail_available: true,
  },
  {
    client_kind: 'hermes',
    native_id: 'hermes-thread-1',
    thread_key: 'hermes:hermes-thread-1:path-a',
    title: 'Hermes 线程',
    model: 'MiniMax',
    updated_at_ms: 1800000000000,
    message_count: 5,
    delete_available: false,
    detail_available: true,
  },
  {
    client_kind: 'hermes',
    native_id: 'hermes-thread-1',
    thread_key: 'hermes:hermes-thread-1:path-b',
    title: 'Hermes 重名线程',
    model: 'MiniMax',
    updated_at_ms: 1800000000001,
    message_count: 1,
    delete_available: false,
    detail_available: true,
  },
]);
assert(rows.includes("deleteThreadRow('codex','codex-thread-1')"));
assert(!rows.includes("deleteThreadRow('hermes','hermes-thread-1')"));
assert(rows.includes("openThread('hermes','hermes-thread-1','hermes:hermes-thread-1:path-a')"));
assert(rows.includes("openThread('hermes','hermes-thread-1','hermes:hermes-thread-1:path-b')"));
assert(rows.includes('5 条消息'));
assert(rows.includes('thread-actions-cell'));
assert(rows.includes('line-action-icon-trash'));

const codexActions = context.renderCodexThreadActions({
  migrated: false,
  non_unified_count: 2,
  calibration_needed: false,
  active_provider: 'deecodex',
  provider_unified_count: 110,
  codex_visible_count: 110,
  codex_desktop_running: true,
  desktop_project_pending_count: 8,
  desktop_project_repair_blocked: true,
  missing_preview_count: 3,
  missing_user_event_count: 13,
  desktop_recent_pending_count: 27,
  context_window: {
    latest_rollout_model_context_window: 258000,
    latest_rollout_last_total_tokens: 132000,
    latest_rollout_token_usage_found: true,
  },
});
assert(codexActions.includes('codex-thread-strip'));
assert(codexActions.includes('Codex 专属操作'));
assert(codexActions.includes('立即归一'));
assert(codexActions.includes('旧备份还原'));
assert(codexActions.includes('索引待同步: 8'));
assert(!codexActions.includes('上下文'));
assert(!codexActions.includes('最近已用'));
assert(!codexActions.includes('Token源'));
assert(!codexActions.includes('Recent'));
assert(!codexActions.includes('缺预览'));
assert(!codexActions.includes('缺用户事件'));

const diagnostics = context.renderThreadSourceDiagnostics(sources);
assert(!diagnostics.includes('OpenClaw'));
assert(!diagnostics.includes('暂未发现可读线程源'));
assert(!diagnostics.includes('source-muted'));

let invokedDetail = null;
const nodes = {
  mainContent: { innerHTML: '' },
  detailTitle: { textContent: '' },
  detailDeleteBtn: { style: {} },
  detailMessages: { innerHTML: '' },
};
context.document = {
  getElementById: id => nodes[id] || null,
};
context.invoke = async (cmd, args) => {
  invokedDetail = { cmd, args };
  return { thread: { title: 'Hermes 重名线程', delete_available: false }, messages: [] };
};
context.openThread('hermes', 'hermes-thread-1', 'hermes:hermes-thread-1:path-b');
assert.strictEqual(invokedDetail.cmd, 'get_client_thread_content');
assert.strictEqual(invokedDetail.args.threadKey, 'hermes:hermes-thread-1:path-b');

console.log('threads render smoke ok');
