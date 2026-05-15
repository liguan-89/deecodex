// DEX助手
// ═══════════════════════════════════════════════════════════════

// ── CSS 注入：DEX 专用样式（一次性）──
(function () {
  if (!document.getElementById('dex-inline-style')) {
    var style = document.createElement('style');
    style.id = 'dex-inline-style';
    style.textContent = [
      '.dex-spinner{display:inline-block;width:14px;height:14px;border:2px solid var(--border-color,#334155);border-top-color:var(--accent-color,#00c8e8);border-radius:50%;animation:dex-spin 0.8s linear infinite;vertical-align:middle;margin-right:6px;flex-shrink:0}',
      '@keyframes dex-spin{to{transform:rotate(360deg)}}',
      '.dex-tool-summary{color:var(--text-secondary,#6b7fa8);font-size:12px;margin-left:6px}',
      '.dex-tool-details{margin-top:4px;font-size:12px}',
      '.dex-tool-details summary{cursor:pointer;color:var(--text-secondary,#6b7fa8)}',
      '.dex-tool-details pre{max-height:300px;overflow-y:auto;font-size:11px;margin-top:4px;padding:6px 8px}',
      '.dex-tool-error-details summary{color:var(--error-color,#ef4444)}',
      '.dex-model-select{font-size:12px;padding:2px 6px;border:1px solid var(--border-color,#334155);border-radius:4px;background:var(--bg-secondary,#0f172a);color:var(--text-primary,#c4d0e4);cursor:pointer;margin-right:6px;max-width:160px}',
      '.dex-md-table{width:100%;border-collapse:collapse;margin:8px 0;font-size:12px}',
      '.dex-md-table th,.dex-md-table td{border:1px solid var(--border-color,#334155);padding:4px 8px;text-align:left}',
      '.dex-md-table th{background:var(--bg-secondary,#0f172a);font-weight:600;color:var(--text-primary,#c4d0e4)}',
      '.dex-bubble-text blockquote{border-left:3px solid var(--accent-color,#00c8e8);margin:8px 0;padding:4px 12px;color:var(--text-secondary,#6b7fa8)}',
      '.dex-bubble-text hr{border:none;border-top:1px solid var(--border-color,#334155);margin:8px 0}',
      '.dex-bubble-text a{color:var(--accent-color,#00c8e8);text-decoration:underline}',
      '.dex-bubble-text a:hover{color:var(--accent-hover,#00e0ff)}',
      '.dex-bubble-text p{margin:4px 0}',
      '.dex-bubble-text ul,.dex-bubble-text ol{margin:4px 0;padding-left:20px}',
      '.dex-bubble-text h3.dex-md-h3{font-size:15px;margin:8px 0 4px;color:var(--text-primary,#c4d0e4)}',
      '.dex-bubble-text h4.dex-md-h4{font-size:14px;margin:6px 0 4px;color:var(--text-primary,#c4d0e4)}',
      '.dex-tool-msg.dex-tool-result .dex-tool-icon{margin-right:6px}',
      '.dex-tool-msg.dex-tool-error .dex-tool-icon{margin-right:6px}',
      '.dex-status-bar{display:flex;align-items:center;gap:8px;padding:2px 0;font-size:12px;color:var(--text-secondary,#6b7fa8)}',
      '.dex-status-dot{width:8px;height:8px;border-radius:50%;display:inline-block;flex-shrink:0;background:#6b7fa8}',
      '.dex-status-dot.dex-status-ok{background:#22c55e}',
      '.dex-status-dot.dex-status-err{background:#ef4444}',
      '.dex-status-dot.dex-status-warn{background:#f59e0b}',
      '.dex-search-bar{display:flex;align-items:center;gap:6px;padding:6px 12px;background:var(--bg-secondary,#0f172a);border-bottom:1px solid var(--border-color,#334155)}',
      '.dex-search-bar input{flex:1;background:var(--bg-primary,#060b14);border:1px solid var(--border-color,#334155);color:var(--text-primary,#c4d0e4);padding:4px 8px;border-radius:4px;font-size:12px}',
      '.dex-search-bar .dex-search-count{font-size:11px;color:var(--text-secondary,#6b7fa8);white-space:nowrap}',
      '.dex-search-bar .btn{padding:2px 8px;font-size:11px}',
      '.dex-msg-highlight .dex-bubble{outline:2px solid var(--accent-color,#00c8e8);outline-offset:2px;border-radius:8px}',
      '.dex-msg-highlight.dex-msg-search-current .dex-bubble{outline-color:#f59e0b;outline-width:3px}',
      '.dex-token-count{font-size:11px;color:var(--text-secondary,#6b7fa8);white-space:nowrap;margin-right:6px;align-self:center}',
      '.dex-tool-preview{font-size:11px;color:var(--accent-color,#00c8e8);margin-top:2px;font-style:italic}'
    ].join('\n');
    document.head.appendChild(style);
  }
})();

