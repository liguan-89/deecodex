import { getConfig } from "./rpc-client.js";
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
export async function sendMessage(token, req) {
    return apiPost("ilink/bot/sendmessage", {
        chat_id: req.chat_id,
        msg_type: req.msg_type,
        ...(req.content ? { content: req.content } : {}),
        ...(req.media ? { media: req.media } : {}),
    }, token, 15_000);
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
    await apiPost("ilink/bot/msg/notifystart", {}, token, 10_000).catch(() => { });
}
export async function notifyStop(token) {
    await apiPost("ilink/bot/msg/notifystop", {}, token, 10_000).catch(() => { });
}
// ── 配置 ──
export async function getConfig_ilink(token) {
    return apiPost("ilink/bot/getconfig", {}, token, 10_000);
}
//# sourceMappingURL=ilink.js.map