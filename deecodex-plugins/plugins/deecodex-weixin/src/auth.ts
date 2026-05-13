// QR 码登录流程
import type { QrCodeResponse, QrCodeStatus, AccountConfig } from "./types.js";
import * as fs from "node:fs";
import * as path from "node:path";
import QRCode from "qrcode";
import { sendNotification, getConfig, getDataDir } from "./rpc-client.js";

const QR_POLL_INTERVAL_MS = 2_000;
const QR_EXPIRE_SECONDS = 120;

interface QrSession {
  accountId: string;
  qrCodeId: string;
  qrDataUrl: string;
  startTime: number;
}

const activeSessions = new Map<string, QrSession>();

// ── iLink QR 码 API ──

async function getBotQrCode(baseUrl: string, botType: string): Promise<QrCodeResponse> {
  const url = `${baseUrl}/ilink/bot/get_bot_qrcode?bot_type=${encodeURIComponent(botType)}`;
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 10_000);

  try {
    const response = await fetch(url, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "iLink-App-Id": "bot",
        "iLink-App-ClientVersion": "0x0001000000",
      },
      body: JSON.stringify({
        base_info: {
          channel_version: "1.0.0",
          bot_agent: getConfig().bot_agent || "deecodex",
        },
      }),
      signal: controller.signal,
    });

  if (!response.ok) {
    const text = await response.text().catch(() => "");
    throw new Error(`获取 QR 码失败 (${response.status}): ${text}`);
  }

  const data = await response.json() as {
    ret: number;
    qrcode?: string;
    qrcode_img_content?: string;
    expire_time?: number;
  };

  if (data.ret !== 0) {
    throw new Error(`获取 QR 码失败: ret=${data.ret}`);
  }

  if (!data.qrcode || !data.qrcode_img_content) {
    throw new Error("获取 QR 码失败: 响应缺少 qrcode");
  }

  // 用 liteapp URL 生成 QR 码图片
  const qrDataUrl = await QRCode.toDataURL(data.qrcode_img_content, {
    width: 256,
    margin: 2,
  });

  return {
    qrcode_id: data.qrcode,
    qrcode_data_url: qrDataUrl,
    expire_seconds: QR_EXPIRE_SECONDS,
  };
  } finally {
    clearTimeout(timer);
  }
}

async function pollQrCodeStatus(baseUrl: string, qrcodeId: string, botType: string): Promise<QrCodeStatus> {
  const url = `${baseUrl}/ilink/bot/get_qrcode_status?qrcode=${encodeURIComponent(qrcodeId)}&bot_type=${encodeURIComponent(botType)}`;
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 30_000);

  try {
    const response = await fetch(url, {
      method: "GET",
      headers: {
        "iLink-App-Id": "bot",
        "iLink-App-ClientVersion": "0x0001000000",
      },
      signal: controller.signal,
    });

    if (!response.ok) {
      return { status: "error", message: `HTTP ${response.status}` };
    }

    const data = await response.json() as {
      ret: number;
      status?: number | string;
      bot_token?: string;
      user_id?: string;
    };

    if (data.ret !== 0) {
      return { status: "error", message: `ret=${data.ret}` };
    }

    // 兼容字符串和数字两种 status 格式
    const statusStr = typeof data.status === "string" ? data.status : String(data.status);
    switch (statusStr) {
      case "0":
      case "waiting": return { status: "waiting" };
      case "1":
      case "scanned": return { status: "scanned" };
      case "2":
      case "confirmed": return { status: "confirmed", bot_token: data.bot_token, user_id: data.user_id };
      case "3":
      case "expired": return { status: "expired" };
      default: return { status: "error", message: `未知状态: ${data.status}` };
    }
  } catch (err) {
    // 长轮询超时（AbortError）视为无变化，继续轮询
    if (err instanceof DOMException && err.name === "AbortError") {
      return { status: "waiting" };
    }
    return { status: "error", message: `轮询异常: ${String(err)}` };
  } finally {
    clearTimeout(timer);
  }
}

