use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tauri::State;

use crate::commands::load_args;
use crate::ServerManager;

use super::dex_protocol::get_active_account_info;
use deecodex::request_history::HistoryFilter;

pub(super) fn dex_config_backup_impl(
    action: String,
    name: Option<String>,
    confirmed: Option<bool>,
) -> Result<Value, String> {
    if action == "restore" && confirmed != Some(true) {
        return Err("安全限制：恢复配置会覆盖当前 config.json/accounts.json，必须先确认".into());
    }

    let args = load_args();
    let data_dir = &args.data_dir;
    let backup_dir = data_dir.join("backups");

    match action.as_str() {
        "backup" => {
            let name = name.ok_or("备份名称不能为空")?;
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let dir_name = format!("{}_{}", name, ts);
            let target = backup_dir.join(&dir_name);
            std::fs::create_dir_all(&target).map_err(|e| format!("创建备份目录失败: {e}"))?;

            let mut files = Vec::new();
            for fname in &["config.json", "accounts.json"] {
                let src = data_dir.join(fname);
                if src.exists() {
                    std::fs::copy(&src, target.join(fname))
                        .map_err(|e| format!("备份 {} 失败: {}", fname, e))?;
                    files.push(*fname);
                }
            }

            Ok(json!({
                "ok": true,
                "backup_name": dir_name,
                "files": files,
            }))
        }
        "restore" => {
            let name = name.ok_or("备份名称不能为空")?;
            let source = backup_dir.join(&name);
            if !source.exists() {
                return Err(format!("备份不存在: {}", name));
            }

            for fname in &["config.json", "accounts.json"] {
                let src = source.join(fname);
                if src.exists() {
                    std::fs::copy(&src, data_dir.join(fname))
                        .map_err(|e| format!("恢复 {} 失败: {}", fname, e))?;
                }
            }

            Ok(json!({ "ok": true }))
        }
        "list" => {
            if !backup_dir.exists() {
                return Ok(json!({ "backups": [] }));
            }

            let mut backups = Vec::new();
            let dir =
                std::fs::read_dir(&backup_dir).map_err(|e| format!("读取备份目录失败: {e}"))?;

            for entry in dir {
                let entry = entry.map_err(|e| format!("读取备份条目失败: {e}"))?;
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }

                let full_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();

                let (base_name, time_secs) = if let Some(pos) = full_name.rfind('_') {
                    let ts = full_name[pos + 1..].parse::<u64>().unwrap_or(0);
                    (full_name[..pos].to_string(), ts)
                } else {
                    (full_name.clone(), 0u64)
                };

                let mut files = Vec::new();
                if path.join("config.json").exists() {
                    files.push("config.json");
                }
                if path.join("accounts.json").exists() {
                    files.push("accounts.json");
                }

                backups.push(json!({
                    "name": full_name,
                    "base_name": base_name,
                    "time": time_secs,
                    "files": files,
                }));
            }

            backups.sort_by_key(|b| std::cmp::Reverse(b["time"].as_u64().unwrap_or(0)));

            Ok(json!({ "backups": backups }))
        }
        _ => Err(format!("未知操作: {}，支持: backup, restore, list", action)),
    }
}

fn extract_toml_value(content: &str, key: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(&format!("{} = ", key)) {
            return rest.trim().trim_matches('"').to_string();
        }
        if let Some(rest) = trimmed.strip_prefix(&format!("{}=", key)) {
            return rest.trim().trim_matches('"').to_string();
        }
    }
    String::new()
}

fn extract_port_from_url(url: &str) -> Option<u16> {
    let host = url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("");
    if let Some(port_str) = host.rsplit(':').next() {
        if port_str != host {
            return port_str.parse().ok();
        }
    }
    None
}

