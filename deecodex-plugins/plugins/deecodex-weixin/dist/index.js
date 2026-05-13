// deecodex-weixin 插件入口
// 注册 RPC handler，启动插件服务
import { startRpcServer, onRequest, onNotification, sendNotification, getConfig, getDataDir } from "./rpc-client.js";
import { startLogin, cancelLogin, loadAccountToken } from "./auth.js";
import { startGateway, stopGateway, isGatewayRunning } from "./gateway.js";
import * as fs from "node:fs";
import * as path from "node:path";
onRequest("weixin.login", async (req) => {
    const params = req.params;
    if (!params?.account_id) {
        throw new Error("缺少 account_id");
    }
    const result = await startLogin(params.account_id);
    return result || { ok: true, message: "QR 码已生成" };
});
onRequest("weixin.login_cancel", async (req) => {
    const params = req.params;
    if (params?.account_id) {
        cancelLogin(params.account_id);
    }
    return { ok: true };
});
onRequest("weixin.start", async (req) => {
    const params = req.params;
    if (!params?.account_id) {
        throw new Error("缺少 account_id");
    }
    await startGateway(params.account_id);
    return { ok: true, message: "网关已启动" };
});
onRequest("weixin.stop", async (req) => {
    const params = req.params;
    if (!params?.account_id) {
        throw new Error("缺少 account_id");
    }
    await stopGateway(params.account_id);
    return { ok: true, message: "网关已停止" };
});
onRequest("weixin.status", async (req) => {
    const params = req.params;
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
    // 恢复已登录的账号：扫描 weixin_accounts 目录自动重启网关
    const dataDir = getDataDir();
    const accountsDir = path.join(dataDir, "weixin_accounts");
    sendNotification("log", {
        level: "info",
        message: `[auto-restore] dataDir=${dataDir} accountsDir=${accountsDir}`,
    });
    try {
        const entries = fs.readdirSync(accountsDir);
        sendNotification("log", {
            level: "info",
            message: `[auto-restore] 找到 ${entries.length} 个文件: ${entries.join(", ")}`,
        });
        for (const entry of entries) {
            if (!entry.endsWith(".json") || entry.endsWith("_buf.json"))
                continue;
            const accountId = entry.replace(/\.json$/, "");
            const tokenInfo = loadAccountToken(accountId);
            if (tokenInfo?.bot_token) {
                sendNotification("log", {
                    level: "info",
                    message: `自动恢复网关: accountId=${accountId}`,
                });
                startGateway(accountId).catch(err => {
                    sendNotification("log", {
                        level: "error",
                        message: `自动恢复网关失败 accountId=${accountId}: ${String(err)}`,
                    });
                });
            }
        }
    }
    catch {
        // 目录不存在则跳过
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
//# sourceMappingURL=index.js.map