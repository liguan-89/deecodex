// ═══════════════════════════════════════════════════════════════
// 个人中心 · DEX AI 主账号登录页（动画角色版）
//
// 交互机制参考 21st.dev 社区组件 animated-characters-login-page：
// 四个纯 CSS 角色（紫 / 黑 / 橙 / 黄）跟随鼠标、随机眨眼，并在
// 输入账号、聚焦密码、切换密码可见性时做出对应反应。
// 本文件只作用于个人中心未登录态，不影响导航栏与其他面板。
// 文案全部使用中文。
// ═══════════════════════════════════════════════════════════════

// 角色静态配置：颜色 / 是否随机眨眼 / 是否白眼球（false 为直接瞳孔）
const PROFILE_LOGIN_CHARACTERS = [
  { id: 'purple', color: '#6C3FF5', blinks: true, ballEyes: true },
  { id: 'black', color: '#2D2D2D', blinks: true, ballEyes: true },
  { id: 'orange', color: '#FF9B6B', blinks: false, ballEyes: false },
  { id: 'yellow', color: '#E8D754', blinks: false, ballEyes: false }
];

const PROFILE_LOGIN_EMAIL_REGEX = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

let profileLoginState = {
  mounted: false,
  mode: 'login', // 'login' | 'register'
  mouseX: 0,
  mouseY: 0,
  isTyping: false,
  isPasswordFocused: false,
  showPassword: false,
  passwordLength: 0,
  purpleBlinking: false,
  blackBlinking: false,
  purplePeeking: false,
  lookingAtEachOther: false,
  error: '',
  // 登录 / 注册必须勾选同意《隐私政策》与《服务条款》，会话内记住勾选状态。
  agreedLogin: false,
  agreedRegister: false
};

let profileLoginBlinkTimer = null;
let profileLoginResizeTimer = null;

// ── 渲染：未登录态登录页（左右分栏：左侧角色区 + 右侧表单） ──

function profileLoginRender() {
  const characters = PROFILE_LOGIN_CHARACTERS.map(function (char) {
    const eyes = [0, 1].map(function () {
      return '<div class="pl-eye">'
        + (char.ballEyes
          ? '<span class="pl-pupil"></span>'
          : '<span class="pl-pupil pl-pupil-raw"></span>')
        + '</div>';
    }).join('');
    return '<div class="pl-char" data-char="' + char.id + '">'
      + '<div class="pl-eyes" data-eyes="' + char.id + '">' + eyes + '</div>'
      + '</div>';
  }).join('');

  return '<div class="pl-login-shell">'
    + '<div class="pl-login-panel">'
    + '<div class="pl-left">'
    + '<div class="pl-brand">'
    + '<span class="pl-brand-mark" aria-hidden="true">'
    + '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round">'
    + '<path d="M12 2L15 9H9L12 2Z"/><path d="M12 22L9 15H15L12 22Z"/>'
    + '<path d="M2 12L9 9V15L2 12Z"/><path d="M22 12L15 15V9L22 12Z"/>'
    + '</svg>'
    + '</span>'
    + '<span>DEX AI</span>'
    + '</div>'
    + '<div class="pl-stage"><div class="pl-characters" id="plCharacters">' + characters + '</div></div>'
    + '<div class="pl-footer-links">'
    + '<button type="button" class="pl-link-btn" onclick="profileLoginContactAuthor()">联系作者</button>'
    + '<span class="pl-dot">·</span>'
    + '<a class="pl-link" href="#" onclick="event.preventDefault(); window.DexLegal && window.DexLegal.open(\'privacy\'); return false;">隐私政策</a>'
    + '<span class="pl-dot">·</span>'
    + '<a class="pl-link" href="#" onclick="event.preventDefault(); window.DexLegal && window.DexLegal.open(\'tos\'); return false;">服务条款</a>'
    + '</div>'
    + '<span class="pl-blur pl-blur-1" aria-hidden="true"></span>'
    + '<span class="pl-blur pl-blur-2" aria-hidden="true"></span>'
    + '</div>'
    + '<div class="pl-right">'
    + profileLoginRenderForm(profileLoginState.mode)
    + '</div>'
    + '</div>'
    + '</div>';
}

// ── 表单：登录 / 注册（同一右栏切换，保持左右分栏全屏） ────────