pub(super) fn dex_config_diff_impl() -> Result<Value, String> {
    let args = load_args();
    let data_dir = &args.data_dir;

    let deecodex_port = args.port;
    let (deecodex_upstream, deecodex_model_count, deecodex_provider) =
        if let Some((up, _, mm, _, provider, _, _, _)) = get_active_account_info(data_dir) {
            (up, mm.len(), provider)
        } else {
            (String::new(), 0, String::new())
        };

    let codex_toml = super::dex::codex_config_path()
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .unwrap_or_default();

    let codex_base_url = extract_toml_value(&codex_toml, "base_url");
    let codex_model_provider = extract_toml_value(&codex_toml, "model_provider");
    let codex_port = extract_port_from_url(&codex_base_url);

    let mut diffs = Vec::new();

    if codex_model_provider != "deecodex" {
        let severity = if codex_model_provider.is_empty() {
            "critical"
        } else {
            "warning"
        };
        diffs.push(json!({
            "field": "model_provider",
            "deecodex_value": "deecodex",
            "codex_value": if codex_model_provider.is_empty() {
                "(未设置)"
            } else {
                &codex_model_provider
            },
            "severity": severity,
        }));
    }

    if let Some(cp) = codex_port {
        if cp != deecodex_port {
            diffs.push(json!({
                "field": "port",
                "deecodex_value": deecodex_port,
                "codex_value": cp,
                "severity": "warning",
            }));
        }
    } else if codex_model_provider == "deecodex" && !codex_base_url.is_empty() {
        diffs.push(json!({
            "field": "port",
            "deecodex_value": deecodex_port,
            "codex_value": "(无法解析)",
            "severity": "warning",
        }));
    }

    Ok(json!({
        "deecodex": {
            "port": deecodex_port,
            "upstream": deecodex_upstream,
            "model_count": deecodex_model_count,
            "provider": deecodex_provider,
        },
        "codex": {
            "base_url": codex_base_url,
            "model_provider": codex_model_provider,
            "port": codex_port,
        },
        "diffs": diffs,
    }))
}