// ── 65 个工具定义 ──
var DEX_TOOLS = [
  // A. 服务管理 (5个)
  {
    name: 'get_service_status',
    tauriCmd: 'get_service_status',
    level: 0,
    confirm: null,
    description: '获取 deecodex 服务运行状态',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'start_service',
    tauriCmd: 'start_service',
    level: 2,
    confirm: null,
    description: '启动 deecodex 服务',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'stop_service',
    tauriCmd: 'stop_service',
    level: 3,
    confirm: '确定要停止 deecodex 服务吗？',
    description: '停止 deecodex 服务',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'launch_codex_cdp',
    tauriCmd: 'launch_codex_cdp',
    level: 2,
    confirm: null,
    description: '启动 Codex 桌面应用（CDP 模式）',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'stop_codex_cdp',
    tauriCmd: 'stop_codex_cdp',
    level: 3,
    confirm: '确定要关闭 Codex 桌面应用吗？',
    description: '关闭 Codex 桌面应用（CDP 模式）',
    parameters: { type: 'object', properties: {}, required: [] }
  },

  // B. 配置管理 (5个)
  {
    name: 'get_config',
    tauriCmd: 'get_config',
    level: 0,
    confirm: null,
    description: '获取当前 deecodex 配置',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'save_config',
    tauriCmd: 'save_config',
    level: 2,
    confirm: null,
    description: '保存 deecodex 配置',
    parameters: {
      type: 'object',
      properties: {
        config_json: { type: 'string', description: 'JSON 格式的配置内容' }
      },
      required: ['config_json']
    }
  },
  {
    name: 'validate_config',
    tauriCmd: 'validate_config',
    level: 0,
    confirm: null,
    description: '校验 deecodex 配置',
    parameters: {
      type: 'object',
      properties: {
        config_json: { type: 'string', description: '待校验的 JSON 配置' }
      },
      required: ['config_json']
    }
  },
  {
    name: 'run_diagnostics',
    tauriCmd: 'run_diagnostics',
    level: 0,
    confirm: null,
    description: '运行标准诊断检查',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'run_full_diagnostics',
    tauriCmd: 'run_full_diagnostics',
    level: 1,
    confirm: null,
    description: '运行完整诊断检查（含网络测试）',
    parameters: { type: 'object', properties: {}, required: [] }
  },

  // C. 账号管理 (8个)
  {
    name: 'list_accounts',
    tauriCmd: 'list_accounts',
    level: 0,
    confirm: null,
    description: '列出所有账号配置',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'get_active_account',
    tauriCmd: 'get_active_account',
    level: 0,
    confirm: null,
    description: '获取当前活跃账号信息',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'add_account',
    tauriCmd: 'add_account',
    level: 2,
    confirm: null,
    description: '添加新账号',
    parameters: {
      type: 'object',
      properties: {
        provider: { type: 'string', description: '供应商标识（如 openai、deepseek、custom）' },
        account_json: { type: 'string', description: 'JSON 格式的账号配置' }
      },
      required: ['provider', 'account_json']
    }
  },
  {
    name: 'update_account',
    tauriCmd: 'update_account',
    level: 2,
    confirm: null,
    description: '更新账号配置',
    parameters: {
      type: 'object',
      properties: {
        id: { type: 'string', description: '账号 ID' },
        account_json: { type: 'string', description: 'JSON 格式的更新内容' }
      },
      required: ['id', 'account_json']
    }
  },
  {
    name: 'delete_account',
    tauriCmd: 'delete_account',
    level: 3,
    confirm: '确定要删除该账号吗？此操作不可撤销。',
    description: '删除账号',
    parameters: {
      type: 'object',
      properties: {
        id: { type: 'string', description: '账号 ID' }
      },
      required: ['id']
    }
  },
  {
    name: 'switch_account',
    tauriCmd: 'switch_account',
    level: 2,
    confirm: null,
    description: '切换活跃账号',
    parameters: {
      type: 'object',
      properties: {
        id: { type: 'string', description: '目标账号 ID' }
      },
      required: ['id']
    }
  },
  {
    name: 'import_codex_config',
    tauriCmd: 'import_codex_config',
    level: 2,
    confirm: null,
    description: '从 Codex 配置文件导入账号',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'get_provider_presets',
    tauriCmd: 'get_provider_presets',
    level: 0,
    confirm: null,
    description: '获取供应商预设列表',
    parameters: { type: 'object', properties: {}, required: [] }
  },

  // D. 上游探测 (3个)
  {
    name: 'fetch_upstream_models',
    tauriCmd: 'fetch_upstream_models',
    level: 1,
    confirm: null,
    description: '从上游获取可用模型列表',
    parameters: {
      type: 'object',
      properties: {
        upstream: { type: 'string', description: '上游 API 地址' },
        api_key: { type: 'string', description: 'API 密钥' }
      },
      required: ['upstream', 'api_key']
    }
  },
  {
    name: 'fetch_balance',
    tauriCmd: 'fetch_balance',
    level: 1,
    confirm: null,
    description: '查询账号余额/额度',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'test_upstream_connectivity',
    tauriCmd: 'test_upstream_connectivity',
    level: 1,
    confirm: null,
    description: '测试上游连通性',
    parameters: { type: 'object', properties: {}, required: [] }
  },

  // E. 会话管理 (3个)
  {
    name: 'list_sessions',
    tauriCmd: 'list_sessions',
    level: 0,
    confirm: null,
    description: '列出历史会话',
    parameters: {
      type: 'object',
      properties: {
        limit: { type: 'number', description: '返回数量上限' }
      },
      required: []
    }
  },
  {
    name: 'delete_session',
    tauriCmd: 'delete_session',
    level: 3,
    confirm: '确定要删除该会话吗？',
    description: '删除指定会话',
    parameters: {
      type: 'object',
      properties: {
        id: { type: 'string', description: '会话 ID' }
      },
      required: ['id']
    }
  },
  {
    name: 'undo_delete_session',
    tauriCmd: 'undo_delete_session',
    level: 2,
    confirm: null,
    description: '撤销删除会话',
    parameters: {
      type: 'object',
      properties: {
        id: { type: 'string', description: '会话 ID' }
      },
      required: ['id']
    }
  },

  // F. 线程聚合 (7个)
  {
    name: 'get_threads_status',
    tauriCmd: 'get_threads_status',
    level: 0,
    confirm: null,
    description: '获取线程聚合状态',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'list_threads',
    tauriCmd: 'list_threads',
    level: 0,
    confirm: null,
    description: '列出所有线程',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'get_thread_content',
    tauriCmd: 'get_thread_content',
    level: 0,
    confirm: null,
    description: '获取指定线程的完整内容',
    parameters: {
      type: 'object',
      properties: {
        thread_id: { type: 'string', description: '线程 ID' }
      },
      required: ['thread_id']
    }
  },
  {
    name: 'migrate_threads',
    tauriCmd: 'migrate_threads',
    level: 3,
    confirm: '确定要迁移所有线程到 deecodex 吗？',
    description: '迁移所有线程到 deecodex 格式',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'restore_threads',
    tauriCmd: 'restore_threads',
    level: 3,
    confirm: '确定要还原线程迁移吗？',
    description: '还原线程迁移操作',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'calibrate_threads',
    tauriCmd: 'calibrate_threads',
    level: 2,
    confirm: null,
    description: '校准线程索引',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'delete_thread',
    tauriCmd: 'delete_thread',
    level: 3,
    confirm: '确定要永久删除该线程吗？',
    description: '永久删除指定线程',
    parameters: {
      type: 'object',
      properties: {
        thread_id: { type: 'string', description: '线程 ID' }
      },
      required: ['thread_id']
    }
  },

  // G. 请求历史 (3个)
  {
    name: 'list_request_history',
    tauriCmd: 'list_request_history',
    level: 0,
    confirm: null,
    description: '列出请求历史记录',
    parameters: {
      type: 'object',
      properties: {
        limit: { type: 'number', description: '返回数量上限' }
      },
      required: []
    }
  },
  {
    name: 'clear_request_history',
    tauriCmd: 'clear_request_history',
    level: 3,
    confirm: '确定要清空所有请求历史吗？',
    description: '清空所有请求历史记录',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'get_monthly_stats',
    tauriCmd: 'get_monthly_stats',
    level: 0,
    confirm: null,
    description: '获取月度统计信息',
    parameters: { type: 'object', properties: {}, required: [] }
  },

  // H. 插件管理 (11个)
  {
    name: 'list_plugins',
    tauriCmd: 'list_plugins',
    level: 0,
    confirm: null,
    description: '列出所有已安装插件',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'install_plugin',
    tauriCmd: 'install_plugin',
    level: 2,
    confirm: null,
    description: '安装插件',
    parameters: {
      type: 'object',
      properties: {
        plugin_path: { type: 'string', description: '插件文件路径' }
      },
      required: ['plugin_path']
    }
  },
  {
    name: 'uninstall_plugin',
    tauriCmd: 'uninstall_plugin',
    level: 3,
    confirm: '确定要卸载该插件吗？',
    description: '卸载指定插件',
    parameters: {
      type: 'object',
      properties: {
        plugin_id: { type: 'string', description: '插件 ID' }
      },
      required: ['plugin_id']
    }
  },
  {
    name: 'start_plugin',
    tauriCmd: 'start_plugin',
    level: 2,
    confirm: null,
    description: '启动指定插件',
    parameters: {
      type: 'object',
      properties: {
        plugin_id: { type: 'string', description: '插件 ID' }
      },
      required: ['plugin_id']
    }
  },
  {
    name: 'stop_plugin',
    tauriCmd: 'stop_plugin',
    level: 2,
    confirm: null,
    description: '停止指定插件',
    parameters: {
      type: 'object',
      properties: {
        plugin_id: { type: 'string', description: '插件 ID' }
      },
      required: ['plugin_id']
    }
  },
  {
    name: 'update_plugin_config',
    tauriCmd: 'update_plugin_config',
    level: 2,
    confirm: null,
    description: '更新插件配置',
    parameters: {
      type: 'object',
      properties: {
        plugin_id: { type: 'string', description: '插件 ID' },
        config_json: { type: 'string', description: 'JSON 格式的插件配置' }
      },
      required: ['plugin_id', 'config_json']
    }
  },
  {
    name: 'get_plugin_qrcode',
    tauriCmd: 'get_plugin_qrcode',
    level: 1,
    confirm: null,
    description: '获取插件扫码登录二维码',
    parameters: {
      type: 'object',
      properties: {
        plugin_id: { type: 'string', description: '插件 ID' }
      },
      required: ['plugin_id']
    }
  },
  {
    name: 'plugin_login_cancel',
    tauriCmd: 'plugin_login_cancel',
    level: 2,
    confirm: null,
    description: '取消插件扫码登录',
    parameters: {
      type: 'object',
      properties: {
        plugin_id: { type: 'string', description: '插件 ID' }
      },
      required: ['plugin_id']
    }
  },
  {
    name: 'query_plugin_status',
    tauriCmd: 'query_plugin_status',
    level: 0,
    confirm: null,
    description: '查询插件运行状态',
    parameters: {
      type: 'object',
      properties: {
        plugin_id: { type: 'string', description: '插件 ID' }
      },
      required: ['plugin_id']
    }
  },
  {
    name: 'start_plugin_account',
    tauriCmd: 'start_plugin_account',
    level: 2,
    confirm: null,
    description: '启动插件账号服务',
    parameters: {
      type: 'object',
      properties: {
        plugin_id: { type: 'string', description: '插件 ID' }
      },
      required: ['plugin_id']
    }
  },
  {
    name: 'stop_plugin_account',
    tauriCmd: 'stop_plugin_account',
    level: 2,
    confirm: null,
    description: '停止插件账号服务',
    parameters: {
      type: 'object',
      properties: {
        plugin_id: { type: 'string', description: '插件 ID' }
      },
      required: ['plugin_id']
    }
  },

  // I. 日志和调试 (4个)
  {
    name: 'get_logs',
    tauriCmd: 'get_logs',
    level: 0,
    confirm: null,
    description: '获取最近日志',
    parameters: {
      type: 'object',
      properties: {
        limit: { type: 'number', description: '返回日志行数' }
      },
      required: []
    }
  },
  {
    name: 'clear_logs',
    tauriCmd: 'clear_logs',
    level: 3,
    confirm: '确定要清空所有日志吗？',
    description: '清空所有日志',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'debug_gui_state',
    tauriCmd: 'debug_gui_state',
    level: 0,
    confirm: null,
    description: '获取 GUI 调试状态信息',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'browse_file',
    tauriCmd: 'browse_file',
    level: 0,
    confirm: null,
    description: '浏览文件内容',
    parameters: {
      type: 'object',
      properties: {
        path: { type: 'string', description: '文件路径' }
      },
      required: ['path']
    }
  },

  // J. 升级管理 (2个)
  {
    name: 'check_upgrade',
    tauriCmd: 'check_upgrade',
    level: 0,
    confirm: null,
    description: '检查是否有可用更新',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'run_upgrade',
    tauriCmd: 'run_upgrade',
    level: 3,
    confirm: '确定要执行一键升级吗？升级期间服务将不可用。',
    description: '执行一键升级',
    parameters: { type: 'object', properties: {}, required: [] }
  },

  // K. DEX 专属 (8个)
  {
    name: 'read_file',
    tauriCmd: 'dex_read_file',
    level: 0,
    confirm: null,
    description: '读取文件内容（支持行数限制）',
    parameters: {
      type: 'object',
      properties: {
        path: { type: 'string', description: '文件路径' },
        max_lines: { type: 'number', description: '最大读取行数' }
      },
      required: ['path']
    }
  },
  {
    name: 'list_directory',
    tauriCmd: 'dex_list_directory',
    level: 0,
    confirm: null,
    description: '列出目录内容',
    parameters: {
      type: 'object',
      properties: {
        path: { type: 'string', description: '目录路径' }
      },
      required: ['path']
    }
  },
  {
    name: 'detect_processes',
    tauriCmd: 'dex_detect_processes',
    level: 0,
    confirm: null,
    description: '检测系统进程信息',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'detect_ports',
    tauriCmd: 'dex_detect_ports',
    level: 0,
    confirm: null,
    description: '检测网络端口使用情况',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'get_env_info',
    tauriCmd: 'dex_get_env_info',
    level: 0,
    confirm: null,
    description: '获取系统环境信息',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'execute_shell',
    tauriCmd: 'dex_execute_shell',
    level: 2,
    confirm: null,
    description: '执行 Shell 命令',
    parameters: {
      type: 'object',
      properties: {
        command: { type: 'string', description: '要执行的命令' },
        timeout_secs: { type: 'number', description: '超时秒数' }
      },
      required: ['command']
    }
  },
  {
    name: 'search_logs',
    tauriCmd: 'dex_search_logs',
    level: 0,
    confirm: null,
    description: '搜索日志内容',
    parameters: {
      type: 'object',
      properties: {
        query: { type: 'string', description: '搜索关键词' },
        context_lines: { type: 'number', description: '上下文行数' }
      },
      required: ['query']
    }
  },
  {
    name: 'get_codex_config_raw',
    tauriCmd: 'dex_get_codex_config_raw',
    level: 0,
    confirm: null,
    description: '获取 Codex 原始配置文件内容',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'health_summary',
    tauriCmd: 'dex_health_summary',
    level: 0,
    confirm: null,
    description: '一键健康概览：服务状态+账号状态+Codex安装+最近错误数',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'analyze_requests',
    tauriCmd: 'dex_analyze_requests',
    level: 0,
    confirm: null,
    description: '分析最近请求：成功率、延迟P50/P99、Token消耗、模型分布',
    parameters: { type: 'object', properties: {}, required: [] }
  },

  // L. 系统运维 (10个)
  {
    name: 'config_backup', tauriCmd: 'dex_config_backup', level: 0,
    confirm: null,
    description: '备份/恢复/列出配置文件',
    parameters: { type: 'object', properties: { action: { type: 'string', description: 'backup|restore|list' } }, required: ['action'] }
  },
  {
    name: 'config_diff', tauriCmd: 'dex_config_diff', level: 0,
    confirm: null,
    description: '对比当前配置与历史版本的差异',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'token_cost', tauriCmd: 'dex_token_cost', level: 0,
    confirm: null,
    description: '分析 Token 消耗与成本',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'speed_test', tauriCmd: 'dex_speed_test', level: 0,
    confirm: null,
    description: '测试 API 响应速度与延迟',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'thread_cleanup', tauriCmd: 'dex_thread_cleanup', level: 0,
    confirm: null,
    description: '清理无用线程数据',
    parameters: { type: 'object', properties: { dry_run: { type: 'boolean', description: '是否为演练模式（不实际删除）' } }, required: [] }
  },
  {
    name: 'auto_tune', tauriCmd: 'dex_auto_tune', level: 0,
    confirm: null,
    description: '自动调优 deecodex 配置参数',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'claude_mcp_check', tauriCmd: 'dex_claude_mcp_check', level: 0,
    confirm: null,
    description: '检查 Claude Code MCP 集成状态',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'network_topology', tauriCmd: 'dex_network_topology', level: 0,
    confirm: null,
    description: '分析网络拓扑与连通性',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'ssl_check', tauriCmd: 'dex_ssl_check', level: 0,
    confirm: null,
    description: '检查 SSL/TLS 证书状态',
    parameters: { type: 'object', properties: {}, required: [] }
  },
  {
    name: 'export_report', tauriCmd: 'dex_export_report', level: 0,
    confirm: null,
    description: '导出系统诊断与健康报告',
    parameters: { type: 'object', properties: {}, required: [] }
  }
];

