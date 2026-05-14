(function () {
  const memoryStorage = new Map();
  let safeStorage;

  try {
    const candidate = window.localStorage;
    const probeKey = '__deecodex_storage_probe__';
    candidate.setItem(probeKey, '1');
    candidate.removeItem(probeKey);
    safeStorage = candidate;
  } catch (_) {
    safeStorage = {
      getItem(key) {
        return memoryStorage.has(key) ? memoryStorage.get(key) : null;
      },
      setItem(key, value) {
        memoryStorage.set(key, String(value));
      },
      removeItem(key) {
        memoryStorage.delete(key);
      },
    };
  }

  function showToast(message, type = 'info') {
    const container = document.getElementById('toastContainer');
    if (!container) {
      console.warn(message);
      return;
    }
    const toast = document.createElement('div');
    toast.className = 'toast ' + type;
    toast.textContent = message;
    container.appendChild(toast);
    toast.addEventListener('animationend', (event) => {
      if (event.animationName === 'toast-out') toast.remove();
    });
  }

  function esc(value) {
    if (value === null || value === undefined) return '';
    return String(value)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  function escAttr(value) {
    if (value === null || value === undefined) return '';
    return String(value)
      .replace(/&/g, '&amp;')
      .replace(/"/g, '&quot;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;');
  }

  function trunc(value, len) {
    if (!value) return '';
    const text = String(value);
    return text.length > len ? text.slice(0, len) + '…' : text;
  }

  function showConfirm(message) {
    return new Promise((resolve) => {
      const existing = document.getElementById('confirmModal');
      if (existing) existing.remove();

      const overlay = document.createElement('div');
      overlay.className = 'modal-overlay';
      overlay.id = 'confirmModal';
      overlay.innerHTML = `
        <div class="modal-box" style="max-width:380px;">
          <div class="modal-header">
            <h3>确认操作</h3>
          </div>
          <div class="modal-body" style="padding:20px;text-align:center;font-family:var(--font-mono);font-size:13px;color:var(--text-secondary);">
            ${esc(message)}
          </div>
          <div style="display:flex;gap:12px;padding:0 20px 16px;justify-content:center;">
            <button class="btn btn-primary" id="confirmOk" type="button">确认</button>
            <button class="btn btn-ghost" id="confirmCancel" type="button">取消</button>
          </div>
        </div>`;
      document.body.appendChild(overlay);

      const cleanup = () => overlay.remove();
      document.getElementById('confirmOk').onclick = () => {
        cleanup();
        resolve(true);
      };
      document.getElementById('confirmCancel').onclick = () => {
        cleanup();
        resolve(false);
      };
      overlay.addEventListener('click', (event) => {
        if (event.target === overlay) {
          cleanup();
          resolve(false);
        }
      });
    });
  }

  window.deeStorage = safeStorage;
  window.showToast = showToast;
  window.showConfirm = showConfirm;
  window.esc = esc;
  window.escAttr = escAttr;
  window.trunc = trunc;
})();
