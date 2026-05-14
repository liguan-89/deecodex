const WIZARD_STEPS = [
  { id: 'wz1', icon: '▣', title: '添加账号', desc: '点「+ 添加账号」选择供应商', panel: 'accounts', quick: '' },
  { id: 'wz2', icon: '🔑', title: '填写 API Key 和上游地址', desc: '保存后点「应用」激活', panel: 'accounts', quick: '' },
  { id: 'wz3', icon: '⬇', title: '获取在线模型列表', desc: '拉取上游可用模型', panel: 'accounts', quick: 'fetchUpstreamModels' },
  { id: 'wz4', icon: '⚙', title: '预设 GPT 模型映射', desc: '账号管理→模板预设 OpenAI→上游对照', panel: 'accounts', quick: 'presetModelMap' },
  { id: 'wz5', icon: '💾', title: '保存配置', desc: '账号管理→模板预设→保存配置', panel: 'accounts', quick: 'saveConfig' },
  { id: 'wz6', icon: '⚠', title: '重要提醒', desc: '先启 deecodex → 再开 Codex，不配第三方代理', panel: '', quick: '' },
  { id: 'wz7', icon: '▶', title: '启动服务', desc: '确认状态变绿后再开 Codex', panel: 'status', quick: 'startService' },
  { id: 'wz8', icon: '◇', title: '运行诊断', desc: '验证上游连通与模型可用', panel: 'diagnostics', quick: '' },
];
let wizardIdx = 0;
let wizardVer = '';

async function checkSetupWizard() {
  // 从服务状态获取版本，失败则从 get_config 侧面获取，再失败用已知版本
  let ver = (window._statusData && window._statusData.version) || '';
  if (ver === '—' || ver === '0.0.0' || !ver) {
    try { const st = await invoke('get_service_status'); ver = st.version; } catch (_) {}
  }
  if (!ver || ver === '—' || ver === '0.0.0') return;
  const completedVer = deeStorage.getItem('setupCompletedVersion');
  if (!completedVer || completedVer !== ver) {
    wizardVer = ver;
    wizardIdx = 0;
    showWizardBar();
  }
}

function showWizardBar() {
  const existing = document.getElementById('setupWizard');
  if (existing) existing.remove();

  const bar = document.createElement('div');
  bar.id = 'setupWizard';
  const isLight = document.documentElement.getAttribute('data-theme') === 'light';
  bar.style.cssText = 'position:fixed;top:0;left:268px;right:0;z-index:100;'
    + (isLight ? 'background:rgba(255,255,255,0.95);' : 'background:var(--bg-elevated);')
    + 'border-bottom:2px solid var(--accent);'
    + 'padding:7px 20px;display:flex;align-items:center;gap:14px;'
    + 'font-family:var(--font-mono);font-size:11px;'
    + 'box-shadow:0 2px 12px ' + (isLight ? 'rgba(217,79,58,0.15)' : 'rgba(0,200,232,0.2)') + ';'
    + 'animation:wzPulse 3s ease-in-out infinite;'
    + (isLight ? 'color:#1a1a2e;' : 'color:var(--text-primary);');
  document.body.appendChild(bar);
  // 主内容区下移避免遮挡
  const main = document.getElementById('mainContent');
  if (main) main.style.paddingTop = '40px';
  renderWizardBar(bar);

  // 第一步强提醒：弹出一个醒目的 toast
  if (wizardIdx === 0) {
    showToast('✦ 欢迎！跟随顶部引导完成首次配置', 'info');
  }

  // 监听面板切换
  const origSwitch = switchPanel;
  switchPanel = function(panelId) {
    origSwitch(panelId);
    autoWizardStep(panelId);
  };
}

function autoWizardStep(panelId) {
  // 根据当前面板自动定位到相关步骤
  const bar = document.getElementById('setupWizard');
  if (!bar) return;
  let found = -1;
  for (let i = 0; i < WIZARD_STEPS.length; i++) {
    if (WIZARD_STEPS[i].panel === panelId) { found = i; break; }
  }
  if (found >= 0) { wizardIdx = found; renderWizardBar(bar); }
}

