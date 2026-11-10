// DEX 助手附件输入
function dexAttachmentBasename(path) {
  if (!path) return '';
  var parts = String(path).split(/[\\/]/);
  return parts[parts.length - 1] || String(path);
}

function dexGetLastAttachment() {
  try {
    return (window.deeStorage && window.deeStorage.getItem(DEX_LAST_ATTACHMENT_KEY)) || '';
  } catch (e) {
    return '';
  }
}

function dexSetLastAttachment(path) {
  try {
    if (window.deeStorage && path) window.deeStorage.setItem(DEX_LAST_ATTACHMENT_KEY, path);
  } catch (e) {
    console.warn('[dexAssistant] 保存最近附件失败:', e);
  }
  dexSyncAttachmentButton();
}

function dexSyncAttachmentButton() {
  var btn = document.querySelector('.dex-input-plus');
  if (!btn) return;
  var last = dexGetLastAttachment();
  if (last) {
    btn.classList.add('has-attachment');
    btn.title = '加入上次附件：' + dexAttachmentBasename(last) + '；按住 Shift 重新选择';
  } else {
    btn.classList.remove('has-attachment');
    btn.title = '添加附件';
  }
}

async function dexPickAttachmentFile() {
  return await DeeCodexTauri.invoke('browse_attachment_file');
}

function dexInsertAttachmentReference(path) {
  var input = document.getElementById('dexInput');
  if (!input || !path) return;
  var marker = '附件: ' + path;
  if (input.value.indexOf(marker) === -1) {
    var prefix = input.value.trim() ? input.value.replace(/\s+$/, '') + '\n\n' : '';
    input.value = prefix + marker + '\n请先读取这个附件，再结合我的问题回答。';
  }
  input.focus();
  input.dispatchEvent(new Event('input', { bubbles: true }));
  dexUpdateTokenCount();
}

async function dexAttachLastFile(event) {
  if (event) event.preventDefault();
  if (window.dexAgent && window.dexAgent.isProcessing) {
    showToast('DEX 正在处理中，稍后再添加附件', 'warn');
    return;
  }
  var forcePick = event && (event.shiftKey || event.altKey || event.metaKey);
  var last = dexGetLastAttachment();
  if (last && !forcePick) {
    dexInsertAttachmentReference(last);
    showToast('已加入上次附件: ' + dexAttachmentBasename(last), 'success');
    return;
  }
  try {
    var path = await dexPickAttachmentFile();
    if (!path) return;
    dexSetLastAttachment(path);
    dexInsertAttachmentReference(path);
    showToast('已添加附件: ' + dexAttachmentBasename(path), 'success');
  } catch (e) {
    showToast('附件选择失败: ' + (e.message || e), 'error');
  }
}