function profileLoginRenderForm(mode) {
  if (mode === 'register') {
    return '<div class="pl-form-head">'
      + '<h3>注册 DEX AI</h3>'
      + '<p>创建你的 DEX AI 主账号</p>'
      + '</div>'
      + '<form class="pl-form pl-form-dense" onsubmit="return profileLoginRegisterSubmit(event)">'
      + '<label class="pl-field"><span>邀请码</span>'
      + '<input id="pfRegisterInvitation" class="pl-input" autocomplete="off" placeholder="邀请码" onfocus="profileLoginTyping(true)" onblur="profileLoginTyping(false)">'
      + '<span class="pl-field-hint">邀请码请联系作者获取</span>'
      + '</label>'
      + '<label class="pl-field"><span>邮箱</span>'
      + '<input id="pfRegisterEmail2" class="pl-input" type="email" autocomplete="email" placeholder="name@example.com" onfocus="profileLoginTyping(true)" onblur="profileLoginTyping(false)">'
      + '</label>'
      + '<label class="pl-field"><span>密码</span>'
      + '<input id="pfRegisterPassword2" class="pl-input" type="password" autocomplete="new-password" placeholder="至少 12 位" onfocus="profileLoginPasswordFocus(true)" onblur="profileLoginPasswordFocus(false)" oninput="profileLoginPasswordInput(this.value)">'
      + '</label>'
      + '<label class="pl-field"><span>确认密码</span>'
      + '<input id="pfRegisterPasswordConfirm" class="pl-input" type="password" autocomplete="new-password" placeholder="再次输入密码" onfocus="profileLoginPasswordFocus(true)" onblur="profileLoginPasswordFocus(false)" oninput="profileLoginPasswordInput(this.value)">'
    + '</label>'
    + '<div class="pl-error" id="plRegisterError" role="alert"></div>'
    + '<label class="pl-agree">'
    + '<input id="pfRegisterAgree" type="checkbox" ' + (profileLoginState.agreedRegister ? 'checked' : '') + ' onchange="profileLoginSetAgreed(\'register\', this.checked)">'
    + '<span>我已阅读并同意 <a href="#" onclick="event.preventDefault(); window.DexLegal && window.DexLegal.open(\'privacy\'); return false;">《隐私政策》</a>和<a href="#" onclick="event.preventDefault(); window.DexLegal && window.DexLegal.open(\'tos\'); return false;">《服务条款》</a></span>'
    + '</label>'
    + '<button type="submit" class="pl-btn pl-btn-primary" id="plRegisterBtn">'
    + '<span class="pl-btn-text">提交注册</span>'
    + '</button>'
      + '<button type="button" class="pl-btn pl-btn-outline" onclick="profileLoginShowLogin()">返回登录</button>'
      + '</form>';
  }
  return '<div class="pl-form-head">'
    + '<h3>登录 DEX AI</h3>'
    + '<p>欢迎回来，请输入你的账号信息</p>'
    + '</div>'
    + '<form class="pl-form" onsubmit="return profileLoginSubmit(event)">'
    + '<label class="pl-field"><span>账号</span>'
    + '<input id="pfMasterEmail" class="pl-input" type="email" autocomplete="username" placeholder="邮箱或手机号" onfocus="profileLoginTyping(true)" onblur="profileLoginTyping(false)">'
    + '</label>'
    + '<label class="pl-field"><span>密码</span>'
    + '<span class="pl-password-wrap">'
    + '<input id="pfMasterPassword" class="pl-input pl-password-input" type="password" autocomplete="current-password" placeholder="至少 6 位" onfocus="profileLoginPasswordFocus(true)" onblur="profileLoginPasswordFocus(false)" oninput="profileLoginPasswordInput(this.value)">'
    + '<button type="button" class="pl-eye-toggle" onclick="profileLoginTogglePassword()" aria-label="显示或隐藏密码" tabindex="-1">'
    + '<svg class="pl-eye-icon pl-eye-open" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">'
    + '<path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/>'
    + '</svg>'
    + '<svg class="pl-eye-icon pl-eye-closed" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">'
    + '<path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24"/>'
    + '<line x1="1" y1="1" x2="23" y2="23"/>'
    + '</svg>'
    + '</button>'
    + '</span>'
    + '</label>'
    + '<div class="pl-error" id="plLoginError" role="alert"></div>'
    + '<label class="pl-agree">'
    + '<input id="pfLoginAgree" type="checkbox" ' + (profileLoginState.agreedLogin ? 'checked' : '') + ' onchange="profileLoginSetAgreed(\'login\', this.checked)">'
    + '<span>我已阅读并同意 <a href="#" onclick="event.preventDefault(); window.DexLegal && window.DexLegal.open(\'privacy\'); return false;">《隐私政策》</a>和<a href="#" onclick="event.preventDefault(); window.DexLegal && window.DexLegal.open(\'tos\'); return false;">《服务条款》</a></span>'
    + '</label>'
    + '<button type="submit" class="pl-btn pl-btn-primary" id="plLoginBtn">'
    + '<span class="pl-btn-text">登录</span>'
    + '<svg class="pl-btn-arrow" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">'
    + '<line x1="5" y1="12" x2="19" y2="12"/><polyline points="12 5 19 12 12 19"/>'
    + '</svg>'
    + '</button>'
    + '<button type="button" class="pl-btn pl-btn-outline" onclick="profileLoginRegister()">注册</button>'
    + '</form>';
}

