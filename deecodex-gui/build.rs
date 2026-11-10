use std::fs;
use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

fn main() {
    println!("cargo:rerun-if-changed=gui");
    println!("cargo:rerun-if-changed=../deecodex-plugins/plugins/deecodex-mobile-codex");
    println!("cargo:rerun-if-changed=../deecodex-plugins/plugins/deecodex-desktop-sync");
    println!("cargo:rerun-if-env-changed=DEX_AI_PREVIEW_BUILD");
    println!("cargo:rerun-if-env-changed=DEECODEX_MOBILE_CODEX_PLUGIN_SHA256");
    println!("cargo:rerun-if-env-changed=DEECODEX_DESKTOP_SYNC_PLUGIN_SHA256");
    println!("cargo:rerun-if-env-changed=DEECODEX_REQUIRE_MOBILE_CODEX_PLUGIN");
    mark_workspace_build_dir_noindex();
    validate_mobile_codex_release_package();
    tauri_build::build();

    // 拼接 gui/nav/*.html 为 fragments.js，避免 fetch 在 webview 中失败
    let nav_dir = Path::new("gui/nav");
    if nav_dir.is_dir() {
        let mut files: Vec<_> = fs::read_dir(nav_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x == "html").unwrap_or(false))
            .collect();
        files.sort_by_key(|e| e.file_name());

        let mut js = String::from("window._navFragments=[\n");
        for entry in &files {
            let content = fs::read_to_string(entry.path()).unwrap_or_default();
            // Windows 检出（core.autocrlf → CRLF）下 nav/*.html 可能是 CRLF；
            // 先剥掉 \r，否则 JS 字符串字面量里残留的原始换行会触发
            // SyntaxError（Invalid or unexpected token），使生成的 fragments.js
            // 整体失效、左侧导航缺失（Windows 专属问题）。
            let escaped = content
                .replace('\r', "")
                .replace('\\', "\\\\")
                .replace('\n', "\\n")
                .replace('\'', "\\'");
            js.push_str(&format!("  '{}',\n", escaped));
        }
        js.push_str("];\n");

        let out = nav_dir.join("fragments.js");
        // 内容未变则跳过写入，避免触发 Tauri 文件监听器无限重建
        if fs::read_to_string(&out).map(|e| e == js).unwrap_or(false) {
            return;
        }
        fs::write(&out, js).unwrap();
        println!(
            "cargo:warning=nav fragments: {} 文件 → {}",
            files.len(),
            out.display()
        );
    }
}

fn validate_mobile_codex_release_package() {
    if std::env::var("PROFILE").as_deref() != Ok("release") {
        return;
    }
    let require_plugin = std::env::var("DEECODEX_REQUIRE_MOBILE_CODEX_PLUGIN")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        });
    let suffix = if std::env::var("TARGET")
        .unwrap_or_default()
        .contains("windows")
    {
        ".exe"
    } else {
        ""
    };
    validate_plugin_package(
        Path::new("../deecodex-plugins/plugins/deecodex-mobile-codex"),
        &[
            "plugin.json".to_string(),
            format!("mobile-codex-dex-plugin{suffix}"),
            format!("mobile-codex-dex-plugin-current{suffix}"),
            format!("mobile-codex-companion{suffix}"),
            format!("mobile-codex-companion-current{suffix}"),
            format!("mobile-codex-netd{suffix}"),
            format!("mobile-codex-shim{suffix}"),
        ],
        "DEECODEX_MOBILE_CODEX_PLUGIN_SHA256",
        require_plugin,
    );
    validate_plugin_package(
        Path::new("../deecodex-plugins/plugins/deecodex-desktop-sync"),
        &[
            "plugin.json".to_string(),
            format!("mobile-codex-desktop-sync-plugin{suffix}"),
        ],
        "DEECODEX_DESKTOP_SYNC_PLUGIN_SHA256",
        require_plugin,
    );
}

fn validate_plugin_package(
    package: &Path,
    required: &[String],
    hash_env: &str,
    require_plugin: bool,
) {
    let missing = required
        .iter()
        .filter(|name| !package.join(name).is_file())
        .cloned()
        .collect::<Vec<_>>();
    let hash = std::env::var(hash_env).unwrap_or_default();
    // 平台二进制是否全部缺失（plugin.json 不算平台产物）。
    // 交叉编译（如 macOS → Windows）时目录里可能只有 macOS/Linux 产物 + plugin.json，
    // 此时应视为"未提供该平台插件包"而跳过，而非 panic——与 CI 上产物全缺的行为一致。
    let any_platform_bin_present = required
        .iter()
        .filter(|name| name.as_str() != "plugin.json")
        .any(|name| package.join(name).is_file());
    let package_absent = !any_platform_bin_present;
    if package_absent && hash.trim().is_empty() && !require_plugin {
        println!(
            "cargo:warning=未提供第一方插件包 {}；本次 release 不打包该可选组件",
            package.display()
        );
        return;
    }
    if !missing.is_empty() {
        panic!(
            "DEX AI release 缺少第一方插件产物：{}。先运行 Mobile Codex scripts/package-dex-plugin.sh 并输出到 {}",
            missing.join(", "),
            package.display()
        );
    }
    if hash.trim().len() != 64 || !hash.chars().all(|ch| ch.is_ascii_hexdigit()) {
        panic!("DEX AI release 必须设置 64 位十六进制 {hash_env}");
    }
    let actual_hash = package_hash(package)
        .unwrap_or_else(|error| panic!("无法计算第一方插件包哈希 {}: {error}", package.display()));
    if !actual_hash.eq_ignore_ascii_case(hash.trim()) {
        panic!(
            "第一方插件包哈希不一致 {}：期望 {}，实际 {}",
            package.display(),
            hash.trim(),
            actual_hash
        );
    }
}

fn package_hash(root: &Path) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    hash_dir(root, root, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

// 注意：插件包哈希按文件原始字节计算。Windows 检出若开启 `core.autocrlf=true`
// 会把 JS/PY 等文本转成 CRLF，导致与 macOS/Linux（LF）计算的哈希不一致，
// release 校验会 panic。检出请保持行尾一致（`core.autocrlf=false` 或 `input`）。
fn hash_dir(root: &Path, dir: &Path, hasher: &mut Sha256) -> std::io::Result<()> {
    let mut entries = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        if name == ".git" || name == ".DS_Store" {
            continue;
        }
        let relative = path.strip_prefix(root).unwrap_or(&path);
        hasher.update(relative.to_string_lossy().as_bytes());
        if path.is_dir() {
            hasher.update(b"/");
            hash_dir(root, &path, hasher)?;
        } else {
            hasher.update(b"\0");
            let mut file = fs::File::open(&path)?;
            let mut buffer = [0_u8; 16 * 1024];
            loop {
                let length = file.read(&mut buffer)?;
                if length == 0 {
                    break;
                }
                hasher.update(&buffer[..length]);
            }
        }
    }
    Ok(())
}

fn mark_workspace_build_dir_noindex() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let target_dir = Path::new("../target-mac");
    if fs::create_dir_all(target_dir).is_ok() {
        let _ = fs::write(target_dir.join(".metadata_never_index"), "");
    }
}
