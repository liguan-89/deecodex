import { getConfig, getDataDir, sendNotification, sendRequest } from "./rpc-client.js";
import { sendMessage, sendTyping, getUploadUrl, uploadToCdn } from "./ilink.js";
import { aesEcbEncrypt } from "./media.js";
import * as fs from "node:fs";
import * as path from "node:path";
// ── 授权检查 ──
function checkAuthorization(ctx) {
    const cfg = getConfig();
    // open 模式：不检查
    if (cfg.dm_policy === "open")
        return true;
    // whitelist 模式：检查白名单
    const allowedUsers = cfg.allowed_users || [];
    if (allowedUsers.length === 0)
        return true; // 空白名单 = 允许所有
    return allowedUsers.includes(ctx.sender_id);
}
// ── 上下文管理 ──
function loadContext(chatId, maxMessages) {
    const dataDir = getDataDir();
    const ctxPath = path.join(dataDir, "weixin_conversations", `${chatId}.json`);
    try {
        const raw = fs.readFileSync(ctxPath, "utf-8");
        const messages = JSON.parse(raw);
        return messages.slice(-maxMessages);
    }
    catch {
        return [];
    }
}
function saveContext(chatId, messages) {
    const dataDir = getDataDir();
    const dir = path.join(dataDir, "weixin_conversations");
    fs.mkdirSync(dir, { recursive: true });
    const ctxPath = path.join(dir, `${chatId}.json`);
    try {
        fs.writeFileSync(ctxPath, JSON.stringify(messages, null, 2));
    }
    catch { /* ignore */ }
}
// ── Markdown 过滤 ──
function filterMarkdown(text) {
    if (!text)
        return "";
    return text
        // 保留：**粗体**、`代码`、```代码块```、表格
        // 移除：*斜体*、~~删除线~~、链接语法、图片语法
        .replace(/(?<!\*)\*(?!\*)([^*]+)\*(?!\*)/g, "$1") // *斜体*
        .replace(/~~(.+?)~~/g, "$1") // ~~删除线~~
        .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1") // [链接](url)
        .replace(/!\[([^\]]*)\]\([^)]+\)/g, "[图片]") // ![图片](url)
        .replace(/\n{3,}/g, "\n\n"); // 压缩换行
}
// ── 文本分块发送 ──
const TEXT_CHUNK_LIMIT = 4000;
async function sendTextReply(token, chatId, text, contextToken, botId) {
    const filtered = filterMarkdown(text);
    sendNotification("log", {
        level: "debug",
        message: `[sendTextReply] chatId=${chatId} text_len=${filtered.length} ctx_token=${(contextToken || "").slice(0, 20)}...`,
    });
    const reqBody = { chat_id: chatId, msg_type: 1, content: filtered };
    if (contextToken) reqBody.context_token = contextToken;
    if (botId) reqBody.bot_id = botId;
    if (filtered.length <= TEXT_CHUNK_LIMIT) {
        const result = await sendMessage(token, reqBody);
        sendNotification("log", {
            level: "debug",
            message: `[sendMessage] 响应: ${JSON.stringify(result).slice(0, 200)}`,
        });
    }
    else {
        const chunks = [];
        for (let i = 0; i < filtered.length; i += TEXT_CHUNK_LIMIT) {
            chunks.push(filtered.slice(i, i + TEXT_CHUNK_LIMIT));
        }
        for (const chunk of chunks) {
            const chunkBody = { ...reqBody, content: chunk };
            const result = await sendMessage(token, chunkBody);
            sendNotification("log", {
                level: "debug",
                message: `[sendMessage] 分块响应: ${JSON.stringify(result).slice(0, 200)}`,
            });
        }
    }
}
// ── 媒体发送 ──
async function sendMediaReply(token, chatId, filePath, mimeType) {
    const fileBuffer = fs.readFileSync(filePath);
    const { upload_url, media_id } = await getUploadUrl(token, fileBuffer.length, mimeType);
    // AES 加密后上传
    const aesKey = crypto.randomUUID().replace(/-/g, "").slice(0, 32);
    const encrypted = aesEcbEncrypt(fileBuffer, aesKey);
    await uploadToCdn(upload_url, encrypted, mimeType);
    await sendMessage(token, {
        chat_id: chatId,
        msg_type: mimeType.startsWith("image/") ? "image"
            : mimeType.startsWith("video/") ? "video"
                : "file",
        media: {
            media_id,
            aes_key: aesKey,
            mime_type: mimeType,
            file_name: path.basename(filePath),
            file_size: fileBuffer.length,
        },
    });
}
// ── 消息处理主流程 ──
export async function processMessage(accountId, ctx, contextTokens) {
    sendNotification("log", {
        level: "info",
        message: `处理消息: ${ctx.sender_id} → "${(ctx.text || "").slice(0, 50)}"`,
    });
    // 1. 授权检查
    if (!checkAuthorization(ctx)) {
        sendNotification("log", {
            level: "info",
            message: `用户 ${ctx.sender_id} 未在白名单中，跳过`,
        });
        return;
    }
    // 2. 获取 token
    const { loadAccountToken } = await import("./auth.js");
    const tokenInfo = loadAccountToken(accountId);
    if (!tokenInfo?.bot_token) {
        sendNotification("log", { level: "warn", message: "未找到 bot_token" });
        return;
    }
    const token = tokenInfo.bot_token;
    // 3. 发送打字指示器
    sendTyping(token, ctx.chat_id);
    // 4. 加载上下文
    const cfg = getConfig();
    const maxMessages = cfg.max_context_messages || 50;
    const history = loadContext(ctx.chat_id, maxMessages);
    // 5. 构建消息
    const userMsg = {
        role: "user",
        content: ctx.text || "[空消息]",
        timestamp: ctx.timestamp,
        msg_id: ctx.msg_id,
    };
    const messages = [...history, userMsg];
    // 6. LLM 调用
    try {
        const llmMessages = messages.map(m => ({
            role: m.role,
            content: m.content,
        }));
        const result = await sendRequest("llm.call", {
            account_id: accountId,
            messages: llmMessages,
            model: "auto",
            system_prompt: "你是运行在 deecodex 智能网关上的 AI Agent。你可以调用各种工具帮助用户解决问题，包括但不限于：信息检索、代码编写、文件处理、联网搜索、知识问答等。请用简体中文与用户交流，回复专业、准确、有帮助。你可以自由使用工具来完成任务，主动为用户提供最佳解决方案。",
        });
        const responseObj = result;
        const responseText = responseObj?.content || "";
        if (responseText) {
            // 7. 发送回复
            await sendTextReply(token, ctx.chat_id, responseText, ctx.context_token, ctx.bot_id);
            // 8. 更新上下文
            const assistantMsg = {
                role: "assistant",
                content: responseText,
                timestamp: Date.now(),
            };
            messages.push(assistantMsg);
        }
    }
    catch (err) {
        sendNotification("log", {
            level: "error",
            message: `LLM 调用失败: ${String(err)}`,
        });
        // LLM 失败时发送错误提示
        sendTextReply(token, ctx.chat_id, "抱歉，处理您的消息时出错了，请稍后再试。", ctx.context_token, ctx.bot_id).catch(() => { });
    }
    // 9. 保存上下文
    saveContext(ctx.chat_id, messages.slice(-maxMessages));
}
//# sourceMappingURL=channel.js.map