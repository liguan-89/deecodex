// ═══════════════════════════════════════════════════════════════
// 个人中心 · 隐私政策 / 服务条款（静态内容与弹窗）
//
// 约定：
// - 本文件只定义数据常量与纯渲染函数，不在加载时访问 DOM / Tauri / localStorage；
// - 内容按项目实际业务风险编写（本地代理、多账号/中转、调试助手、
//   微信通道、浏览器注入、广告位、云同步、匿名统计、强制更新等）；
// - 弹窗复用 index.html 的 qrOverlay 容器，样式见 overlays.css 的 .legal-dialog。
// ═══════════════════════════════════════════════════════════════
(function () {
  'use strict';

  const PRIVACY = {
    kind: 'privacy',
    title: '隐私政策',
    version: 'v2.0',
    date: '2026-08-19',
    sections: [
      {
        heading: '一、概述',
        blocks: [
          { type: 'p', text: 'DEX AI（以下简称「本产品」或「我们」）是本地代理与桌面工具套件：将 Codex 等客户端的请求翻译并转发至您配置的上游模型服务商，并提供账号管理、插件授权、多账号与中转渠道、配置同步、调试助手与桌面管理能力。' },
          { type: 'p', text: '本政策说明我们如何处理您的信息。核心原则：您的对话内容只在您本机与您主动选择的上游服务商之间流转；我们不读取、不存储、不上传您的 Prompt、回复与线程内容。' }
        ]
      },
      {
        heading: '二、我们收集的信息',
        blocks: [
          { type: 'p', text: '1. 您主动提供的信息' },
          {
            type: 'ul',
            items: [
              { lead: '账号信息', text: 'DEX AI 主账号/Mobile Codex 账号的邮箱与密码（密码经安全哈希处理；前端不保存明文，凭证由本机插件持久化于私有目录）。' },
              { lead: '设备信息', text: '设备 ID、平台、客户端类型、设备名称，用于设备管理与安全校验。' },
              { lead: '授权信息', text: '您对插件/应用授予的授权范围（scope，如 dexai:vault）、授权与撤销记录（审计日志）。' },
              { lead: '上游账号信息', text: '您自行配置的模型服务商 API Key、中转渠道地址与密钥。此类信息仅存储于本机（~/.deecodex/），界面显示时做脱敏处理。' }
            ]
          },
          { type: 'p', text: '2. 本地处理、不对外收集的信息' },
          {
            type: 'ul',
            items: [
              { lead: '对话内容', text: 'Prompt、回复、线程内容仅在本地代理与您选择的上游服务商之间传输，我们不在本产品侧存储、统计或上传。' },
              { lead: '本地配置与日志', text: 'config.json、accounts.json、请求历史、运行日志等默认仅保存在本机，由您自行管理。' },
              { lead: '调试助手读取的文件', text: '仅当您主动发起调试/修复并授权时，调试助手才读取受限目录（~/.deecodex、~/.codex、/tmp、当前工作区）内的文件；我们不会将其上传或统计。' }
            ]
          },
          { type: 'p', text: '3. 匿名使用统计（默认开启，可随时关闭）' },
          { type: 'p', text: '仅当您保持开启时，我们收集：安装标识（install_id）、会话标识（session_id）、应用版本、操作系统与架构；页面 PV/UV、广告位曝光与点击、部分业务事件及少量非敏感属性。' },
          { type: 'p', text: '我们不采集：API Key、Token、密码、Prompt、消息/线程内容、账号邮箱、本地路径或文件名（前后端均显式脱敏）。您可在「个人中心 → 匿名使用统计」关闭；关闭后不再上报，本地缓存事件将被丢弃。' },
          { type: 'p', text: '4. 云同步数据（默认关闭，需主动开启）' },
          { type: 'p', text: '开启账号云同步后，accounts.json 与 config.json 使用您设置的云同步密码做端到端加密后上传，云端仅存密文，不存明文与密码。请牢记云同步密码，丢失将无法恢复。您可随时「清除云端副本并关闭」。' },
          { type: 'p', text: '5. 广告与第三方内容' },
          { type: 'p', text: '应用内可能展示广告位（如服务概览卡片）。广告位的曝光与点击仅按上述匿名统计规则上报（不携带您的账号、对话或设备敏感信息）。如广告跳转至第三方页面，该页面的数据收集受第三方隐私政策约束，请您自行留意。' }
        ]
      },
      {
        heading: '三、信息的使用',
        blocks: [
          {
            type: 'ul',
            items: [
              '提供登录、授权、设备管理、云同步、多账号与中转渠道管理；',
              '向您选择的上游服务商转发您主动发起的请求；',
              '运行您授权的插件与调试助手功能；',
              '匿名统计用于产品改进、故障排查与广告效果衡量；',
              '安全防护：识别异常登录、撤销泄露设备、审计授权与高危操作。'
            ]
          }
        ]
      },
      {
        heading: '四、信息的共享与第三方',
        blocks: [
          { type: 'p', text: '我们不出售个人信息。可能交互的第三方包括：' },
          {
            type: 'ul',
            items: [
              { lead: '上游模型服务商', text: '按您发起的请求接收对应对话内容，处理受其自身政策约束；' },
              { lead: '中转渠道服务方', text: '您配置的中转渠道将作为请求路径，数据按该渠道规则处理；' },
              { lead: '控制平面', text: 'api.liguan.me 提供账号认证、设备与会话管理；' },
              { lead: 'iLink Bot 等第三方消息服务', text: '使用微信通道时，消息经其 API 收发，该通道收到的消息内容对第三方服务可见，请勿在其中传输敏感信息；' },
              { lead: '匿名统计服务', text: '仅接收匿名化事件；' },
              { lead: '支付渠道', text: '支付由微信/支付宝等按其政策处理，我们仅获取交易结果。' }
            ]
          }
        ]
      },
      {
        heading: '五、信息的存储与期限',
        blocks: [
          {
            type: 'ul',
            items: [
              '本地数据存于 ~/.deecodex/，卸载后残留数据需手动删除；',
              '云同步密文存于云端，直到您清除副本或注销账号；',
              '匿名统计事件按运营需要保留，用于统计与故障排查。'
            ]
          }
        ]
      },
      {
        heading: '六、您的权利',
        blocks: [
          {
            type: 'ul',
            items: [
              '关闭匿名统计：个人中心 → 匿名使用统计；',
              '关闭/清除云同步：个人中心 → 账号云同步；',
              '撤销授权：个人中心 → 已授权应用 → 撤销（令牌立即失效）；',
              '管理设备：个人中心 → Mobile Codex 设备与服务 → 重命名/撤销设备；',
              '退出登录：清除本机登录态并停止相关服务；',
              '删除本地数据：删除 ~/.deecodex/ 目录即可清除本地配置、账号、日志与请求历史；',
              '注销账号：按账号服务规则申请，注销后云端副本与授权关系一并清除。'
            ]
          }
        ]
      },
      {
        heading: '七、未成年人保护',
        blocks: [
          { type: 'p', text: '本产品面向具备独立技术能力的成年用户。未满 18 周岁请在监护人陪同下使用，不得处理敏感个人信息，亦不得使用广告/自动化相关功能进行不适宜行为。' }
        ]
      },
      {
        heading: '八、安全措施与您的责任',
        blocks: [
          { type: 'p', text: '我们采取敏感字段脱敏、传输加密（TLS）、云同步端到端加密、授权令牌即时撤销、设备会话管理等措施。同时请您：' },
          {
            type: 'ul',
            items: [
              '妥善保管账号密码、API Key、中转渠道密钥与云同步密码；',
              '使用调试助手、插件授权、浏览器注入等涉及本机资源的功能前，确认操作对象与影响范围；',
              '定期检查已授权应用与绑定设备，撤销可疑授权。'
            ]
          }
        ]
      },
      {
        heading: '九、政策更新',
        blocks: [
          { type: 'p', text: '我们可能适时更新本政策，并在应用内显著位置提示。重大变更（新增采集范围）会征求您的再次确认。' }
        ]
      },
      {
        heading: '十、联系我们',
        blocks: [
          { type: 'p', text: '通过「联系作者」（微信扫码）或 GitHub Issues 与我们联系，我们将在合理期限内回复。' }
        ]
      }
    ]
  };

  const TOS = {
    kind: 'tos',
    title: '服务条款',
    version: 'v2.0',
    date: '2026-08-19',
    sections: [
      {
        heading: '一、服务说明',
        blocks: [
          { type: 'p', text: 'DEX AI 提供：本地代理（客户端与上游模型服务商之间的请求翻译与转发）、桌面图形界面、DEX AI 主账号与插件授权体系、多账号与中转渠道管理、账号云同步、调试助手、微信通道、匿名统计与广告位等功能。' },
          { type: 'p', text: '您理解并同意：本产品是本地代理与增强层，自身不提供大模型能力；实际对话由您选择的上游服务商完成，其可用性与输出质量不由我们保证。' },
          { type: 'p', text: '本产品依赖本机进程持续运行。本地代理（GUI/服务进程）退出后，走该代理的客户端请求将立即失败，这是本地代理架构的预期行为。' }
        ]
      },
      {
        heading: '二、账号、多账号与上游服务',
        blocks: [
          {
            type: 'ul',
            items: [
              '您须对账号下的全部行为负责，且保证对所使用的每个上游账号、API Key 或中转渠道拥有合法使用权；',
              '您不得将本产品用于转售、出租或分发上游模型服务、账号或中转额度；',
              '上游服务商的条款、风控与封禁规则由其独立制定。因您使用多账号、中转渠道或违反上游规则导致的账号封禁、额度损失或服务中断，由您自行承担；我们仅提供工具能力，不承担由此产生的损失；',
              '您应妥善保管所有 API Key 与渠道密钥，界面已做脱敏展示，泄露风险由您自担。'
            ]
          }
        ]
      },
      {
        heading: '三、调试助手与本地工具授权',
        blocks: [
          {
            type: 'ul',
            items: [
              '调试助手可读取受限目录文件、检测进程、执行 Shell 命令与调用插件工具，并采用安全分级（L0–L3）：低风险直接执行、高风险（如 Shell）需您在前端内联确认；',
              '经您确认后执行的任何命令、文件操作与进程操作，后果由您负责；请在执行前核对命令、目标文件与影响范围；',
              '我们不对调试助手执行结果（含代码修改、文件删除、进程终止）造成的任何直接或间接损失承担责任。'
            ]
          }
        ]
      },
      {
        heading: '四、浏览器与网页注入',
        blocks: [
          {
            type: 'ul',
            items: [
              '部分诊断功能可能通过浏览器调试协议读取页面状态或注入脚本，仅用于故障诊断与模型路由；',
              '涉及登录态页面的注入可能影响页面行为，请仅在您完全可控的页面/账号下使用；由此产生的账号风险由您自担。'
            ]
          }
        ]
      },
      {
        heading: '五、微信通道与自动化风险',
        blocks: [
          {
            type: 'ul',
            items: [
              '微信通道通过第三方 iLink Bot 服务收发消息。微信平台对自动化/非官方接入有明确限制，使用本功能存在微信账号被限制、封禁的风险，您须自行评估并承担后果；',
              '您不得利用微信通道进行骚扰、营销轰炸、诈骗或任何违反微信平台规则与法律法规的行为；',
              '通过微信通道传输的消息内容会经第三方服务转发，请勿发送敏感或机密信息；相关隐私处理见《隐私政策》第四节。'
            ]
          }
        ]
      },
      {
        heading: '六、广告与商业内容',
        blocks: [
          {
            type: 'ul',
            items: [
              '应用内广告位用于维持产品运营，广告内容与跳转页面由相关广告主负责，我们不对第三方广告页面内容承担担保责任；',
              '您不得通过脚本、模拟点击等方式虚构广告曝光或点击，否则我们有权限制相关功能或账号；',
              '如您为未成年人，请在监护人指导下使用广告相关功能。'
            ]
          }
        ]
      },
      {
        heading: '七、支付、邀请码与退款',
        blocks: [
          {
            type: 'ul',
            items: [
              '部分功能可能收费，价格以应用内展示为准；支付通过微信/支付宝等第三方渠道完成；',
              '邀请码仅用于邀请与身份核验，不得买卖、倒卖或批量注册，否则我们有权终止相关账号；',
              '虚拟服务一经开通，除法律法规另有规定外原则上不支持无理由退款。'
            ]
          }
        ]
      },
      {
        heading: '八、用户行为规范',
        blocks: [
          { type: 'p', text: '您承诺不利用本产品：' },
          {
            type: 'ul',
            items: [
              '从事违反法律法规或侵犯他人权益的行为（含生成恶意程序、侵权、诈骗、色情、暴力内容）；',
              '滥用、攻击或干扰上游服务商、控制平面、第三方通道及本产品（含绕过限流、反向工程、抓取接口、批量注册）；',
              '在未人工审核的情况下，将 AI 输出直接用于人身、财产重大安全决策。'
            ]
          }
        ]
      },
      {
        heading: '九、AI 输出免责',
        blocks: [
          {
            type: 'ul',
            items: [
              '大模型输出可能错误、过时或具有偏见，不构成法律、医疗、财务等专业意见；',
              '代码类输出须经您自行测试与审查，因直接使用 AI 生成代码造成的损失，我们不承担责任；',
              '请勿向模型输入您无权处理或高敏感的机密信息。'
            ]
          }
        ]
      },
      {
        heading: '十、第三方服务与插件',
        blocks: [
          {
            type: 'ul',
            items: [
              '上游模型商、中转渠道、控制平面、iLink Bot、支付渠道、统计服务等各有独立条款，因第三方中断、变更或终止导致的损失，我们在法律允许范围内不承担责任；',
              '您授权使用的第三方插件由对应提供方负责，其索取的权限与行为受插件自身说明约束。'
            ]
          }
        ]
      },
      {
        heading: '十一、版本更新',
        blocks: [
          { type: 'p', text: '我们可能推送功能更新、安全补丁或强制更新（force update）。强制更新时应用会自动下载并安装、可能重启；您应保证网络与磁盘空间充足，因拒绝更新导致的兼容性或安全问题由您自担。' }
        ]
      },
      {
        heading: '十二、服务变更、中断与终止',
        blocks: [
          {
            type: 'ul',
            items: [
              '我们可能调整、暂停或终止全部或部分功能，并在可行时提前通知；',
              '上游服务商风控、网络故障、第三方通道限制等不可归责于我们的因素导致的服务中断，我们不承担赔偿责任；',
              '若您违反本条款，我们有权限制、撤销授权或终止账号；您可随时停止使用、删除本地数据并注销账号。'
            ]
          }
        ]
      },
      {
        heading: '十三、责任限制',
        blocks: [
          {
            type: 'ul',
            items: [
              '我们对使用或无法使用本产品造成的间接、附带、特殊或惩罚性损失（含数据丢失、业务中断、利润损失、微信/上游账号封禁）不承担责任；',
              '我们的累计赔偿责任不超过您为使用本产品实际支付的费用；',
              '本条款不排除法律不允许排除的责任（如故意或重大过失）。'
            ]
          }
        ]
      },
      {
        heading: '十四、适用法律与争议解决',
        blocks: [
          { type: 'p', text: '本条款受中华人民共和国法律管辖。争议应友好协商；协商不成的，提交运营方所在地有管辖权的人民法院解决。' }
        ]
      },
      {
        heading: '十五、条款更新与联系我们',
        blocks: [
          { type: 'p', text: '我们可能更新本条款并在应用内显著提示；继续使用视为接受。疑问可通过「联系作者」（微信扫码）或 GitHub Issues 联系我们。' }
        ]
      }
    ]
  };

  function escapeHtml(value) {
    return String(value == null ? '' : value)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  function renderBlocks(blocks) {
    return (blocks || []).map(function (block) {
      if (block.type === 'ul') {
        const items = (block.items || []).map(function (item) {
          if (typeof item === 'string') {
            return '<li>' + escapeHtml(item) + '</li>';
          }
          const lead = item.lead ? '<strong>' + escapeHtml(item.lead) + '：</strong>' : '';
          return '<li>' + lead + escapeHtml(item.text || '') + '</li>';
        });
        return '<ul>' + items.join('') + '</ul>';
      }
      return '<p>' + escapeHtml(block.text || '') + '</p>';
    }).join('');
  }

  function renderDocument(doc) {
    const head = '<div class="legal-doc-head">'
      + '<div class="legal-doc-title">' + escapeHtml(doc.title) + '</div>'
      + '<div class="legal-doc-meta">版本：' + escapeHtml(doc.version)
      + '　生效日期：' + escapeHtml(doc.date) + '</div>'
      + '</div>';
    const body = (doc.sections || []).map(function (section) {
      return '<h4>' + escapeHtml(section.heading) + '</h4>' + renderBlocks(section.blocks);
    }).join('');
    return '<div class="legal-doc">' + head + body + '</div>';
  }

  // 打开法律文档弹窗：复用 qrOverlay 容器，加载 .legal-dialog 宽版样式。
  function open(kind) {
    const doc = kind === 'tos' ? TOS : PRIVACY;
    const overlay = document.getElementById('qrOverlay');
    const content = document.getElementById('qrOverlayContent');
    if (!overlay || !content) return false;
    const title = document.querySelector('.qr-overlay .qr-title');
    const account = document.getElementById('qrOverlayAccount');
    const dialog = document.querySelector('.qr-dialog');
    if (title) title.textContent = doc.title;
    if (account) account.textContent = '';
    content.innerHTML = renderDocument(doc);
    if (dialog) dialog.classList.add('qr-dialog-wide', 'legal-dialog');
    overlay.classList.add('show');
    overlay.setAttribute('aria-hidden', 'false');
    return true;
  }

  window.DexLegal = {
    PRIVACY: PRIVACY,
    TOS: TOS,
    render: renderDocument,
    open: open
  };
})();
