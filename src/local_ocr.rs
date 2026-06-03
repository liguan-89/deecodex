use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::Value;
use time::OffsetDateTime;
use tracing::warn;

const MAX_OCR_IMAGES: usize = 4;
const MAX_DATA_URL_BYTES: usize = 8 * 1024 * 1024;
const MAX_OCR_TEXT_CHARS: usize = 12_000;

const MACOS_VISION_OCR_SCRIPT: &str = r#"
import Foundation
import Vision

let args = CommandLine.arguments
guard args.count >= 3 else {
    exit(2)
}
let imagePath = URL(fileURLWithPath: args[2])
let request = VNRecognizeTextRequest()
request.recognitionLevel = .accurate
request.usesLanguageCorrection = true
if #available(macOS 13.0, *) {
    request.automaticallyDetectsLanguage = true
}
let handler = VNImageRequestHandler(url: imagePath, options: [:])
do {
    try handler.perform([request])
    let text = (request.results ?? [])
        .compactMap { $0.topCandidates(1).first?.string }
        .joined(separator: "\n")
    print(text)
} catch {
    fputs(String(describing: error), stderr)
    exit(1)
}
"#;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OcrFallbackReport {
    pub image_count: usize,
    pub text: String,
}

pub async fn recognize_images_from_value(value: &Value) -> OcrFallbackReport {
    let mut image_urls = Vec::new();
    collect_image_urls(value, &mut image_urls);
    image_urls.truncate(MAX_OCR_IMAGES);
    recognize_image_urls(&image_urls).await
}

async fn recognize_image_urls(image_urls: &[String]) -> OcrFallbackReport {
    if image_urls.is_empty() {
        return OcrFallbackReport::default();
    }

    #[cfg(target_os = "macos")]
    {
        let mut sections = Vec::new();
        for (index, image_url) in image_urls.iter().enumerate() {
            match recognize_data_url_with_macos_vision(image_url).await {
                Ok(text) if !text.trim().is_empty() => {
                    sections.push(format!("[图片 {}]\n{}", index + 1, text.trim()));
                }
                Ok(_) => {
                    warn!(image_index = index + 1, "本机 OCR 未识别到文字");
                }
                Err(err) => {
                    warn!(image_index = index + 1, error = %err, "本机 OCR 失败，跳过该图片");
                }
            }
        }
        OcrFallbackReport {
            image_count: image_urls.len(),
            text: truncate_chars(sections.join("\n\n"), MAX_OCR_TEXT_CHARS),
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        OcrFallbackReport {
            image_count: image_urls.len(),
            text: String::new(),
        }
    }
}

#[cfg(target_os = "macos")]
async fn recognize_data_url_with_macos_vision(image_url: &str) -> Result<String> {
    let (mime, bytes) = decode_image_data_url(image_url)?;
    let extension = image_extension(mime);
    let file_name = format!(
        "deecodex-ocr-{}-{extension}",
        OffsetDateTime::now_utc().unix_timestamp_nanos()
    );
    let image_path = std::env::temp_dir().join(file_name);
    std::fs::write(&image_path, bytes).context("写入 OCR 临时图片失败")?;

    let result = run_macos_vision_ocr(&image_path).await;
    let _ = std::fs::remove_file(&image_path);
    result
}

#[cfg(target_os = "macos")]
async fn run_macos_vision_ocr(image_path: &std::path::Path) -> Result<String> {
    let image_path = image_path.to_string_lossy().to_string();
    tokio::task::spawn_blocking(move || -> Result<String> {
        let mut child = Command::new("swift")
            .arg("-")
            .arg("--")
            .arg(&image_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("启动 macOS Vision OCR 失败")?;
        if let Some(stdin) = child.stdin.as_mut() {
            stdin
                .write_all(MACOS_VISION_OCR_SCRIPT.as_bytes())
                .context("写入 OCR 脚本失败")?;
        }
        let output = child.wait_with_output().context("等待 OCR 进程失败")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(anyhow!("macOS Vision OCR 返回失败: {stderr}"));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    })
    .await
    .context("OCR 阻塞任务失败")?
}

fn collect_image_urls(value: &Value, urls: &mut Vec<String>) {
    match value {
        Value::String(text) if text.starts_with("data:image/") => {
            urls.push(text.to_string());
        }
        Value::String(_) => {}
        Value::Array(items) => {
            for item in items {
                collect_image_urls(item, urls);
            }
        }
        Value::Object(map) => {
            if let Some(url) = map.get("image_url").and_then(Value::as_str) {
                urls.push(url.to_string());
            } else if let Some(url) = map
                .get("image_url")
                .and_then(|v| v.get("url"))
                .and_then(Value::as_str)
            {
                urls.push(url.to_string());
            }
            if let Some(screenshot) = map.get("screenshot") {
                collect_image_urls(screenshot, urls);
            }
            for (key, value) in map {
                if matches!(key.as_str(), "image_url" | "screenshot") {
                    continue;
                }
                collect_image_urls(value, urls);
            }
        }
        _ => {}
    }
}

fn decode_image_data_url(data_url: &str) -> Result<(&str, Vec<u8>)> {
    let Some(rest) = data_url.strip_prefix("data:") else {
        return Err(anyhow!("仅支持 data URL 图片"));
    };
    let Some((mime, encoded)) = rest.split_once(";base64,") else {
        return Err(anyhow!("图片 data URL 缺少 base64 标记"));
    };
    if !mime.starts_with("image/") {
        return Err(anyhow!("不是图片 MIME: {mime}"));
    }
    if encoded.len() > MAX_DATA_URL_BYTES {
        return Err(anyhow!("图片 data URL 超过 OCR 限制"));
    }
    let bytes = STANDARD
        .decode(encoded)
        .context("图片 data URL base64 解码失败")?;
    Ok((mime, bytes))
}

fn image_extension(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/tiff" => "tiff",
        _ => "png",
    }
}

fn truncate_chars(value: String, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value;
    }
    value.chars().take(max_chars).collect::<String>() + "\n[OCR 文本过长，已截断]"
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn collect_image_urls_reads_input_image_and_screenshot() {
        let value = json!({
            "input": [{
                "content": [
                    {"type": "input_text", "text": "看图"},
                    {"type": "input_image", "image_url": "data:image/png;base64,abc"}
                ]
            }],
            "output": {"screenshot": "data:image/jpeg;base64,def"}
        });
        let mut urls = Vec::new();
        collect_image_urls(&value, &mut urls);
        assert!(urls.contains(&"data:image/png;base64,abc".to_string()));
        assert!(urls.contains(&"data:image/jpeg;base64,def".to_string()));
    }

    #[test]
    fn decode_image_data_url_rejects_non_image() {
        assert!(decode_image_data_url("data:text/plain;base64,abc").is_err());
    }
}