// ── System Prompt ──
var DEX_SYSTEM_PROMPT = [
  '你是 deecodex 运维专家 Agent，运行在 deecodex GUI 内置的 DEX助手 面板中。',
  '你通过调用后端工具获取实时数据、诊断问题、管理配置、操作服务和插件。',
  '',
  '## 核心原则',
  '1. **工具优先**：始终优先调用工具获取实时数据，不要猜测或假设系统状态。',
  '2. **安全级别**：L0/L1 工具直接执行；L2 工具自动执行并报告结果；L3 操作必须先征得用户确认。',
  '3. **验证结果**：每次操作后检查返回结果，确认操作是否成功。',
  '4. **失败不重复**：工具失败后分析原因换方案，不要用相同参数重复调用同一个失败的工具。',
  '5. **先说结论**：回复以结论开头，一行说清结果，细节可折叠。不用啰嗦重复用户已知信息。',
  '',
  '## 工具概览（共 71 个）',
  '- 服务管理 (5): get_service_status, start_service, stop_service(L3), launch_codex_cdp, stop_codex_cdp(L3)',
  '- 配置管理 (5): get_config, save_config, validate_config, run_diagnostics, run_full_diagnostics',
  '- 账号管理 (8): list_accounts, get_active_account, add_account, update_account, delete_account(L3), switch_account, import_codex_config, get_provider_presets',
  '- 上游探测 (3): fetch_upstream_models, fetch_balance, test_upstream_connectivity',
  '- 会话管理 (3): list_sessions, delete_session(L3), undo_delete_session',
  '- 线程聚合 (7): get_threads_status, list_threads, get_thread_content, migrate_threads(L3), restore_threads(L3), calibrate_threads, delete_thread(L3)',
  '- 请求历史 (3): list_request_history, clear_request_history(L3), get_monthly_stats',
  '- 插件管理 (11): list_plugins, install_plugin, uninstall_plugin(L3), start_plugin, stop_plugin, update_plugin_config, get_plugin_qrcode, plugin_login_cancel, query_plugin_status, start_plugin_account, stop_plugin_account',
  '- 日志调试 (4): get_logs, clear_logs(L3), debug_gui_state, browse_file',
  '- 升级管理 (2): check_upgrade, run_upgrade(L3)',
  '- DEX专属 (10): health_summary, analyze_requests, read_file, list_directory, detect_processes, detect_ports, get_env_info, execute_shell, search_logs, get_codex_config_raw',
  '- 系统运维 (10): config_backup, config_diff, token_cost, speed_test, thread_cleanup, auto_tune, claude_mcp_check, network_topology, ssl_check, export_report',
  '',
  '## 诊断知识库',
  '',
  '### Codex CLI 配置',
  '- 配置文件: ~/.codex/config.toml',
  '- 关键字段: model（当前使用模型）、model_provider（必须设为 custom 才能使用自定义配置）',
  '- [model_providers.custom] 节: base_url（API 地址）、name（显示名）、requires_openai_auth（是否需要 OpenAI 认证）、wire_api（协议类型: responses 或 chat_completions）',
  '- 常见故障:',
  '  - base_url 末尾多余的 / 导致 404 错误',
  '  - 端口号与实际服务端口不匹配（deecodex 默认 4446）',
  '  - model_provider 未设为 custom，导致使用默认 OpenAI 配置',
  '  - requires_openai_auth 设置不正确导致认证失败',
  '- 注入机制: codex_auto_inject 在启动 Codex 时自动修改 config.toml；codex_persistent_inject 保持注入状态',
  '',
  '### Claude Code 集成',
  '- MCP 配置文件: ~/.claude/mcp.json',
  '- API 请求地址: http://127.0.0.1:4446/v1',
  '- deecodex 作为 MCP 服务器提供工具调用能力',
  '',
  '### 模型映射',
  '- 键名大小写敏感，Codex 模型名列表: gpt-5.5, gpt-5.4, gpt-5.4-mini, gpt-5.3-codex, gpt-5, codex-auto-review',
  '- model_map 格式: { "Codex模型名": "上游模型名", ... }',
  '- 上游模型名需与实际 API 返回的模型名完全一致',
  '',
  '### 协议兼容性',
  '- deecodex 在中间层翻译请求：上游 Chat Completions API → 对外 Responses API',
  '- wire_api 指定上游协议类型',
  '- translate_enabled 控制是否启用协议翻译（DeepSeek 等仅支持 Chat Completions 的供应商需保持开启）',
  '- DeepSeek 的 reasoning_content 需要在翻译过程中保留并正确传递',
  '',
  '### deecodex 自身',
  '- 默认端口: 4446，PID 文件: data_dir/deecodex.pid',
  '- 15 项诊断：服务状态、端口占用、配置完整性、网络连通性、上游可用性等',
  '- 日志: data_dir/deecodex.log，用 get_logs/search_logs 读取',
  '',
  '## 自主修复模式',
  '当用户说「修复」「自动修」「帮我修」等时，进入自主修复模式：',
  '1. run_full_diagnostics 获取全貌',
  '2. 对每个 fail 项分析原因，按优先级排序',
  '3. L2 操作直接执行修复（start_service / save_config / switch_account / calibrate_threads）',
  '4. L3 操作先简要说明 + 获取确认',
  '5. 修复后重新诊断验证',
  '6. 最后用表格报告「修复前 → 修复后」',
  '',
  '## 主动建议',
  '完成诊断或分析后，主动给出建议：',
  '- 如有 fail 项 → 建议修复方案并询问是否执行',
  '- 如有 warn 项 → 列出风险和建议',
  '- 如一切正常 → 一句话确认 + 可选优化建议',
  '- 用 analyze_requests 分析请求模式，提示异常（成功率<95%、延迟突增）',
  '',
  '## 常见问题修复速查',
  '| 问题 | 诊断信号 | 修复操作 |',
  '|------|---------|---------|',
  '| 服务未启动 | 诊断 fail:"服务未运行" | start_service |',
  '| 端口冲突 | 诊断 fail:"端口被占用" | detect_ports → save_config 改端口 → start_service |',
  '| Codex 路由丢失 | 诊断 fail:"未路由到 deecodex" | save_config(codex_auto_inject:true) |',
  '| 账号连通失败 | 诊断 fail:"上游不可达" | test_upstream_connectivity → 如果另一个账号通就 switch_account |',
  '| 线程冲突 | 诊断 warn:"差异" | calibrate_threads |',
  '| 上游模型不足 | fetch_upstream_models 返回空 | 检查 upstream/api_key 是否正确 |',
  '| Codex 未安装 | 诊断 fail:"Codex 未安装" | 提示用户安装 Codex CLI |',
  '| 余额不足 | fetch_balance 余额低 | 提示用户充值或切换账号 |',
  '| 注入失效 | 诊断 fail:"注入" | save_config 触发 codex_auto_inject；如不行，提示重启 Codex |',
  '| 配置不一致 | 诊断 warn:"不一致" | get_config 获取当前配置 → save_config 修正 |',
  '',
  '## 回复风格（务必遵守）',
      '- 使用中文回复',
      '- **极度精简**：每句话都有信息量，不寒暄、不废话。结果用一行说清楚',
      '- 先说结论，细节可省略。表格、列表优先于大段文字',
      '- L2 操作直接执行并一句话报告结果，不写长篇说明',
      '- L3 操作说明原因后等待确认即可',
      '- 不重复用户已知信息，不啰嗦解释显而易见的操作',
].join('\n');

