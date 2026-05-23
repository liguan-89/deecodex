const WIZARD_STEPS = [
  { id: 'wz1', step: '01', title: '添加账号', desc: '选择供应商并创建账号', panel: 'accounts', quick: '' },
  { id: 'wz2', step: '02', title: '填写密钥', desc: '填入 API Key 和上游 URL', panel: 'accounts', quick: '' },
  { id: 'wz3', step: '03', title: '同步模型', desc: '拉取上游可用模型', panel: 'accounts', quick: 'fetchUpstreamModels' },
  { id: 'wz4', step: '04', title: '模型映射', desc: '设置 Codex 模型到上游模型的对应关系', panel: 'accounts', quick: 'presetModelMap' },
  { id: 'wz5', step: '05', title: '保存配置', desc: '写入当前账号配置', panel: 'accounts', quick: 'saveConfig' },
  { id: 'wz6', step: '06', title: '启动顺序', desc: '先启动 deecodex，再打开 Codex', panel: '', quick: '' },
  { id: 'wz7', step: '07', title: '启动服务', desc: '确认服务状态正常', panel: 'status', quick: 'startService' },
  { id: 'wz8', step: '08', title: '运行诊断', desc: '验证上游连通与模型可用', panel: 'diagnostics', quick: '' },
];
let wizardIdx = 0;
let wizardVer = '';

async function checkSetupWizard() {
  hideWizardBar();
}

function showWizardBar() {
  hideWizardBar();
  return;
}

function hideWizardBar() {
  const existing = document.getElementById('setupWizard');
  if (existing) existing.remove();
  const main = document.getElementById('mainContent');
  if (main) main.style.paddingTop = '';
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
  let btnHtml = '';
  if (s.quick) {
    btnHtml = '<button id="wzAct" class="setup-wizard-btn setup-wizard-btn-act">执行</button>';
  }

  bar.innerHTML = ''
    + '<span class="setup-wizard-icon">' + s.step + '</span>'
    + '<span class="setup-wizard-count">' + (wizardIdx + 1) + '/' + WIZARD_STEPS.length + '</span>'
    + '<div class="setup-wizard-progress">'
    +   '<div style="width:' + pct + '%"></div>'
    + '</div>'
    + '<b class="setup-wizard-title">' + s.title + '</b>'
    + '<span class="setup-wizard-desc">' + s.desc + '</span>'
    + btnHtml
    + (!isFirst ? '<button id="wzPrev" class="setup-wizard-btn setup-wizard-btn-prev">上一步</button>' : '')
    + (!isLast ? '<button id="wzNext" class="setup-wizard-btn setup-wizard-btn-next">下一步</button>'
      : '<button id="wzDone" class="setup-wizard-btn setup-wizard-btn-done">完成</button>')
    + '<button id="wzClose" class="setup-wizard-close" aria-label="关闭引导">×</button>';

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
          const account = await invoke('get_active_account');
          await invoke('fetch_upstream_models', { accountId: account.id });
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
        actBtn.textContent = '完成'; actBtn.style.color = 'var(--green)'; actBtn.style.borderColor = 'var(--green)';
        wizardCheer();
      } catch (e) {
        actBtn.textContent = '失败'; actBtn.style.color = 'var(--red)'; actBtn.style.borderColor = 'var(--red)';
        actBtn.title = String(e);
        setTimeout(() => { actBtn.textContent = '重试'; actBtn.style.color = ''; actBtn.style.borderColor = ''; actBtn.disabled = false; }, 3000);
      }
    };
  }
  updateWizardMainOffset(bar);
}

function updateWizardMainOffset(bar) {
  const main = document.getElementById('mainContent');
  if (!main || !bar) return;
  const height = Math.ceil(bar.getBoundingClientRect().height || 0);
  main.style.paddingTop = `${height + 18}px`;
}

function wizardCheer() {
  const cheers = ['做得不错！', '很好，继续！', '太棒了！', '又完成一步！', '厉害！', '加油，快完成了！', '完美！'];
  const msg = cheers[wizardIdx % cheers.length];
  // 短时绿色闪烁
  const bar = document.getElementById('setupWizard');
  if (bar) {
    bar.style.transition = 'background 0.3s';
    bar.style.background = 'rgba(0,214,143,0.3)';
    setTimeout(() => { bar.style.background = ''; }, 600);
  }
  showToast(msg, 'success');
}

function goWizardStep() {
  const s = WIZARD_STEPS[wizardIdx];
  if (s.panel && s.panel !== currentPanel) switchPanel(s.panel);
  else renderWizardBar(document.getElementById('setupWizard'));
}

/* end wizard */