// ── 生命周期：挂载 / 卸载 ──────────────────────────────────────

function profileLoginMounted() {
  return Boolean(document.getElementById('plCharacters'));
}

function profileLoginHydrate() {
  if (!profileLoginMounted()) return;
  if (profileLoginState.mounted) return;
  profileLoginState.mounted = true;
  profileLoginState.mouseX = 0;
  profileLoginState.mouseY = 0;
  window.addEventListener('mousemove', profileLoginOnMouseMove);
  window.addEventListener('resize', profileLoginOnResize);
  profileLoginScheduleBlink();
  profileLoginFitStage();
  profileLoginApplyState();
}

function profileLoginTeardown() {
  if (!profileLoginState.mounted) return;
  profileLoginState.mounted = false;
  window.removeEventListener('mousemove', profileLoginOnMouseMove);
  window.removeEventListener('resize', profileLoginOnResize);
  if (profileLoginBlinkTimer) {
    clearTimeout(profileLoginBlinkTimer);
    profileLoginBlinkTimer = null;
  }
}

// ── 鼠标跟随 ──────────────────────────────────────────────────

function profileLoginOnMouseMove(event) {
  if (!profileLoginMounted()) return;
  profileLoginState.mouseX = event.clientX;
  profileLoginState.mouseY = event.clientY;
  profileLoginApplyState();
}

function profileLoginOnResize() {
  if (!profileLoginMounted()) return;
  if (profileLoginResizeTimer) clearTimeout(profileLoginResizeTimer);
  profileLoginResizeTimer = setTimeout(function () {
    profileLoginResizeTimer = null;
    if (profileLoginMounted()) profileLoginFitStage();
  }, 120);
}

// 按舞台实际宽度缩放角色画布（550x400），让角色区随面板宽度填满。
function profileLoginFitStage() {
  const stage = document.querySelector('.pl-stage');
  const chars = document.querySelector('.pl-characters');
  if (!stage || !chars) return;
  const width = stage.clientWidth || 360;
  const scale = Math.max(0.4, Math.min(1.25, width / 550));
  chars.style.transform = 'scale(' + scale + ')';
  stage.style.height = Math.round(400 * scale) + 'px';
}

function profileLoginMousePos(charEl) {
  const rect = charEl.getBoundingClientRect();
  const centerX = rect.left + rect.width / 2;
  const centerY = rect.top + rect.height / 3;
  const deltaX = profileLoginState.mouseX - centerX;
  const deltaY = profileLoginState.mouseY - centerY;
  return {
    faceX: Math.max(-15, Math.min(15, deltaX / 20)),
    faceY: Math.max(-10, Math.min(10, deltaY / 30)),
    bodySkew: Math.max(-6, Math.min(6, -deltaX / 120))
  };
}

function profileLoginFollowMouse(pupil, charId) {
  const rect = pupil.getBoundingClientRect();
  const centerX = rect.left + rect.width / 2;
  const centerY = rect.top + rect.height / 2;
  const deltaX = profileLoginState.mouseX - centerX;
  const deltaY = profileLoginState.mouseY - centerY;
  const distance = Math.min(Math.sqrt(deltaX * deltaX + deltaY * deltaY), 5);
  const angle = Math.atan2(deltaY, deltaX);
  pupil.style.transform = 'translate(' + (Math.cos(angle) * distance) + 'px,' + (Math.sin(angle) * distance) + 'px)';
}

