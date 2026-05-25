const assert = require('assert');
const fs = require('fs');
const path = require('path');
const vm = require('vm');

const expectedOrder = [
  'dex-agent-state.js',
  'dex-render-markdown.js',
  'dex-assistant.js',
  'placeholder-pages.js',
];
const indexHtml = fs.readFileSync(path.join(__dirname, '..', 'index.html'), 'utf8');
const loadedDexScripts = Array.from(indexHtml.matchAll(/<script src="js\/(dex-[^"?]+\.js|placeholder-pages\.js)\?/g))
  .map(match => match[1])
  .filter(file => expectedOrder.includes(file));
assert.deepStrictEqual(loadedDexScripts, expectedOrder);

function escapeHtml(value) {
  return String(value ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

const context = {
  console,
  module: undefined,
  setTimeout: () => 0,
  clearTimeout() {},
  window: {},
  document: {
    head: { appendChild() {} },
    addEventListener() {},
    getElementById() { return null; },
    querySelector() { return null; },
    createElement() {
      return {
        id: '',
        textContent: '',
        className: '',
        innerHTML: '',
        addEventListener() {},
        appendChild() {},
        querySelector() { return null; },
        querySelectorAll() { return []; },
        remove() {},
      };
    },
  },
  deeStorage: {
    data: {},
    getItem(key) { return this.data[key] || ''; },
    setItem(key, value) { this.data[key] = String(value); },
    removeItem(key) { delete this.data[key]; },
  },
  DeeCodexTauri: {
    invoke: async () => ({}),
    listen: undefined,
  },
  showToast() {},
  switchPanel() {},
  loadAccountsData: async () => {},
  loadPluginsData: async () => {},
  esc: escapeHtml,
  escAttr: escapeHtml,
};
context.window = context;
context.window.deeStorage = context.deeStorage;
context.window.DeeCodexTauri = context.DeeCodexTauri;

vm.createContext(context);
vm.runInContext(fs.readFileSync(path.join(__dirname, 'dex-render-markdown.js'), 'utf8'), context, { filename: 'dex-render-markdown.js' });
vm.runInContext(fs.readFileSync(path.join(__dirname, 'dex-agent-state.js'), 'utf8'), context, { filename: 'dex-agent-state.js' });
vm.runInContext(fs.readFileSync(path.join(__dirname, 'dex-assistant.js'), 'utf8'), context, { filename: 'dex-assistant.js' });
vm.runInContext(fs.readFileSync(path.join(__dirname, 'placeholder-pages.js'), 'utf8'), context, { filename: 'placeholder-pages.js' });

assert.strictEqual(typeof context.renderDexAssistant, 'function');
assert.strictEqual(typeof context.renderProfile, 'function');
assert.strictEqual(typeof context.window.dexAgent.run, 'function');
assert.strictEqual(typeof context.dexAppendMessage, 'function');
assert.strictEqual(typeof context.dexAttachLastFile, 'function');

const dexHtml = context.renderDexAssistant();
assert(dexHtml.includes('primary-page-shell-dex-assistant'));
assert(dexHtml.includes('id="dexMessages"'));
assert(dexHtml.includes('id="dexInput"'));
assert(dexHtml.includes('onclick="dexSendMessage()"'));
assert(dexHtml.includes('onclick="dexAttachLastFile(event)"'));

const welcome = context.dexWelcomeHTML();
assert(welcome.includes('AI 链总览'));
assert(welcome.includes('插件状态'));

const profileHtml = context.renderProfile();
assert(profileHtml.includes('个人中心'));
assert(!profileHtml.includes('DEX助手'));

console.log('dex assistant render smoke ok');
