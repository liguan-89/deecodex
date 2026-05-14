const invoke = window.DeeCodexTauri.invoke;
window.invoke = invoke;

// ═══════════════════════════════════════════════════════════════
// 主题管理
// ═══════════════════════════════════════════════════════════════
const THEME_KEY = 'deecodex.theme';

function getStoredTheme() {
  return deeStorage.getItem(THEME_KEY) || 'auto';
}

function getAutoTheme() {
  const h = new Date().getHours();
  return h >= 6 && h < 18 ? 'light' : 'dark';
}

function applyTheme(theme) {
  const effective = theme === 'auto' ? getAutoTheme() : theme;
  document.documentElement.setAttribute('data-theme', effective);
  deeStorage.setItem(THEME_KEY, theme);

  // 更新切换按钮激活状态
  const btns = document.querySelectorAll('.theme-toggle-btn');
  btns.forEach(b => b.classList.toggle('active', b.dataset.theme === theme));
}

function initTheme() {
  const saved = getStoredTheme();
  applyTheme(saved);

  // 点击事件
  document.getElementById('themeToggle').addEventListener('click', (e) => {
    const btn = e.target.closest('.theme-toggle-btn');
    if (!btn) return;
    applyTheme(btn.dataset.theme);
  });

  // 自动模式下每小时重新评估
  setInterval(() => {
    if (getStoredTheme() === 'auto') {
      applyTheme('auto');
    }
  }, 3600000);
}

// 尽早初始化主题（DOM 就绪前设置 data-theme 属性，减少闪烁）
(function() {
  const saved = deeStorage.getItem(THEME_KEY) || 'auto';
  const effective = saved === 'auto' ? getAutoTheme() : saved;
  document.documentElement.setAttribute('data-theme', effective);
})();

// ═══════════════════════════════════════════════════════════════
// 数据定义（与 GuiConfig 对应）
// ═══════════════════════════════════════════════════════════════
const CONFIG_SECTIONS = [
  {
    id: 'basic', icon: '●', label: '基础设置',
    fields: [
      { key: 'port', label: '监听端口', hint: '服务监听的本地端口，默认 4446', type: 'number', min: 1, max: 65535 },
{ key: 'max_body_mb', label: '最大请求体 (MB)', hint: '上传/请求体大小上限', type: 'number', min: 1, max: 2048 },
      { key: 'chinese_thinking', label: '中文思考模式', hint: '强制中文思维链输出', type: 'checkbox' },
      { key: 'codex_auto_inject', label: '自动注入 Codex 配置', hint: '启动时将 deecodex 路由注入 Codex config.toml，停止时移除', type: 'checkbox' },
      { key: 'codex_persistent_inject', label: '持久注入', hint: '配置持久保留在 Codex config.toml 中，不再自动移除', type: 'checkbox' },
      { key: 'data_dir', label: '数据目录', hint: '配置文件、日志、PID 文件存储位置', type: 'text', placeholder: '.deecodex' },
      { key: 'prompts_dir', label: 'Prompts 目录', hint: 'Hosted Prompts 模板存放目录', type: 'text', placeholder: 'prompts' },
    ]
  },
  {
    id: 'token', icon: '◈', label: 'Token 异常检测',
    fields: [
      { key: 'token_anomaly_prompt_max', label: '最大提示词 Token', hint: '单次请求提示词 Token 上限，0=禁用', type: 'number', min: 0, placeholder: '200000' },
      { key: 'token_anomaly_spike_ratio', label: '飙升比率阈值', hint: '相对于滑动平均的突然飙升倍数，0=禁用', type: 'number', min: 0, step: 0.1, placeholder: '5.0' },
      { key: 'token_anomaly_burn_window', label: '燃烧检测窗口 (秒)', hint: '突发消耗的统计窗口大小', type: 'number', min: 1, placeholder: '120' },
      { key: 'token_anomaly_burn_rate', label: '燃烧速率阈值', hint: '窗口内 Token/分钟 消耗速率上限，0=禁用', type: 'number', min: 0, placeholder: '500000' },
    ]
  },
  {
    id: 'security', icon: '◈', label: '安全策略 & 执行器',
    fields: [
      { key: 'allowed_mcp_servers', label: 'MCP 服务器白名单', hint: '逗号分隔，如 filesystem,github', type: 'text', placeholder: 'filesystem,github' },
      { key: 'allowed_computer_displays', label: '显示器/环境白名单', hint: '逗号分隔的允许显示器标识', type: 'text' },
      { key: 'computer_executor', label: 'Computer 后端', hint: 'computer_use 工具的执行后端', type: 'select', options: ['disabled', 'playwright', 'browser-use'] },
      { key: 'computer_executor_timeout_secs', label: 'Computer 超时 (秒)', hint: '单步操作最大执行时间', type: 'number', min: 1, max: 600 },
      { key: 'mcp_executor_config', label: 'MCP 执行器配置', hint: 'JSON 配置或 JSON 文件路径', type: 'json', placeholder: '{}' },
      { key: 'mcp_executor_timeout_secs', label: 'MCP 超时 (秒)', hint: '单次工具调用最大时间', type: 'number', min: 1, max: 600 },
      { key: 'playwright_state_dir', label: 'Playwright 状态目录', hint: '浏览器状态 (cookies/localStorage) 持久化目录', type: 'text' },
      { key: 'browser_use_bridge_url', label: 'Browser-Use Bridge URL', hint: 'Browser-Use HTTP 桥接地址', type: 'text' },
      { key: 'browser_use_bridge_command', label: 'Browser-Use Bridge 命令', hint: '通过环境变量接收 JSON 的命令', type: 'text' },
    ]
  },
  {
    id: 'cdp', icon: '⬢', label: 'CDP 注入',
    fields: [
      { key: 'codex_launch_with_cdp', label: '自动启动 Codex', hint: '启动服务时自动打开 Codex 桌面版并注入插件解锁脚本', type: 'checkbox' },
      { key: 'cdp_port', label: 'CDP 调试端口', hint: 'Codex Electron 远程调试端口', type: 'number', min: 9222, max: 9999 },
    ]
  },
];

// ═══════════════════════════════════════════════════════════════
// 全局状态
// ═══════════════════════════════════════════════════════════════
