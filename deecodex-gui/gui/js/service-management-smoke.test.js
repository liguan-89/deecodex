const assert = require('assert');
const fs = require('fs');
const path = require('path');
const vm = require('vm');

const source = fs.readFileSync(path.join(__dirname, 'service-management.js'), 'utf8');
const calls = [];
const toasts = [];
const telemetry = [];
const toggleButton = { disabled: false };

const context = {
  console,
  currentPanel: 'status',
  document: {
    body: { appendChild() {} },
    createElement() {
      return {
        className: '',
        insertBefore() {},
        querySelector() { return null; },
      };
    },
    getElementById(id) {
      return id === 'btnToggle' ? toggleButton : null;
    },
  },
  deeStorage: {
    getItem() { return null; },
    setItem() {},
    removeItem() {},
  },
  esc(value) { return String(value); },
  showConfirm: async () => true,
  showToast(message, type) {
    toasts.push({ message, type });
  },
  setTimeout,
  clearTimeout,
  window: {
    DexTelemetry: {
      track(name, props) {
        telemetry.push({ name, props });
      },
    },
    async invoke(name) {
      calls.push(name);
      if (name === 'get_service_status') {
        return { running: false, port: 54600, preferred_port: 54600 };
      }
      if (name === 'start_service') {
        return {
          running: true,
          port: 54601,
          preferred_port: 54600,
          auto_selected: true,
        };
      }
      throw new Error(`unexpected invoke: ${name}`);
    },
    async loadStatus() {},
  },
};
context.window.window = context.window;
context.window.document = context.document;
context.window.showToast = context.showToast;

vm.createContext(context);
vm.runInContext(source, context);

(async () => {
  await context.window.mgmtToggle();

  assert.deepStrictEqual(calls, ['get_service_status', 'start_service']);
  assert.deepStrictEqual(toasts.at(-1), {
    message: '端口已自动切换：54600 → 54601',
    type: 'success',
  });
  assert.strictEqual(telemetry.length, 1);
  assert.strictEqual(telemetry[0].name, 'service_start');
  assert.strictEqual(telemetry[0].props.source, 'manual_toggle');
  assert.strictEqual(telemetry[0].props.port, 54601);
  assert.strictEqual(telemetry[0].props.preferred_port, 54600);
  assert.strictEqual(telemetry[0].props.auto_selected, true);
  assert.strictEqual(toggleButton.disabled, false);

  console.log('service-management smoke tests passed');
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
