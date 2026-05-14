// 启动
// ═══════════════════════════════════════════════════════════════
document.addEventListener('DOMContentLoaded', () => {
  try {
    document.getElementById('sidebarSaveBtn')?.addEventListener('click', saveConfig);
    document.getElementById('qrCloseBtn')?.addEventListener('click', closeQrOverlay);
    document.getElementById('qrOverlay')?.addEventListener('click', (event) => {
      if (event.target === event.currentTarget) closeQrOverlay();
    });
    init().catch((error) => {
      console.error('[deecodex] GUI 初始化失败:', error);
      const main = document.getElementById('mainContent');
      if (main) {
        main.innerHTML = '<div class="page-header"><h2>界面加载失败</h2><p>' + esc(String(error)) + '</p></div>';
      }
    });
  } catch (error) {
    console.error('[deecodex] GUI 初始化失败:', error);
    const main = document.getElementById('mainContent');
    if (main) {
      main.innerHTML = '<div class="page-header"><h2>界面加载失败</h2><p>' + esc(String(error)) + '</p></div>';
    }
  }
});