// ── 公开接口 ──

export async function startLogin(accountId: string): Promise<void> {
  const cfg = getConfig();
  const baseUrl = cfg.base_url || "https://ilinkai.weixin.qq.com";
  const account = cfg.accounts?.[accountId];

  if (!account) {
    sendNotification("qr_code", {
      account_id: accountId,
      data_url: "",
      error: "未找到账号配置",
    });
    return;
  }
  const botType = account.bot_type || cfg.bot_type || "3";

  try {
    const qrResult = await getBotQrCode(baseUrl, botType);

    const session: QrSession = {
      accountId,
      qrCodeId: qrResult.qrcode_id,
      qrDataUrl: qrResult.qrcode_data_url || "",
      startTime: Date.now(),
    };
    activeSessions.set(accountId, session);

    // 通知 deecodex QR 码已就绪
    sendNotification("qr_code", {
      account_id: accountId,
      data_url: qrResult.qrcode_data_url,
    });

    sendNotification("status", {
      account_id: accountId,
      status: "connecting",
    });

    // 开始轮询
    startPolling(accountId, baseUrl, botType);
  } catch (err) {
    sendNotification("status", {
      account_id: accountId,
      status: "error",
      detail: String(err),
    });
  }
}

async function startPolling(accountId: string, baseUrl: string, botType: string): Promise<void> {
  const session = activeSessions.get(accountId);
  if (!session) return;

  const expireAt = session.startTime + QR_EXPIRE_SECONDS * 1000;

  const poll = async () => {
    if (!activeSessions.has(accountId)) return;

    if (Date.now() > expireAt) {
      sendNotification("status", {
        account_id: accountId,
        status: "error",
        detail: "QR 码已过期",
      });
      activeSessions.delete(accountId);
      return;
    }

    try {
      const result = await pollQrCodeStatus(baseUrl, session.qrCodeId, botType);

      if (result.status === "confirmed" && result.bot_token) {
        // 保存 token
        saveAccountToken(accountId, result.bot_token, result.user_id);
        sendNotification("status", {
          account_id: accountId,
          status: "connected",
          detail: "登录成功",
        });
        activeSessions.delete(accountId);
        return;
      }

      if (result.status === "expired") {
        sendNotification("status", {
          account_id: accountId,
          status: "error",
          detail: "QR 码已过期，请重新扫码",
        });
        activeSessions.delete(accountId);
        return;
      }

      if (result.status === "error") {
        sendNotification("status", {
          account_id: accountId,
          status: "error",
          detail: result.message || "未知错误",
        });
        activeSessions.delete(accountId);
        return;
      }

      if (result.status === "scanned") {
        sendNotification("log", {
          level: "info",
          message: `账号 ${accountId}: 已扫码，等待确认...`,
        });
      }

      // 继续轮询
      setTimeout(poll, QR_POLL_INTERVAL_MS);
    } catch (err) {
      sendNotification("status", {
        account_id: accountId,
        status: "error",
        detail: `轮询失败: ${String(err)}`,
      });
      activeSessions.delete(accountId);
    }
  };

  setTimeout(poll, QR_POLL_INTERVAL_MS);
}

export function cancelLogin(accountId: string): void {
  activeSessions.delete(accountId);
}

function saveAccountToken(accountId: string, token: string, userId?: string): void {
  const dir = path.join(getDataDir(), "weixin_accounts");
  fs.mkdirSync(dir, { recursive: true });

  const data = {
    account_id: accountId,
    bot_token: token,
    user_id: userId || "",
    saved_at: Date.now(),
  };

  fs.writeFileSync(path.join(dir, `${accountId}.json`), JSON.stringify(data, null, 2));
}

export function loadAccountToken(accountId: string): { bot_token?: string; user_id?: string } | null {
  try {
    const filePath = path.join(getDataDir(), "weixin_accounts", `${accountId}.json`);
    const raw = fs.readFileSync(filePath, "utf-8");
    return JSON.parse(raw);
  } catch {
    return null;
  }
}
