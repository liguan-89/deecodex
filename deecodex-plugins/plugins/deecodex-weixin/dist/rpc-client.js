import * as readline from "node:readline";
let requestId = 0;
const pendingRequests = new Map();
const requestHandlers = new Map();
const notificationHandlers = new Map();
const methods = new Map(); // RPC methods that deecodex can call on plugin
let config = {};
let llmBaseUrl = "";
let dataDir = "";
// ── stdin/stdout ──
const rl = readline.createInterface({ input: process.stdin });
export function startRpcServer() {
    rl.on("line", (line) => {
        let msg;
        try {
            msg = JSON.parse(line.trim());
        }
        catch {
            return;
        }
        if (msg.jsonrpc !== "2.0")
            return;
        if (msg.id !== undefined && msg.method) {
            handleRequest(msg);
        }
        else if (msg.id !== undefined) {
            handleResponse(msg);
        }
        else if (msg.method) {
            handleNotification(msg);
        }
    });
}
// ── Request handlers ──
function handleRequest(req) {
    const handler = requestHandlers.get(req.method);
    if (!handler) {
        sendResponse({ jsonrpc: "2.0", id: req.id, error: { code: -32601, message: `Method not found: ${req.method}` } });
        return;
    }
    Promise.resolve(handler(req)).then(result => sendResponse({ jsonrpc: "2.0", id: req.id, result }), err => sendResponse({ jsonrpc: "2.0", id: req.id, error: { code: -32603, message: String(err) } }));
}
function handleResponse(resp) {
    const pending = pendingRequests.get(resp.id);
    if (pending) {
        pendingRequests.delete(resp.id);
        if (resp.error) {
            pending.reject(new Error(resp.error.message));
        }
        else {
            pending.resolve(resp.result);
        }
    }
}
function handleNotification(notif) {
    const handler = notificationHandlers.get(notif.method);
    if (handler) {
        handler(notif);
    }
}
// ── Public API ──
export function sendRequest(method, params) {
    const id = ++requestId;
    const req = { jsonrpc: "2.0", id, method, params };
    return new Promise((resolve, reject) => {
        pendingRequests.set(id, { resolve, reject });
        writeLine(JSON.stringify(req));
    });
}
export function sendNotification(method, params) {
    const notif = { jsonrpc: "2.0", method, params };
    writeLine(JSON.stringify(notif));
}
export function onRequest(method, handler) {
    requestHandlers.set(method, handler);
}
export function onNotification(method, handler) {
    notificationHandlers.set(method, handler);
}
export function getConfig() {
    return config;
}
export function getLlmBaseUrl() {
    return llmBaseUrl;
}
export function getDataDir() {
    return dataDir;
}
// ── Private ──
function writeLine(line) {
    process.stdout.write(line + "\n");
}
function sendResponse(resp) {
    writeLine(JSON.stringify(resp));
}
// ── Initialize ──
onRequest("initialize", async (req) => {
    const params = req.params;
    config = params?.config || {};
    llmBaseUrl = params?.llm_base_url || "";
    dataDir = params?.data_dir || "";
    return {
        name: "WeChat Channel",
        version: "1.0.0",
        capabilities: ["channel", "qr_login"],
    };
});
onNotification("shutdown", () => {
    // 清理资源
    rl.close();
    setTimeout(() => process.exit(0), 500);
});
onNotification("config.update", (notif) => {
    const newConfig = notif.params?.config;
    if (newConfig) {
        config = newConfig;
    }
});
//# sourceMappingURL=rpc-client.js.map