// ── 状态应用：按当前状态更新角色身体 / 眼睛 / 瞳孔 ──────────────

function profileLoginApplyState() {
  const state = profileLoginState;
  const hiding = state.passwordLength > 0 && !state.showPassword;
  const lookingAway = state.isPasswordFocused && !state.showPassword;
  const showing = state.passwordLength > 0 && state.showPassword;

  PROFILE_LOGIN_CHARACTERS.forEach(function (char) {
    const charEl = document.querySelector('.pl-char[data-char="' + char.id + '"]');
    if (!charEl) return;
    const eyes = charEl.querySelector('.pl-eyes');
    const pos = profileLoginMousePos(charEl);
    let bodyTransform = null;
    let height = null;
    let eyesLeft = null;
    let eyesTop = null;
    let pupilForce = null;

    if (char.id === 'purple') {
      height = (lookingAway || state.isTyping || hiding) ? '440px' : '400px';
      if (showing) bodyTransform = 'skewX(0deg)';
      else if (lookingAway) bodyTransform = 'skewX(-14deg) translateX(-20px)';
      else if (state.isTyping || hiding) bodyTransform = 'skewX(' + (pos.bodySkew - 12) + 'deg) translateX(40px)';
      else bodyTransform = 'skewX(' + pos.bodySkew + 'deg)';
      eyesLeft = lookingAway ? 20 : (showing ? 20 : (state.lookingAtEachOther ? 55 : 45 + pos.faceX));
      eyesTop = lookingAway ? 25 : (showing ? 35 : (state.lookingAtEachOther ? 65 : 40 + pos.faceY));
      if (lookingAway) pupilForce = { x: -5, y: -5 };
      else if (showing) pupilForce = { x: state.purplePeeking ? 4 : -4, y: state.purplePeeking ? 5 : -4 };
      else if (state.lookingAtEachOther) pupilForce = { x: 3, y: 4 };
    } else if (char.id === 'black') {
      if (showing) bodyTransform = 'skewX(0deg)';
      else if (lookingAway) bodyTransform = 'skewX(12deg) translateX(-10px)';
      else if (state.lookingAtEachOther) bodyTransform = 'skewX(' + (pos.bodySkew * 1.5 + 10) + 'deg) translateX(20px)';
      else bodyTransform = 'skewX(' + (pos.bodySkew * 1.5) + 'deg)';
      eyesLeft = lookingAway ? 10 : (showing ? 10 : (state.lookingAtEachOther ? 32 : 26 + pos.faceX));
      eyesTop = lookingAway ? 20 : (showing ? 28 : (state.lookingAtEachOther ? 12 : 32 + pos.faceY));
      if (lookingAway) pupilForce = { x: -4, y: -5 };
      else if (showing) pupilForce = { x: -4, y: -4 };
      else if (state.lookingAtEachOther) pupilForce = { x: 0, y: -4 };
    } else if (char.id === 'orange') {
      bodyTransform = showing ? 'skewX(0deg)' : 'skewX(' + pos.bodySkew + 'deg)';
      eyesLeft = lookingAway ? 50 : (showing ? 50 : 82 + pos.faceX);
      eyesTop = lookingAway ? 75 : (showing ? 85 : 90 + pos.faceY);
      if (lookingAway || showing) pupilForce = { x: -5, y: lookingAway ? -5 : -4 };
    } else if (char.id === 'yellow') {
      bodyTransform = showing ? 'skewX(0deg)' : 'skewX(' + pos.bodySkew + 'deg)';
      eyesLeft = lookingAway ? 20 : (showing ? 20 : 52 + pos.faceX);
      eyesTop = lookingAway ? 30 : (showing ? 35 : 40 + pos.faceY);
      if (lookingAway || showing) pupilForce = { x: -5, y: lookingAway ? -5 : -4 };
    }

    if (bodyTransform !== null) charEl.style.transform = bodyTransform;
    if (height !== null) charEl.style.height = height;
    if (eyes && eyesLeft !== null) eyes.style.left = eyesLeft + 'px';
    if (eyes && eyesTop !== null) eyes.style.top = eyesTop + 'px';

    if (eyes) {
      const pupils = eyes.querySelectorAll('.pl-pupil');
      pupils.forEach(function (pupil) {
        if (pupilForce) {
          pupil.style.transform = 'translate(' + pupilForce.x + 'px,' + pupilForce.y + 'px)';
        } else {
          profileLoginFollowMouse(pupil, char.id);
        }
      });
    }
  });
}

