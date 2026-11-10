// UI 工具 — toast、confirm、HTML 转义、存储
// ═══════════════════════════════════════════════════════════════
(function () {
  // localStorage 在 Tauri WebView 中始终可用
  window.deeStorage = window.localStorage;

  function showToast(message, type) {
    type = type || 'info';
    var container = document.getElementById('toastContainer');
    if (!container) return;
    var toast = document.createElement('div');
    toast.className = 'toast ' + type;
    toast.textContent = message;
    container.appendChild(toast);
    toast.addEventListener('animationend', function (e) {
      if (e.animationName === 'toast-out') toast.remove();
    });
  }

  function esc(value) {
    if (value === null || value === undefined) return '';
    return String(value).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
  }

  function escAttr(value) {
    if (value === null || value === undefined) return '';
    return String(value).replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  }

  function trunc(value, len) {
    if (!value) return '';
    var text = String(value);
    return text.length > len ? text.slice(0, len) + '…' : text;
  }

  function showConfirm(message) {
    return new Promise(function (resolve) {
      var existing = document.getElementById('confirmModal');
      if (existing) existing.remove();

      var overlay = document.createElement('div');
      overlay.className = 'modal-overlay';
      overlay.id = 'confirmModal';
      overlay.innerHTML =
        '<div class="modal-box" style="max-width:380px;">' +
        '<div class="modal-header"><h3>确认操作</h3></div>' +
        '<div class="modal-body" style="padding:20px;text-align:center;font-family:var(--font-mono);font-size:13px;color:var(--text-secondary);">' +
        esc(message) +
        '</div>' +
        '<div style="display:flex;gap:12px;padding:0 20px 16px;justify-content:center;">' +
        '<button class="btn btn-primary" id="confirmOk" type="button">确认</button>' +
        '<button class="btn btn-ghost" id="confirmCancel" type="button">取消</button></div></div>';
      document.body.appendChild(overlay);

      function cleanup() { overlay.remove(); }
      document.getElementById('confirmOk').onclick = function () { cleanup(); resolve(true); };
      document.getElementById('confirmCancel').onclick = function () { cleanup(); resolve(false); };
      overlay.addEventListener('click', function (e) { if (e.target === overlay) { cleanup(); resolve(false); } });
    });
  }

  // 带可选勾选框的确认（用于「清理配置文件」是否同时移除 DEX AI 账号）。
  // 返回 { ok, checked }；checked 为勾选框状态（未勾选时 false）。
  function showConfirmWithCheckbox(message, checkboxLabel) {
    return new Promise(function (resolve) {
      var existing = document.getElementById('confirmModal');
      if (existing) existing.remove();

      var overlay = document.createElement('div');
      overlay.className = 'modal-overlay';
      overlay.id = 'confirmModal';
      overlay.innerHTML =
        '<div class="modal-box" style="max-width:420px;">' +
        '<div class="modal-header"><h3>确认操作</h3></div>' +
        '<div class="modal-body" style="padding:20px;text-align:left;font-family:var(--font-mono);font-size:13px;color:var(--text-secondary);line-height:1.6;">' +
        esc(message) +
        '</div>' +
        '<label style="display:flex;align-items:flex-start;gap:8px;margin:0 20px 16px;font-size:13px;color:var(--text-primary);cursor:pointer;">' +
        '<input type="checkbox" id="confirmCheckbox" style="margin-top:3px;flex:none;">' +
        '<span>' + esc(checkboxLabel) + '</span></label>' +
        '<div style="display:flex;gap:12px;padding:0 20px 16px;justify-content:center;">' +
        '<button class="btn btn-primary" id="confirmOk" type="button">确认</button>' +
        '<button class="btn btn-ghost" id="confirmCancel" type="button">取消</button></div></div>';
      document.body.appendChild(overlay);

      function cleanup() { overlay.remove(); }
      document.getElementById('confirmOk').onclick = function () {
        var checked = document.getElementById('confirmCheckbox').checked;
        cleanup();
        resolve({ ok: true, checked: checked });
      };
      document.getElementById('confirmCancel').onclick = function () {
        cleanup();
        resolve({ ok: false, checked: false });
      };
      overlay.addEventListener('click', function (e) {
        if (e.target === overlay) { cleanup(); resolve({ ok: false, checked: false }); }
      });
    });
  }

  window.showToast = showToast;
  window.showConfirm = showConfirm;
  window.showConfirmWithCheckbox = showConfirmWithCheckbox;
  window.esc = esc;
  window.escAttr = escAttr;
  window.trunc = trunc;
})();
