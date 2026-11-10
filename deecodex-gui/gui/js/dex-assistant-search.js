// ── 搜索功能 ──
window._dexSearchIndex = -1;
window._dexSearchMatches = [];

function dexToggleSearch() {
  var bar = document.getElementById('dexSearchBar');
  if (!bar) return;
  if (bar.style.display === 'none') {
    bar.style.display = '';
    var input = document.getElementById('dexSearchInput');
    if (input) { input.value = ''; input.focus(); }
    var countEl = document.getElementById('dexSearchCount');
    if (countEl) countEl.textContent = '';
  } else {
    dexCloseSearch();
  }
}

function dexPerformSearch() {
  var input = document.getElementById('dexSearchInput');
  var countEl = document.getElementById('dexSearchCount');
  if (!input || !countEl) return;
  var query = input.value.trim().toLowerCase();
  dexClearHighlights();
  window._dexSearchMatches = [];
  window._dexSearchIndex = -1;
  if (!query) { countEl.textContent = ''; return; }
  var msgs = document.getElementById('dexMessages');
  if (!msgs) return;
  var bubbles = msgs.querySelectorAll('.dex-msg');
  for (var i = 0; i < bubbles.length; i++) {
    var msg = bubbles[i];
    var text = (msg.textContent || '').toLowerCase();
    if (text.indexOf(query) !== -1) {
      msg.classList.add('dex-msg-highlight');
      window._dexSearchMatches.push(msg);
    }
  }
  countEl.textContent = window._dexSearchMatches.length + ' 个匹配';
  if (window._dexSearchMatches.length > 0) dexNavigateSearch(1);
}

function dexNavigateSearch(direction) {
  if (window._dexSearchMatches.length === 0) return;
  for (var i = 0; i < window._dexSearchMatches.length; i++) {
    window._dexSearchMatches[i].classList.remove('dex-msg-search-current');
  }
  window._dexSearchIndex += direction;
  if (window._dexSearchIndex >= window._dexSearchMatches.length) window._dexSearchIndex = 0;
  if (window._dexSearchIndex < 0) window._dexSearchIndex = window._dexSearchMatches.length - 1;
  var current = window._dexSearchMatches[window._dexSearchIndex];
  if (current) {
    current.classList.add('dex-msg-search-current');
    current.scrollIntoView({ behavior: 'smooth', block: 'center' });
  }
  var countEl = document.getElementById('dexSearchCount');
  if (countEl) countEl.textContent = (window._dexSearchIndex + 1) + '/' + window._dexSearchMatches.length;
}

function dexCloseSearch() {
  dexClearHighlights();
  var bar = document.getElementById('dexSearchBar');
  if (bar) bar.style.display = 'none';
  window._dexSearchMatches = [];
  window._dexSearchIndex = -1;
}

function dexClearHighlights() {
  var highlighted = document.querySelectorAll('.dex-msg-highlight');
  for (var i = 0; i < highlighted.length; i++) {
    highlighted[i].classList.remove('dex-msg-highlight', 'dex-msg-search-current');
  }
}
