// DEX 助手快捷键绑定
function dexBindShortcuts() {
  if (window._dexShortcutsBound) return;
  window._dexShortcutsBound = true;
  document.addEventListener('keydown', function (e) {
    if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
      var panel = document.getElementById('dexMessages');
      if (!panel || panel.offsetParent === null) return;
      e.preventDefault();
      var input = document.getElementById('dexInput');
      if (input) input.focus();
      return;
    }
    if ((e.ctrlKey || e.metaKey) && e.key === 'l') {
      var panel2 = document.getElementById('dexMessages');
      if (!panel2 || panel2.offsetParent === null) return;
      e.preventDefault();
      dexClearChat();
      return;
    }
    if (e.key === 'Escape') {
      var searchBar = document.getElementById('dexSearchBar');
      if (searchBar && searchBar.style.display !== 'none') {
        dexCloseSearch();
        return;
      }
      if (window.dexAgent && window.dexAgent.isProcessing) {
        dexStopAgent();
      }
    }
  });
}
