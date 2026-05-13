export declare function startLogin(accountId: string): Promise<void>;
export declare function cancelLogin(accountId: string): void;
export declare function loadAccountToken(accountId: string): {
    bot_token?: string;
    user_id?: string;
} | null;