// ── Agent 核心对象 ──
window.dexAgent = {
  messages: [],
  isProcessing: false,
  roundCount: 0,
  maxRounds: 30,
  maxHistorySize: 50,
  selectedModel: 'auto',

  init: function () {
    this.messages = [{ role: 'system', content: DEX_SYSTEM_PROMPT }];
    this.isProcessing = false;
    this.roundCount = 0;
    this._lastErrorKey = null;
    this._toolCache = {};
    this.saveHistory();
  },

  clear: function () {
    this.messages = [{ role: 'system', content: DEX_SYSTEM_PROMPT }];
    this.isProcessing = false;
    this.roundCount = 0;
    this._lastErrorKey = null;
    this._toolCache = {};
    this.saveHistory();
  },

  compressContext: function () {
    if (this.messages.length <= 40) return;
    var systemMsg = this.messages[0];
    var recentMsgs = this.messages.slice(this.messages.length - 20);
    var summaryParts = [];
    for (var i = 1; i < this.messages.length - 20; i++) {
      var m = this.messages[i];
      if (m.role === 'user') summaryParts.push('用户: ' + (m.content || '').substring(0, 80));
      else if (m.role === 'assistant' && m.content) summaryParts.push('助手: ' + m.content.substring(0, 80));
      else if (m.role === 'tool') summaryParts.push('工具结果');
    }
    var summary = summaryParts.slice(0, 20).join('; ');
    this.messages = [systemMsg, { role: 'system', content: '[对话摘要] 之前讨论了: ' + summary }].concat(recentMsgs);
    console.log('[dexAgent] 上下文已压缩: ' + (this.messages.length) + ' 条消息');
  },

  _canStream: function () {
    try {
      return typeof window.__TAURI__ !== 'undefined'
        && window.__TAURI__
        && window.__TAURI__.event
        && typeof window.__TAURI__.event.listen === 'function';
    } catch (e) { return false; }
  },

  sendToLLM: async function (messages, tools) {
    try { if (this._canStream()) return await this.sendToLLMStream(messages, tools); }
    catch (e) { console.warn('[dexAgent] 流式不可用，降级为非流式:', e); }
    try {
      var result = await DeeCodexTauri.invoke('dex_chat', {
        messages: messages, tools: tools, stream: false,
        model: (this.selectedModel && this.selectedModel !== 'auto') ? this.selectedModel : null
      });
      return result;
    } catch (e) { console.error('[dexAgent] LLM 调用失败:', e); throw e; }
  },

  sendToLLMStream: async function (messages, tools) {
    var self = this;
    var fullContent = '';
    var fullReasoning = '';
    var finishReason = '';
    var toolCalls = null;
    _dexLastAssistantEl = null;
    var resolveStream, rejectStream;
    var streamPromise = new Promise(function (resolve, reject) {
      resolveStream = resolve; rejectStream = reject;
    });

    var unlisten = await window.__TAURI__.event.listen('dex-chat-chunk', function (event) {
      try {
        if (self._aborted) { unlisten(); rejectStream(new Error('用户中止')); return; }
        var payload = event.payload;
        if (payload.done) {
          unlisten();
          dexHideThinking();
          _dexLastAssistantEl = null;
          self._streamed = true;
          var finalMsg = { role: 'assistant', content: fullContent || null };
          if (fullReasoning) finalMsg.reasoning_content = fullReasoning;
          if (toolCalls) finalMsg.tool_calls = toolCalls;
          resolveStream({ choices: [{ message: finalMsg, finish_reason: finishReason || 'stop' }] });
          return;
        }
        var chunk = payload.chunk;
        if (!chunk || !chunk.choices || !chunk.choices.length) return;
        var delta = chunk.choices[0].delta;
        if (!delta) return;
        if (delta.reasoning_content) { fullReasoning += delta.reasoning_content; dexUpdateLastAssistant(fullContent, fullReasoning); }
        if (delta.content) { fullContent += delta.content; dexUpdateLastAssistant(fullContent, fullReasoning); }
        if (delta.tool_calls) {
          if (!toolCalls) toolCalls = [];
          for (var i = 0; i < delta.tool_calls.length; i++) {
            var dtc = delta.tool_calls[i];
            var idx = dtc.index || 0;
            if (!toolCalls[idx]) toolCalls[idx] = { id: dtc.id || '', type: 'function', function: { name: '', arguments: '' } };
            if (dtc.id) toolCalls[idx].id = dtc.id;
            if (dtc.function) {
              if (dtc.function.name) toolCalls[idx].function.name += dtc.function.name;
              if (dtc.function.arguments) toolCalls[idx].function.arguments += dtc.function.arguments;
            }
          }
        }
        if (chunk.choices[0].finish_reason) finishReason = chunk.choices[0].finish_reason;
      } catch (e) { console.error('[dexAgent] 流式处理异常:', e); }
    });

    DeeCodexTauri.invoke('dex_chat', {
      messages: messages, tools: tools, stream: true,
      model: (self.selectedModel && self.selectedModel !== 'auto') ? self.selectedModel : null
    }).catch(function (e) { unlisten(); rejectStream(e); });

    return streamPromise;
  },

  listenStream: async function (messages, tools) { return await this.sendToLLMStream(messages, tools); },

  abort: function () { this._aborted = true; this.isProcessing = false; },

  run: async function (userMessage) {
    if (this.isProcessing) { showToast('Agent 正在处理中，请等待', 'warn'); return; }
    this.isProcessing = true;
    this._aborted = false;
    this._streamed = false;
    this._toolCache = {};
    this.roundCount = 0;
    this.messages.push({ role: 'user', content: userMessage });
    this.compressContext();
    this.saveHistory();
    dexShowStopButton();

    while (this.roundCount < this.maxRounds && !this._aborted) {
      this.roundCount++;
      this.compressContext(); // 每轮检查，避免 tool_calls 累积撑爆上下文
      try {
        var response = await this.sendToLLM(this.messages, this.buildOpenAITools());
        var choice = response.choices && response.choices[0];
        if (!choice) { dexAppendMessage('system', 'LLM 返回了空的响应，请重试'); break; }
        var msg = choice.message;

        if (msg.tool_calls && msg.tool_calls.length > 0) {
          this.messages.push(msg);
          if (msg.content && !this._streamed) {
            dexAppendMessage('assistant', msg.content);
          }
          for (var i = 0; i < msg.tool_calls.length; i++) {
            var tc = msg.tool_calls[i];
            var toolResult = await this.executeTool(tc);
            // 发回 LLM 时精简：诊断/配置/日志等大结果用摘要代替
            var compact = toolResult;
            if (toolResult && toolResult.success) {
              var fn = tc.function.name;
              if (fn === 'get_config') compact = { success: true, summary: '已获取完整配置，含 ' + Object.keys(toolResult.data||{}).length + ' 项' };
              else if (fn === 'get_logs') compact = { success: true, summary: (Array.isArray(toolResult.data)?toolResult.data.length:'?') + ' 行日志' };
              else if (fn === 'list_threads' || fn === 'list_request_history' || fn === 'list_sessions' || fn === 'list_plugins' || fn === 'list_accounts')
                compact = { success: true, count: Array.isArray(toolResult.data)?toolResult.data.length:'?', summary: '列表已获取' };
            }
            compact = dexMaskApiKey(compact);
            var resultStr = JSON.stringify(compact);
            if (resultStr.length > 2000) resultStr = resultStr.substring(0, 2000) + '…';
            this.messages.push({ role: 'tool', tool_call_id: tc.id, content: resultStr });
          }
          this.saveHistory();
          continue;
        }

        if (msg.content) {
          if (!this._streamed) dexAppendMessage('assistant', msg.content);
          this.messages.push(msg);
          this.saveHistory();
        }
        break;
      } catch (e) {
        dexAppendMessage('system', '请求失败: ' + (e.message || e));
        showToast('请求失败: ' + (e.message || e), 'error');
        break;
      }
    }
    if (this._aborted)
      dexAppendMessage('system', '已停止生成');
    else if (this.roundCount >= this.maxRounds && this.isProcessing)
      dexAppendMessage('system', '已达到最大对话轮数（' + this.maxRounds + '），请开启新对话继续。');
    this.isProcessing = false;
    this._aborted = false;
    this.roundCount = 0;
    dexHideStopButton();
    dexHideThinking();
    _dexLastAssistantEl = null;
  },

  executeTool: async function (toolCall) {
    var fnName = toolCall.function.name;
    var fnArgs = {};
    try { fnArgs = JSON.parse(toolCall.function.arguments || '{}'); }
    catch (e) { return { error: '参数解析失败: ' + e.message }; }

    var toolDef = null;
    for (var i = 0; i < DEX_TOOLS.length; i++) { if (DEX_TOOLS[i].name === fnName) { toolDef = DEX_TOOLS[i]; break; } }
    if (!toolDef) return { error: '未知工具: ' + fnName };

    // 工具缓存检查：同一轮中相同工具+相同参数只执行一次
    var cacheKey = fnName + '|' + JSON.stringify(fnArgs);
    if (this._toolCache[cacheKey] !== undefined) {
      var cached = this._toolCache[cacheKey];
      console.log('[dexAgent] 缓存命中:', fnName);
      if (cached.success) {
        dexAppendMessage('tool-result', fnName, { result: cached, history: false });
        return cached;
      }
      return cached;
    }

    var statusEl = dexAppendMessage('tool-start', fnName, { args: fnArgs });

    if (toolDef.level === 3 && toolDef.confirm) {
      var confirmed = await dexShowInlineConfirm(fnName, toolDef.confirm, toolCall.id);
      if (!confirmed) {
        dexUpdateMessage(statusEl, 'tool-error', fnName + ': 用户取消了操作', { error: '用户取消了 L3 操作' });
        return { error: '用户取消了 L3 操作: ' + fnName };
      }
    }

    // 诊断/校验命令自动注入当前配置
    if (['run_full_diagnostics','run_diagnostics','validate_config'].indexOf(toolDef.tauriCmd) >= 0) {
      if (!fnArgs.config) {
        try { var cfg = await DeeCodexTauri.invoke('get_config'); fnArgs.config = cfg; }
        catch (e) { /* 降级：无 config 也能跑部分检查 */ }
      }
    }

    // 错误去重：同一工具同参数连续失败不重试
    var errKey = fnName + '|' + JSON.stringify(fnArgs);
    if (this._lastErrorKey === errKey) {
      dexUpdateMessage(statusEl, 'tool-error', fnName + ': 跳过（重复失败）', { error: '与上次相同错误，不再重试' });
      return { error: '工具重复失败，已跳过: ' + fnName };
    }

    var lastError = null;
    for (var retry = 0; retry < 3; retry++) {
      try {
        var result = await DeeCodexTauri.invoke(toolDef.tauriCmd, fnArgs);
        this._lastErrorKey = null;
        this._toolCache[cacheKey] = { success: true, data: result };
        dexUpdateMessage(statusEl, 'tool-result', fnName, { result: result, success: true });
        // 影响全局状态的工具执行后刷新状态栏 + 通知其他面板
        dexAfterMutate(fnName);
        return { success: true, data: result };
      } catch (e) {
        lastError = e;
        var isTransient = /timeout|timed.?out|network|connection|ECONN|abort/i.test(String(e.message || e || ''));
        if (!isTransient) break;
        console.warn('[dexAgent] 瞬态错误，重试 ' + (retry + 1) + '/3:', fnName, e);
        if (retry < 2) dexUpdateMessage(statusEl, 'tool-start', fnName + ': 第' + (retry + 1) + '次尝试失败，正在重试…', { args: fnArgs });
      }
    }
    this._lastErrorKey = errKey;
    var finalErr = (lastError && (lastError.message || lastError)) ? String(lastError.message || lastError) : '未知错误';
    this._toolCache[cacheKey] = { error: '工具执行失败: ' + finalErr };
    dexUpdateMessage(statusEl, 'tool-error', fnName + ': 失败', { error: finalErr });
    return { error: '工具执行失败: ' + finalErr };
  },

  buildOpenAITools: function () {
    var tools = [];
    for (var i = 0; i < DEX_TOOLS.length; i++) {
      var t = DEX_TOOLS[i];
      tools.push({ type: 'function', function: { name: t.name, description: t.description, parameters: t.parameters } });
    }
    return tools;
  },

  handleStream: async function () {},

  saveHistory: function () {
    try {
      var history = [];
      for (var i = 0; i < this.messages.length; i++)
        if (this.messages[i].role !== 'system') history.push(this.messages[i]);
      if (history.length > this.maxHistorySize) history = history.slice(history.length - this.maxHistorySize);
      window.deeStorage.setItem('dex_chat_history', JSON.stringify(history));
    } catch (e) { console.warn('[dexAgent] 保存历史失败:', e); }
  },

  loadHistory: function () {
    try {
      var raw = window.deeStorage.getItem('dex_chat_history');
      if (raw) {
        var history = JSON.parse(raw);
        this.messages = [{ role: 'system', content: DEX_SYSTEM_PROMPT }];
        for (var i = 0; i < history.length; i++) this.messages.push(history[i]);
        return history;
      }
    } catch (e) { console.warn('[dexAgent] 加载历史失败:', e); }
    return [];
  }
};

