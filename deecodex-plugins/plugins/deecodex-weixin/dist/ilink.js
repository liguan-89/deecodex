import { getConfig, sendNotification } from "./rpc-client.js";
const DEFAULT_BASE_URL = "https://ilinkai.weixin.qq.com";
const DEFAULT_TIMEOUT_MS = 35_000;
function buildHeaders(token) {
    const cfg = getConfig();
    return {
        "Content-Type": "application/json",
        "iLink-App-Id": "bot",
        "iLink-App-ClientVersion": "0x0001000000",
        "AuthorizationType": "ilink_bot_token",
        ...(token ? { "Authorization": `Bearer ${token}` } : {}),
    };
}
function buildBaseInfo() {
    const cfg = getConfig();
    return {
        channel_version: "1.0.0",
        bot_agent: cfg.bot_agent || "deecodex",
        bot_type: cfg.bot_type || "3",
    };
}
async function apiPost(path, body, token, timeoutMs = DEFAULT_TIMEOUT_MS) {
    const cfg = getConfig();
    const baseUrl = cfg.base_url || DEFAULT_BASE_URL;
    const url = `${baseUrl}/${path}`;
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), timeoutMs);
    try {
        const response = await fetch(url, {
            method: "POST",
            headers: buildHeaders(token),
            body: JSON.stringify({ ...body, base_info: buildBaseInfo() }),
            signal: controller.signal,
        });
        if (!response.ok) {
            const text = await response.text().catch(() => "");
            throw new Error(`iLink API 返回 ${response.status}: ${text}`);
        }
        return (await response.json());
    }
    finally {
        clearTimeout(timer);
    }
}
async function apiGet(path, params, token, timeoutMs = DEFAULT_TIMEOUT_MS) {
    const cfg = getConfig();
    const baseUrl = cfg.base_url || DEFAULT_BASE_URL;
    const query = params ? "?" + new URLSearchParams(params).toString() : "";
    const url = `${baseUrl}/${path}${query}`;
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), timeoutMs);
    try {
        const response = await fetch(url, {
            method: "GET",
            headers: buildHeaders(token),
            signal: controller.signal,
        });
        if (!response.ok) {
            const text = await response.text().catch(() => "");
            throw new Error(`iLink API 返回 ${response.status}: ${text}`);
        }
        return (await response.json());
    }
    finally {
        clearTimeout(timer);
    }
}
// ── 消息相关 ──
export async function getUpdates(token, buf) {
    return apiPost("ilink/bot/getupdates", {
        ...(buf ? { get_updates_buf: buf } : {}),
    }, token);
}
function generateClientId() {
    const ts = Date.now().toString(36);
    const rand = Math.random().toString(36).slice(2, 10);
    return `${ts}-${rand}-weixin`;
}

export async function sendMessage(token, req) {
    // 参考 BytePioneer-AI/weixin-agent-gateway：消息体嵌套在 msg 字段中，
    // message_type=2 (BOT), message_state=2 (FINISH), from_user_id 留空
    const msg = {
        from_user_id: "",
        to_user_id: req.chat_id || "",
        client_id: generateClientId(),
        message_type: 2,  // BOT
        message_state: 2, // FINISH
        ...(req.content ? { item_list: [{ type: 1, text_item: { text: req.content } }] } : {}),
        ...(req.media ? { media: req.media } : {}),
        ...(req.context_token ? { context_token: req.context_token } : {}),
    };
    const body = { msg };
    sendNotification("log", {
        level: "debug",
        message: `[iLink sendmessage] body=${JSON.stringify(body).slice(0, 400)}`,
    });
    const result = await apiPost("ilink/bot/sendmessage", body, token, 15_000);
    sendNotification("log", {
        level: "debug",
        message: `[iLink sendmessage] resp=${JSON.stringify(result).slice(0, 300)}`,
    });
    if (result.ret !== undefined && result.ret !== 0) {
        throw new Error(`iLink sendmessage 返回 ret=${result.ret}${result.errcode ? " errcode=" + result.errcode : ""}`);
    }
    return result;
}
export async function sendTyping(token, chatId) {
    await apiPost("ilink/bot/sendtyping", { chat_id: chatId }, token, 10_000).catch(() => { });
}
// ── 上传 ──
export async function getUploadUrl(token, fileSize, mimeType) {
    return apiPost("ilink/bot/getuploadurl", {
        file_size: fileSize,
        mime_type: mimeType,
    }, token, 15_000);
}
export async function uploadToCdn(url, buffer, mimeType) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 30_000);
    try {
        const resp = await fetch(url, {
            method: "PUT",
            headers: { "Content-Type": mimeType },
            body: new Uint8Array(buffer),
            signal: controller.signal,
        });
        if (!resp.ok) {
            throw new Error(`CDN 上传失败: ${resp.status}`);
        }
    }
    finally {
        clearTimeout(timer);
    }
}
// ── 鉴权 ──
export async function notifyStart(token) {
    try {
        const result = await apiPost("ilink/bot/msg/notifystart", {}, token, 10_000);
        sendNotification("log", { level: "debug", message: `[notifyStart] 响应: ${JSON.stringify(result)}` });
        return result;
    } catch (err) {
        sendNotification("log", { level: "error", message: `[notifyStart] 失败: ${String(err)}` });
        throw err;
    }
}
export async function notifyStop(token) {
    try {
        const result = await apiPost("ilink/bot/msg/notifystop", {}, token, 10_000);
        sendNotification("log", { level: "debug", message: `[notifyStop] 响应: ${JSON.stringify(result)}` });
        return result;
    } catch (err) {
        sendNotification("log", { level: "error", message: `[notifyStop] 失败: ${String(err)}` });
    }
}
// ── 配置 ──
export async function getConfig_ilink(token) {
    return apiPost("ilink/bot/getconfig", {}, token, 10_000);
}
//# sourceMappingURL=ilink.js.map