// ── 眨眼 ──────────────────────────────────────────────────────

function profileLoginScheduleBlink() {
  if (!profileLoginState.mounted) return;
  const char = Math.random() < 0.5 ? 'purple' : 'black';
  const delay = 2500 + Math.random() * 3500;
  profileLoginBlinkTimer = setTimeout(function () {
    if (!profileLoginMounted()) {
      profileLoginBlinkTimer = null;
      return;
    }
    profileLoginBlink(char, true);
    setTimeout(function () { profileLoginBlink(char, false); }, 150);
    profileLoginScheduleBlink();
  }, delay);
}

function profileLoginBlink(charId, blinking) {
  const charEl = document.querySelector('.pl-char[data-char="' + charId + '"]');
  if (!charEl) return;
  charEl.classList.toggle('is-blinking', blinking);
}

// ── 表单状态联动 ──────────────────────────────────────────────

function profileLoginTyping(isTyping) {
  profileLoginState.isTyping = isTyping;
  if (isTyping) {
    profileLoginState.lookingAtEachOther = true;
    setTimeout(function () {
      profileLoginState.lookingAtEachOther = false;
      profileLoginApplyState();
    }, 800);
  } else {
    profileLoginState.lookingAtEachOther = false;
  }
  profileLoginApplyState();
}

function profileLoginPasswordFocus(focused) {
  profileLoginState.isPasswordFocused = focused;
  profileLoginApplyState();
}

function profileLoginPasswordInput(value) {
  profileLoginState.passwordLength = String(value || '').length;
  profileLoginApplyState();
}

function profileLoginTogglePassword() {
  profileLoginState.showPassword = !profileLoginState.showPassword;
  const input = document.getElementById('pfMasterPassword');
  if (input) input.type = profileLoginState.showPassword ? 'text' : 'password';
  const toggle = document.querySelector('.pl-eye-toggle');
  if (toggle) toggle.classList.toggle('is-open', profileLoginState.showPassword);
  if (profileLoginState.showPassword && profileLoginState.passwordLength > 0) {
    setTimeout(function () {
      profileLoginState.purplePeeking = true;
      profileLoginApplyState();
      setTimeout(function () {
        profileLoginState.purplePeeking = false;
        profileLoginApplyState();
      }, 800);
    }, 200);
  }
  profileLoginApplyState();
}

// ── 表单动作 ──────────────────────────────────────────────────

function profileLoginSubmit(event) {
  if (event && event.preventDefault) event.preventDefault();
  const email = String((document.getElementById('pfMasterEmail') || {}).value || '').trim();
  const password = String((document.getElementById('pfMasterPassword') || {}).value || '');
  profileLoginSetError('');
  if (!profileLoginState.agreedLogin) {
    profileLoginSetError('请先阅读并勾选同意《隐私政策》和《服务条款》后再登录');
    profileLoginShake();
    return false;
  }
  if (!email || !PROFILE_LOGIN_EMAIL_REGEX.test(email)) {
    profileLoginSetError('请输入有效的邮箱地址');
    profileLoginShake();
    return false;
  }
  if (!password || password.length < 6) {
    profileLoginSetError('密码至少 6 位');
    profileLoginShake();
    return false;
  }
  profileLoginSetLoading(true);
  profileMasterLogin();
  return false;
}

function profileLoginFail(message) {
  profileLoginSetLoading(false);
  profileLoginSetError('登录失败：' + String(message || '未知错误'));
  profileLoginShake();
}

function profileLoginRegister() {
  profileLoginState.mode = 'register';
  profileLoginSwitchForm();
}

function profileLoginShowLogin() {
  profileLoginState.mode = 'login';
  profileLoginSwitchForm();
}

function profileLoginSetAgreed(kind, checked) {
  if (kind === 'register') {
    profileLoginState.agreedRegister = Boolean(checked);
  } else {
    profileLoginState.agreedLogin = Boolean(checked);
  }
}

function profileLoginSwitchForm() {
  const right = document.querySelector('.pl-right');
  if (right) right.innerHTML = profileLoginRenderForm(profileLoginState.mode);
  profileLoginSetError('');
  const first = right && right.querySelector('input');
  if (first && typeof first.focus === 'function') first.focus();
}

