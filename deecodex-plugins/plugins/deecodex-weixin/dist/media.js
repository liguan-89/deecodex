// 媒体处理：CDN 下载、AES-128-ECB 解密、SILK→WAV 转码
import * as crypto from "node:crypto";
import * as fs from "node:fs";
import * as path from "node:path";
import { getDataDir, sendNotification } from "./rpc-client.js";
const MEDIA_DIR = "weixin_media";
// ── CDN 下载 ──
export async function downloadMedia(media) {
    const cdnUrl = media.cdn_url;
    if (!cdnUrl)
        return null;
    const dataDir = getDataDir();
    const mediaDir = path.join(dataDir, MEDIA_DIR);
    fs.mkdirSync(mediaDir, { recursive: true });
    const ext = media.mime_type ? mimeToExt(media.mime_type) : ".bin";
    const baseName = media.media_id || crypto.randomBytes(8).toString("hex");
    const encPath = path.join(mediaDir, `${baseName}_enc${ext}`);
    const decPath = path.join(mediaDir, `${baseName}${ext}`);
    try {
        // 下载
        const response = await fetch(cdnUrl, {
            headers: {
                "User-Agent": "deecodex-weixin/1.0",
            },
            signal: AbortSignal.timeout(30_000),
        });
        if (!response.ok) {
            sendNotification("log", { level: "warn", message: `CDN 下载失败: ${response.status}` });
            return null;
        }
        const buffer = Buffer.from(await response.arrayBuffer());
        fs.writeFileSync(encPath, buffer);
        // 解密
        if (media.aes_key) {
            try {
                const decrypted = aesEcbDecrypt(buffer, media.aes_key);
                fs.writeFileSync(decPath, decrypted);
                // 删除加密文件
                fs.unlinkSync(encPath);
                return decPath;
            }
            catch (err) {
                sendNotification("log", { level: "warn", message: `AES 解密失败: ${String(err)}` });
                // 解密失败仍返回加密文件（某些文件可能未加密）
                return encPath;
            }
        }
        // 无加密 key，直接返回
        fs.renameSync(encPath, decPath);
        return decPath;
    }
    catch (err) {
        sendNotification("log", { level: "error", message: `媒体下载失败: ${String(err)}` });
        return null;
    }
}
// ── AES-128-ECB 解密 ──
export function aesEcbDecrypt(buffer, keyHex) {
    const key = Buffer.from(keyHex, "hex");
    // AES-128-ECB 无 iv
    const decipher = crypto.createDecipheriv("aes-128-ecb", key, Buffer.alloc(0));
    decipher.setAutoPadding(true);
    return Buffer.concat([decipher.update(buffer), decipher.final()]);
}
// ── AES-128-ECB 加密（用于 CDN 上传） ──
export function aesEcbEncrypt(buffer, keyHex) {
    const key = Buffer.from(keyHex, "hex");
    const cipher = crypto.createCipheriv("aes-128-ecb", key, Buffer.alloc(0));
    cipher.setAutoPadding(true);
    return Buffer.concat([cipher.update(buffer), cipher.final()]);
}
// ── SILK→WAV 转码 ──
export async function transcodeSilkToWav(silkPath) {
    const wavPath = silkPath.replace(/\.silk$/i, "") + ".wav";
    try {
        // 尝试用 silk-wasm（需要预先安装）
        // 回退到 ffmpeg（如果可用）
        const { execSync } = await import("node:child_process");
        // 先试 ffmpeg（更通用）
        try {
            execSync(`ffmpeg -y -f s16le -ar 24000 -ac 1 -i "${silkPath}" "${wavPath}"`, {
                stdio: "pipe",
                timeout: 30_000,
            });
            return wavPath;
        }
        catch {
            // ffmpeg 不可用，尝试 silk-v3-decoder
            try {
                execSync(`silk-v3-decoder "${silkPath}" "${wavPath}"`, {
                    stdio: "pipe",
                    timeout: 30_000,
                });
                return wavPath;
            }
            catch {
                sendNotification("log", {
                    level: "warn",
                    message: "SILK 转码失败：未找到 ffmpeg 或 silk-v3-decoder",
                });
                return silkPath; // 返回原始文件
            }
        }
    }
    catch {
        return null;
    }
}
// ── MIME 类型映射 ──
export function mimeToExt(mime) {
    const map = {
        "image/jpeg": ".jpg",
        "image/png": ".png",
        "image/gif": ".gif",
        "image/webp": ".webp",
        "audio/silk": ".silk",
        "audio/mp3": ".mp3",
        "audio/wav": ".wav",
        "audio/amr": ".amr",
        "video/mp4": ".mp4",
        "application/pdf": ".pdf",
        "application/octet-stream": ".bin",
    };
    return map[mime] || ".bin";
}
export function extToMime(ext) {
    const map = {
        ".jpg": "image/jpeg",
        ".jpeg": "image/jpeg",
        ".png": "image/png",
        ".gif": "image/gif",
        ".webp": "image/webp",
        ".silk": "audio/silk",
        ".mp3": "audio/mp3",
        ".wav": "audio/wav",
        ".amr": "audio/amr",
        ".mp4": "video/mp4",
        ".pdf": "application/pdf",
    };
    return map[ext.toLowerCase()] || "application/octet-stream";
}
//# sourceMappingURL=media.js.map