// ── Chat UI 渲染 ──
function renderDexAssistant() {
  window._dexInitialized = false;
  setTimeout(function () {
    if (window._dexInitialized) return;
    window._dexInitialized = true;
    if (window.dexAgent.messages.length === 0) window.dexAgent.init();

    dexLoadModels();

    var history = window.dexAgent.loadHistory();
    if (history && history.length > 0) {
      var container = document.getElementById('dexMessages');
      if (!container) return;
      container.innerHTML = '';
      var lastToolNames = {};
      for (var i = 0; i < history.length; i++) {
        var msg = history[i];
        if (msg.role === 'user') {
          dexAppendMessage('user', msg.content);
        } else if (msg.role === 'assistant') {
          if (msg.tool_calls) {
            for (var j = 0; j < msg.tool_calls.length; j++) lastToolNames[msg.tool_calls[j].id] = msg.tool_calls[j].function.name;
            if (msg.content) dexAppendMessage('assistant', msg.content);
            for (var k = 0; k < msg.tool_calls.length; k++)
              dexAppendMessage('tool-start', msg.tool_calls[k].function.name, { args: msg.tool_calls[k].function.arguments, history: true });
          } else {
            dexAppendMessage('assistant', msg.content);
          }
        } else if (msg.role === 'tool') {
          var toolName = lastToolNames[msg.tool_call_id] || '工具结果';
          try {
            var parsed = JSON.parse(msg.content);
            if (parsed.error) dexAppendMessage('tool-error', toolName + ': ' + parsed.error, { error: parsed.error, history: true });
            else dexAppendMessage('tool-result', toolName, { result: parsed, history: true });
          } catch (e) { dexAppendMessage('tool-result', toolName, { result: msg.content, history: true }); }
        }
      }
      dexScrollToBottom();
    }

    // 输入框事件：Enter 发送、Tab 补全斜杠指令
    var input = document.getElementById('dexInput');
    if (input) input.addEventListener('keydown', function (e) {
      if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); dexSendMessage(); return; }
      if (e.key === 'Tab') {
        var val = input.value.trim();
        var expanded = dexExpandSlashCommand(val);
        if (expanded !== val) { e.preventDefault(); input.value = expanded; dexUpdateTokenCount(); }
      }
    });

    // 输入时显示工具预览
    if (input) input.addEventListener('input', function () {
      var val = input.value.trim();
      var preview = dexPreviewTools(val);
      var existing = document.getElementById('dexToolPreview');
      if (!existing) return;
      if (preview) {
        existing.textContent = preview;
        existing.style.display = '';
      } else {
        existing.style.display = 'none';
      }
    });

    // 搜索输入事件
    var searchInput = document.getElementById('dexSearchInput');
    if (searchInput) {
      searchInput.addEventListener('input', function () { dexPerformSearch(); });
      searchInput.addEventListener('keydown', function (e) {
        if (e.key === 'Enter') { e.preventDefault(); dexNavigateSearch(1); }
        if (e.key === 'Escape') { dexCloseSearch(); }
      });
    }

    // 状态栏初始刷新
    dexRefreshStatus();
    if (!window._dexStatusTimer) {
      window._dexStatusTimer = setInterval(dexRefreshStatus, 30000);
    }

    // Token 计数初始
    dexUpdateTokenCount();

    // 快捷键
    dexBindShortcuts();
  }, 0);

  return '<div class="dex-chat-panel"><div class="dex-chat-header"><div><h3>DEX助手 — AI工具运维专家</h3>'
    + '<div class="dex-status-bar" id="dexStatusBar"><span class="dex-status-dot" id="dexStatusDot"></span> <span id="dexStatusText">加载中...</span></div></div><div class="dex-header-actions">'
    + '<div class="dex-model-drop" id="dexModelDrop"><button class="dex-model-btn" id="dexModelBtn" onclick="dexToggleModelMenu(event)">模型 ▾</button><div class="dex-model-menu" id="dexModelMenu" style="display:none"></div></div>'
    + '<button class="btn btn-ghost btn-sm" onclick="dexExportChat()" title="导出对话">导出</button>'
    + '<button class="btn btn-ghost btn-sm" id="dexSearchBtn" onclick="dexToggleSearch()" title="搜索对话">搜索</button>'
    + '<button class="btn btn-ghost btn-sm" onclick="dexNewChat()">+新对话</button>'
    + '<button class="btn btn-ghost btn-sm" onclick="dexClearChat()">清空</button></div></div>'
    + '<div class="dex-search-bar" id="dexSearchBar" style="display:none"><input id="dexSearchInput" placeholder="搜索对话..." /><span class="dex-search-count" id="dexSearchCount"></span><button class="btn btn-ghost btn-sm" onclick="dexCloseSearch()">关闭</button></div>'
    + '<div class="dex-chat-messages" id="dexMessages">' + dexWelcomeHTML() + '</div>'
    + '<div class="dex-input-area" id="dexInputAreaWrap">'
    + '<div id="dexToolPreview" class="dex-tool-preview" style="display:none"></div>'
    + '<div class="dex-input-row">'
    + '<textarea id="dexInput" placeholder="输入消息…（Enter 发送，Shift+Enter 换行 / /diag /fix 快捷指令）" rows="2"></textarea>'
    + '<button class="btn btn-primary" id="dexSendBtn" onclick="dexSendMessage()">发送</button>'
    + '<button class="btn btn-danger" id="dexStopBtn" onclick="dexStopAgent()" style="display:none">停止</button>'
    + '</div>'
    + '<div class="dex-input-foot"><span class="dex-token-count" id="dexTokenCount">~0 tokens</span></div>'
    + '</div></div>';
}

function dexWelcomeHTML() {
  return '<div class="dex-msg dex-msg-assistant"><div class="dex-bubble"><div class="dex-bubble-text">'
    + '<p>DEX助手 就绪。直接描述问题，或快速操作：</p></div>'
    + '<div class="dex-quick-actions">'
    + '<button class="btn btn-sm btn-primary" onclick="dexQuickAction(\'运行完整诊断，自动修复所有发现的问题\')">一键修复</button>'
    + '<button class="btn btn-sm btn-ghost" onclick="dexQuickAction(\'运行完整诊断，分析结果\')">诊断</button>'
    + '<button class="btn btn-sm btn-ghost" onclick="dexQuickAction(\'服务状态\')">服务状态</button>'
    + '<button class="btn btn-sm btn-ghost" onclick="dexQuickAction(\'测连通性\')">测连通性</button>'
    + '<button class="btn btn-sm btn-ghost" onclick="dexQuickAction(\'查余额\')">查余额</button>'
    + '<button class="btn btn-sm btn-ghost" onclick="dexQuickAction(\'读日志，检查异常\')">读日志</button></div></div></div>';
}

