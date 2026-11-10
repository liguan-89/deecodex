// CDP 线程结构探针（一次性）：只读抓取 Codex 桌面版 sidebar 的全部线程 / section / 字段。
//
// 主目标：枚举 [data-app-action-sidebar-thread-id] 节点，把它的所有 data-* 属性、
//         所属 section（"置顶" / "全部" / "Recent"）、关联的 title/cwd/git/archived/status
//         一次扫干净，输出到 window.__cdp_threads.full_threads。
// 副目标：枚举 section 容器（"置顶" / "全部" / "Recent" / "已安排任务" / "归档"）的折叠状态。

(function () {
  // 不早返回：每次都重置 full_threads / sections / raw_attr_keys，让重新扫描能拿到最新 sidebar 状态
  window.__cdp_threads = {
    probe_installed: false,
    full_threads: [],
    sections: [],
    sidebar: {},
    raw_attr_keys: [],
  };

  // 1) 全量枚举 [data-app-action-sidebar-thread-id] 节点
  const threadEls = document.querySelectorAll('[data-app-action-sidebar-thread-id]');
  const attrKeySet = new Set();
  for (const el of threadEls) {
    const data = {};
    for (const a of el.attributes || []) {
      data[a.name] = a.value;
      attrKeySet.add(a.name);
    }

    // 找最近的祖先 section：section 容器通常带 [data-app-action-sidebar-section-toggle] 节点
    // 用 closest 向上找祖先，再在祖先内部找 toggle button
    let sectionLabel = null;
    let sectionExpanded = null;
    let p = el.parentElement;
    let hops = 0;
    while (p && hops < 15) {
      const toggle = p.querySelector('[data-app-action-sidebar-section-toggle]');
      if (toggle) {
        sectionLabel = (toggle.innerText || toggle.textContent || '').trim();
        sectionExpanded = toggle.getAttribute('aria-expanded');
        break;
      }
      p = p.parentElement;
      hops++;
    }

    // 优先用节点自带的 data-app-action-sidebar-thread-title
    const title = data['data-app-action-sidebar-thread-title']
      || (el.querySelector('[data-thread-title]') ? (el.querySelector('[data-thread-title]').innerText || '').trim() : '');

    // 找时间标签（"1 周" / "18 小时" / "1 天"）
    const timeCandidates = el.querySelectorAll('div, span');
    let timeText = '';
    for (const c of timeCandidates) {
      const t = (c.innerText || c.textContent || '').trim();
      if (/^\d+\s*(秒|分|小时|天|周|月|年|second|minute|hour|day|week|month|year)s?$/i.test(t)) {
        timeText = t;
        break;
      }
    }

    // 找 git/cwd/project 类属性（Codex 桌面版通常挂在 data-cwd / data-git-branch / data-git-repo）
    const cwd = data['data-cwd'] || data['data-project-path'] || data['data-workspace-root'] || null;
    const gitBranch = data['data-git-branch'] || data['data-git-branch-name'] || null;
    const gitRepo = data['data-git-repo'] || data['data-git-repo-url'] || null;
    const archived = data['data-app-action-sidebar-thread-archived']
                  || data['data-archived']
                  || data['data-app-action-sidebar-archived']
                  || null;
    const status = data['data-app-action-sidebar-thread-status']
                || data['data-status']
                || null;

    window.__cdp_threads.full_threads.push({
      host: data['data-app-action-sidebar-thread-host-id'] || null,
      kind: data['data-app-action-sidebar-thread-kind'] || null,
      thread_id: data['data-app-action-sidebar-thread-id'] || null,
      pinned: data['data-app-action-sidebar-thread-pinned'] || null,
      active: data['data-app-action-sidebar-thread-active'] || null,
      archived: archived,
      status: status,
      cwd: cwd,
      git_branch: gitBranch,
      git_repo: gitRepo,
      created_at_ms: data['data-app-action-sidebar-thread-created-at-ms']
                  || data['data-created-at']
                  || data['data-created-at-ms']
                  || null,
      updated_at_ms: data['data-app-action-sidebar-thread-updated-at-ms']
                  || data['data-updated-at']
                  || data['data-updated-at-ms']
                  || null,
      section_label: sectionLabel,
      section_expanded: sectionExpanded,
      title: title,
      time_text: timeText,
      // 全部 data-* 留底，方便后续看到新字段
      all_data_attrs: data,
    });
  }

  // 2) 抓 section 容器
  const sectionEls = document.querySelectorAll('[data-app-action-sidebar-section-toggle]');
  for (const el of sectionEls) {
    const data = {};
    for (const a of el.attributes || []) data[a.name] = a.value;
    const label = (el.innerText || el.textContent || '').trim();
    window.__cdp_threads.sections.push({
      label: label,
      expanded: el.getAttribute('aria-expanded'),
      attrs: data,
    });
  }

  // 3) 侧边栏根容器
  const sidebarCandidates = [
    'aside', 'nav', '[role="navigation"]', '[role="complementary"]',
    '[class*="sidebar"]', '[class*="Sidebar"]',
  ];
  for (const sel of sidebarCandidates) {
    for (const el of document.querySelectorAll(sel)) {
      const cls = (el.className && typeof el.className === 'string') ? el.className.slice(0, 120) : '';
      const aria = el.getAttribute('aria-label') || '';
      window.__cdp_threads.sidebar[sel] = window.__cdp_threads.sidebar[sel] || [];
      window.__cdp_threads.sidebar[sel].push({
        class: cls,
        aria: aria,
        child_count: el.children.length,
        id: el.id || '',
      });
    }
  }

  // 4) 收集所有出现过的 data-* key（去重）方便发现新字段
  window.__cdp_threads.raw_attr_keys = Array.from(attrKeySet).sort();

  window.__cdp_threads.probe_installed = true;
  return 'installed: full_threads=' + window.__cdp_threads.full_threads.length +
         ' sections=' + window.__cdp_threads.sections.length +
         ' data_attr_keys=' + window.__cdp_threads.raw_attr_keys.length;
})();
