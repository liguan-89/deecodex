// JSON-RPC 客户端：管理 stdin/stdout 通信
import type { JsonRpcRequest, JsonRpcResponse, JsonRpcNotification, WeixinPluginConfig } from "./types.js";
import * as readline from "node:readline";

type RequestHandler = (req: JsonRpcRequest) => Promise<unknown>;
type NotificationHandler = (notif: JsonRpcNotification) => void;

let requestId = 0;
const pendingRequests = new Map<number, {
  resolve: (value: unknown) => void;
  reject: (error: Error) => void;
}>();
const requestHandlers = new Map<string, RequestHandler>();
const notificationHandlers = new Map<string, NotificationHandler>();
const methods = new Map<string, unknown>(); // RPC methods that deecodex can call on plugin

let config: WeixinPluginConfig = {};
let llmBaseUrl = "";
let dataDir = "";

// ── stdin/stdout ──

const rl = readline.createInterface({ input: process.stdin });

export function startRpcServer() {
  rl.on("line", (line: string) => {
    let msg: { jsonrpc: string; id?: number; method?: string; params?: Record<string, unknown> };
    try {
      msg = JSON.parse(line.trim());
    } catch {
      return;
    }
    if (msg.jsonrpc !== "2.0") return;

    if (msg.id !== undefined && msg.method) {
      handleRequest(msg as JsonRpcRequest);
    } else if (msg.id !== undefined) {
      handleResponse(msg as JsonRpcResponse);
    } else if (msg.method) {
      handleNotification(msg as JsonRpcNotification);
    }
  });
}

// ── Request handlers ──

function handleRequest(req: JsonRpcRequest) {
  const handler = requestHandlers.get(req.method);
  if (!handler) {
    sendResponse({ jsonrpc: "2.0", id: req.id, error: { code: -32601, message: `Method not found: ${req.method}` }});
    return;
  }
  Promise.resolve(handler(req)).then(
    result => sendResponse({ jsonrpc: "2.0", id: req.id, result }),
    err => sendResponse({ jsonrpc: "2.0", id: req.id, error: { code: -32603, message: String(err) }}),
  );
}

function handleResponse(resp: JsonRpcResponse) {
  const pending = pendingRequests.get(resp.id);
  if (pending) {
    pendingRequests.delete(resp.id);
    if (resp.error) {
      pending.reject(new Error(resp.error.message));
    } else {
      pending.resolve(resp.result);
    }
  }
}

function handleNotification(notif: JsonRpcNotification) {
  const handler = notificationHandlers.get(notif.method);
  if (handler) {
    handler(notif);
  }
}

// ── Public API ──

export function sendRequest(method: string, params?: Record<string, unknown>): Promise<unknown> {
  const id = ++requestId;
  const req: JsonRpcRequest = { jsonrpc: "2.0", id, method, params };
  return new Promise((resolve, reject) => {
    pendingRequests.set(id, { resolve, reject });
    writeLine(JSON.stringify(req));
  });
}

export function sendNotification(method: string, params?: Record<string, unknown>): void {
  const notif: JsonRpcNotification = { jsonrpc: "2.0", method, params };
  writeLine(JSON.stringify(notif));
}

export function onRequest(method: string, handler: RequestHandler) {
  requestHandlers.set(method, handler);
}

export function onNotification(method: string, handler: NotificationHandler) {
  notificationHandlers.set(method, handler);
}

export function getConfig(): WeixinPluginConfig {
  return config;
}

export function getLlmBaseUrl(): string {
  return llmBaseUrl;
}

export function getDataDir(): string {
  return dataDir;
}

// ── Private ──

function writeLine(line: string) {
  process.stdout.write(line + "\n");
}

function sendResponse(resp: JsonRpcResponse) {
  writeLine(JSON.stringify(resp));
}

// ── Initialize ──

onRequest("initialize", async (req) => {
  const params = req.params as Record<string, unknown> | undefined;
  config = (params?.config as WeixinPluginConfig) || {};
  llmBaseUrl = (params?.llm_base_url as string) || "";
  dataDir = (params?.data_dir as string) || "";
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
  const newConfig = (notif.params as Record<string, unknown>)?.config as WeixinPluginConfig | undefined;
  if (newConfig) {
    config = newConfig;
  }
});