// ── UI 交互 ──
function dexAppendMessage(type, content, meta) {
  var container = document.getElementById('dexMessages');
  if (!container) return null;
  // 新消息到达时自动收起旧详情
  if (type === 'assistant' || type === 'user' || type === 'tool-start') {
    var details = container.querySelectorAll('details[open]');
    for (var d = 0; d < details.length; d++) details[d].open = false;
  }
  var el = document.createElement('div');
  el.className = 'dex-msg dex-msg-' + type;
  var isHistory = meta && meta.history;

  switch (type) {
    case 'user':
      el.innerHTML = '<div class="dex-bubble dex-bubble-user"><div class="dex-bubble-text">' + esc(content) + '</div></div>';
      break;
    case 'assistant':
      var modelTag = (meta && meta.model && meta.model !== 'auto') ? '<span class="dex-model-tag">' + esc(meta.model) + '</span>' : '';
      el.innerHTML = '<div class="dex-bubble dex-bubble-assistant">'
        + modelTag
        + '<div class="dex-reasoning-wrap" style="display:none"><details class="dex-reasoning"><summary>思考过程</summary><div class="dex-reasoning-content"></div></details></div>'
        + '<div class="dex-bubble-text">' + dexRenderMarkdown(content) + '</div></div>';
      break;
    case 'system':
      el.innerHTML = '<div class="dex-system-msg">' + esc(content) + '</div>';
      break;
    case 'tool-start':
      el.innerHTML = '<div class="dex-tool-msg dex-tool-start">'
        + (isHistory ? '<span class="dex-tool-icon">🔧</span>' : '<span class="dex-spinner"></span>')
        + '<span class="dex-tool-name">' + esc(content) + '</span>'
        + (meta && meta.args && Object.keys(meta.args).length > 0 ? ' <span class="dex-tool-args">' + esc(JSON.stringify(meta.args)) + '</span>' : '')
        + '</div>';
      break;
    case 'tool-result':
      var summary = dexToolSummary(content, meta && meta.result ? meta.result : null);
      var rawData = meta && meta.result ? (meta.result.data !== undefined ? meta.result.data : meta.result) : null;
      var detailText = rawData ? dexFormatResultText(content, rawData) : '';
      el.innerHTML = '<div class="dex-tool-msg dex-tool-result">'
        + '<span class="dex-tool-icon">✅</span>'
        + '<span class="dex-tool-name">' + esc(content) + '</span>'
        + '<span class="dex-tool-summary">' + esc(summary) + '</span>'
        + (detailText ? '<details class="dex-tool-details"><summary>详情</summary><pre>' + esc(detailText) + '</pre></details>' : '')
        + '</div>';
      break;
    case 'tool-error':
      var errMsg = content || '未知错误';
      var errDetail = (meta && meta.error) ? String(meta.error) : '';
      el.innerHTML = '<div class="dex-tool-msg dex-tool-error">'
        + '<span class="dex-tool-icon">❌</span>'
        + '<span class="dex-tool-name">' + esc(errMsg) + '</span>'
        + (errDetail ? '<details class="dex-tool-details dex-tool-error-details"><summary>错误详情</summary><pre>' + esc(errDetail) + '</pre></details>' : '')
        + '</div>';
      break;
    default:
      el.textContent = content;
  }
  container.appendChild(el);
  dexScrollToBottom();
  return el;
}

function dexUpdateMessage(el, type, content, meta) {
  if (!el) return;
  switch (type) {
    case 'tool-result':
      var summary = dexToolSummary(content, meta && meta.result ? meta.result : null);
      var rawData = meta && meta.result ? (meta.result.data !== undefined ? meta.result.data : meta.result) : null;
      var detailText = rawData ? dexFormatResultText(content, rawData) : '';
      el.className = 'dex-msg dex-msg-tool-result';
      el.innerHTML = '<div class="dex-tool-msg dex-tool-result">'
        + '<span class="dex-tool-icon">✅</span>'
        + '<span class="dex-tool-name">' + esc(content) + '</span>'
        + '<span class="dex-tool-summary">' + esc(summary) + '</span>'
        + (detailText ? '<details class="dex-tool-details"><summary>详情</summary><pre>' + esc(detailText) + '</pre></details>' : '')
        + '</div>';
      break;
    case 'tool-error':
      var errMsg = content || '未知错误';
      var errDetail = (meta && meta.error) ? String(meta.error) : '';
      el.className = 'dex-msg dex-msg-tool-error';
      el.innerHTML = '<div class="dex-tool-msg dex-tool-error">'
        + '<span class="dex-tool-icon">❌</span>'
        + '<span class="dex-tool-name">' + esc(errMsg) + '</span>'
        + (errDetail ? '<details class="dex-tool-details dex-tool-error-details"><summary>错误详情</summary><pre>' + esc(errDetail) + '</pre></details>' : '')
        + '</div>';
      break;
    case 'tool-start':
      el.className = 'dex-msg dex-msg-tool-start';
      el.innerHTML = '<div class="dex-tool-msg dex-tool-start">'
        + '<span class="dex-spinner"></span>'
        + '<span class="dex-tool-name">' + esc(content) + '</span>'
        + (meta && meta.args && Object.keys(meta.args).length > 0 ? ' <span class="dex-tool-args">' + esc(JSON.stringify(meta.args)) + '</span>' : '')
        + '</div>';
      break;
  }
}

// 格式化工具结果详情文本（日志等特殊处理）
function dexFormatResultText(fnName, data) {
  if (fnName === 'get_logs' && Array.isArray(data)) {
    return data.map(function(l) { return dexStripAnsi(l); }).join('\n');
  }
  if (fnName === 'search_logs' && data && Array.isArray(data.results)) {
    return data.results.map(function(r) {
      return 'L' + r.line_number + ': ' + dexStripAnsi(r.line || r);
    }).join('\n');
  }
  try {
    var str = JSON.stringify(dexMaskApiKey(dexStripAnsi(data)), null, 2);
    return str.length > 2000 ? str.substring(0, 2000) + '\n…(已截断)' : str;
  } catch (e) {
    return String(data || '');
  }
}

function dexToolSummary(fnName, result) {
  if (!result) return '完成';
  var data = (result.data !== undefined) ? result.data : result;
  switch (fnName) {
    case 'run_full_diagnostics': case 'run_diagnostics':
      if (data && data.summary) return (data.summary.pass||0)+'通过 '+(data.summary.warn||0)+'警告 '+(data.summary.fail||0)+'失败'; break;
    case 'list_accounts': if (Array.isArray(data)) return data.length + ' 个账号'; break;
    case 'get_active_account': if (data && data.provider) return data.provider; break;
    case 'get_service_status': if (data && data.running) return '✅运行中 :'+data.port; return '⏸已停止';
    case 'fetch_balance': if (data) { var b = data.balance || data.total_balance || data.remaining; if (b) return b; } return '已查询';
    case 'get_config': return '已获取';
    case 'get_logs': if (Array.isArray(data)) return data.length + ' 行'; if (typeof data === 'string') return data.split('\n').length + ' 行'; break;
    case 'search_logs': if (data && data.matches !== undefined) return data.matches + ' 处匹配'; break;
    case 'list_sessions': if (Array.isArray(data)) return data.length + ' 个会话'; break;
    case 'list_threads': if (Array.isArray(data)) return data.length + ' 个线程'; break;
    case 'list_plugins': if (Array.isArray(data)) return data.length + ' 个插件'; break;
    case 'list_request_history': if (Array.isArray(data)) return data.length + ' 条'; break;
    case 'test_upstream_connectivity': if (data && data.ok !== undefined) return data.ok ? '✅连通'+(data.latency_ms||'')+'ms' : '❌失败'; break;
    case 'check_upgrade': if (data && data.latest) return data.latest; break;
    case 'get_threads_status': if (data && data.total !== undefined) return data.total + ' 个线程'; break;
    case 'get_env_info': if (data && data.os) return data.os + ' · deecodex ' + data.deecodex_version; break;
    case 'health_summary': if (data) return (data.service.running?'🟢':'🔴') + ' svc ' + (data.account.ok?'🟢':'🔴') + ' acct · ' + data.recent_errors + ' err'; break;
    case 'analyze_requests': if (data && data.total) return data.total + '请求 · ' + data.success_rate + '%成功 · ' + data.avg_latency_ms + 'ms均值'; return '无数据';
    case 'detect_processes': if (data && Array.isArray(data.processes)) { var r = data.processes.filter(function(p){return p.running;}).length; return r + ' 个进程运行中'; } break;
    case 'detect_ports': if (data && Array.isArray(data.ports)) { var u = data.ports.filter(function(p){return p.in_use;}).length; return u + ' 个端口占用'; } break;
    case 'execute_shell': if (data && data.success) return '成功 (exit ' + (data.exit_code||0) + ')'; return '失败'; break;
    case 'start_service': case 'stop_service': return data && data.running !== undefined ? (data.running ? '已启动' : '已停止') : '完成'; break;
    case 'config_backup': if (data && data.action) return '备份: ' + (data.count || '完成'); return '完成';
    case 'config_diff': if (data && data.changes !== undefined) return data.changes + ' 处变更'; return '无差异';
    case 'token_cost': if (data && data.total_cost) return data.total_cost; return '已分析';
    case 'speed_test': if (data && data.avg_latency_ms) return data.avg_latency_ms + 'ms'; return '已测速';
    case 'thread_cleanup': if (data && data.removed !== undefined) return '清理 ' + data.removed + ' 条'; return '完成';
    case 'auto_tune': if (data && data.applied) return '已应用 ' + data.applied + ' 项优化'; return '已分析';
    case 'claude_mcp_check': if (data && data.ok !== undefined) return data.ok ? 'MCP正常' : 'MCP异常'; return '已检查';
    case 'network_topology': if (data && data.nodes) return data.nodes + ' 节点'; return '已分析';
    case 'ssl_check': if (data && data.valid !== undefined) return data.valid ? '证书有效' : '证书异常'; return '已检查';
    case 'export_report': if (data && data.path) return '已导出: ' + data.path; return '已导出';
    default:
      if (typeof data === 'string' && data.length <= 60) return data;
      if (typeof data === 'number' || typeof data === 'boolean') return String(data);
      var s = JSON.stringify(data);
      return s.length > 80 ? s.substring(0, 80) + '…' : s;
  }
  return '完成';
}