pub(super) async fn dex_token_cost_impl(
    manager: State<'_, ServerManager>,
) -> Result<Value, String> {
    let rh = manager.request_history.lock().await;
    let store = rh
        .as_ref()
        .ok_or("请求历史不可用（服务未启动或数据库未初始化）")?;
    let entries = store.list(1000, &HistoryFilter::default()).await;

    let total = entries.len();
    if total == 0 {
        return Ok(json!({
            "total_tokens": 0,
            "total_input": 0,
            "total_output": 0,
            "estimated_cost_usd": 0.0,
            "by_model": [],
        }));
    }

    let mut total_input: u64 = 0;
    let mut total_output: u64 = 0;
    let mut by_model: HashMap<String, (u64, u64)> = HashMap::new();

    for e in &entries {
        total_input += e.input_tokens as u64;
        total_output += e.output_tokens as u64;
        let entry = by_model.entry(e.model.clone()).or_insert((0, 0));
        entry.0 += e.input_tokens as u64;
        entry.1 += e.output_tokens as u64;
    }

    let input_cost = total_input as f64 / 1_000_000.0 * 0.5;
    let output_cost = total_output as f64 / 1_000_000.0 * 2.0;
    let total_cost = input_cost + output_cost;

    let mut models: Vec<Value> = by_model
        .into_iter()
        .map(|(model, (input, output))| {
            let cost = input as f64 / 1_000_000.0 * 0.5 + output as f64 / 1_000_000.0 * 2.0;
            json!({
                "model": model,
                "input_tokens": input,
                "output_tokens": output,
                "total_tokens": input + output,
                "estimated_cost_usd": (cost * 10000.0).round() / 10000.0,
            })
        })
        .collect();
    models.sort_by(|a, b| {
        b["estimated_cost_usd"]
            .as_f64()
            .unwrap_or(0.0)
            .partial_cmp(&a["estimated_cost_usd"].as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(json!({
        "total_tokens": total_input + total_output,
        "total_input": total_input,
        "total_output": total_output,
        "estimated_cost_usd": (total_cost * 10000.0).round() / 10000.0,
        "by_model": models,
    }))
}

pub(super) async fn dex_speed_test_impl() -> Result<Value, String> {
    let args = load_args();
    let (upstream, api_key, model_map, _, provider, profile, _, _) =
        get_active_account_info(&args.data_dir)
            .ok_or_else(|| "请先在账号管理中配置一个活跃账号".to_string())?;

    let base = upstream.trim_end_matches('/');
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

    let provider_profile = profile.slug.clone();
    let wire_protocol = format!("{:?}", profile.wire_protocol);
    let mut results = Vec::new();
    let model_pairs: Vec<(&String, &String)> = model_map.iter().take(5).collect();

    for (deecodex_model, upstream_model) in &model_pairs {
        let mut chat_req = deecodex::types::ChatRequest {
            model: (*upstream_model).clone(),
            messages: vec![deecodex::types::ChatMessage {
                role: "user".into(),
                content: Some(json!("hi")),
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
            tools: vec![],
            temperature: None,
            top_p: None,
            max_tokens: Some(1),
            stream: false,
            reasoning_effort: None,
            thinking: None,
            reasoning_split: None,
            tool_choice: None,
            parallel_tool_calls: None,
            response_format: None,
            user: None,
            stream_options: None,
            web_search_options: None,
        };
        deecodex::providers::adapt_chat_request(&profile, &mut chat_req);
        let (url, body) = match profile.wire_protocol {
            deecodex::providers::WireProtocol::ChatCompletions => (
                format!("{base}/chat/completions"),
                serde_json::to_value(&chat_req).unwrap_or_else(|_| json!({})),
            ),
            deecodex::providers::WireProtocol::AnthropicMessages
            | deecodex::providers::WireProtocol::GeminiNative => {
                let url = deecodex::native_protocols::native_endpoint(
                    &profile.wire_protocol,
                    &upstream,
                    &chat_req.model,
                    false,
                    &api_key,
                )
                .unwrap_or_else(|| format!("{base}/chat/completions"));
                let body = deecodex::native_protocols::to_native_request(
                    &profile.wire_protocol,
                    &chat_req,
                )
                .unwrap_or_else(|| json!({}));
                (url, body)
            }
            deecodex::providers::WireProtocol::Responses => (
                format!("{base}/responses"),
                serde_json::to_value(&chat_req).unwrap_or_else(|_| json!({})),
            ),
        };

        let start = std::time::Instant::now();
        let mut req = client.post(&url).json(&body);
        for (name, value) in deecodex::providers::request_headers(&profile, &api_key) {
            req = req.header(name, value);
        }

        match req.send().await {
            Ok(resp) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                let code = resp.status().as_u16();
                results.push(json!({
                    "model": deecodex_model,
                    "upstream_model": upstream_model,
                    "provider": &provider,
                    "provider_profile": &provider_profile,
                    "latency_ms": latency_ms,
                    "status": if code == 200 { "ok".to_string() } else { format!("http_{}", code) },
                }));
            }
            Err(e) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                results.push(json!({
                    "model": deecodex_model,
                    "upstream_model": upstream_model,
                    "provider": &provider,
                    "provider_profile": &provider_profile,
                    "latency_ms": latency_ms,
                    "status": format!("error: {}", e),
                }));
            }
        }
    }

    Ok(json!({
        "provider": provider,
        "provider_profile": provider_profile,
        "wire_protocol": wire_protocol,
        "results": results,
        "upstream": upstream,
    }))
}

pub(super) fn dex_thread_cleanup_impl(dry_run: Option<bool>) -> Result<Value, String> {
    let dry_run = dry_run.unwrap_or(true);

    let threads =
        deecodex::codex_threads::list_all().map_err(|e| format!("获取线程列表失败: {e}"))?;

    let mut empty_count = 0u64;
    let mut orphan_count = 0u64;
    let mut duplicate_count = 0u64;
    let mut seen_titles: HashSet<String> = HashSet::new();

    for t in &threads {
        if let Ok(content) = deecodex::codex_threads::get_thread_content(&t.id) {
            let msgs = content
                .get("messages")
                .and_then(|m| m.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            if msgs == 0 {
                empty_count += 1;
            }
        } else {
            orphan_count += 1;
        }

        if !t.title.is_empty() && !seen_titles.insert(t.title.clone()) {
            duplicate_count += 1;
        }
    }

    Ok(json!({
        "dry_run": dry_run,
        "total_threads": threads.len(),
        "empty_count": empty_count,
        "orphan_count": orphan_count,
        "duplicate_count": duplicate_count,
        "total_removable": empty_count + orphan_count + duplicate_count,
    }))
}

pub(super) fn dex_auto_tune_impl() -> Result<Value, String> {
    let args = load_args();

    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(4);
    let total_memory_gb = super::dex_diagnostics::get_total_memory_gb();
    let disk_free_gb = super::dex_diagnostics::get_disk_free_gb(&args.data_dir);

    let mut recommendations = Vec::new();

    let recommended_body_mb = if total_memory_gb >= 16.0 {
        200
    } else if total_memory_gb >= 8.0 {
        100
    } else {
        50
    };
    if (args.max_body_mb as u32) < recommended_body_mb {
        recommendations.push(json!({
            "param": "max_body_mb",
            "current": args.max_body_mb,
            "recommended": recommended_body_mb,
            "reason": format!("基于 {:.0}GB 内存推荐", total_memory_gb),
        }));
    }

    if disk_free_gb < 10.0 {
        recommendations.push(json!({
            "param": "disk_space",
            "current": format!("{:.1}GB", disk_free_gb),
            "recommended": "清理 data_dir 中的日志和备份",
            "reason": "磁盘剩余空间不足 10GB",
        }));
    }

    if cpu_cores >= 8 {
        recommendations.push(json!({
            "param": "concurrency",
            "current": "默认",
            "recommended": "可适当提高请求并发限制",
            "reason": format!("{} 核 CPU 有充足并行能力", cpu_cores),
        }));
    }

    Ok(json!({
        "system": {
            "cpu_cores": cpu_cores,
            "total_memory_gb": total_memory_gb,
            "disk_free_gb": (disk_free_gb * 10.0).round() / 10.0,
        },
        "recommendations": recommendations,
    }))
}

pub(super) fn dex_network_topology_impl() -> Result<Value, String> {
    let args = load_args();
    let upstream = args.upstream.clone();

    let host = upstream
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("");

    let dns_servers = get_dns_servers();

    #[cfg(target_os = "macos")]
    let ping_args: &[&str] = &["-c", "1", "-t", "1"];
    #[cfg(not(target_os = "macos"))]
    let ping_args: &[&str] = &["-c", "1", "-W", "1"];

    let (upstream_reachable, latency_ms) = if let Ok(out) = std::process::Command::new("ping")
        .args(ping_args)
        .arg(host)
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            let ms = parse_ping_latency(&s);
            (true, ms)
        } else {
            (false, None)
        }
    } else {
        (false, None)
    };

    Ok(json!({
        "dns_servers": dns_servers,
        "upstream_host": host,
        "upstream_reachable": upstream_reachable,
        "latency_ms": latency_ms,
    }))
}

fn get_dns_servers() -> Vec<String> {
    let mut servers = Vec::new();
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("scutil")
            .args(["--dns"])
            .output()
        {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout);
                for line in s.lines() {
                    if let Some(ip) = line
                        .trim()
                        .strip_prefix("nameserver[")
                        .and_then(|rest| rest.split("] : ").nth(1))
                    {
                        let ip = ip.trim();
                        if !servers.contains(&ip.to_string()) {
                            servers.push(ip.to_string());
                        }
                    }
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/etc/resolv.conf") {
            for line in content.lines() {
                if let Some(ip) = line.trim().strip_prefix("nameserver ") {
                    let ip = ip.trim();
                    if !servers.contains(&ip.to_string()) {
                        servers.push(ip.to_string());
                    }
                }
            }
        }
    }
    servers
}

fn parse_ping_latency(stdout: &str) -> Option<u64> {
    for part in stdout.split_whitespace() {
        if let Some(time_str) = part.strip_prefix("time=") {
            if let Ok(ms) = time_str.trim_end_matches("ms").trim().parse::<f64>() {
                return Some(ms as u64);
            }
        }
    }
    None
}

pub(super) fn dex_ssl_check_impl() -> Result<Value, String> {
    let args = load_args();
    let upstream = if args.upstream.is_empty() {
        "https://api.openai.com".to_string()
    } else {
        args.upstream.clone()
    };

    let provider = deecodex::providers::guess_provider(&upstream);
    let profile = deecodex::providers::profile_by_slug(provider);
    let check_url = deecodex::providers::model_discovery_url(&profile, &upstream, "")
        .unwrap_or_else(|| upstream.trim_end_matches('/').to_string());

    let output = std::process::Command::new("curl")
        .args(["-sI", "--max-time", "10", &check_url])
        .output();

    let (https_ok, status) = match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);

            if out.status.success() {
                if let Some(line) = stdout.lines().next() {
                    if line.contains("200") || line.contains("401") || line.contains("403") {
                        (
                            true,
                            format!(
                                "HTTPS 连接正常 (HTTP {})",
                                line.split_whitespace().nth(1).unwrap_or("?")
                            ),
                        )
                    } else if line.contains("301") || line.contains("302") {
                        (true, "HTTPS 连接正常 (重定向)".to_string())
                    } else {
                        (false, format!("异常响应: {}", line))
                    }
                } else {
                    (false, "无响应".to_string())
                }
            } else {
                let err = if !stderr.is_empty() { stderr } else { stdout };
                (false, format!("连接失败: {}", err.trim()))
            }
        }
        Err(e) => (false, format!("执行 curl 失败: {}", e)),
    };

    Ok(json!({
        "url": check_url,
        "https_ok": https_ok,
        "status": status,
    }))
}

