import { getUpdates, notifyStart, notifyStop } from "./ilink.js";
import { loadAccountToken, cancelLogin } from "./auth.js";
import { downloadMedia, transcodeSilkToWav } from "./media.js";
import { processMessage } from "./channel.js";
import { getDataDir, sendNotification } from "./rpc-client.js";
import * as fs from "node:fs";
import * as path from "node:path";
const POLL_TIMEOUT_MS = 35_000;
const MAX_CONSECUTIVE_FAILURES = 3;
const BACKOFF_MS = 30_000;
const SESSION_EXPIRED_ERRCODE = -14;
const SESSION_EXPIRED_PAUSE_MS = 3_600_000;
const gateways = new Map();
// ── 公开接口 ──
export async function startGateway(accountId) {
    const tokenInfo = loadAccountToken(accountId);
    if (!tokenInfo?.bot_token) {
        sendNotification("status", {
            account_id: accountId,
            status: "disconnected",
            detail: "未找到 bot_token，请先登录",
        });
        throw new Error("未找到 bot_token，请先扫码登录");
    }
    // 通知 iLink 启动
    await notifyStart(tokenInfo.bot_token);
    const state = {
        accountId,
        token: tokenInfo.bot_token,
        running: true,
        abortController: new AbortController(),
        consecutiveFailures: 0,
        getUpdatesBuf: loadBuf(accountId),
        contextTokens: new Map(),
    };
    gateways.set(accountId, state);
    sendNotification("status", { account_id: accountId, status: "connected" });
    sendNotification("log", { level: "info", message: `微信账号 ${accountId} 已启动监控` });
    // 开始长轮询循环（fire-and-forget，捕获未处理异常防止静默崩溃）
    pollLoop(state).catch((err) => {
        sendNotification("log", {
            level: "error",
            message: `轮询循环崩溃: ${String(err)}`,
        });
        sendNotification("status", {
            account_id: state.accountId,
            status: "error",
            detail: `轮询循环异常退出: ${String(err)}`,
        });
    });
}
export async function stopGateway(accountId) {
    const state = gateways.get(accountId);
    if (!state)
        return;
    state.running = false;
    state.abortController.abort();
    // 通知 iLink 停止
    try {
        await notifyStop(state.token);
    }
    catch { /* ignore */ }
    gateways.delete(accountId);
    cancelLogin(accountId);
    sendNotification("status", { account_id: accountId, status: "disconnected" });
    sendNotification("log", { level: "info", message: `微信账号 ${accountId} 已停止监控` });
}
export function isGatewayRunning(accountId) {
    return gateways.has(accountId) && gateways.get(accountId).running;
}
// ── 长轮询循环 ──
async function pollLoop(state) {
    sendNotification("log", {
        level: "info",
        message: `轮询循环已启动 (accountId=${state.accountId})`,
    });
    while (state.running) {
        try {
            const resp = await getUpdates(state.token, state.getUpdatesBuf || undefined);
            if (resp.errcode) {
                const errcode = resp.errcode;
                if (errcode === SESSION_EXPIRED_ERRCODE) {
                    sendNotification("status", {
                        account_id: state.accountId,
                        status: "login_expired",
                        detail: "会话已过期，暂停1小时后重试",
                    });
                    // 暂停1小时后继续
                    await sleep(SESSION_EXPIRED_PAUSE_MS);
                    state.consecutiveFailures = 0;
                    continue;
                }
                state.consecutiveFailures++;
                if (state.consecutiveFailures >= MAX_CONSECUTIVE_FAILURES) {
                    sendNotification("status", {
                        account_id: state.accountId,
                        status: "error",
                        detail: `连续${MAX_CONSECUTIVE_FAILURES}次失败，等待重试`,
                    });
                    await sleep(BACKOFF_MS);
                    state.consecutiveFailures = 0;
                }
                continue;
            }
            // 成功：重置计数
            state.consecutiveFailures = 0;
            // 保存 buf
            if (resp.get_updates_buf !== undefined) {
                state.getUpdatesBuf = resp.get_updates_buf;
                saveBuf(state.accountId, resp.get_updates_buf);
            }
            // 处理消息（iLink API 返回 msgs 字段）
            const msgCount = resp.msgs?.length || 0;
            if (msgCount > 0) {
                sendNotification("log", {
                    level: "info",
                    message: `收到 ${msgCount} 条新消息`,
                });
                // 调试：打印第一条消息的完整结构
                sendNotification("log", {
                    level: "debug",
                    message: `[MSG_RAW] ${JSON.stringify(resp.msgs[0])}`,
                });
                for (const msg of resp.msgs) {
                    await handleIncomingMessage(state, msg);
                }
            }
        }
        catch (err) {
            state.consecutiveFailures++;
            sendNotification("log", {
                level: "warn",
                message: `轮询错误: ${String(err)}`,
            });
            if (state.consecutiveFailures >= MAX_CONSECUTIVE_FAILURES) {
                await sleep(BACKOFF_MS);
                state.consecutiveFailures = 0;
            }
        }
    }
}
// ── 入站消息处理 ──
async function handleIncomingMessage(state, msg) {
    // 提取文本内容（从 item_list[0].text_item.text）
    const textItem = msg.item_list?.find(it => it.type === 1 && it.text_item);
    const text = textItem?.text_item?.text || "";
    // chat_id：群聊用 group_id，单聊用 from_user_id
    const chatId = msg.group_id || msg.from_user_id;
    const ctx = {
        msg_id: String(msg.message_id),
        chat_id: chatId,
        sender_id: msg.from_user_id,
        bot_id: msg.to_user_id || "",
        text,
        timestamp: msg.create_time_ms,
        is_group: !!msg.group_id,
        context_token: msg.context_token || "",
        session_id: msg.session_id || "",
    };
    // 媒体消息：下载并转码
    const mediaItem = msg.item_list?.find(it => it.type !== 1 && it.media_item);
    if (mediaItem?.media_item) {
        const media = mediaItem.media_item;
        const mediaPath = await downloadMedia(media);
        if (mediaPath) {
            ctx.media_paths = [mediaPath];
            ctx.media_types = [media.mime_type || "application/octet-stream"];
            // SILK→WAV 转码（语音消息）
            if (msg.message_type === 3 || media.mime_type === "audio/silk") {
                const wavPath = await transcodeSilkToWav(mediaPath);
                if (wavPath && wavPath !== mediaPath) {
                    ctx.media_paths = [wavPath];
                    ctx.media_types = ["audio/wav"];
                }
            }
            // 媒体消息无文字时，添加媒体描述
            if (!ctx.text) {
                const mediaType = ctx.media_types?.[0] || "未知";
                ctx.text = `[${mediaType} 媒体消息]`;
            }
        }
    }
    // 调用消息处理管道
    try {
        await processMessage(state.accountId, ctx, state.contextTokens);
    }
    catch (err) {
        sendNotification("log", {
            level: "error",
            message: `处理消息失败: ${String(err)}`,
        });
    }
}
// ── buf 持久化 ──
function bufPath(accountId) {
    return path.join(getDataDir(), "weixin_accounts", `${accountId}_buf.json`);
}
function loadBuf(accountId) {
    try {
        const raw = fs.readFileSync(bufPath(accountId), "utf-8");
        return JSON.parse(raw).buf || "";
    }
    catch {
        return "";
    }
}
function saveBuf(accountId, buf) {
    try {
        const dir = path.dirname(bufPath(accountId));
        fs.mkdirSync(dir, { recursive: true });
        fs.writeFileSync(bufPath(accountId), JSON.stringify({ buf, updated_at: Date.now() }));
    }
    catch { /* ignore */ }
}
function sleep(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
}
//# sourceMappingURL=gateway.js.map