function dexShowInlineConfirm(toolName, message, toolCallId) {
  return new Promise(function (resolve) {
    var container = document.getElementById('dexMessages');
    if (!container) { resolve(false); return; }
    var el = document.createElement('div');
    el.className = 'dex-msg dex-msg-confirm-inline';
    el.id = 'dexConfirm-' + (toolCallId || Date.now());
    el.innerHTML = '<div class="dex-confirm-card"><div class="dex-confirm-header">⚠ 需要确认操作</div>'
      + '<div class="dex-confirm-body"><div class="dex-confirm-tool">' + esc(toolName) + '</div>'
      + '<div class="dex-confirm-msg">' + esc(message) + '</div></div>'
      + '<div class="dex-confirm-actions">'
      + '<button class="btn btn-primary btn-sm dex-confirm-ok">确认执行</button>'
      + '<button class="btn btn-ghost btn-sm dex-confirm-cancel">取消</button></div></div>';
    container.appendChild(el);
    dexScrollToBottom();
    el.querySelector('.dex-confirm-ok').onclick = function () { el.remove(); resolve(true); };
    el.querySelector('.dex-confirm-cancel').onclick = function () { el.remove(); resolve(false); };
  });
}

// ── Markdown 渲染（增强版）──
function dexRenderMarkdown(text) {
  if (!text) return '';
  var codeBlocks = [];
  var safe = text.replace(/```(\w*)\n([\s\S]*?)```/g, function (m, lang, code) {
    codeBlocks.push('<pre><code class="dex-code-block">' + esc(code.trim()) + '</code></pre>');
    return '%%CODEBLOCK_' + (codeBlocks.length - 1) + '%%';
  });
  var inlineCodes = [];
  safe = safe.replace(/`([^`]+)`/g, function (m, code) {
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
    if (/^\|.+\|$/.test(line) && line.indexOf('|') > 0) {
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
  html = html.replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2" target="_blank" rel="noopener">$1</a>');
  html = html.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  html = html.replace(/\*([^*]+)\*/g, '<em>$1</em>');
  html = html.replace(/%%CODEBLOCK_(\d+)%%/g, function (m, n) { return codeBlocks[parseInt(n)]; });
  html = html.replace(/%%INLINECODE_(\d+)%%/g, function (m, n) { return inlineCodes[parseInt(n)]; });
  html = html.replace(/\n\n/g, '</p><p>');
  html = html.replace(/\n/g, '<br>');
  // 不包裹表格/代码块等块级元素
  if (/<(table|pre|blockquote|h[3-4]|ul|ol|hr)/.test(html))
    return '<div class="dex-md">' + html + '</div>';
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

async function dexSendMessage() {
  var input = document.getElementById('dexInput');
  if (!input) return;
  var text = input.value.trim();
  if (!text) return;
  if (window.dexAgent.isProcessing) { showToast('Agent 正在处理中，请等待', 'warn'); return; }
  input.value = '';
  dexAppendMessage('user', text);
  dexUpdateTokenCount();
  dexShowThinking();
  try { await window.dexAgent.run(text); }
  catch (e) { console.error('[dexAssistant] sendMessage 失败:', e); dexAppendMessage('system', '消息处理失败: ' + (e.message || e)); }
  dexHideThinking();
  dexUpdateTokenCount();
  input.focus();
}

function dexShowThinking() {
  dexHideThinking();
  var c = document.getElementById('dexMessages');
  if (!c) return;
  var el = document.createElement('div');
  el.className = 'dex-msg dex-msg-thinking';
  el.id = 'dexThinking';
  el.innerHTML = '<div class="dex-bubble dex-bubble-assistant"><div class="dex-thinking-dots"><span></span><span></span><span></span></div></div>';
  c.appendChild(el);
  dexScrollToBottom();
}

function dexHideThinking() { var el = document.getElementById('dexThinking'); if (el) el.remove(); }

var _dexLastAssistantEl = null;
var _dexLastReasoningEl = null;
function dexUpdateLastAssistant(text, reasoning) {
  if (!_dexLastAssistantEl) {
    _dexLastAssistantEl = dexAppendMessage('assistant', text, { model: window.dexAgent.selectedModel });
    if (reasoning) {
      var rEl = _dexLastAssistantEl.querySelector('.dex-reasoning-content');
      if (rEl) rEl.textContent = reasoning;
      var rWrap = _dexLastAssistantEl.querySelector('.dex-reasoning-wrap');
      if (rWrap) rWrap.style.display = '';
    }
    return;
  }
  var bubble = _dexLastAssistantEl.querySelector('.dex-bubble-text');
  if (bubble) bubble.innerHTML = dexRenderMarkdown(text);
  if (reasoning) {
    var rWrap2 = _dexLastAssistantEl.querySelector('.dex-reasoning-wrap');
    if (rWrap2) rWrap2.style.display = '';
    var rEl2 = _dexLastAssistantEl.querySelector('.dex-reasoning-content');
    if (rEl2) rEl2.textContent = reasoning;
  }
  dexScrollToBottom();
}

function dexScrollToBottom() { var c = document.getElementById('dexMessages'); if (c) c.scrollTop = c.scrollHeight; }

function dexQuickAction(prompt) { var i = document.getElementById('dexInput'); if (i) i.value = prompt; dexSendMessage(); }

function dexNewChat() {
  if (window.dexAgent.isProcessing) { showToast('Agent 正在处理中，请等待', 'warn'); return; }
  window.dexAgent.init();
  var c = document.getElementById('dexMessages'); if (c) c.innerHTML = dexWelcomeHTML();
  dexCloseSearch();
  dexUpdateTokenCount();
  dexScrollToBottom(); showToast('已开始新对话', 'success');
}

function dexClearChat() {
  if (window.dexAgent.isProcessing) { showToast('Agent 正在处理中，请等待', 'warn'); return; }
  window.dexAgent.clear();
  var c = document.getElementById('dexMessages'); if (c) c.innerHTML = dexWelcomeHTML();
  dexCloseSearch();
  dexUpdateTokenCount();
  dexScrollToBottom(); showToast('对话已清空', 'success');
}

function dexRefreshChat() { var c = document.getElementById('dexMessages'); if (c) c.innerHTML = dexWelcomeHTML(); }

// ── 自定义模型下拉 ──
window.dexAgent.selectedModel = 'auto';
function dexToggleModelMenu(e) { e.stopPropagation();
  var m = document.getElementById('dexModelMenu');
  if (m) m.style.display = m.style.display === 'none' ? '' : 'none';
}
function dexSelectModel(v, label) {
  var btn = document.getElementById('dexModelBtn');
  if (btn) btn.innerHTML = label + ' ▾';
  window.dexAgent.selectedModel = v;
  document.getElementById('dexModelMenu').style.display = 'none';
}
function dexChangeModel() {} // 兼容旧引用

function dexLoadModels() {
  var menu = document.getElementById('dexModelMenu');
  if (!menu) return;
  DeeCodexTauri.invoke('get_active_account', {}).then(function(account) {
    if (!account || !account.model_map) return;
    var mm = account.model_map;
    if (typeof mm === 'string') { try { mm = JSON.parse(mm); } catch(e) { return; } }
    if (typeof mm !== 'object') return;
    var vals = Object.values(mm);
    var seen = {}, html = '';
    html += '<div class="dex-model-item" onclick="dexSelectModel(\'auto\',\'模型\')">自动</div>';
    for (var i = 0; i < vals.length; i++) {
      var v = vals[i];
      if (seen[v]) continue; seen[v] = true;
      html += '<div class="dex-model-item" onclick="dexSelectModel(\'' + esc(v) + '\',\'' + esc(v) + '\')">' + esc(v) + '</div>';
    }
    menu.innerHTML = html;
  }).catch(function(e) { console.warn('[dexAgent] 加载模型列表失败:', e); });
}
// 点击其他地方关闭下拉
document.addEventListener('click', function() {
  var m = document.getElementById('dexModelMenu');
  if (m) m.style.display = 'none';
});

// ── 停止按钮 ──
function dexShowStopButton() {
  var sendBtn = document.getElementById('dexSendBtn');
  var stopBtn = document.getElementById('dexStopBtn');
  if (sendBtn) sendBtn.style.display = 'none';
  if (stopBtn) stopBtn.style.display = '';
}
function dexHideStopButton() {
  var sendBtn = document.getElementById('dexSendBtn');
  var stopBtn = document.getElementById('dexStopBtn');
  if (sendBtn) sendBtn.style.display = '';
  if (stopBtn) stopBtn.style.display = 'none';
}
function dexStopAgent() {
  if (window.dexAgent) window.dexAgent.abort();
  dexHideStopButton();
  dexHideThinking();
}

// ── API Key 脱敏 ──
function dexMaskApiKey(text) {
  if (!text) return text;
  if (typeof text === 'object') {
    var obj = JSON.parse(JSON.stringify(text));
    if (obj.api_key) obj.api_key = 'sk-***';
    if (obj.vision_api_key) obj.vision_api_key = '***';
    return obj;
  }
  return String(text).replace(/"api_key":"[^"]+"/g, '"api_key":"sk-***"');
}

// ── ANSI 转义码剥离 ──
function dexStripAnsi(text) {
  if (!text) return text;
  if (Array.isArray(text)) return text.map(dexStripAnsi);
  if (typeof text !== 'string') return text;
  return text.replace(/\x1B\[[0-9;]*[a-zA-Z]/g, '').replace(/\[[0-9;]*[a-zA-Z]/g, '');
}

// ── 工具执行后联动刷新 GUI ──
var DEX_MUTATE_TOOLS = {
  'save_config': '配置已更新，切换到「协议配置」面板查看',
  'start_service': '服务已启动',
  'stop_service': '服务已停止',
  'switch_account': '账号已切换',
  'add_account': '账号已添加',
  'update_account': '账号已更新',
  'delete_account': '账号已删除',
  'migrate_threads': '线程已迁移',
  'restore_threads': '线程已还原',
  'calibrate_threads': '线程已校准',
  'run_upgrade': '升级完成',
  'install_plugin': '插件已安装',
  'uninstall_plugin': '插件已卸载',
  'launch_codex_cdp': 'Codex CDP 已启动',
  'stop_codex_cdp': 'Codex CDP 已停止',
};
function dexAfterMutate(fnName) {
  if (DEX_MUTATE_TOOLS[fnName]) {
    dexRefreshStatus();
    showToast(DEX_MUTATE_TOOLS[fnName], 'success');
    // 刷新全局配置缓存，让其他面板立即看到变更
    if (fnName === 'save_config' && typeof loadConfig === 'function') {
      loadConfig().catch(function(){});
    }
    if (fnName === 'start_service' || fnName === 'stop_service') {
      // 刷新服务概览面板的 _statusData
      DeeCodexTauri.invoke('get_service_status').then(function(s) {
        window._statusData = {
          running: s && s.running, port: s ? s.port : '—',
          uptime_secs: s && s.running ? s.uptime_secs : 0,
          version: (s && s.version) || (window._statusData && window._statusData.version) || '—',
          upstream: (window._statusData && window._statusData.upstream) || '—',
          vision_enabled: (window._statusData && window._statusData.vision_enabled) || false,
          computer_executor: (window._statusData && window._statusData.computer_executor) || 'disabled',
          chinese_thinking: (window._statusData && window._statusData.chinese_thinking) || false,
          cdp_port: (window._statusData && window._statusData.cdp_port) || 9222,
          codex_launch_with_cdp: (window._statusData && window._statusData.codex_launch_with_cdp) || false,
        };
        // 更新侧边栏连接指示器
        var dot = document.getElementById('connDot');
        var label = document.getElementById('connLabel');
        if (dot && label && s) {
          if (s.running) { dot.className = 'indicator ok'; label.textContent = '服务运行中'; }
          else { dot.className = 'indicator off'; label.textContent = '服务已停止'; }
        }
      }).catch(function(){});
    }
    if (fnName.indexOf('account') >= 0 && typeof loadAccountsData === 'function') {
      loadAccountsData();
    }
    if (fnName.indexOf('plugin') >= 0 && typeof loadPluginsData === 'function') {
      loadPluginsData();
    }
  }
}

// ── 状态栏刷新 ──
function dexRefreshStatus() {
  var dot = document.getElementById('dexStatusDot');
  var text = document.getElementById('dexStatusText');
  if (!dot || !text) return;
  DeeCodexTauri.invoke('dex_health_summary', {}).then(function (data) {
    if (!data) { dot.className = 'dex-status-dot dex-status-warn'; text.textContent = '无数据'; return; }
    var svcOk = data.service && data.service.running;
    var acctOk = data.account && data.account.ok;
    var errCount = data.recent_errors || 0;
    var m = window.dexAgent.selectedModel;
    var modelLabel = (m && m !== 'auto') ? m : (data.account && data.account.provider) || '';
    var parts = [];
    if (svcOk) parts.push('🟢服务');
    else parts.push('🔴服务');
    if (acctOk) parts.push('账号正常');
    else parts.push('账号异常');
    if (errCount > 0) parts.push(errCount + 'err');
    if (modelLabel) parts.push(modelLabel);
    text.textContent = parts.join(' · ');
    if (!svcOk || !acctOk) { dot.className = 'dex-status-dot dex-status-err'; }
    else if (errCount > 0) { dot.className = 'dex-status-dot dex-status-warn'; }
    else { dot.className = 'dex-status-dot dex-status-ok'; }
  }).catch(function () {
    dot.className = 'dex-status-dot dex-status-err';
    text.textContent = '状态获取失败';
  });
}

// ── 对话导出 ──
function dexExportChat() {
  var msgs = window.dexAgent.messages;
  var md = '# DEX助手 对话导出\n\n';
  md += '> 导出时间: ' + new Date().toLocaleString() + '\n\n---\n\n';
  for (var i = 0; i < msgs.length; i++) {
    var m = msgs[i];
    if (m.role === 'system' && m.content && m.content.indexOf('[对话摘要]') !== 0) continue;
    if (m.role === 'user') md += '**用户:** ' + (m.content || '') + '\n\n';
    else if (m.role === 'assistant' && m.content) md += '**DEX助手:** ' + m.content + '\n\n';
    else if (m.role === 'system' && m.content && m.content.indexOf('[对话摘要]') === 0) md += '> ' + m.content + '\n\n';
    else if (m.role === 'tool') { try { var d = JSON.parse(m.content); md += '*工具: ' + (d.error || '完成') + '*\n\n'; } catch(e) {} }
  }
  navigator.clipboard.writeText(md).then(function () {
    showToast('对话已复制到剪贴板', 'success');
  }).catch(function () {
    showToast('复制失败，请手动复制', 'error');
  });
}

// ── 斜杠指令映射 ──
var DEX_SLASH_COMMANDS = {
  '/diag': '运行完整诊断，分析结果',
  '/fix': '自动修复所有发现的问题',
  '/health': '健康概览',
  '/cost': '分析请求成本',
  '/status': '服务状态',
  '/log': '读日志，检查异常',
  '/help': '你能做什么'
};

function dexExpandSlashCommand(text) {
  if (!text || text[0] !== '/') return text;
  var cmd = text.split(' ')[0];
  if (DEX_SLASH_COMMANDS[cmd]) return DEX_SLASH_COMMANDS[cmd];
  return text;
}

// ── Token 计数器 ──
function dexUpdateTokenCount() {
  var el = document.getElementById('dexTokenCount');
  if (!el) return;
  var msgs = window.dexAgent.messages;
  if (!msgs || msgs.length <= 1) { el.textContent = '~0 tokens'; return; }
  try {
    var totalLen = 0;
    for (var i = 0; i < msgs.length; i++) {
      totalLen += JSON.stringify(msgs[i]).length;
    }
    var estimated = Math.max(1, Math.round(totalLen / 4));
    el.textContent = '~' + estimated + ' tokens';
  } catch (e) { el.textContent = '~? tokens'; }
}

// ── 工具建议预览 ──
var DEX_TOOL_KEYWORDS = [
  { keywords: ['诊断', 'diag', '问题', '修复', 'fix', '错误', '失败', '异常'], tools: 'get_service_status, run_diagnostics, run_full_diagnostics' },
  { keywords: ['账号', '账户', 'account', '余额', '切换', '供应商', '导入'], tools: 'list_accounts, get_active_account, fetch_balance' },
  { keywords: ['配置', 'config', 'conf', '备份', 'backup', '恢复'], tools: 'get_config, save_config, config_backup' },
  { keywords: ['日志', 'log', '错误日志', '搜索日志'], tools: 'get_logs, search_logs' },
  { keywords: ['插件', 'plugin', '安装', '卸载', '扫码'], tools: 'list_plugins, query_plugin_status' },
  { keywords: ['线程', 'thread', '会话', 'session', '清理', '迁移'], tools: 'list_threads, get_threads_status, thread_cleanup' },
  { keywords: ['服务', 'service', '启动', '停止', '状态', '端口', '重启'], tools: 'get_service_status, start_service, stop_service' },
  { keywords: ['速度', 'speed', '延迟', 'latency', '测速', '连通'], tools: 'speed_test, test_upstream_connectivity' },
  { keywords: ['成本', 'cost', 'token', '花费', '消耗'], tools: 'token_cost, analyze_requests' },
  { keywords: ['安全', 'ssl', '证书', 'tls', '网络'], tools: 'ssl_check, network_topology' },
  { keywords: ['claude', 'mcp', '集成'], tools: 'claude_mcp_check' },
  { keywords: ['升级', 'upgrade', '更新', '版本'], tools: 'check_upgrade, run_upgrade' }
];

function dexPreviewTools(userMessage) {
  if (!userMessage) return null;
  var lower = userMessage.toLowerCase();
  var matched = {};
  for (var i = 0; i < DEX_TOOL_KEYWORDS.length; i++) {
    var entry = DEX_TOOL_KEYWORDS[i];
    for (var j = 0; j < entry.keywords.length; j++) {
      if (lower.indexOf(entry.keywords[j]) !== -1) {
        var tools = entry.tools.split(', ');
        for (var k = 0; k < tools.length; k++) matched[tools[k]] = true;
        break;
      }
    }
  }
  var names = Object.keys(matched);
  if (names.length === 0) return null;
  return names.slice(0, 5).join(', ');
}

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

// ── 快捷键绑定 ──
function dexBindShortcuts() {
  if (window._dexShortcutsBound) return;
  window._dexShortcutsBound = true;
  document.addEventListener('keydown', function (e) {
    // Ctrl+K / Cmd+K → 聚焦输入框
    if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
      var panel = document.getElementById('dexMessages');
      if (!panel || panel.offsetParent === null) return;
      e.preventDefault();
      var input = document.getElementById('dexInput');
      if (input) input.focus();
      return;
    }
    // Ctrl+L / Cmd+L → 清空对话
    if ((e.ctrlKey || e.metaKey) && e.key === 'l') {
      var panel2 = document.getElementById('dexMessages');
      if (!panel2 || panel2.offsetParent === null) return;
      e.preventDefault();
      dexClearChat();
      return;
    }
    // Escape → 停止 Agent / 关闭搜索
    if (e.key === 'Escape') {
      var searchBar = document.getElementById('dexSearchBar');
      if (searchBar && searchBar.style.display !== 'none') {
        dexCloseSearch();
        return;
      }
      if (window.dexAgent && window.dexAgent.isProcessing) {
        dexStopAgent();
        return;
      }
    }
  });
}

// ═══════════════════════════════════════════════════════════════
// 个人中心
// ═══════════════════════════════════════════════════════════════
function renderProfile() {
  return '<div class="page-header"><h2>个人中心</h2><p>账户信息与偏好设置</p></div><div class="empty-state">即将推出</div>';
}


// ═══════════════════════════════════════════════════════════════
