// 微信插件类型定义

// ── JSON-RPC ──

export interface JsonRpcRequest {
  jsonrpc: "2.0";
  id: number;
  method: string;
  params?: Record<string, unknown>;
}

export interface JsonRpcResponse {
  jsonrpc: "2.0";
  id: number;
  result?: unknown;
  error?: { code: number; message: string; data?: unknown };
}

export interface JsonRpcNotification {
  jsonrpc: "2.0";
  method: string;
  params?: Record<string, unknown>;
}

export type JsonRpcMessage = JsonRpcRequest | JsonRpcResponse | JsonRpcNotification;

// ── 插件配置 ──

export interface WeixinPluginConfig {
  accounts?: Record<string, AccountConfig>;
  base_url?: string;
  cdn_base_url?: string;
  dm_policy?: "open" | "whitelist";
  allowed_users?: string[];
  max_context_messages?: number;
  bot_agent?: string;
  bot_type?: string;
}

export interface AccountConfig {
  name?: string;
  enabled?: boolean;
  bot_token?: string;
  base_url?: string;
  cdn_base_url?: string;
  route_tag?: number;
  bot_type?: string;
}

// ── iLink 协议 ──

export interface WeixinMessage {
  message_id: number;
  from_user_id: string;
  to_user_id: string;
  group_id: string;
  session_id: string;
  create_time_ms: number;
  message_type: number;
  item_list: WeixinMsgItem[];
}

export interface WeixinMsgItem {
  type: number;
  text_item?: { text: string };
  media_item?: WeixinMedia;
}

export interface WeixinMedia {
  media_id?: string;
  cdn_url?: string;
  aes_key?: string;
  file_name?: string;
  mime_type?: string;
  file_size?: number;
  duration_ms?: number;
}

export interface WeixinMsgContext {
  msg_id: string;
  chat_id: string;
  sender_id: string;
  text?: string;
  media_paths?: string[];
  media_types?: string[];
  timestamp: number;
  is_group: boolean;
}

export interface SendMessageReq {
  chat_id: string;
  msg_type: string | number;
  content?: string;
  media?: WeixinMedia;
  context_token?: string;
  bot_id?: string;
}

export interface QrCodeResponse {
  qrcode_id: string;
  qrcode_data_url?: string;
  url?: string;
  expire_seconds: number;
}

export interface QrCodeStatus {
  status: "waiting" | "scanned" | "confirmed" | "expired" | "need_verifycode" | "error";
  bot_token?: string;
  user_id?: string;
  message?: string;
}

// ── 会话上下文 ──

export interface ConversationContext {
  chat_id: string;
  messages: ChatMessage[];
  context_token?: string;
  last_active_at: number;
}

export interface ChatMessage {
  role: "user" | "assistant" | "system";
  content: string;
  timestamp: number;
  msg_id?: string;
}
