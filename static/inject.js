// deecodex 注入脚本 — 通过 CDP 注入 Codex 渲染进程。
// 提供两个功能：
// 1. 插件解锁 — 篡改 React auth context 使 API Key 用户也能使用插件
// 2. 会话删除 — 在侧边栏注入删除按钮，支持确认/撤销

(function () {
    "use strict";

    // ── 配置 ──
    const BUTTON_CLASS = "deecodex-delete-btn";
    const STYLE_ID = "deecodex-style";
    const MENU_ID = "deecodex-menu";
    const BRIDGE_NAME = "deecodexBridge";
    const SETTINGS_KEY = "deecodexSettings";
    const VERSION = "1.0.0";

    // ── 工具函数 ──
    function getSettings() {
        try {
            return JSON.parse(localStorage.getItem(SETTINGS_KEY)) || defaultSettings();
        } catch (_) {
            return defaultSettings();
        }
    }

    function defaultSettings() {
        return { pluginUnlock: true, forceInstall: true, sessionDelete: true };
    }

    function reactFiberFrom(el) {
        const key = Object.keys(el).find((k) => k.startsWith("__reactFiber"));
        return key ? el[key] : null;
    }

    // ── 插件解锁 ──
    function authContextValueFrom(element) {
        for (let fiber = reactFiberFrom(element); fiber; fiber = fiber.return) {
            for (const value of [fiber.memoizedProps?.value, fiber.pendingProps?.value]) {
                if (
                    value &&
                    typeof value === "object" &&
                    typeof value.setAuthMethod === "function" &&
                    "authMethod" in value
                ) {
                    return value;
                }
            }
        }
        return null;
    }

    function spoofChatGPTAuthMethod(element) {
        const auth = authContextValueFrom(element);
        if (!auth || auth.authMethod === "chatgpt") return false;
        auth.setAuthMethod("chatgpt");
        return true;
    }

    function pluginEntryButton() {
        return document.querySelector(
            'nav[role="navigation"] button.h-token-nav-row.w-full svg path[d^="M7.94562 14.0277"]'
        )?.closest("button");
    }

    function enablePluginEntry() {
        if (!getSettings().pluginUnlock) return;
        const btn = pluginEntryButton();
        if (!btn || btn.disabled === false) return;
        // 仅启用 DOM 按钮，不修改全局 authMethod（避免模型加载器卡住）
        btn.disabled = false;
        btn.removeAttribute("disabled");
        const reactKey = Object.keys(btn).find((k) => k.startsWith("__reactProps"));
        if (reactKey && btn[reactKey]) {
            btn[reactKey].disabled = false;
        }
        // 点击时临时切换 auth 以绕过插件入口的内部权限检查
        btn.addEventListener("click", () => {
            const auth = authContextValueFrom(btn);
            if (auth && auth.authMethod !== "chatgpt") {
                auth.setAuthMethod("chatgpt");
                // 1 秒后恢复，避免影响模型加载等全局行为
                setTimeout(() => {
                    try { auth.setAuthMethod("api_key"); } catch (_) {}
                }, 1000);
            }
        }, { once: true });
        const label = btn.querySelector("span");
        if (label) {
            const text = label.textContent || "";
            label.textContent = text.includes("插件") ? "插件 (已解锁)" : text.includes("Plugins") ? "Plugins - Unlocked" : text;
        }
    }

    function pluginInstallCandidates() {
        return document.querySelectorAll("button:disabled.w-full.justify-center");
    }

    function unblockButtonElement(btn) {
        btn.disabled = false;
        btn.removeAttribute("disabled");
        btn.removeAttribute("aria-disabled");
        btn.classList.remove("opacity-50", "cursor-not-allowed", "pointer-events-none");
        btn.style.opacity = "";
        btn.style.cursor = "";
        btn.querySelectorAll("*").forEach((c) => {
            c.style.opacity = "";
            c.style.pointerEvents = "";
        });
    }

    function unblockPluginInstallButtons() {
        if (!getSettings().forceInstall) return;
        pluginInstallCandidates().forEach((btn) => {
            const text = btn.textContent?.trim() || "";
            if (/^安装\s|^Install\s|强制安装/.test(text)) {
                unblockButtonElement(btn);
                if (!text.startsWith("强制")) {
                    const span = btn.querySelector("span");
                    if (span) span.textContent = "强制安装";
                }
            }
        });
    }

    // ── 会话删除 UI ──
    function injectStyles() {
        if (document.getElementById(STYLE_ID)) return;
        const style = document.createElement("style");
        style.id = STYLE_ID;
        style.textContent = `
            .${BUTTON_CLASS} {
                position: absolute;
                right: 28px;
                top: 50%;
                transform: translateY(-50%);
                opacity: 0;
                background: #dc2626;
                color: #fff;
                border: none;
                border-radius: 4px;
                padding: 3px 8px;
                font-size: 11px;
                cursor: pointer;
                z-index: 10;
                transition: opacity 0.15s;
                white-space: nowrap;
            }
            [data-app-action-sidebar-thread-id]:hover .${BUTTON_CLASS},
            .${BUTTON_CLASS}:hover {
                opacity: 1;
            }
            .${BUTTON_CLASS}:hover {
                background: #b91c1c;
            }
            .deecodex-confirm-overlay {
                position: fixed;
                inset: 0;
                background: rgba(0,0,0,0.5);
                display: flex;
                align-items: center;
                justify-content: center;
                z-index: 99999;
            }
            .deecodex-confirm-box {
                background: #1e1e1e;
                border: 1px solid #333;
                border-radius: 8px;
                padding: 24px;
                min-width: 320px;
                color: #eee;
                font-family: -apple-system, BlinkMacSystemFont, sans-serif;
            }
            .deecodex-confirm-title { font-size: 16px; font-weight: 600; margin-bottom: 8px; }
            .deecodex-confirm-msg { font-size: 13px; color: #999; margin-bottom: 20px; }
            .deecodex-confirm-btns { display: flex; gap: 8px; justify-content: flex-end; }
            .deecodex-confirm-btns button { padding: 6px 16px; border-radius: 4px; border: none; font-size: 13px; cursor: pointer; }
            .deecodex-btn-cancel { background: #333; color: #ccc; }
            .deecodex-btn-delete { background: #dc2626; color: #fff; }
            .deecodex-btn-delete:hover { background: #b91c1c; }
            .deecodex-toast {
                position: fixed;
                right: 18px;
                bottom: 18px;
                background: #1e1e1e;
                border: 1px solid #333;
                border-radius: 6px;
                padding: 10px 16px;
                color: #eee;
                font-size: 13px;
                z-index: 999999;
                display: flex;
                align-items: center;
                gap: 12px;
                font-family: -apple-system, BlinkMacSystemFont, sans-serif;
            }
            .deecodex-toast-undo { color: #60a5fa; cursor: pointer; background: none; border: none; font-size: 13px; }
            .deecodex-toast-undo:hover { text-decoration: underline; }
        `;
        document.head.appendChild(style);
    }

    // ── 桥接通信 ──
    async function postJson(path, payload) {
        if (!window.__deecodexBridge) {
            return { status: "failed", message: "桥接不可用" };
        }
        try {
            return await window.__deecodexBridge(path, payload);
        } catch (e) {
            return { status: "failed", message: String(e) };
        }
    }

    // ── 删除确认对话框 ──
    function confirmDelete(title) {
        return new Promise(function (resolve) {
            const overlay = document.createElement("div");
            overlay.className = "deecodex-confirm-overlay";
            overlay.innerHTML = `
                <div class="deecodex-confirm-box">
                    <div class="deecodex-confirm-title">删除会话</div>
                    <div class="deecodex-confirm-msg">确定删除「${title}」？删除后可撤销。</div>
                    <div class="deecodex-confirm-btns">
                        <button class="deecodex-btn-cancel">取消</button>
                        <button class="deecodex-btn-delete">删除</button>
                    </div>
                </div>
            `;
            overlay.querySelector(".deecodex-btn-cancel").onclick = function () {
                overlay.remove();
                resolve(false);
            };
            overlay.querySelector(".deecodex-btn-delete").onclick = function () {
                overlay.remove();
                resolve(true);
            };
            overlay.onclick = function (e) {
                if (e.target === overlay) { overlay.remove(); resolve(false); }
            };
            document.addEventListener("keydown", function esc(e) {
                if (e.key === "Escape") { overlay.remove(); resolve(false); }
            }, { once: true });
            document.body.appendChild(overlay);
        });
    }

    function showToast(message, undoToken) {
        const toast = document.createElement("div");
        toast.className = "deecodex-toast";
        toast.innerHTML = `<span>${message}</span>`;
        if (undoToken) {
            const undoBtn = document.createElement("button");
            undoBtn.className = "deecodex-toast-undo";
            undoBtn.textContent = "撤销";
            undoBtn.onclick = async function () {
                const result = await postJson("/undo", { undo_token: undoToken });
                toast.textContent = result.message || (result.status === "undone" ? "已撤销" : "撤销失败");
                setTimeout(function () { toast.remove(); }, 3000);
            };
            toast.appendChild(undoBtn);
        }
        document.body.appendChild(toast);
        setTimeout(function () { toast.remove(); }, 10000);
    }

    // ── 从行提取会话引用 ──
    function sessionRefFromRow(row) {
        var sessionId = row.getAttribute("data-app-action-sidebar-thread-id") || "";
        var title = row.getAttribute("data-thread-title") || row.textContent?.trim()?.split("\n")[0] || "";
        return { session_id: sessionId, title: title };
    }

    function removeDeletedRow(row, button) {
        if (button && button.parentNode) button.remove();
        if (row && row.parentNode) row.remove();
    }

    // ── 删除按钮 ──
    function attachDeleteButton(row) {
        if (!getSettings().sessionDelete) return;
        if (row.querySelector("." + BUTTON_CLASS)) return;
        var btn = document.createElement("button");
        btn.className = BUTTON_CLASS;
        btn.textContent = "删除";
        btn.addEventListener("pointerdown", function (e) { e.stopPropagation(); });
        btn.addEventListener("mousedown", function (e) { e.stopPropagation(); });
        btn.addEventListener("mouseup", function (e) { e.stopPropagation(); });
        btn.addEventListener("click", async function (e) {
            e.preventDefault();
            e.stopPropagation();
            var ref = sessionRefFromRow(row);
            if (!ref.session_id) return;
            var confirmed = await confirmDelete(ref.title);
            if (!confirmed) return;
            var result = await postJson("/delete", ref);
            if (result.status === "deleted" || result.status === "server_deleted" || result.status === "local_deleted") {
                removeDeletedRow(row, btn);
                showToast(result.message || "删除成功", result.undo_token);
            } else {
                showToast(result.message || "删除失败", null);
            }
        });
        row.appendChild(btn);
    }

    function attachAllDeleteButtons() {
        var rows = document.querySelectorAll("[data-app-action-sidebar-thread-id]");
        rows.forEach(function (row) { attachDeleteButton(row); });
    }

    // ── 扫描 ──
    var scanTimer = null;

    function scheduleScan() {
        if (scanTimer) return;
        scanTimer = setTimeout(function () {
            scanTimer = null;
            scan();
        }, 200);
    }

    function scan() {
        injectStyles();
        enablePluginEntry();
        unblockPluginInstallButtons();
        attachAllDeleteButtons();
    }

    // ── 启动 ──
    // MutationObserver 监听 DOM 变化
    var observer = new MutationObserver(scheduleScan);
    observer.observe(document.body || document.documentElement, { childList: true, subtree: true });
    // 初始扫描
    scan();
})();
