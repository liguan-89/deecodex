const fs = require('fs');
const path = require('path');
const vm = require('vm');
const assert = require('assert');

const calls = [];
const context = {
  console,
  setInterval: () => 1,
  Date,
  Promise,
  window: {},
  document: {
    addEventListener: () => {},
    visibilityState: 'visible',
  },
};
context.window = context;
context.currentPanel = 'status';
context.selectedClientKind = 'codex';
context.selectedConfigClientKind = 'global';
context.currentConfig = { codex_router_mode: 'relay' };
context.invoke = async (cmd, args) => {
  calls.push({ cmd, args });
  if (cmd === 'get_telemetry_preferences') {
    return {
      enabled: true,
      endpoint: 'https://api.liguan.me/telemetry/v1/events',
      install_id: 'install_test',
      session_id: 'session_test',
      app_version: 'test',
      os: 'macos',
    };
  }
  return { accepted: true };
};
context.addEventListener = () => {};

vm.createContext(context);
vm.runInContext(fs.readFileSync(path.join(__dirname, 'telemetry.js'), 'utf8'), context, { filename: 'telemetry.js' });

(async () => {
  await context.DexTelemetry.init();
  await context.DexTelemetry.track('ad_click', {
    ad_slot: 'service_overview_card',
    api_key: 'sk-secret',
    prompt: 'private prompt',
    page: 'status',
  });
  const eventCall = calls.find(call => call.cmd === 'track_telemetry_event' && call.args.event === 'ad_click');
  assert(eventCall, 'ad_click event should be tracked');
  assert.strictEqual(eventCall.args.props.api_key, '[REDACTED]');
  assert.strictEqual(eventCall.args.props.prompt, '[REDACTED]');
  assert.strictEqual(eventCall.args.props.ad_slot, 'service_overview_card');
  assert.strictEqual(eventCall.args.props.router_mode, 'relay');
  console.log('telemetry smoke ok');
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
