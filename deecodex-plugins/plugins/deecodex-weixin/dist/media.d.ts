import type { WeixinMedia } from "./types.js";
export declare function downloadMedia(media: WeixinMedia): Promise<string | null>;
export declare function aesEcbDecrypt(buffer: Buffer, keyHex: string): Buffer;
export declare function aesEcbEncrypt(buffer: Buffer, keyHex: string): Buffer;
export declare function transcodeSilkToWav(silkPath: string): Promise<string | null>;
export declare function mimeToExt(mime: string): string;
export declare function extToMime(ext: string): string;