function profileLoginRegisterSubmit(event) {
  if (event && event.preventDefault) event.preventDefault();
  const invitation = String((document.getElementById('pfRegisterInvitation') || {}).value || '').trim();
  const email = String((document.getElementById('pfRegisterEmail2') || {}).value || '').trim();
  const password = String((document.getElementById('pfRegisterPassword2') || {}).value || '');
  const confirm = String((document.getElementById('pfRegisterPasswordConfirm') || {}).value || '');
  const errorEl = document.getElementById('plRegisterError');
  const setError = function (message) {
    profileLoginState.error = String(message || '');
    if (errorEl) {
      errorEl.textContent = String(message || '');
      errorEl.style.display = message ? 'block' : 'none';
    }
    if (message) profileLoginShake();
  };
  setError('');
  if (!profileLoginState.agreedRegister) {
    setError('请先阅读并勾选同意《隐私政策》和《服务条款》后再注册');
    return false;
  }
  if (!invitation) { setError('请填写邀请码'); return false; }
  if (!PROFILE_LOGIN_EMAIL_REGEX.test(email)) { setError('请输入有效的邮箱地址'); return false; }
  if (password.length < 12) { setError('密码至少需要 12 个字符'); return false; }
  if (password !== confirm) { setError('两次输入的密码不一致'); return false; }
  const button = document.getElementById('plRegisterBtn');
  if (button) {
    button.disabled = true;
    button.textContent = '注册中…';
  }
  invoke('dexai_master_register', { invitationCode: invitation, email: email, password: password })
    .then(function (result) {
      showToast('注册成功，正在登录…', 'success');
      return invoke('dexai_master_login', { email: email, password: password });
    })
    .then(function () {
      if (button) { button.disabled = false; button.textContent = '提交注册'; }
      if (typeof profileMasterRerender === 'function') profileMasterRerender();
    })
    .catch(function (error) {
      if (button) { button.disabled = false; button.textContent = '提交注册'; }
      const message = String(error || '注册失败');
      if (message.indexOf('注册') === 0 || message.indexOf('请') === 0 || message.indexOf('无法') === 0) {
        setError(message);
      } else {
        // 注册成功但登录被拒（如邮箱未验证）：提示并切回登录。
        showToast('注册成功，请登录（如提示需验证请先查收邮件）', 'info');
        profileLoginState.mode = 'login';
        profileLoginSwitchForm();
        const emailInput = document.getElementById('pfMasterEmail');
        if (emailInput) emailInput.value = email;
      }
    });
  return false;
}

function profileLoginContactAuthor() {
  const content = document.getElementById('qrOverlayContent');
  const title = document.querySelector('.qr-overlay .qr-title');
  const account = document.getElementById('qrOverlayAccount');
  if (content) {
    content.innerHTML = '<img src="assets/qr-author.png" alt="联系作者二维码">'
      + '<p class="qr-hint">微信扫码添加作者，获取邀请码或反馈问题</p>';
  }
  document.querySelector('.qr-dialog')?.classList.remove('qr-dialog-wide');
  if (title) title.textContent = '扫码联系作者';
  if (account) account.textContent = '';
  if (typeof showQrOverlay === 'function') showQrOverlay();
  else showToast('联系作者：请通过 GitHub Issues 提交反馈', 'info');
}

function profileLoginSetError(message) {
  profileLoginState.error = String(message || '');
  const el = document.getElementById('plLoginError');
  if (el) {
    el.textContent = profileLoginState.error;
    el.style.display = profileLoginState.error ? 'block' : 'none';
  }
}

function profileLoginSetLoading(loading) {
  const btn = document.getElementById('plLoginBtn');
  if (!btn) return;
  btn.disabled = Boolean(loading);
  btn.classList.toggle('is-loading', Boolean(loading));
  const text = btn.querySelector('.pl-btn-text');
  if (text) text.textContent = loading ? '登录中…' : '登录';
}

function profileLoginShake() {
  const panel = document.querySelector('.pl-login-panel');
  if (!panel) return;
  panel.classList.remove('is-shaking');
  void panel.offsetWidth;
  panel.classList.add('is-shaking');
  setTimeout(function () { panel.classList.remove('is-shaking'); }, 450);
}
