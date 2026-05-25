// DEX 助手 Markdown 渲染
function dexRenderMarkdown(text) {
  if (!text) return '';
  var codeBlocks = [];
  var safe = text.replace(/```(\w*)\n([\s\S]*?)```/g, function (_match, _lang, code) {
    codeBlocks.push('<pre><code class="dex-code-block">' + esc(code.trim()) + '</code></pre>');
    return '%%CODEBLOCK_' + (codeBlocks.length - 1) + '%%';
  });
  var inlineCodes = [];
  safe = safe.replace(/`([^`]+)`/g, function (_match, code) {
    inlineCodes.push('<code class="dex-inline-code">' + esc(code) + '</code>');
    return '%%INLINECODE_' + (inlineCodes.length - 1) + '%%';
  });
  var html = esc(safe);
  var lines = html.split('\n');
  var result = [], i = 0;
  var inTable = false, tableRows = [];
  var inBQ = false, bqLines = [];
  var inList = false, listItems = [], listOrd = false;

  function fBQ() { if (bqLines.length) { result.push('<blockquote><p>' + bqLines.join('<br>') + '</p></blockquote>'); bqLines = []; inBQ = false; } }
  function fTbl() { if (tableRows.length) { result.push(dexRenderTable(tableRows)); tableRows = []; inTable = false; } }
  function fList() { if (listItems.length) { var h = ''; for (var li = 0; li < listItems.length; li++) h += '<li>' + listItems[li] + '</li>'; result.push(listOrd ? '<ol>' + h + '</ol>' : '<ul>' + h + '</ul>'); listItems = []; inList = false; listOrd = false; } }
  function fAll() { fBQ(); fTbl(); fList(); }

  while (i < lines.length) {
    var line = lines[i].trim();
    if (line === '') { fAll(); result.push(''); i++; continue; }
    if (/^(-{3,}|\*{3,})$/.test(line)) { fAll(); result.push('<hr>'); i++; continue; }
    if (/^&gt; /.test(line)) { fTbl(); fList(); inBQ = true; bqLines.push(line.replace(/^&gt; /, '')); i++; continue; }
    if (/^\|.+\|$/.test(line) && line.lastIndexOf('|') > 0) {
      fBQ(); fList();
      if (i + 1 < lines.length && /^\|[-: |]+\|$/.test(lines[i + 1].trim())) { inTable = true; tableRows.push(line); i++; continue; }
      if (inTable) { tableRows.push(line); i++; continue; }
    }
    if (inTable && !/^\|.+\|$/.test(line)) fTbl();
    if (!inBQ && bqLines.length) fBQ();
    if (/^\d+\. .+/.test(line)) { if (!inList || !listOrd) { fList(); inList = true; listOrd = true; } listItems.push(line.replace(/^\d+\. /, '')); i++; continue; }
    if (/^[-*] .+/.test(line)) { if (!inList || listOrd) { fList(); inList = true; listOrd = false; } listItems.push(line.replace(/^[-*] /, '')); i++; continue; }
    if (inList) fList();
    if (/^### .+/.test(line)) { result.push('<h4 class="dex-md-h4">' + line.replace(/^### /, '') + '</h4>'); i++; continue; }
    if (/^## .+/.test(line)) { result.push('<h3 class="dex-md-h3">' + line.replace(/^## /, '') + '</h3>'); i++; continue; }
    result.push(line); i++;
  }
  fAll();
  html = result.join('\n');
  html = html.replace(/\[([^\]]+)\]\(([^)]+)\)/g, function (_match, label, href) {
    var url = String(href || '').trim();
    var protocol = url.toLowerCase();
    if (protocol.startsWith('http://') || protocol.startsWith('https://')) {
      return '<a href="' + url + '" target="_blank" rel="noopener">' + label + '</a>';
    }
    return label + ' (' + url + ')';
  });
  html = html.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  html = html.replace(/\*([^*]+)\*/g, '<em>$1</em>');
  html = html.replace(/%%CODEBLOCK_(\d+)%%/g, function (_match, n) { return codeBlocks[parseInt(n)]; });
  html = html.replace(/%%INLINECODE_(\d+)%%/g, function (_match, n) { return inlineCodes[parseInt(n)]; });
  html = html.replace(/\n\n/g, '</p><p>');
  html = html.replace(/\n/g, '<br>');
  if (/<(table|pre|blockquote|h[3-4]|ul|ol|hr)/.test(html)) {
    return '<div class="dex-md">' + html + '</div>';
  }
  return '<p>' + html + '</p>';
}

function dexRenderTable(rows) {
  if (rows.length === 0) return '';
  var h = '<table class="dex-md-table"><thead><tr>';
  var hdr = rows[0].replace(/^\|/, '').replace(/\|$/, '').split('|');
  for (var ci = 0; ci < hdr.length; ci++) h += '<th>' + hdr[ci].trim() + '</th>';
  h += '</tr></thead><tbody>';
  for (var ri = 1; ri < rows.length; ri++) {
    h += '<tr>';
    var cells = rows[ri].replace(/^\|/, '').replace(/\|$/, '').split('|');
    for (var cj = 0; cj < cells.length; cj++) h += '<td>' + cells[cj].trim() + '</td>';
    h += '</tr>';
  }
  return h + '</tbody></table>';
}

if (typeof module !== 'undefined') {
  module.exports = { dexRenderMarkdown, dexRenderTable };
}
