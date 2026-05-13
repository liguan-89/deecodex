// deecodex-weixin 插件入口
// 注册 RPC handler，启动插件服务
import { startRpcServer, onRequest, onNotification, sendNotification, getConfig } from "./rpc-client.js";
import type { WeixinPluginConfig } from "./types.js";
import { startLogin, cancelLogin, loadAccountToken } from "./auth.js";
import { startGateway, stopGateway, isGatewayRunning } from "./gateway.js";

// ── 注册 RPC handler ──

interface LoginParams {
  account_id: string;
}

interface AccountParams {
  account_id: string;
}

onRequest("weixin.login", async (req) => {
  const params = (req.params as unknown) as LoginParams | undefined;
  if (!params?.account_id) {
    throw new Error("缺少 account_id");
  }
  await startLogin(params.account_id);
  return { ok: true, message: "QR 码已生成" };
});

onRequest("weixin.login_cancel", async (req) => {
  const params = (req.params as unknown) as AccountParams | undefined;
  if (params?.account_id) {
    cancelLogin(params.account_id);
  }
  return { ok: true };
});

onRequest("weixin.start", async (req) => {
  const params = (req.params as unknown) as AccountParams | undefined;
  if (!params?.account_id) {
    throw new Error("缺少 account_id");
  }
  await startGateway(params.account_id);
  return { ok: true, message: "网关已启动" };
});

onRequest("weixin.stop", async (req) => {
  const params = (req.params as unknown) as AccountParams | undefined;
  if (!params?.account_id) {
    throw new Error("缺少 account_id");
  }
  await stopGateway(params.account_id);
  return { ok: true, message: "网关已停止" };
});

onRequest("weixin.status", async (req) => {
  const params = (req.params as unknown) as AccountParams | undefined;
  const running = params?.account_id ? isGatewayRunning(params.account_id) : false;
  return {
    account_id: params?.account_id || "",
    running,
  };
});

// ── 初始化完成后发送通知 ──

onNotification("initialized", () => {
  sendNotification("log", {
    level: "info",
    message: "微信通道插件已就绪",
  });
  sendNotification("status", {
    account_id: "",
    status: "connected",
    detail: "插件服务已启动",
  });

  // 恢复已登录的账号状态：检查磁盘上是否有保存的 token
  const cfg = getConfig();
  if (cfg.accounts) {
    for (const accountId of Object.keys(cfg.accounts)) {
      const tokenInfo = loadAccountToken(accountId);
      if (tokenInfo?.bot_token) {
        sendNotification("status", {
          account_id: accountId,
          status: "connected",
          detail: "已恢复登录状态",
        });
      }
    }
  }
});

// ── 启动 RPC 服务 ──

startRpcServer();

// 向宿主报到
process.stdout.write(JSON.stringify({
  jsonrpc: "2.0",
  method: "log",
  params: { level: "info", message: "deecodex-weixin 插件启动中..." },
}) + "\n");
