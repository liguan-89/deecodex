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
    id: 'gateway', scope: 'global', icon: '●', label: '网关运行',
    fields: [
      { key: 'host', label: '服务地址', hint: '网关监听地址，默认 127.0.0.1；局域网接入可填 0.0.0.0', type: 'text', placeholder: '127.0.0.1', layout: 'half' },
      { key: 'port', label: '服务监听端口', hint: '所有已接入客户端连接 deecodex 网关的本地端口', type: 'number', min: 1, max: 65535, layout: 'half' },
      { key: 'max_body_mb', label: '请求体上限 (MB)', hint: '网关允许转发的单次上传或请求体大小', type: 'number', min: 1, max: 2048, layout: 'half' },
      { key: 'data_dir', label: '运行数据目录', hint: '网关配置、日志、PID 和运行状态存储位置', type: 'text', placeholder: '.deecodex', layout: 'half' },
    ]
  },
  {
    id: 'codex', scope: 'codex', icon: '◇', label: 'Codex 配置注入',
    fields: [
      { key: 'codex_router_mode', label: '路由模式', hint: 'API 模式不需要 Codex 官方账号；智能路由使用 Codex 登录态和账号模型直选', type: 'select', options: ['api', 'smart'], optionLabels: { api: 'API 模式', smart: '智能路由' }, layout: 'half' },
      { key: 'codex_auto_inject', label: '自动注入 Codex 配置', hint: '启动时将 deecodex 路由注入 Codex config.toml，停止时移除', type: 'checkbox', layout: 'half' },
      { key: 'codex_persistent_inject', label: '持久注入', hint: '配置持久保留在 Codex config.toml 中，不再自动移除', type: 'checkbox', layout: 'half' },
    ]
  },
  {
    id: 'translation', scope: 'codex', icon: '◇', label: 'Codex 响应翻译',
    fields: [
      { key: 'chinese_thinking', label: '中文推理输出', hint: '在 Codex Responses → Chat 翻译层追加中文输出约束', type: 'checkbox', layout: 'third' },
      { key: 'prompts_dir', label: 'Hosted Prompts 目录', hint: 'Codex Responses 请求可复用的托管 prompt 模板目录', type: 'text', placeholder: 'prompts', layout: 'half' },
    ]
  },
  {
    id: 'token', scope: 'codex', icon: '◈', label: 'Codex 请求治理',
    fields: [
      { key: 'token_anomaly_prompt_max', label: '提示词 Token 上限', hint: '单次 Codex 请求的提示词 Token 上限，0=禁用', type: 'number', min: 0, placeholder: '200000', layout: 'half' },
      { key: 'token_anomaly_spike_ratio', label: '突增倍率阈值', hint: '相对滑动平均的异常突增倍数，0=禁用', type: 'number', min: 0, step: 0.1, placeholder: '5.0', layout: 'half' },
      { key: 'token_anomaly_burn_window', label: '突发消耗窗口 (秒)', hint: '统计短时 Token 消耗的时间窗口', type: 'number', min: 1, placeholder: '120', layout: 'half' },
      { key: 'token_anomaly_burn_rate', label: '消耗速率阈值', hint: '窗口内 Token/分钟 消耗速率上限，0=禁用', type: 'number', min: 0, placeholder: '500000', layout: 'half' },
    ]
  },
  {
    id: 'tools', scope: 'codex', icon: '◈', label: 'Codex 工具执行',
    collapsed: true,
    summary: 'Codex Responses tools 的 MCP、Computer、Playwright 与 Browser-Use 底层执行参数，通常无需修改。',
    fields: [
      { key: 'allowed_mcp_servers', label: '允许的 MCP 服务器', hint: '逗号分隔，如 filesystem,github；控制 Codex 可调用的 MCP 服务', type: 'text', placeholder: 'filesystem,github', layout: 'half' },
      { key: 'allowed_computer_displays', label: '允许的显示器/环境', hint: '逗号分隔的显示器或执行环境标识', type: 'text', layout: 'half' },
      { key: 'computer_executor', label: 'Computer 执行后端', hint: 'Codex computer_use 工具执行后端', type: 'select', options: ['disabled', 'playwright', 'browser-use'], layout: 'half' },
      { key: 'computer_executor_timeout_secs', label: 'Computer 执行超时 (秒)', hint: '单步操作最大执行时间', type: 'number', min: 1, max: 600, layout: 'half' },
      { key: 'mcp_executor_config', label: 'MCP 执行配置', hint: 'Codex MCP 调用配置，支持 JSON 配置或 JSON 文件路径', type: 'json', placeholder: '{}', layout: 'wide' },
      { key: 'mcp_executor_timeout_secs', label: 'MCP 执行超时 (秒)', hint: '单次工具调用最大时间', type: 'number', min: 1, max: 600, layout: 'half' },
      { key: 'playwright_state_dir', label: 'Playwright 状态目录', hint: '浏览器状态 (cookies/localStorage) 持久化目录', type: 'text', layout: 'half' },
      { key: 'browser_use_bridge_url', label: 'Browser-Use 桥接地址', hint: 'Browser-Use HTTP 桥接服务地址', type: 'text', layout: 'half' },
      { key: 'browser_use_bridge_command', label: 'Browser-Use 桥接命令', hint: '通过环境变量接收 JSON 的桥接启动命令', type: 'text', layout: 'half' },
    ]
  },
  {
    id: 'cdp', scope: 'codex', icon: '⬢', label: 'Codex CDP 调试',
    fields: [
      { key: 'codex_launch_with_cdp', label: '自动启动 Codex', hint: '启动服务时自动打开 Codex 桌面版并注入插件解锁脚本', type: 'checkbox', layout: 'half' },
      { key: 'cdp_port', label: 'CDP 调试端口', hint: 'Codex Electron 远程调试端口', type: 'number', min: 9222, max: 9999, layout: 'half' },
    ]
  },
];

// ═══════════════════════════════════════════════════════════════
// 全局状态
// ═══════════════════════════════════════════════════════════════
