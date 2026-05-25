const assert = require('assert');

global.esc = value => String(value ?? '')
  .replace(/&/g, '&amp;')
  .replace(/</g, '&lt;')
  .replace(/>/g, '&gt;')
  .replace(/"/g, '&quot;');

const { dexRenderMarkdown } = require('./dex-render-markdown');

const rich = dexRenderMarkdown([
  '## 标题',
  '',
  '- **重点**',
  '- `inline`',
  '',
  '[OpenAI](https://openai.com)',
].join('\n'));
assert(rich.includes('<h3 class="dex-md-h3">标题</h3>'));
assert(rich.includes('<ul>'));
assert(rich.includes('<strong>重点</strong>'));
assert(rich.includes('<code class="dex-inline-code">inline</code>'));
assert(rich.includes('<a href="https://openai.com" target="_blank" rel="noopener">OpenAI</a>'));

const unsafe = dexRenderMarkdown('[bad](javascript:alert(1)) [file](file:///tmp/a)');
assert(!unsafe.includes('<a href="javascript:'));
assert(!unsafe.includes('<a href="file:'));
assert(unsafe.includes('bad (javascript:alert(1))'));
assert(unsafe.includes('file (file:///tmp/a)'));

const table = dexRenderMarkdown('| A | B |\n|---|---|\n| 1 | 2 |');
assert(table.includes('<table class="dex-md-table">'));
assert(table.includes('<th>A</th>'));
assert(table.includes('<td>2</td>'));

const code = dexRenderMarkdown('```js\n<script>alert("x")</script>\n```');
assert(code.includes('<pre><code class="dex-code-block">'));
assert(code.includes('&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;'));
assert(!code.includes('<script>alert'));

console.log('dex markdown render ok');
