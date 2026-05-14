function maskKey(key) {
  if (!key) return '';
  if (key.length <= 8) return '****';
  return key.substring(0, 4) + '****' + key.substring(key.length - 4);
}

function fmtUptime(secs) {
  if (!secs || secs <= 0) return '—';
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const parts = [];
  if (d > 0) parts.push(d + '天');
  if (h > 0) parts.push(h + '小时');
  parts.push(m + '分钟');
  return parts.join(' ');
}

function computerLabel(v) {
  if (!v || v === 'disabled') return '已禁用';
  if (v === 'playwright') return 'Playwright';
  if (v === 'browser-use') return 'Browser-Use';
  return v;
}

		// ═══════════════════════════════════════════════════════════════