function renderWizardBar(bar) {
  const s = WIZARD_STEPS[wizardIdx];
  const isFirst = wizardIdx === 0;
  const isLast = wizardIdx === WIZARD_STEPS.length - 1;
  const pct = Math.round((wizardIdx + 1) / WIZARD_STEPS.length * 100);
  const isLightWz = document.documentElement.getAttribute('data-theme') === 'light';
  const tc = isLightWz ? '#1a1a2e' : 'var(--text-primary)';
  const tc2 = isLightWz ? '#4a5568' : 'var(--text-secondary)';
  const tc3 = isLightWz ? '#718096' : 'var(--text-muted)';
  const progBg = isLightWz ? '#e2e8f0' : 'var(--border-subtle)';
  const prevBg = isLightWz ? '#e2e8f0' : 'var(--bg-input)';
  const prevBorder = isLightWz ? '#cbd5e0' : 'var(--border-default)';
  const accentHex = isLightWz ? 'rgba(217,79,58,0.25)' : 'rgba(0,200,232,0.25)';

  let btnHtml = '';
  if (s.quick) {
    btnHtml = '<button id="wzAct" style="flex-shrink:0;padding:3px 10px;font-size:10px;border-radius:var(--radius-sm);cursor:pointer;'
      + 'background:var(--accent-dim);border:1px solid ' + accentHex + ';color:var(--accent);font-family:var(--font-mono);">执行</button>';
  }

  bar.innerHTML = ''
    + '<span style="color:' + tc3 + ';flex-shrink:0;font-size:10px;">' + s.icon + '</span>'
    + '<span style="color:' + tc3 + ';flex-shrink:0;font-size:10px;">' + (wizardIdx + 1) + '/' + WIZARD_STEPS.length + '</span>'
    + '<div style="flex-shrink:0;width:60px;height:2px;background:' + progBg + ';border-radius:1px;">'
    +   '<div style="width:' + pct + '%;height:100%;background:var(--accent);border-radius:1px;transition:width 300ms;"></div>'
    + '</div>'
    + '<b style="flex-shrink:0;color:' + tc + ';">' + s.title + '</b>'
    + '<span style="flex:1;color:' + tc2 + ';min-width:0;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;">' + s.desc + '</span>'
    + btnHtml
    + (!isFirst ? '<button id="wzPrev" style="flex-shrink:0;padding:3px 8px;font-size:10px;border-radius:var(--radius-sm);cursor:pointer;'
      + 'background:' + prevBg + ';border:1px solid ' + prevBorder + ';color:' + tc2 + ';font-family:var(--font-mono);">←</button>' : '')
    + (!isLast ? '<button id="wzNext" style="flex-shrink:0;padding:3px 8px;font-size:10px;border-radius:var(--radius-sm);cursor:pointer;'
      + 'background:var(--accent);border:1px solid var(--accent);color:#fff;font-family:var(--font-mono);">→</button>'
      : '<button id="wzDone" style="flex-shrink:0;padding:3px 10px;font-size:10px;border-radius:var(--radius-sm);cursor:pointer;'
      + 'background:var(--green);border:1px solid var(--green);color:#fff;font-family:var(--font-mono);">完成</button>')
    + '<button id="wzClose" style="flex-shrink:0;background:none;border:none;color:var(--text-muted);cursor:pointer;font-size:12px;padding:2px;">×</button>';

  // 事件绑定
  document.getElementById('wzClose').onclick = () => { bar.remove(); const m = document.getElementById('mainContent'); if (m) m.style.paddingTop = ''; };
  document.getElementById('wzPrev')?.addEventListener('click', () => { if (wizardIdx > 0) { wizardIdx--; goWizardStep(); } });
  document.getElementById('wzNext')?.addEventListener('click', () => { if (wizardIdx < WIZARD_STEPS.length - 1) { wizardIdx++; wizardCheer(); goWizardStep(); } });
  document.getElementById('wzDone')?.addEventListener('click', () => {
    deeStorage.setItem('setupCompletedVersion', wizardVer);
    bar.remove(); const m = document.getElementById('mainContent'); if (m) m.style.paddingTop = '';
  });

  const actBtn = document.getElementById('wzAct');
  if (actBtn) {
    actBtn.onclick = async () => {
      actBtn.disabled = true; actBtn.textContent = '...';
      try {
        if (s.quick === 'fetchUpstreamModels') {
          await invoke('fetch_upstream_models', { provider: null });
        } else if (s.quick === 'presetModelMap') {
          await presetModelMapping();
        } else if (s.quick === 'saveConfig') {
          await invoke('save_config', { config: currentConfig });
        } else if (s.quick === 'startService') {
          const st = await invoke('get_service_status');
          if (!st.running) await invoke('start_service');
          for (let j = 0; j < 30; j++) {
            await new Promise(r => setTimeout(r, 500));
            if ((await invoke('get_service_status')).running) break;
          }
          await loadStatus(); renderPanel('status');
        }
        actBtn.textContent = '✓'; actBtn.style.color = 'var(--green)'; actBtn.style.borderColor = 'var(--green)';
        wizardCheer();
      } catch (e) {
        actBtn.textContent = '✗'; actBtn.style.color = 'var(--red)'; actBtn.style.borderColor = 'var(--red)';
        actBtn.title = String(e);
        setTimeout(() => { actBtn.textContent = '重试'; actBtn.style.color = ''; actBtn.style.borderColor = ''; actBtn.disabled = false; }, 3000);
      }
    };
  }
}

function wizardCheer() {
  const cheers = ['做得不错！', '很好，继续！', '太棒了！', '又完成一步！', '厉害！', '加油，快完成了！', '完美！'];
  const msg = cheers[wizardIdx % cheers.length];
  // 短时绿色闪烁
  const bar = document.getElementById('setupWizard');
  if (bar) {
    bar.style.transition = 'background 0.3s';
    bar.style.background = 'rgba(0,214,143,0.3)';
    setTimeout(() => { bar.style.background = 'rgba(255,255,255,0.95)'; }, 600);
  }
  showToast('✓ ' + msg, 'success');
}

function goWizardStep() {
  const s = WIZARD_STEPS[wizardIdx];
  if (s.panel && s.panel !== currentPanel) switchPanel(s.panel);
  else renderWizardBar(document.getElementById('setupWizard'));
}

/* end wizard */