pub(super) fn dex_export_report_impl() -> Result<Value, String> {
    let args = load_args();
    let data_dir = &args.data_dir;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut report = String::new();
    report.push_str("# deecodex 诊断报告\n\n");
    report.push_str(&format!("生成时间: {}\n", now));

    report.push_str("\n## 环境信息\n\n");
    report.push_str(&format!("- 操作系统: {}\n", std::env::consts::OS));
    report.push_str(&format!("- deecodex 版本: {}\n", env!("CARGO_PKG_VERSION")));
    report.push_str(&format!("- 数据目录: {}\n", data_dir.display()));

    report.push_str("\n## 配置文件\n\n");
    let config_exists = data_dir.join("config.json").exists();
    let accounts_exists = data_dir.join("accounts.json").exists();
    let codex_exists = super::dex::codex_config_path().is_some_and(|p| p.exists());
    report.push_str(&format!(
        "- config.json: {}\n",
        if config_exists { "存在" } else { "缺失" }
    ));
    report.push_str(&format!(
        "- accounts.json: {}\n",
        if accounts_exists { "存在" } else { "缺失" }
    ));
    report.push_str(&format!(
        "- Codex config.toml: {}\n",
        if codex_exists { "存在" } else { "缺失" }
    ));

    report.push_str("\n## 账号信息\n\n");
    let store = deecodex::accounts::load_accounts(data_dir);
    report.push_str(&format!("- 账号总数: {}\n", store.accounts.len()));
    report.push_str(&format!(
        "- 活跃账号: {}\n",
        store.active_id.as_deref().unwrap_or("无")
    ));
    for acc in &store.accounts {
        let active = if Some(&acc.id) == store.active_id.as_ref() {
            " [活跃]"
        } else {
            ""
        };
        let target = if acc.client_kind.is_codex() {
            "Codex 代理账号"
        } else {
            "客户端配置账号"
        };
        report.push_str(&format!(
            "  - {}{} ({}, {}), 模型数: {}, 最近检查: {}\n",
            acc.name,
            active,
            target,
            acc.provider,
            acc.model_map.len(),
            acc.last_check
                .as_ref()
                .map(|check| check.message.as_str())
                .unwrap_or("无")
        ));
    }

    report.push_str("\n## 线程状态\n\n");
    match deecodex::codex_threads::status(data_dir) {
        Ok(s) => {
            report.push_str(&format!("- 线程总数: {}\n", s.total));
            report.push_str(&format!("- 存在旧迁移备份: {}\n", s.migrated));
            report.push_str(&format!(
                "- 待归一 Codex Desktop 线程: {}\n",
                s.non_deecodex_count
            ));
        }
        Err(e) => report.push_str(&format!("- 获取失败: {}\n", e)),
    }

    report.push_str("\n## Codex 状态\n\n");
    report.push_str(&format!("- 已安装: {}\n", super::dex::codex_is_installed()));

    let report_path = data_dir.join("diagnostic_report.md");
    let saved_to = report_path.to_string_lossy().to_string();
    std::fs::write(&report_path, &report).map_err(|e| format!("保存报告失败: {e}"))?;

    Ok(json!({
        "markdown": report,
        "saved_to": saved_to,
    }))
}
