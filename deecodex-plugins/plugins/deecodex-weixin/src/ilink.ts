// iLink Bot API HTTP 客户端
import type { WeixinMessage, SendMessageReq, QrCodeResponse, QrCodeStatus } from "./types.js";
import { getConfig } from "./rpc-client.js";

const DEFAULT_BASE_URL = "https://ilinkai.weixin.qq.com";
const DEFAULT_TIMEOUT_MS = 35_000;

function buildHeaders(token?: string): Record<string, string> {
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

async function apiPost<T>(
  path: string,
  body: Record<string, unknown>,
  token?: string,
  timeoutMs = DEFAULT_TIMEOUT_MS,
): Promise<T> {
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

    return (await response.json()) as T;
  } finally {
    clearTimeout(timer);
  }
}

async function apiGet<T>(
  path: string,
  params?: Record<string, string>,
  token?: string,
  timeoutMs = DEFAULT_TIMEOUT_MS,
): Promise<T> {
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

    return (await response.json()) as T;
  } finally {
    clearTimeout(timer);
  }
}

// ── 消息相关 ──

export async function getUpdates(token: string, buf?: string): Promise<{
  ret: number;
  errcode?: number;
  msgs?: WeixinMessage[];
  get_updates_buf?: string;
}> {
  return apiPost("ilink/bot/getupdates", {
    ...(buf ? { get_updates_buf: buf } : {}),
  }, token);
}

export async function sendMessage(token: string, req: SendMessageReq): Promise<{ ret: number; msg_id?: string }> {
  // 使用 apiPost 附加 base_info（含 bot_type），让平台根据 bot 类型授权
  const body: Record<string, unknown> = {
    to_user_id: req.chat_id,
    from_user_id: req.bot_id || "",
    message_type: req.msg_type,
    ...(req.content ? { item_list: [{ type: 1, text_item: { text: req.content }, is_completed: true }] } : {}),
    ...(req.media ? { media: req.media } : {}),
    ...(req.context_token ? { context_token: req.context_token } : {}),
  };

  const result = await apiPost("ilink/bot/sendmessage", body, token, 15_000);
  if (result.ret !== undefined && result.ret !== 0) {
    throw new Error(`iLink sendmessage 返回 ret=${result.ret}${result.errcode ? " errcode=" + result.errcode : ""}`);
  }
  return result as { ret: number; msg_id?: string };
}

export async function sendTyping(token: string, chatId: string): Promise<void> {
  await apiPost("ilink/bot/sendtyping", { chat_id: chatId }, token, 10_000).catch(() => {});
}

// ── 上传 ──

export async function getUploadUrl(token: string, fileSize: number, mimeType: string): Promise<{
  upload_url: string;
  media_id: string;
}> {
  return apiPost("ilink/bot/getuploadurl", {
    file_size: fileSize,
    mime_type: mimeType,
  }, token, 15_000);
}

export async function uploadToCdn(url: string, buffer: Buffer, mimeType: string): Promise<void> {
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
  } finally {
    clearTimeout(timer);
  }
}

// ── 鉴权 ──

export async function notifyStart(token: string): Promise<void> {
  await apiPost("ilink/bot/msg/notifystart", {}, token, 10_000).catch(() => {});
}

export async function notifyStop(token: string): Promise<void> {
  await apiPost("ilink/bot/msg/notifystop", {}, token, 10_000).catch(() => {});
}

// ── 配置 ──

export async function getConfig_ilink(token: string): Promise<{ typing_ticket?: string }> {
  return apiPost("ilink/bot/getconfig", {}, token, 10_000);
}
