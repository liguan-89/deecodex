import type { WeixinMsgContext } from "./types.js";
export declare function processMessage(accountId: string, ctx: WeixinMsgContext, contextTokens: Map<string, string>): Promise<void>;
