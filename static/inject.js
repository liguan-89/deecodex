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
        return { pluginUnlock: true, forceInstall: true, sessionDelete: true, modelUnlock: true };
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
        const byIcon = document.querySelector(
            'nav[role="navigation"] button.h-token-nav-row.w-full svg path[d^="M7.94562 14.0277"]'
        )?.closest("button");
        if (byIcon) return byIcon;
        return Array.from(document.querySelectorAll('nav[role="navigation"] button.h-token-nav-row.w-full')).find(function (btn) {
            var text = (btn.textContent || "").trim();
            return /^(插件|Plugins)(\s+-\s+.*)?$/i.test(text);
        }) || null;
    }

    function enablePluginEntry() {
        if (!getSettings().pluginUnlock) return;
        const btn = pluginEntryButton();
        if (!btn) return;

        // 注入时立即切换 authMethod，与 codex-plugin-unlocker 行为一致
        spoofChatGPTAuthMethod(btn);

        btn.disabled = false;
        btn.removeAttribute("disabled");
        btn.style.display = "";
        btn.querySelectorAll("*").forEach(function (node) {
            node.style.display = "";
        });
        const reactKey = Object.keys(btn).find(function (k) { return k.startsWith("__reactProps"); });
        if (reactKey && btn[reactKey]) {
            btn[reactKey].disabled = false;
        }

        if (btn.dataset.codexPluginUnlockerEnabled === "true") return;
        btn.dataset.codexPluginUnlockerEnabled = "true";
        // 捕获阶段拦截，确保在 React 事件系统之前处理
        btn.addEventListener("click", function () {
            spoofChatGPTAuthMethod(btn);
        }, true);
    }

    function pluginInstallCandidates() {
        return document.querySelectorAll("button:disabled.w-full.justify-center");
    }

    function unblockButtonElement(btn) {
        btn.disabled = false;
        btn.removeAttribute("disabled");
        btn.removeAttribute("aria-disabled");
        btn.classList.remove("disabled", "opacity-50", "cursor-not-allowed", "pointer-events-none");
        btn.style.pointerEvents = "auto";
        btn.style.opacity = "";
        btn.style.cursor = "";
        btn.tabIndex = 0;
        btn.querySelectorAll("*").forEach(function (c) {
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

    // ── 模型列表解锁 ──
    let statsigHookInstalled = false;

    function hookStatsigForModels() {
        if (!getSettings().modelUnlock) return;
        if (statsigHookInstalled) return;

        // 查找 Statsig SDK 实例
        const statsigInstance = window.__STATSIG__?.firstInstance || window.__STATSIG__?.instance;
        if (!statsigInstance) return;

        // Hook getDynamicConfig 方法
        const originalGetConfig = statsigInstance.getDynamicConfig;
        if (originalGetConfig && typeof originalGetConfig === "function") {
            statsigInstance.getDynamicConfig = function(configName) {
                const result = originalGetConfig.apply(this, arguments);

                // 拦截模型选择相关配置
                if (configName === "model_selection_config" || configName === "chat_model_picker_config") {
                    // 强制覆盖 use_hidden_models 为 false
                    if (result && result.value) {
                        const originalValue = result.value;
                        result.value = new Proxy(originalValue, {
                            get(target, prop) {
                                if (prop === "use_hidden_models") {
                                    console.log("[DeeCodex] 拦截 use_hidden_models，原值:", target[prop], "→ false");
                                    return false; // 强制返回 false，显示所有模型
                                }
                                return target[prop];
                            }
                        });
                    }
                }

                return result;
            };

            statsigHookInstalled = true;
            console.log("[DeeCodex] 模型列表解锁已激活 (Statsig getDynamicConfig hook)");
        }

        // 额外 hook checkGate (如果使用 feature gate)
        const originalCheckGate = statsigInstance.checkGate;
        if (originalCheckGate && typeof originalCheckGate === "function") {
            statsigInstance.checkGate = function(gateName) {
                const result = originalCheckGate.apply(this, arguments);

                // 如果存在与模型可见性相关的 gate，强制返回 true
                if (gateName && (gateName.includes("hidden_model") || gateName.includes("model_visibility"))) {
                    return true;
                }

                return result;
            };
        }
    }

    // ── 模型选择器扩展 ──
    // Codex 桌面版的 React fiber 链中，模型选择器父组件（混淆名为 oUt）的
    // memoizedProps.models 数组决定下拉菜单的选项。Codex 默认从 ChatGPT 后端
    // 拉到 5 个 GPT 模型。CDP 注入可在每次 React 渲染时把 deecodex 的 78 个模型
    // push 进这个数组，让 Codex UI 显示完整模型列表。
    let deecodexModelsCache = null;

    async function loadDeecodexModels() {
        if (deecodexModelsCache) return deecodexModelsCache;
        if (!window.__deecodexBridge) return null;
        try {
            const result = await window.__deecodexBridge("/models", {});
            if (result && result.models && Array.isArray(result.models)) {
                deecodexModelsCache = result.models;
                return deecodexModelsCache;
            }
        } catch (e) {
            console.warn("[DeeCodex] 加载 deecodex 模型失败:", e);
        }
        return null;
    }

    function fiberFromElement(el) {
        const key = Object.keys(el).find(function (k) { return k.startsWith("__reactFiber"); });
        return key ? el[key] : null;
    }

    function findPickerFiberByText() {
        // Codex 模型选择器当前显示的 span 文本可能是：
        // - 模型版本号："5.5" / "gpt-5.4" / "GPT-5.4"
        // - 自定义："自定义"
        // - 上次选过的 deecodex 模型："DeepSeek V4 Flash" / "MiMo V2 Pro"
        // 策略：从所有可能候选的 span 往上爬，看 fiber 链里有没有 models + onSelectModel
        const candidates = [...document.querySelectorAll("span")].filter(function (s) {
            const t = (s.textContent || "").trim();
            if (t.length === 0 || t.length > 50) return false;
            // 匹配：5.x / gpt-5.x / DeepSeek* / MiMo* / Kimi* / GPT-* / 自定义 / o3 / o4
            return /^(gpt-?)?5\.?\d/.test(t)
                || /^(gpt-)?o[3-9]/.test(t)
                || /^DeepSeek|^MiMo|^Kimi|^Qwen|^Claude|^Llama/.test(t)
                || t === "自定义"
                || /^(gpt-5\.|gpt-4\.|codex-)/.test(t);
        });
        for (let i = 0; i < candidates.length; i++) {
            const span = candidates[i];
            const fiberKey = Object.keys(span).find(function (k) { return k.startsWith("__reactFiber"); });
            if (!fiberKey) continue;
            let fiber = span[fiberKey];
            for (let j = 0; j < 60 && fiber; j++) {
                if (fiber.memoizedProps && Array.isArray(fiber.memoizedProps.models) && typeof fiber.memoizedProps.onSelectModel === "function") {
                    return fiber;
                }
                fiber = fiber.return;
            }
        }
        return null;
    }

    function deecodexModelToPickerEntry(m) {
        const id = m.slug || m.model;
        const displayName = m.displayName || m.display_name || id;
        // 简化显示名：去掉 "桌面版 账号 / " 前缀（Codex UI 不适合太长）
        const shortName = String(displayName).replace(/^.*\/\s*/, "");
        return {
            id: id,
            model: id,
            upgrade: null,
            upgradeInfo: null,
            availabilityNux: null,
            displayName: shortName,
            description: String(displayName),
            hidden: false,
            supportedReasoningEfforts: [
                { reasoningEffort: "low", description: "Fast responses with lighter reasoning" },
                { reasoningEffort: "medium", description: "Balances speed and reasoning depth for everyday tasks" },
                { reasoningEffort: "high", description: "Greater reasoning depth for complex problems" }
            ],
            defaultReasoningEffort: "medium",
            inputModalities: ["text"],
            supportsPersonality: false,
            additionalSpeedTiers: [],
            serviceTiers: [],
            defaultServiceTier: null,
            isDefault: false
        };
    }

    function patchPickerModels(pickerFiber, deecodexModels) {
        if (!pickerFiber || !pickerFiber.memoizedProps) return false;
        const props = pickerFiber.memoizedProps;
        if (!Array.isArray(props.models)) return false;

        const existingIds = new Set(props.models.map(function (m) { return m.id || m.model; }));

        // 调试：第一次 patch 时打印当前已有哪些 id
        if (!window.__deecodexPickerLogOnce) {
            window.__deecodexPickerLogOnce = true;
            console.log("[DeeCodex] picker 当前 models:", [...existingIds]);
        }

        let added = 0;
        deecodexModels.forEach(function (m) {
            const entry = deecodexModelToPickerEntry(m);
            if (!existingIds.has(entry.id)) {
                props.models.push(entry);
                existingIds.add(entry.id);
                added++;
            }
        });

        if (added > 0) {
            props.modelOptionsDisabled = false;
            console.log("[DeeCodex] 模型选择器扩展: 新增", added, "个, 当前总数", props.models.length);
            return true;
        }
        return false;
    }

    function tryPatchPicker() {
        if (!deecodexModelsCache) return false;
        const picker = findPickerFiberByText();
        if (!picker) return false;
        // 注意：React 重新渲染时 picker fiber 是新实例，_deecodexPatched 标记会丢。
        // 因此不能依赖 fiber 上的标记 — patchPickerModels 内部用 existingIds 去重，
        // 重复调用是安全的，只是 noop。
        const changed = patchPickerModels(picker, deecodexModelsCache);
        return changed;
    }

    async function patchModelPicker() {
        if (!getSettings().modelUnlock) return;
        const models = await loadDeecodexModels();
        if (!models || models.length === 0) return;
        tryPatchPicker();
    }

    // ── 模型选择器定时重试 ──
    // Codex 桌面版的 picker button 不是始终渲染的：
    // - 侧边栏空状态（未进入聊天）→ 无 picker
    // - 进入聊天界面后 → picker 才出现
    // 因此 patch 需要在用户每次进入聊天界面时触发。我们用一个长期轮询：
    // 每 1.5s 检查一次，patch 成功后停止；SPA 路由切换会触发新的 picker，
    // 此时 memoizedProps.models 引用变化（picker._deecodexPatched 是 fiber 实例属性），
    // 自动失效，因此需要持续监控。
    let pickerPollTimer = null;
    let pickerPollStopped = false;

    function startPickerRetryLoop() {
        if (!getSettings().modelUnlock) return;
        if (pickerPollTimer) return;

        const tick = async function () {
            if (pickerPollStopped) return;

            // 1) 确保 deecodex 模型缓存加载
            const models = await loadDeecodexModels();
            if (models && models.length > 0) {
                // 2) 尝试 patch（内部已处理"已 patch 过"的情况）
                tryPatchPicker();
            }

            pickerPollTimer = setTimeout(tick, 1500);
        };

        pickerPollTimer = setTimeout(tick, 100);
        console.log("[DeeCodex] 模型选择器监控已启动（每 1.5s 检查）");
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
        hookStatsigForModels();
        attachAllDeleteButtons();
        startPickerRetryLoop();
        patchModelPicker();
    }

    // ── 启动 ──
    // MutationObserver 监听 DOM 变化
    var observer = new MutationObserver(scheduleScan);
    observer.observe(document.body || document.documentElement, { childList: true, subtree: true });
    // 初始扫描
    scan();
})();
