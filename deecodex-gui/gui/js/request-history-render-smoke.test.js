const assert = require('assert');
const fs = require('fs');
const path = require('path');
const vm = require('vm');

const context = {
  console,
  window: {},
  document: {
    getElementById() { return null; },
    querySelector() { return null; },
    querySelectorAll() { return []; },
  },
  deeStorage: {
    getItem() { return ''; },
    setItem() {},
    removeItem() {},
  },
  invoke: async () => ({}),
  showToast() {},
  showConfirm: async () => false,
  esc(value) {
    return String(value ?? '')
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;');
  },
  escAttr(value) {
    return String(value ?? '')
      .replace(/&/g, '&amp;')
      .replace(/"/g, '&quot;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;');
  },
  trunc(value, len) {
    const text = String(value ?? '');
    return text.length > len ? text.slice(0, len) + '...' : text;
  },
};

context.window.window = context.window;
context.window.document = context.document;

vm.createContext(context);
const source = fs.readFileSync(path.join(__dirname, 'request-history.js'), 'utf8');
vm.runInContext(source, context);

const fallbackTrace = {
  route_surface: 'codex_router',
  requested_model: 'gpt-5',
  anchor: { account_id: 'anchor', account_name: 'Codex Desktop' },
  selected: {
    account_id: 'healthy',
    account_name: 'Healthy Chat',
    mapped_model: 'deepseek-chat',
    health_score: 88,
    capabilities: { protocol: 'chat_translate', tool_mode: 'translated', web: true },
  },
  candidate_count: 2,
  eligible_count: 1,
  skipped_count: 1,
  candidates: [
    { account_id: 'failed', account_name: 'Failed Responses', eligible: false, reason: 'attempt_failed' },
    { account_id: 'healthy', account_name: 'Healthy Chat', eligible: true, reason: 'ready' },
  ],
  fallback_count: 1,
  fallback_attempts: [
    {
      attempt: 1,
      account_id: 'failed',
      account_name: 'Failed Responses',
      endpoint_kind: 'OpenAI Responses',
      mapped_model: 'gpt-5',
      status: 503,
      code: '503',
      message: 'temporary unavailable',
    },
  ],
};

const fallbackHtml = context.renderHistoryRouteTrace({ route_trace: JSON.stringify(fallbackTrace) });
assert(fallbackHtml.includes('降级 Failed Responses · HTTP 503'), fallbackHtml);
assert(fallbackHtml.includes('最终 Healthy Chat'), fallbackHtml);
assert(fallbackHtml.includes('本次已失败'), fallbackHtml);

const preflightTrace = {
  route_surface: 'codex_router',
  requested_model: 'gpt-5',
  anchor: { account_id: 'anchor', account_name: 'Codex Desktop' },
  selected: {
    account_id: 'healthy',
    account_name: 'Healthy Stream',
    mapped_model: 'deepseek-chat',
    health_score: 91,
    capabilities: { protocol: 'chat_translate', tool_mode: 'translated' },
  },
  candidate_count: 2,
  eligible_count: 1,
  skipped_count: 1,
  candidates: [
    { account_id: 'risky', account_name: 'Risky Stream', eligible: false, reason: 'stream_preflight_risk' },
    { account_id: 'healthy', account_name: 'Healthy Stream', eligible: true, reason: 'ready' },
  ],
  stream_preflight: {
    action: 'rerouted',
    reason: 'recent_failure_rate',
    from: {
      account_id: 'risky',
      account_name: 'Risky Stream',
      health_score: 20,
      recent_failed: 2,
      failure_rate_percent: 100,
    },
    to: {
      account_id: 'healthy',
      account_name: 'Healthy Stream',
      mapped_model: 'deepseek-chat',
    },
  },
};

const preflightHtml = context.renderHistoryRouteTrace({ route_trace: JSON.stringify(preflightTrace) });
assert(preflightHtml.includes('流式预选 切换 · Healthy Stream · 近期失败率高'), preflightHtml);
assert(preflightHtml.includes('流式预选避开'), preflightHtml);

console.log('request-history render smoke ok');
