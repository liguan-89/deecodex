const assert = require('assert');
const fs = require('fs');
const path = require('path');
const vm = require('vm');

function makeSelect(values = []) {
  let opts = values.map(value => ({ value }));
  return {
    get options() { return opts; },
    set innerHTML(html) {
      opts = Array.from(String(html).matchAll(/<option value="([^"]*)"/g)).map(match => ({ value: match[1] }));
      this._innerHTML = html;
    },
    get innerHTML() { return this._innerHTML || ''; },
  };
}

const elements = {
  codexQuickDefaultModel: makeSelect(['gpt-5.5', 'gpt-5.4']),
  codexQuickAccountName: { value: 'DEX AI 中转' },
  codexQuickUpstream: { value: 'https://api.dexai.vip/v1' },
  codexQuickUpstreamLabel: { textContent: '' },
  codexQuickApiKey: { placeholder: '' },
  codexQuickApiKeyLabel: { textContent: '' },
  codexQuickDefaultModelLabel: { textContent: '' },
  codexQuickFetchModelsBtn: { style: {} },
  codexQuickOpenDeepseekBtn: { style: {}, title: '', querySelector: () => ({ textContent: '' }) },
  codexQuickPasteRelayBtn: { style: {} },
};

const context = {
  console,
  window: {
    codexQuickStartStatus: {
      default_model: 'gpt-5.5',
      known_models: ['gpt-5.5', 'deepseek-v4-pro', 'deepseek-v4-flash'],
      relay_default_model: 'gpt-5.5',
      relay_known_models: ['gpt-5.5', 'gpt-5.4'],
      upstream: 'https://api.deepseek.com/v1',
      relay_upstream: 'https://api.dexai.vip/v1',
    },
  },
  document: {
    getElementById: id => elements[id] || null,
    querySelector: selector => selector === 'input[name="codexQuickMode"]:checked' ? { value: 'api' } : null,
    querySelectorAll: () => [],
  },
  esc: value => String(value ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;'),
  escAttr: value => String(value ?? '')
    .replace(/&/g, '&amp;')
    .replace(/"/g, '&quot;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;'),
};

vm.createContext(context);
const source = fs.readFileSync(path.join(__dirname, 'setup-wizard.js'), 'utf8');
vm.runInContext(source, context, { filename: 'setup-wizard.js' });

assert.strictEqual(context.codexQuickDeepseekDefaultModel(), 'deepseek-v4-pro');
context.codexQuickUpdateModeFields();
assert.strictEqual(elements.codexQuickUpstream.readOnly, true);
assert.strictEqual(elements.codexQuickUpstream.value, 'https://api.deepseek.com/v1');
assert.deepStrictEqual(
  Array.from(elements.codexQuickDefaultModel.options).map(option => option.value),
  ['deepseek-v4-pro', 'deepseek-v4-flash'],
);
assert(!elements.codexQuickDefaultModel.innerHTML.includes('gpt-5.5'));

console.log('setup wizard smoke ok');
