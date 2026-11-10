use super::load_args;
use serde_json::json;

#[tauri::command]
pub fn debug_gui_state() -> serde_json::Value {
    let args = load_args();
    let log_path = args.data_dir.join("deecodex.log");
    let (exists, size, modified) = match std::fs::metadata(&log_path) {
        Ok(m) => (
            true,
            m.len(),
            m.modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0),
        ),
        Err(_) => (false, 0, 0),
    };
    json!({
        "data_dir": args.data_dir.to_string_lossy(),
        "log_path": log_path.to_string_lossy(),
        "log_exists": exists,
        "log_size": size,
        "log_modified": modified,
        "runtime": {
            "hasTauri": true,
            "invoke_available": true
        }
    })
}

fn recent_log_lines(content: &str, max_lines: usize) -> Vec<String> {
    let lines: Vec<String> = content
        .lines()
        .map(|line| line.trim_start_matches('\u{feff}'))
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].to_vec()
}

fn clear_log_file(log_path: &std::path::Path) -> Result<(), String> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .create(true)
        .open(log_path)
        .map_err(|e| format!("无法打开日志文件: {e}"))?;
    use std::io::Write;
    f.write_all(&[0xEF, 0xBB, 0xBF])
        .map_err(|e| format!("写入日志文件失败: {e}"))?;
    f.flush().map_err(|e| format!("刷新日志文件失败: {e}"))?;
    Ok(())
}

#[tauri::command]
pub fn get_logs() -> Vec<String> {
    let args = load_args();
    let log_path = args.data_dir.join("deecodex.log");
    match std::fs::read_to_string(&log_path) {
        Ok(content) => {
            let lines = recent_log_lines(&content, 200);
            if lines.is_empty() {
                vec!["(暂无日志)".to_string()]
            } else {
                lines
            }
        }
        Err(_) => vec!["(暂无日志)".to_string()],
    }
}

#[tauri::command]
pub fn clear_logs() -> Result<(), String> {
    let args = load_args();
    let log_path = args.data_dir.join("deecodex.log");
    clear_log_file(&log_path)
}

#[cfg(test)]
mod tests {
    use super::{clear_log_file, recent_log_lines};
    use std::io::Read;

    #[test]
    fn test_clear_logs_logic() {
        let dir = std::env::temp_dir().join("deecodex_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test_clear.log");
        std::fs::write(&path, "line1\nline2\nline3\n").unwrap();

        clear_log_file(&path).unwrap();

        let mut buf = Vec::new();
        std::fs::File::open(&path)
            .unwrap()
            .read_to_end(&mut buf)
            .unwrap();
        assert_eq!(buf, &[0xEF, 0xBB, 0xBF]);
        assert!(recent_log_lines(&String::from_utf8_lossy(&buf), 200).is_empty());

        std::fs::remove_file(&path).unwrap();
    }
}
