import type { WeixinMessage, SendMessageReq } from "./types.js";
export declare function getUpdates(token: string, buf?: string): Promise<{
    ret: number;
    errcode?: number;
    msgs?: WeixinMessage[];
    get_updates_buf?: string;
}>;
export declare function sendMessage(token: string, req: SendMessageReq): Promise<{
    ret: number;
    msg_id?: string;
}>;
export declare function sendTyping(token: string, chatId: string): Promise<void>;
export declare function getUploadUrl(token: string, fileSize: number, mimeType: string): Promise<{
    upload_url: string;
    media_id: string;
}>;
export declare function uploadToCdn(url: string, buffer: Buffer, mimeType: string): Promise<void>;
export declare function notifyStart(token: string): Promise<void>;
export declare function notifyStop(token: string): Promise<void>;
export declare function getConfig_ilink(token: string): Promise<{
    typing_ticket?: string;
}>;
