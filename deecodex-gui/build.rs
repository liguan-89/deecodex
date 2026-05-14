use std::fs;
use std::path::Path;

fn main() {
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
            let escaped = content
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
        println!("cargo:warning=nav fragments: {} 文件 → {}", files.len(), out.display());
    }
}
