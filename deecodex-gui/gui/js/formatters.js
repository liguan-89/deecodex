function maskKey(key) {
  if (!key) return '';
  if (key.length <= 8) return '****';
  return key.substring(0, 4) + '****' + key.substring(key.length - 4);
}

// 千分位格式化（账号余额、配额等展示）
function formatThousands(value) {
  const n = Number(value || 0);
  if (!Number.isFinite(n)) return '0';
  return n.toLocaleString();
}

// 压缩小数位，用于 B/M/k 后缀数值（如 1.2B、13.4k）
function formatCompactDecimal(value) {
  const digits = value >= 100 ? 1 : value >= 10 ? 1 : 2;
  return value.toFixed(digits).replace(/\.0$/, '');
}
