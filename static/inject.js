// deecodex 注入脚本 — 通过 CDP 注入 Codex 渲染进程。
// 提供五个功能：
// 1. 插件解锁 — 篡改 React auth context 使 API Key 用户也能使用插件
// 2. 强制安装 — 解除 Codex 插件安装按钮的 disabled 限制
// 3. 模型选择器扩展 — 通过 React fiber hook 把 deecodex 的 78 个模型 push 进
//    Codex UI 模型下拉菜单（原本只显示 5 个 GPT）
// 4. Statsig 离线中文 — 首次启动时捕获 ab.chatgpt.com/v1/rgstr 响应保存到本地；
//    后续启动由 Rust 端 CDP Fetch.fulfillRequest 直接回填，ab.chatgpt.com 不可达
//    时也能让 Codex 拿到合法的 Statsig 响应
// 5. UI 中文兜底 — Codex 内置的 i18n 字典不含中文（app.asar 里搜不到翻译），
//    即使 locale=zh-CN 系统标签仍显示英文。MutationObserver 把 sidebar/顶部
//    的英文系统标签（New chat / Search / Plugins / Pinned / Projects / Chats 等）
//    实时替换为中文，不影响用户内容

(function () {
    "use strict";

    // ── 配置 ──
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
        return { pluginUnlock: true, forceInstall: true, modelUnlock: true };
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

    let deecodexModeCache = null;
    let deecodexModelsLoadedAt = 0;
    let deecodexAccountByModelId = {};
    let deecodexAccountByModelName = {};
    // picker fiber 的 props.models（正确顺序 + dexacct id），分组重建的数据源。
    // ChatGPT 渲染的 menuitem 拿不到 dexacct id（fiber 里是 radix id），
    // 必须用这份数据驱动分组，而不是按 DOM 文本猜。
    let deecodexPickerModels = null;
    const DEECODEX_MODE_REFRESH_MS = 5000;

    // 从 catalog 原始 display_name（"账号 / 模型"）建立 模型 → 账号 映射。
    // 分组依据始终来自 catalog，而不是 picker 显示名：relay/smart 模式下 picker
    // 会去掉账号前缀，仅靠显示名无法还原分组。
    function base64UrlDecode(s) {
        if (!s) return null;
        try {
            const pad = s.length % 4 === 0 ? "" : "=".repeat(4 - (s.length % 4));
            const b64 = s.replace(/-/g, "+").replace(/_/g, "/") + pad;
            const bytes = atob(b64);
            // 账号 id 是 ASCII（hex），直接按字节解码即可
            let out = "";
            for (let i = 0; i < bytes.length; i++) out += String.fromCharCode(bytes.charCodeAt(i));
            return out;
        } catch (e) {
            return null;
        }
    }

    function buildDeecodexAccountMaps(models) {
        deecodexAccountByModelId = {};
        deecodexAccountByModelName = {};
        // 先建立 账号id → 账号名（分组占位条目 display_name），再按 slug 解码
        // 把模型 id 映射到账号名。dexacct 模型 display_name 是纯模型名（无 "/"），
        // 旧实现依赖 display_name.split('/') 导致映射全空、模型被归到“其他”。
        const groupNamesByAccountId = {};
        (models || []).forEach(function (m) {
            const id = m.slug || m.model || "";
            if (String(id).indexOf("dexacct.group.") !== 0) return;
            const accB64 = String(id).split(".").pop() || "";
            const accId = base64UrlDecode(accB64);
            if (accId) groupNamesByAccountId[accId] = m.displayName || m.display_name || accId;
        });
        (models || []).forEach(function (m) {
            const id = m.slug || m.model || "";
            if (String(id).indexOf("dexacct.") !== 0 || String(id).indexOf("dexacct.group.") === 0) return;
            const parts = String(id).split(".");
            if (parts.length !== 4) return;
            const accId = base64UrlDecode(parts[1]);
            if (!accId) return;
            const account = groupNamesByAccountId[accId] || accId;
            if (id && account) deecodexAccountByModelId[id] = account;
            const dn = m.displayName || m.display_name || id;
            const modelName = String(dn).replace(/^.*\/\s*/, "").trim();
            if (modelName && account) deecodexAccountByModelName[modelName] = account;
        });
    }

    async function loadDeecodexModels() {
        const now = Date.now();
        if (deecodexModelsCache && now - deecodexModelsLoadedAt < DEECODEX_MODE_REFRESH_MS) {
            return deecodexModelsCache;
        }
        if (!window.__deecodexBridge) return deecodexModelsCache;
        try {
            const result = await window.__deecodexBridge("/models", {});
            if (result && result.models && Array.isArray(result.models)) {
                deecodexModelsCache = result.models;
                deecodexModeCache = (result && result.mode) || "api";
                buildDeecodexAccountMaps(deecodexModelsCache);
                deecodexModelsLoadedAt = now;
                return deecodexModelsCache;
            }
        } catch (e) {
            console.warn("[DeeCodex] 加载 deecodex 模型失败:", e);
        }
        return deecodexModelsCache;
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
        let shortName;
        if (deecodexModeCache === "api") {
            // API 模式：保留 catalog 原始 "账号 / 模型" 格式
            const parts = String(displayName).split('/').map(function (s) { return s.trim(); });
            shortName = parts.length >= 2 && parts[0]
                ? parts[0] + ' / ' + parts.slice(1).join(' ')
                : String(displayName);
        } else {
            // 智能路由 / 中转模式：不改显示名，保持原始 deecodex 行为。
            shortName = String(displayName).replace(/^.*\/\s*/, "");
        }
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

        const existingById = new Map(props.models.map(function (m) {
            return [m.id || m.model, m];
        }));

        // 调试：第一次 patch 时打印当前已有哪些 id
        if (!window.__deecodexPickerLogOnce) {
            window.__deecodexPickerLogOnce = true;
            console.log("[DeeCodex] picker 当前 models:", [...existingById.keys()]);
        }

        let added = 0;
        let updated = 0;
        deecodexModels.forEach(function (m) {
            const id0 = m.slug || m.model || "";
            if (String(id0).indexOf("dexacct.group.") === 0) return; // 分组占位条目不注入
            // catalog 标记 hidden 的模型（codex-auto-review 等内部辅助模型）不注入，
            // 避免泄漏进模型选择器（此前强制 hidden:false 导致 Auto Review 显示）。
            if (m.hidden === true || String(id0) === "codex-auto-review") return;
            const entry = deecodexModelToPickerEntry(m);
            const existing = existingById.get(entry.id);
            if (!existing) {
                props.models.push(entry);
                existingById.set(entry.id, entry);
                added++;
            } else if (existing.displayName !== entry.displayName || existing.description !== entry.description) {
                existing.displayName = entry.displayName;
                existing.description = entry.description;
                updated++;
            }
        });

        if (added > 0 || updated > 0) {
            props.modelOptionsDisabled = false;
            // 保存数据源供 applyModelMenuGrouping 使用（分组重建不能依赖 DOM 文本）
            deecodexPickerModels = props.models;
            console.log(
                "[DeeCodex] 模型选择器扩展: 新增",
                added,
                "个, 更新",
                updated,
                "个, 当前总数",
                props.models.length
            );
            return true;
        }
        return false;
    }

    // ── 模型选择器分组显示（方案 B）──
    // Codex 原生把账号模型与内置 GPT 平铺成单行 menuitem（并行显示）。注入脚本
    // 在菜单渲染后按账号/供应商把模型项归组，插入分组标题，形成两级层级：
    //   账号名（分组标题） → 模型项（缩进平级）
    // React 每次重建菜单后分组标题会被清掉，scan() 的 MutationObserver 会在
    // DOM 变化时重新应用（applyModelMenuGrouping 幂等，重复调用安全）。
    function codexModelGroupOf(text, modelId) {
        if (modelId && deecodexAccountByModelId[modelId]) {
            return deecodexAccountByModelId[modelId];
        }
        const t = String(text || "").trim();
        if (deecodexAccountByModelName[t]) {
            return deecodexAccountByModelName[t];
        }
        const m = t.match(/^(.+?)\s*\/\s*/);
        if (m) return m[1];
        if (/^(GPT-|gpt-|codex-|\d)/.test(t)) return "内置 GPT";
        return "其他";
    }

    function codexIsModelMenuItem(text) {
        const t = String(text || "").trim();
        return /^(DeepSeek|MiniMax|MiMo|Anthropic|Kimi|Qwen|GLM|Llama|Claude|88\s*\/|GPT-|gpt-|codex-|\d+(\.\d+)?(\s|$))/.test(t);
    }

    // 去掉 "账号 / " 前缀，只保留模型名（分组标题已展示账号，组内不重复）
    function codexStripGroupPrefix(text) {
        const t = String(text || "").trim();
        const i = t.indexOf(" / ");
        return i > 0 ? t.slice(i + 3).trim() : t;
    }

    // 找到 menuitem 里承载完整模型名的展示 span（Tailwind 稳定类）
    function codexModelNameSpan(item) {
        if (!item || !item.querySelector) return null;
        const spans = item.querySelectorAll("span.truncate.whitespace-nowrap");
        if (spans.length === 0) return null;
        return spans[spans.length - 1];
    }

    // 从 menuitem 的 React fiber 链取模型 id（dexacct.*），用于按 catalog 映射分组
    function codexModelIdFromItem(item) {
        let fiber = reactFiberFrom(item);
        for (let i = 0; i < 40 && fiber; i++) {
            const p = fiber.memoizedProps;
            if (p) {
                const id = p.id || p.model;
                if (id && typeof id === "string" && id.indexOf("dexacct") !== -1) {
                    return id;
                }
            }
            fiber = fiber.return;
        }
        return null;
    }

    function codexMenuHeaderColor(menu) {
        try {
            const bg = getComputedStyle(menu).backgroundColor;
            const rgb = (bg.match(/\d+(\.\d+)?/g) || []).map(Number);
            const lum = rgb.length >= 3 ? 0.299 * rgb[0] + 0.587 * rgb[1] + 0.114 * rgb[2] : 255;
            return lum < 128 ? "#7ab8ff" : "#1f5fbf";
        } catch (_) {
            return "#7ab8ff";
        }
    }

    function applyModelMenuGrouping() {
        const menus = document.querySelectorAll('[role="menu"]');
        for (let mi = 0; mi < menus.length; mi++) {
            const menu = menus[mi];
            const items = Array.prototype.slice.call(menu.querySelectorAll('[role="menuitem"]'));
            if (items.length < 5) continue;
            const headerCss =
                "padding:7px 12px 3px;font-size:14px;font-weight:700;color:" + codexMenuHeaderColor(menu) + ";letter-spacing:.02em;line-height:1.3;user-select:none;";
            // 数据驱动：用 picker props.models（正确顺序 + dexacct id），而不是 DOM 文本。
            // ChatGPT 渲染的 menuitem fiber 只有 radix id，拿不到 dexacct slug；
            // 88 与 Codex 官方存在同名 GPT-5.x，仅靠 DOM 文本无法区分归属。
            const models = deecodexPickerModels;
            if (!models || models.length === 0) continue;

            // 占位条目的显示名（如 "Codex · user@example.com" 不匹配
            // codexIsModelMenuItem，会落进 control），统一识别并隐藏。
            const placeholderNames = {};
            models.forEach(function (m) {
                if (String(m.id || m.model || "").indexOf("dexacct.group.") === 0) {
                    placeholderNames[String(m.displayName || m.display_name || "")] = true;
                }
            });
            const control = [];
            const pool = [];
            items.forEach(function (it) {
                const t = (it.textContent || "").trim();
                if (codexIsModelMenuItem(t)) {
                    pool.push({ it: it, text: codexStripGroupPrefix(t), used: false });
                } else {
                    control.push(it);
                }
            });

            const fragment = document.createDocumentFragment();
            control.forEach(function (c) {
                const t = (c.textContent || "").trim();
                if (placeholderNames[t] || placeholderNames[codexStripGroupPrefix(t)]) {
                    c.style.display = "none";
                    return; // 占位条目隐藏，不渲染
                }
                fragment.appendChild(c);
            });
            let lastGroup = null;
            models.forEach(function (m) {
                const id = String(m.id || m.model || "");
                const dn = String(m.displayName || m.display_name || "");
                const isPlaceholder = id.indexOf("dexacct.group.") === 0;
                if (isPlaceholder) {
                    // 分组占位条目：隐藏对应 menuitem（文本=账号名），不渲染
                    const it = pool.find(function (p) { return !p.used && (p.text === dn || p.text === codexStripGroupPrefix(dn)); });
                    if (it) { it.used = true; it.it.style.display = "none"; }
                    return;
                }
                const group = deecodexAccountByModelId[id] || codexModelGroupOf(dn, id);
                if (group && group !== lastGroup) {
                    const header = document.createElement("div");
                    header.className = "deecodex-model-group-header";
                    header.setAttribute("data-deecodex-group", "1");
                    header.setAttribute("data-deecodex-group-version", "2");
                    header.style.cssText = headerCss;
                    header.textContent = group;
                    fragment.appendChild(header);
                    lastGroup = group;
                }
                const short = codexStripGroupPrefix(dn);
                const it = pool.find(function (p) { return !p.used && p.text === short; });
                if (it) { it.used = true; fragment.appendChild(it.it); }
            });
            // 未匹配的剩余项（兜底）追加到末尾，避免丢失
            pool.forEach(function (p) { if (!p.used) fragment.appendChild(p.it); });

            if (fragment.childNodes.length > 0) {
                menu.innerHTML = "";
                menu.appendChild(fragment);
            }
        }
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

    // ── Statsig 离线中文支持 ──
    // Codex 启动时会请求 ab.chatgpt.com/v1/initialize 拉取配置，里面包含 locale
    // 字段（zh-CN）。如果该请求被屏蔽，Codex 默认 fallback 到 en，整页变英文。
    //
    // 工作流：
    // 1. 首次启动（无本地缓存）：hook window.fetch + XMLHttpRequest，
    //    让请求真实发出但把响应体通过桥接写回 ~/.deecodex/statsig_init_zh.json
    // 2. 后续启动（有缓存）：由 Rust 端 CDP Fetch.fulfillRequest 在网络层
    //    直接回填缓存的响应，本函数什么都不做
    let statsigCaptureInstalled = false;
    let statsigCaptured = false;

    async function checkStatsigCache() {
        if (!window.__deecodexBridge) return false;
        try {
            const result = await window.__deecodexBridge("/statsig-init", { method: "GET" });
            return !!(result && result.status === "ok");
        } catch (_) {
            return false;
        }
    }

    function utf8ToBase64(text) {
        const bytes = new TextEncoder().encode(text);
        let bin = "";
        for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
        return btoa(bin);
    }

    async function saveStatsigInit(text) {
        if (statsigCaptured) return;
        if (!window.__deecodexBridge) return;
        statsigCaptured = true;
        try {
            const b64 = utf8ToBase64(text);
            const result = await window.__deecodexBridge("/statsig-init", {
                method: "POST",
                body_b64: b64,
            });
            if (result && result.status === "ok") {
                console.log(
                    "[DeeCodex] Statsig 初始化响应已捕获并保存到本地（" +
                        (result.size || 0) +
                        " 字节）。下次启动 Codex 时将自动离线回退。"
                );
            } else {
                console.warn("[DeeCodex] Statsig 响应保存失败:", result);
                statsigCaptured = false;
            }
        } catch (e) {
            console.warn("[DeeCodex] Statsig 响应保存异常:", e);
            statsigCaptured = false;
        }
    }

    function isStatsigInitUrl(url) {
        if (typeof url !== "string") return false;
        const isHost = url.indexOf("ab.chatgpt.com") !== -1
            || url.indexOf("api.statsigcdn.com") !== -1
            || url.indexOf("statsigapi.net") !== -1;
        if (!isHost) return false;
        // Codex 用 /v1/rgstr（register）端点，不是标准 SDK 的 /v1/initialize
        return url.indexOf("/v1/initialize") !== -1
            || url.indexOf("/v1/rgstr") !== -1
            || url.indexOf("/v1/evaluate") !== -1;
    }

    function installStatsigCapture() {
        if (statsigCaptureInstalled) return;
        statsigCaptureInstalled = true;

        // hook window.fetch
        const originalFetch = window.fetch;
        if (typeof originalFetch === "function") {
            window.fetch = function (input, init) {
                let url = "";
                try {
                    url = typeof input === "string" ? input : (input && input.url) || "";
                } catch (_) {}
                if (isStatsigInitUrl(url)) {
                    const promise = originalFetch.apply(this, arguments);
                    promise
                        .then(function (response) {
                            if (response && typeof response.clone === "function") {
                                response
                                    .clone()
                                    .text()
                                    .then(saveStatsigInit)
                                    .catch(function () {});
                            }
                        })
                        .catch(function () {});
                    return promise;
                }
                return originalFetch.apply(this, arguments);
            };
        }

        // hook XMLHttpRequest
        const originalOpen = XMLHttpRequest.prototype.open;
        const originalSend = XMLHttpRequest.prototype.send;
        XMLHttpRequest.prototype.open = function (method, url) {
            this.__deecodexUrl = typeof url === "string" ? url : "";
            return originalOpen.apply(this, arguments);
        };
        XMLHttpRequest.prototype.send = function () {
            if (isStatsigInitUrl(this.__deecodexUrl)) {
                this.addEventListener("readystatechange", function () {
                    if (this.readyState === 4 && !statsigCaptured) {
                        try {
                            const text =
                                this.responseType === "" || this.responseType === "text"
                                    ? this.responseText
                                    : null;
                            if (text) saveStatsigInit(text);
                        } catch (_) {}
                    }
                });
            }
            return originalSend.apply(this, arguments);
        };
    }

    async function setupStatsigOfflineSupport() {
        const hasCache = await checkStatsigCache();
        if (hasCache) {
            console.log("[DeeCodex] Statsig 本地缓存已存在，离线中文回退已就绪（CDP 层）");
            return;
        }
        console.log("[DeeCodex] 未找到 Statsig 本地缓存，安装一次性捕获 hook");
        installStatsigCapture();
    }

    // ── DOM 级 UI 中文兜底 ──
    // Codex 桌面版没有内置中文 i18n 字典（app.asar 里搜不到"新对话/搜索/插件"等），
    // IntlProvider 拿到 locale="zh-CN" 但 messages={}，所以系统标签仍显示英文。
    //
    // 兜底方案：MutationObserver 监听整个 body，把 sidebar/顶部的英文系统标签
    // 替换成中文。翻译字典只覆盖 Codex 固定的几个系统标签（不会误伤用户内容）。
    let domTranslationInstalled = false;

    const DOM_I18N_DICT = {
        "New chat": "新对话",
        "Search": "搜索",
        "Plugins": "插件",
        "Pinned": "置顶",
        "Projects": "项目",
        "Chats": "聊天",
        "Settings": "设置",
        "Account": "账号",
        "Profile": "个人资料",
        "Log out": "退出登录",
        "Sign in": "登录",
        "Get started": "开始使用",
        "Try again": "重试",
        "Loading...": "加载中...",
        "Failed to load": "加载失败",
        "Open in editor": "在编辑器中打开",
        "Open in terminal": "在终端中打开",
        "Open in folder": "在文件夹中打开",
        "Copy": "复制",
        "Copied": "已复制",
        "Delete": "删除",
        "Rename": "重命名",
        "Cancel": "取消",
        "Confirm": "确认",
        "Save": "保存",
        "Close": "关闭",
    };

    function translateTextNode(node) {
        if (!node || !node.nodeValue) return false;
        const orig = node.nodeValue;
        const trimmed = orig.trim();
        if (!DOM_I18N_DICT[trimmed]) return false;
        const leadingWs = orig.match(/^\s*/)[0];
        const trailingWs = orig.match(/\s*$/)[0];
        node.nodeValue = leadingWs + DOM_I18N_DICT[trimmed] + trailingWs;
        return true;
    }

    function translateSubtree(root) {
        if (!root) return 0;
        let count = 0;
        const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
            acceptNode: function (n) {
                return translateTextNode(n) ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT;
            }
        });
        let n;
        while ((n = walker.nextNode())) {
            translateTextNode(n);
            count++;
        }
        return count;
    }

    function installDomTranslation() {
        if (domTranslationInstalled) return;
        domTranslationInstalled = true;
        const initial = translateSubtree(document.body);
        const observer = new MutationObserver(function (mutations) {
            for (const m of mutations) {
                for (const node of m.addedNodes) {
                    if (node.nodeType === Node.ELEMENT_NODE) {
                        translateSubtree(node);
                    }
                }
                // 直接改 textContent 的情况
                if (m.type === "characterData" && m.target) {
                    translateTextNode(m.target);
                }
            }
        });
        observer.observe(document.body || document.documentElement, {
            childList: true,
            subtree: true,
            characterData: true,
        });
        if (initial > 0) {
            console.log("[DeeCodex] UI 中文兜底：已翻译 " + initial + " 个系统标签");
        }
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
        enablePluginEntry();
        unblockPluginInstallButtons();
        hookStatsigForModels();
        startPickerRetryLoop();
        patchModelPicker();
        applyModelMenuGrouping();
        setupStatsigOfflineSupport();
        installDomTranslation();
    }

    // ── 启动 ──
    // MutationObserver 监听 DOM 变化
    var observer = new MutationObserver(scheduleScan);
    observer.observe(document.body || document.documentElement, { childList: true, subtree: true });
    // 初始扫描
    scan();
})();
