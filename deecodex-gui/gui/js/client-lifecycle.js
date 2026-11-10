// 服务概览客户端一键接入流程
// ═══════════════════════════════════════════════════════════════
(function () {
  const LAUNCH_DIR_PREFIX = 'deecodex.clientLaunchDir.';
  window._clientLifecycleMap = window._clientLifecycleMap || {};
  window._clientLifecycleRefreshing = false;

  function cssEscape(value) {
    if (window.CSS && typeof window.CSS.escape === 'function') return window.CSS.escape(String(value));
    return String(value).replace(/["\\]/g, '\\$&');
  }

  function lifecycleKinds() {
    if (typeof statusClientKinds === 'function') return statusClientKinds().map(item => item.slug);
    return ['codex_desktop', 'dsh', 'workbuddy', 'opencode_desktop', 'hermes_desktop', 'claude_desktop', 'claude_cli', 'pi', 'codex_cli', 'opencode', 'hermes'];
  }

  function lifecycleLabel(kind) {
    if (typeof statusClientDockItem === 'function') return statusClientDockItem(kind).label || kind;
    return kind;
  }

  function lifecycleAccountKind(kind) {
    const item = typeof statusClientDockItem === 'function' ? statusClientDockItem(kind) : {};
    return item.accountKind || kind;
  }

  function lifecycleSurface(kind, status) {
    if (status?.surface) return String(status.surface);
    if (typeof statusClientSurface === 'function') return statusClientSurface(kind);
    return String(kind || '').endsWith('_desktop') ? 'desktop' : 'cli';
  }

  function lifecycleRequiresLaunchDir(kind, status) {
    if (typeof status?.launch?.requires_cwd === 'boolean') return status.launch.requires_cwd;
    return kind === 'codex_cli' || kind === 'claude_cli';
  }

  function lifecycleUsesCodexProxy(kind, status) {
    if (status?.account_kind) return status.account_kind === 'codex';
    return lifecycleAccountKind(kind) === 'codex';
  }

  async function waitForServiceRunning() {
    for (let i = 0; i < 24; i++) {
      await new Promise(resolve => setTimeout(resolve, 500));
      const status = await invoke('get_service_status');
      window._statusData = {
        ...(window._statusData || {}),
        ...status,
      };
      if (status?.running) return status;
    }
    throw new Error('deecodex 服务启动超时');
  }

  async function ensureCodexServiceReady(kind, status) {
    if (!lifecycleUsesCodexProxy(kind, status)) return;
    const current = await invoke('get_service_status');
    window._statusData = {
      ...(window._statusData || {}),
      ...current,
    };
    if (!current?.running) {
      showToast('正在启动 deecodex 服务...', 'info');
      await invoke('start_service');
      await waitForServiceRunning();
      await window.loadStatus?.();
      showToast('deecodex 服务已生效，正在启动 Codex', 'success');
      return;
    }
    await window.loadStatus?.();
  }

  function setButtonLifecycleStatus(kind, status) {
    const button = document.querySelector(`.client-dock-item[data-client-kind="${cssEscape(kind)}"]`);
    if (!button) return;
    window._clientLifecycleMap[kind] = status;
    const info = typeof statusClientInfo === 'function'
      ? statusClientInfo(kind, Boolean(window._statusData?.running))
      : { processRunning: Boolean(status?.runtime?.running), state: status?.runtime?.running ? 'on' : 'off', text: status?.runtime?.running ? '运行中' : '未运行' };
    const lifecycle = typeof statusClientLifecycleMeta === 'function'
      ? statusClientLifecycleMeta(kind, info)
      : { classes: '', action: status?.next_action || 'launch', installLabel: '', accountLabel: '', runLabel: '' };
    ['is-installed', 'needs-install', 'has-account', 'needs-account', 'config-ready', 'config-pending', 'next-install', 'next-configure', 'next-launch', 'next-running'].forEach(cls => {
      button.classList.remove(cls);
    });
    lifecycle.classes.split(/\s+/).filter(Boolean).forEach(cls => button.classList.add(cls));
    button.classList.toggle('process-running', Boolean(status?.runtime?.running));
    button.classList.toggle('on', Boolean(status?.runtime?.running));
    button.classList.toggle('off', !status?.runtime?.running);
    button.dataset.nextAction = lifecycle.action || '';
    button.setAttribute('aria-label', `${button.dataset.clientLabel || lifecycleLabel(kind)} · ${status.next_action === 'install' ? '需要安装' : (status.next_action === 'configure' ? '需要配置账号' : info.text)}`);
    const runtime = button.querySelector('.client-dock-runtime');
    if (runtime) {
      runtime.classList.toggle('live', Boolean(status?.runtime?.running));
      runtime.classList.toggle('idle', !status?.runtime?.running);
      runtime.title = status?.runtime?.running ? '客户端进程运行中' : '未检测到客户端进程';
    }
    const installDot = button.querySelector('.client-dock-state-dot.install');
    const accountDot = button.querySelector('.client-dock-state-dot.account');
    const runtimeDot = button.querySelector('.client-dock-state-dot.runtime');
    if (installDot) installDot.title = lifecycle.installLabel;
    if (accountDot) accountDot.title = lifecycle.accountLabel;
    if (runtimeDot) runtimeDot.title = lifecycle.runLabel;
  }

  async function getLifecycleStatus(kind) {
    const status = await invoke('dex_client_lifecycle_status', { kind });
    window._clientLifecycleMap[kind] = status;
    return status;
  }

  async function refreshClientLifecycleDock() {
    if (window._clientLifecycleRefreshing || currentPanel !== 'status') return;
    window._clientLifecycleRefreshing = true;
    try {
      const kinds = lifecycleKinds();
      const results = await Promise.all(kinds.map(kind => getLifecycleStatus(kind).then(status => ({ kind, status })).catch(error => ({ kind, error }))));
      results.forEach(result => {
        if (result.status) setButtonLifecycleStatus(result.kind, result.status);
      });
    } finally {
      window._clientLifecycleRefreshing = false;
    }
  }

  async function ensureLifecycleData() {
    if (!Array.isArray(clientProfiles) || !clientProfiles.length) await loadClientProfiles?.();
    if (!Array.isArray(providerPresets) || !providerPresets.length) await loadProviderPresets?.();
    if (!Array.isArray(endpointTemplates) || !endpointTemplates.length) await loadEndpointTemplates?.();
    if (!currentConfig) await loadConfig?.();
  }

  function defaultProviderForLifecycle(kind) {
    // 所有客户端的快捷配置统一默认 DeepSeek 账号模板。
    return 'deepseek';
  }

  function quickProviders(kind) {
    const accountKind = lifecycleAccountKind(kind);
    if (typeof providersForClientKind === 'function') {
      const providers = providersForClientKind(accountKind);
      if (providers.length) return providers;
    }
    return providerPresets || [];
  }

  function quickDefaults(kind, provider) {
    const accountKind = lifecycleAccountKind(kind);
    const preset = typeof getProviderPreset === 'function'
      ? getProviderPreset(provider)
      : (providerPresets || []).find(item => item.slug === provider);
    if (typeof clientProviderDefaults === 'function') {
      return clientProviderDefaults(accountKind, provider, preset);
    }
    return {
      upstream: preset?.default_upstream || '',
      default_model: Array.isArray(preset?.known_models) ? (preset.known_models[0] || '') : '',
      known_models: Array.isArray(preset?.known_models) ? preset.known_models : [],
      api_key_env: 'OPENAI_API_KEY',
    };
  }

  function providerApiKeyUrl(provider) {
    return typeof providerApiKeyConsoleUrl === 'function' ? providerApiKeyConsoleUrl(provider) : '';
  }

  // 默认模型控件：有已知模型列表时用下拉，否则退回输入框。
  function renderQuickModelControl(kind, provider) {
    const defaults = quickDefaults(kind, provider);
    const models = Array.isArray(defaults.known_models) ? defaults.known_models : [];
    const selected = defaults.default_model || models[0] || '';
    if (!models.length) {
      return `<input type="text" id="clientQuickModel" value="${escAttr(selected)}" placeholder="${escAttr(defaults.default_model || 'model-name')}">`;
    }
    return `<select id="clientQuickModel">
      ${models.map(m => `<option value="${escAttr(m)}" ${m === selected ? 'selected' : ''}>${esc(m)}</option>`).join('')}
    </select>`;
  }

  function fillQuickModelSelect(models) {
    const wrap = document.getElementById('clientQuickModelWrap');
    if (!wrap) return;
    const list = Array.isArray(models) ? models : [];
    if (!list.length) return;
    const current = document.getElementById('clientQuickModel')?.value || '';
    const selected = list.includes(current) ? current : list[0];
    wrap.innerHTML = `<select id="clientQuickModel">
      ${list.map(m => `<option value="${escAttr(m)}" ${m === selected ? 'selected' : ''}>${esc(m)}</option>`).join('')}
    </select>`;
  }

  function quickAccountName(kind, provider) {
    const providerLabel = (typeof getProviderPreset === 'function' ? getProviderPreset(provider)?.label : '') || provider || '自定义';
    return `${lifecycleLabel(kind)} · ${providerLabel}`;
  }

  function buildQuickAccountPayload(kind, values) {
    const accountKind = lifecycleAccountKind(kind);
    const surface = lifecycleSurface(kind, values.status);
    const preset = typeof getProviderPreset === 'function' ? getProviderPreset(values.provider) : null;
    const providerOptions = preset?.provider_options || { capability_labels: preset?.capability_labels || [] };
    const model = values.model.trim();
    const base = {
      id: '',
      name: values.name.trim() || quickAccountName(kind, values.provider),
      provider: values.provider,
      client_kind: accountKind,
      client_surface: surface,
      target: accountKind,
      upstream: values.upstream.trim(),
      api_key: values.apiKey.trim(),
      default_model: accountKind === 'codex' ? '' : model,
      client_options: { client_surface: surface },
      model_map: {},
      provider_options: providerOptions,
      translate_enabled: accountKind === 'codex',
      endpoints: [],
    };

    if (accountKind === 'codex') {
      base.model_map = {};
      if (typeof providerDefaultTemplate === 'function' && typeof createEndpointFromTemplate === 'function') {
        base.endpoints = [createEndpointFromTemplate(providerDefaultTemplate(values.provider), base)];
      }
      if (base.endpoints[0]) {
        base.endpoints[0].model_map = {};
      }
      if (typeof applyProviderSpecificEndpointDefaults === 'function') {
        applyProviderSpecificEndpointDefaults(base);
      }
      return base;
    }

    base.translate_enabled = false;
    base.client_options = {
      client_surface: surface,
      api_key_env: values.apiKeyEnv.trim() || 'OPENAI_API_KEY',
      model_map: model ? { default: model } : {},
      // hermes 直连供应商，不走 DEX 代理；其余非 Codex 客户端保留请求历史代理。
      proxy_recording_enabled: accountKind === 'hermes' ? false : Boolean(values.proxyRecording),
    };
    if (accountKind === 'claude_code') {
      base.client_options.auth_env = base.client_options.api_key_env;
    }
    return base;
  }

  function renderQuickConfigModal(kind, status) {
    const accountKind = lifecycleAccountKind(kind);
    const providers = quickProviders(kind);
    const defaultProvider = providers.some(p => p.slug === defaultProviderForLifecycle(kind))
      ? defaultProviderForLifecycle(kind)
      : (providers[0]?.slug || 'custom');
    const defaults = quickDefaults(kind, defaultProvider);
    const profile = typeof getClientProfile === 'function' ? getClientProfile(accountKind) : null;
    const apiKeyEnv = defaults.api_key_env || (typeof defaultApiKeyEnvForClient === 'function'
      ? defaultApiKeyEnvForClient({ provider: defaultProvider, client_kind: accountKind })
      : 'OPENAI_API_KEY');
    const officialEntry = (accountKind === 'codex' || accountKind === 'claude_code')
      ? `<button type="button" class="btn btn-ghost client-quick-official" id="clientQuickOfficialBtn">官方登录</button>`
      : '';
    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay client-quick-overlay';
    overlay.id = 'clientQuickConfigModal';
    overlay.innerHTML = `
      <div class="modal-box client-quick-box">
        <div class="modal-header">
          <h3>${esc(lifecycleLabel(kind))}</h3>
          <button class="modal-close" id="clientQuickCloseBtn" type="button">✕</button>
        </div>
        <div class="modal-body client-quick-body">
          <div class="config-fields client-quick-fields">
            <div class="config-field">
              <label>账号名称</label>
              <input type="text" id="clientQuickName" value="${escAttr(quickAccountName(kind, defaultProvider))}" placeholder="输入账号显示名">
            </div>
            <div class="config-field">
              <label>供应商</label>
              <select id="clientQuickProvider">
                ${providers.map(p => `<option value="${escAttr(p.slug)}" ${p.slug === defaultProvider ? 'selected' : ''}>${esc(p.label || p.slug)}</option>`).join('')}
              </select>
            </div>
            <div class="config-field">
              <label>Base URL</label>
              <input type="text" id="clientQuickUpstream" value="${escAttr(defaults.upstream || profile?.default_base_url || '')}" placeholder="${escAttr(profile?.default_base_url || 'https://api.example.com/v1')}">
            </div>
            <div class="config-field">
              <label>API Key</label>
              <div class="client-quick-key-row">
                <input type="password" id="clientQuickApiKey" value="" placeholder="输入 API 密钥" autocomplete="off">
                <button type="button" class="btn btn-ghost client-quick-key-btn" id="clientQuickKeyBtn" ${providerApiKeyUrl(defaultProvider) ? '' : 'style="display:none"'}>获取 Key</button>
              </div>
            </div>
            <div class="config-field">
              <label>Key 环境变量名</label>
              <input type="text" id="clientQuickApiKeyEnv" value="${escAttr(apiKeyEnv)}" placeholder="OPENAI_API_KEY">
            </div>
            <div class="config-field">
              <label>默认模型</label>
              <div class="client-quick-model-row">
                <span id="clientQuickModelWrap">${renderQuickModelControl(kind, defaultProvider)}</span>
                <button type="button" class="btn btn-ghost client-quick-fetch" id="clientQuickFetchModelsBtn">获取模型</button>
              </div>
            </div>
            ${accountKind !== 'codex' && accountKind !== 'hermes' ? `<div class="config-field wide">
              <label class="toggle-label">
                <input type="checkbox" id="clientQuickProxyRecording" checked>
                启用请求历史代理
              </label>
            </div>` : ''}
          </div>
        </div>
        <div class="client-quick-actions">
          ${officialEntry}
          <button type="button" class="btn btn-ghost" id="clientQuickCancelBtn">取消</button>
          <button type="button" class="btn btn-primary" id="clientQuickSaveBtn">保存并启动</button>
        </div>
      </div>`;
    overlay.addEventListener('click', event => {
      if (event.target === overlay) closeQuickConfigModal();
    });
    document.body.appendChild(overlay);

    document.getElementById('clientQuickCloseBtn')?.addEventListener('click', closeQuickConfigModal);
    document.getElementById('clientQuickCancelBtn')?.addEventListener('click', closeQuickConfigModal);
    document.getElementById('clientQuickProvider')?.addEventListener('change', event => {
      const provider = event.target.value;
      const next = quickDefaults(kind, provider);
      const name = document.getElementById('clientQuickName');
      const upstream = document.getElementById('clientQuickUpstream');
      const modelWrap = document.getElementById('clientQuickModelWrap');
      const env = document.getElementById('clientQuickApiKeyEnv');
      const keyBtn = document.getElementById('clientQuickKeyBtn');
      if (name) name.value = quickAccountName(kind, provider);
      if (upstream) upstream.value = next.upstream || '';
      if (modelWrap) modelWrap.innerHTML = renderQuickModelControl(kind, provider);
      if (env) env.value = next.api_key_env || env.value || 'OPENAI_API_KEY';
      if (keyBtn) keyBtn.style.display = providerApiKeyUrl(provider) ? '' : 'none';
    });
    document.getElementById('clientQuickFetchModelsBtn')?.addEventListener('click', async () => {
      const btn = document.getElementById('clientQuickFetchModelsBtn');
      const upstream = document.getElementById('clientQuickUpstream')?.value?.trim();
      const apiKey = document.getElementById('clientQuickApiKey')?.value?.trim() || '';
      const provider = document.getElementById('clientQuickProvider')?.value || defaultProvider;
      if (!upstream) {
        showToast('请先填写 Base URL', 'error');
        return;
      }
      const endpointKind = accountKind === 'claude_code' || provider === 'anthropic'
        ? 'anthropic_messages'
        : 'open_ai_chat';
      if (btn) {
        btn.disabled = true;
        btn.textContent = '获取中...';
      }
      try {
        const models = await invoke('fetch_upstream_models', { upstream, apiKey, endpointKind });
        const list = Array.isArray(models) ? models : [];
        if (!list.length) {
          showToast('上游未返回模型，可手动输入', 'info');
        } else {
          fillQuickModelSelect(list);
          showToast(`获取到 ${list.length} 个模型`, 'success');
        }
      } catch (e) {
        showToast('获取上游模型失败: ' + e, 'error');
      } finally {
        if (btn) {
          btn.disabled = false;
          btn.textContent = '获取模型';
        }
      }
    });
    document.getElementById('clientQuickKeyBtn')?.addEventListener('click', () => {
      const provider = document.getElementById('clientQuickProvider')?.value || defaultProvider;
      if (typeof openProviderApiKeyPageFor === 'function') {
        openProviderApiKeyPageFor(provider);
      } else {
        showToast('该厂商暂无配置 API Key 申请地址', 'info');
      }
    });
    document.getElementById('clientQuickOfficialBtn')?.addEventListener('click', () => {
      closeQuickConfigModal();
      selectedClientKind = accountKind;
      selectedClientSurface = lifecycleSurface(kind, status);
      if (typeof persistClientSelection === 'function') persistClientSelection();
      switchPanel('accounts');
      accountsView = 'add';
      renderPanel('accounts');
    });
    document.getElementById('clientQuickSaveBtn')?.addEventListener('click', async () => {
      await saveQuickConfig(kind, status);
    });
  }

  function closeQuickConfigModal() {
    document.getElementById('clientQuickConfigModal')?.remove();
  }

  async function saveQuickConfig(kind, status) {
    const btn = document.getElementById('clientQuickSaveBtn');
    if (btn) btn.disabled = true;
    const provider = document.getElementById('clientQuickProvider')?.value || defaultProviderForLifecycle(kind);
    const values = {
      status,
      provider,
      name: document.getElementById('clientQuickName')?.value || '',
      upstream: document.getElementById('clientQuickUpstream')?.value || '',
      model: document.getElementById('clientQuickModel')?.value || '',
      apiKey: document.getElementById('clientQuickApiKey')?.value || '',
      apiKeyEnv: document.getElementById('clientQuickApiKeyEnv')?.value || '',
      proxyRecording: document.getElementById('clientQuickProxyRecording')?.checked !== false,
    };
    if (!values.upstream.trim()) {
      showToast('Base URL 不能为空', 'error');
      if (btn) btn.disabled = false;
      return;
    }
    if (!values.apiKey.trim()) {
      showToast('API Key 不能为空', 'error');
      if (btn) btn.disabled = false;
      return;
    }
    // 防脏值：API Key 必须为纯 ASCII 且不含空白（复制粘贴常把聊天文本/中文形似字符带进来）。
    if (/[^\x00-\x7F]/.test(values.apiKey) || /\s/.test(values.apiKey)) {
      showToast('API Key 含非 ASCII 字符或空白（疑似粘贴了错误内容），请从官网重新复制', 'error');
      if (btn) btn.disabled = false;
      return;
    }
    const payload = buildQuickAccountPayload(kind, values);
    try {
      showToast('正在保存账号并写入配置...', 'info');
      const result = await invoke('dex_quick_configure_client', {
        kind,
        surface: lifecycleSurface(kind, status),
        accountJson: JSON.stringify(payload),
      });
      closeQuickConfigModal();
      await loadAccountsData?.();
      if (result && result.native) {
        // pi/dsh 等原生客户端：配置已写入客户端自身，不创建 DEX 账号、不自动启动。
        showToast(`${lifecycleLabel(kind)} 配置已写入`, 'success');
        await refreshClientLifecycleDock();
        return;
      }
      showToast('客户端账号已配置', 'success');
      await refreshClientLifecycleDock();
      await launchLifecycleClient(kind);
    } catch (error) {
      showToast('配置客户端失败: ' + error, 'error');
    } finally {
      if (btn) btn.disabled = false;
    }
  }

  async function installLifecycleClient(kind, status) {
    const label = status?.label || lifecycleLabel(kind);
    const command = status?.install?.command;
    const hint = command ? `\n\n将执行：${command}` : '';
    if (!await showConfirm(`检测到 ${label} 尚未安装，是否现在启动安装/下载流程？${hint}`)) return;
    try {
      const result = await invoke('dex_install_client', { kind });
      showToast(result.mode === 'download_page' ? '已打开下载页面，安装完成后请再次点击图标' : '已打开终端执行安装命令', 'success');
      setTimeout(refreshClientLifecycleDock, 1500);
    } catch (error) {
      showToast(`${label} 安装流程启动失败: ` + error, 'error');
    }
  }

  async function launchLifecycleClient(kind) {
    const status = window._clientLifecycleMap[kind] || await getLifecycleStatus(kind);
    await ensureCodexServiceReady(kind, status);
    const isCli = status.cli || lifecycleSurface(kind, status) === 'cli';
    const requiresLaunchDir = isCli && lifecycleRequiresLaunchDir(kind, status);
    let cwd = null;
    if (requiresLaunchDir) {
      const key = LAUNCH_DIR_PREFIX + kind;
      cwd = deeStorage.getItem(key);
      if (!cwd) {
        cwd = await invoke('dex_pick_client_launch_dir');
        if (!cwd) {
          showToast('已取消启动目录选择', 'info');
          return;
        }
        deeStorage.setItem(key, cwd);
      }
    }
    // Codex 桌面：API/中转模式走 CDP 启动（带 JS 注入：模型解锁 + 中文支持）；
    // 智能路由模式仍走原 dex_launch_client（纯启动，不注入）。
    if (kind === 'codex_desktop') {
      let mode = 'api';
      try {
        const cfg = await invoke('get_config');
        if (cfg) mode = cfg.codex_router_mode || 'api';
      } catch (_) {}
      if (mode === 'api' || mode === 'relay') {
        try {
          await invoke('launch_codex_cdp');
          showToast(`${status.label || lifecycleLabel(kind)} 已启动（${mode === 'relay' ? '中转模式' : 'API 模式'}）`, 'success');
          setTimeout(refreshClientLifecycleDock, 900);
        } catch (error) {
          showToast(`${status.label || lifecycleLabel(kind)} 启动失败: ` + error, 'error');
        }
        return;
      }
    }
    try {
      const result = await invoke('dex_launch_client', { kind, cwd });
      const label = status.label || lifecycleLabel(kind);
      if (result?.mode === 'web_open') {
        showToast(`${label} 已在运行，正在打开 Web 界面...`, 'success');
      } else if (result?.web_url) {
        showToast(`${label} 已启动，正在打开 Web 界面...`, 'success');
      } else {
        showToast(`${label} 已启动`, 'success');
      }
      setTimeout(refreshClientLifecycleDock, 900);
    } catch (error) {
      if (isCli) deeStorage.removeItem(LAUNCH_DIR_PREFIX + kind);
      showToast(`${status.label || lifecycleLabel(kind)} 启动失败: ` + error, 'error');
    }
  }

  async function handleClientDockClick(kind) {
    const normalized = String(kind || '');
    const button = document.querySelector(`.client-dock-item[data-client-kind="${cssEscape(normalized)}"]`);
    if (button) button.disabled = true;
    try {
      await ensureLifecycleData();
      const status = await getLifecycleStatus(normalized);
      setButtonLifecycleStatus(normalized, status);
      if (status.next_action === 'install') {
        await installLifecycleClient(normalized, status);
        return;
      }
      if (status.next_action === 'configure') {
        renderQuickConfigModal(normalized, status);
        return;
      }
      await launchLifecycleClient(normalized);
    } catch (error) {
      showToast(`${lifecycleLabel(normalized)} 检测失败: ` + error, 'error');
    } finally {
      if (button) button.disabled = false;
    }
  }

  window.refreshClientLifecycleDock = refreshClientLifecycleDock;
  window.handleClientDockClick = handleClientDockClick;
  window.openClientQuickConfigModal = async (kind) => {
    const normalized = String(kind || '');
    try {
      await ensureLifecycleData();
      const status = await getLifecycleStatus(normalized);
      renderQuickConfigModal(normalized, status);
    } catch (error) {
      showToast(`${lifecycleLabel(normalized)} 配置读取失败: ` + error, 'error');
    }
  };
})();
