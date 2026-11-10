use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use crate::config::{client_url_host, ensure_private_directory, host_is_loopback};

pub const DEFAULT_PORT_SEARCH_COUNT: u16 = 20;
pub const GATEWAY_RUNTIME_FILE: &str = "gateway-runtime.json";

#[derive(Debug)]
pub struct BoundGateway {
    pub listener: TcpListener,
    pub port: u16,
    pub preferred_port: u16,
    pub selection_reason: Option<String>,
    pub attempted_ports: Vec<u16>,
}

impl BoundGateway {
    pub fn auto_selected(&self) -> bool {
        self.port != self.preferred_port
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayRuntimeEndpoint {
    pub schema_version: u32,
    pub pid: u32,
    pub host: String,
    pub port: u16,
    pub preferred_port: u16,
    pub port_auto: bool,
    pub auto_selected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_reason: Option<String>,
}

impl GatewayRuntimeEndpoint {
    pub fn new(
        host: impl Into<String>,
        port: u16,
        preferred_port: u16,
        port_auto: bool,
        selection_reason: Option<String>,
    ) -> Self {
        Self {
            schema_version: 1,
            pid: std::process::id(),
            host: host.into(),
            port,
            preferred_port,
            port_auto,
            auto_selected: port != preferred_port,
            selection_reason,
        }
    }
}

pub struct GatewayRuntimeLease {
    data_dir: PathBuf,
    pid: u32,
}

impl GatewayRuntimeLease {
    pub fn create(data_dir: &Path, endpoint: &GatewayRuntimeEndpoint) -> Result<Self> {
        write_runtime_endpoint(data_dir, endpoint)?;
        Ok(Self {
            data_dir: data_dir.to_path_buf(),
            pid: endpoint.pid,
        })
    }
}

impl Drop for GatewayRuntimeLease {
    fn drop(&mut self) {
        if let Err(error) = clear_runtime_endpoint(&self.data_dir, self.pid) {
            tracing::warn!(
                path = %runtime_endpoint_path(&self.data_dir).display(),
                %error,
                "清理网关运行时端点文件失败"
            );
        }
    }
}

pub async fn bind_gateway(
    host: &str,
    preferred_port: u16,
    auto_select: bool,
) -> Result<BoundGateway> {
    if preferred_port == 0 {
        bail!("服务监听端口不能为 0");
    }

    let allow_auto_select = auto_select && host_is_loopback(host);
    let candidates = candidate_ports(preferred_port, allow_auto_select);
    let mut attempted_ports = Vec::with_capacity(candidates.len());
    let mut last_error = None;

    for port in candidates {
        attempted_ports.push(port);
        match TcpListener::bind((host, port)).await {
            Ok(listener) => {
                let selection_reason = (port != preferred_port)
                    .then(|| format!("首选端口 {preferred_port} 被占用，自动选择端口 {port}"));
                return Ok(BoundGateway {
                    listener,
                    port,
                    preferred_port,
                    selection_reason,
                    attempted_ports,
                });
            }
            Err(error) => {
                let error_kind = error.kind();
                last_error = Some(error.to_string());
                if error_kind == ErrorKind::AddrInUse
                    && host_is_loopback(host)
                    && existing_deecodex_gateway(host, port).await
                {
                    bail!(
                        "候选端口 {port} 已有 DEX AI 网关运行；为避免启动重复网关，已停止自动选择端口"
                    );
                }
                if !allow_auto_select || error_kind != ErrorKind::AddrInUse {
                    break;
                }
            }
        }
    }

    let attempted = attempted_ports
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let source = last_error.unwrap_or_else(|| "没有可用候选端口".to_string());
    if allow_auto_select {
        bail!("无法绑定服务地址 {host}；已尝试端口 [{attempted}]: {source}");
    }
    bail!(
        "无法绑定固定服务地址 {}: {source}",
        crate::config::format_host_port(host, preferred_port)
    )
}

fn candidate_ports(preferred_port: u16, allow_auto_select: bool) -> Vec<u16> {
    let count = if allow_auto_select {
        DEFAULT_PORT_SEARCH_COUNT
    } else {
        1
    };
    (0..count)
        .filter_map(|offset| preferred_port.checked_add(offset))
        .collect()
}

async fn existing_deecodex_gateway(host: &str, port: u16) -> bool {
    let url = format!("http://{}:{port}/health", client_url_host(host));
    let client = match reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_millis(250))
        .timeout(Duration::from_millis(400))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };
    let response = match client.get(url).send().await {
        Ok(response) if response.status().is_success() => response,
        _ => return false,
    };
    let body = match response.json::<serde_json::Value>().await {
        Ok(body) => body,
        Err(_) => return false,
    };
    let service_matches =
        body.get("service").and_then(serde_json::Value::as_str) == Some("deecodex");
    let legacy_health_matches = body
        .get("version")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|version| !version.trim().is_empty());
    body.get("status").and_then(serde_json::Value::as_str) == Some("ok")
        && (service_matches || legacy_health_matches)
}

pub fn runtime_endpoint_path(data_dir: &Path) -> PathBuf {
    data_dir.join(GATEWAY_RUNTIME_FILE)
}

pub fn write_runtime_endpoint(data_dir: &Path, endpoint: &GatewayRuntimeEndpoint) -> Result<()> {
    ensure_private_directory(data_dir)?;
    let path = runtime_endpoint_path(data_dir);
    let temp_path = data_dir.join(format!(
        ".{GATEWAY_RUNTIME_FILE}.{}.{}.tmp",
        endpoint.pid, endpoint.port
    ));
    let content = serde_json::to_vec_pretty(endpoint)?;
    std::fs::write(&temp_path, content)
        .with_context(|| format!("写入运行时端点临时文件失败: {}", temp_path.display()))?;
    crate::config::set_private_file_permissions(&temp_path)?;
    #[cfg(windows)]
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("替换运行时端点文件失败: {}", path.display()))?;
    }
    std::fs::rename(&temp_path, &path)
        .with_context(|| format!("发布运行时端点文件失败: {}", path.display()))?;
    crate::config::set_private_file_permissions(&path)?;
    Ok(())
}

pub fn read_runtime_endpoint(data_dir: &Path) -> Result<Option<GatewayRuntimeEndpoint>> {
    let path = runtime_endpoint_path(data_dir);
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("读取运行时端点文件失败: {}", path.display()))
        }
    };
    let endpoint = serde_json::from_str(&content)
        .with_context(|| format!("解析运行时端点文件失败: {}", path.display()))?;
    Ok(Some(endpoint))
}

pub fn read_live_runtime_endpoint(data_dir: &Path) -> Result<Option<GatewayRuntimeEndpoint>> {
    let Some(endpoint) = read_runtime_endpoint(data_dir)? else {
        return Ok(None);
    };
    if process_is_running(endpoint.pid) {
        return Ok(Some(endpoint));
    }

    let path = runtime_endpoint_path(data_dir);
    if clear_runtime_endpoint(data_dir, endpoint.pid)? {
        tracing::warn!(
            path = %path.display(),
            stale_pid = endpoint.pid,
            "已清理失效的网关运行时端点文件"
        );
    }
    Ok(None)
}

pub fn process_is_running(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        return std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
    }
    #[cfg(windows)]
    {
        let filter = format!("PID eq {pid}");
        let mut command = std::process::Command::new("tasklist");
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        return command
            .args(["/FI", &filter, "/FO", "CSV", "/NH"])
            .output()
            .map(|output| {
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .any(|line| {
                            // tasklist CSV 首列是 Image Name，第 2 列是 PID：
                            // 按 CSV 结构判断，不依赖本地化文本（中文 Windows 的
                            // “没有运行的任务”提示不是 CSV，且编码可能是 GBK）。
                            let pid_str = pid.to_string();
                            line.split(',')
                                .nth(1)
                                .map(|field| field.trim_matches('"'))
                                == Some(pid_str.as_str())
                        })
            })
            .unwrap_or(false);
    }
    #[allow(unreachable_code)]
    false
}

pub fn clear_runtime_endpoint(data_dir: &Path, expected_pid: u32) -> Result<bool> {
    let path = runtime_endpoint_path(data_dir);
    let Some(endpoint) = read_runtime_endpoint(data_dir)? else {
        return Ok(false);
    };
    if endpoint.pid != expected_pid {
        return Ok(false);
    }
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => {
            Err(error).with_context(|| format!("删除运行时端点文件失败: {}", path.display()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    async fn serve_health_once(listener: TcpListener, body: &'static str) {
        let (mut stream, _) = listener.accept().await.unwrap();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    }

    async fn bind_consecutive_listeners(count: u16) -> Vec<TcpListener> {
        for base in 30_000..=(u16::MAX - count) {
            let mut listeners = Vec::with_capacity(count as usize);
            let mut available = true;
            for offset in 0..count {
                match TcpListener::bind(("127.0.0.1", base + offset)).await {
                    Ok(listener) => listeners.push(listener),
                    Err(_) => {
                        available = false;
                        break;
                    }
                }
            }
            if available {
                return listeners;
            }
        }
        panic!("没有找到连续的测试端口");
    }

    fn temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "deecodex-gateway-port-{label}-{}",
            uuid::Uuid::new_v4().simple()
        ))
    }

    #[tokio::test]
    async fn binds_preferred_port_when_available() {
        let probe = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);

        let bound = bind_gateway("127.0.0.1", port, true).await.unwrap();

        assert_eq!(bound.port, port);
        assert!(!bound.auto_selected());
        assert_eq!(bound.attempted_ports, vec![port]);
    }

    #[tokio::test]
    async fn selects_next_port_when_preferred_is_occupied() {
        let occupied = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = occupied.local_addr().unwrap().port();

        let bound = bind_gateway("127.0.0.1", port, true).await.unwrap();

        assert_ne!(bound.port, port);
        assert!(bound.auto_selected());
        assert!(bound.port > port);
    }

    #[tokio::test]
    async fn fixed_mode_fails_when_preferred_is_occupied() {
        let occupied = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = occupied.local_addr().unwrap().port();

        let error = bind_gateway("127.0.0.1", port, false)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("固定服务地址"));
    }

    #[tokio::test]
    async fn non_loopback_auto_mode_still_uses_fixed_port() {
        let occupied = TcpListener::bind(("0.0.0.0", 0)).await.unwrap();
        let port = occupied.local_addr().unwrap().port();

        let error = bind_gateway("0.0.0.0", port, true)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("固定服务地址"));
    }

    #[tokio::test]
    async fn existing_deecodex_gateway_blocks_auto_selection() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let responder = tokio::spawn(serve_health_once(
            listener,
            r#"{"status":"ok","version":"4.0.9"}"#,
        ));

        let error = bind_gateway("127.0.0.1", port, true)
            .await
            .unwrap_err()
            .to_string();
        responder.await.unwrap();

        assert!(error.contains("已有 DEX AI 网关运行"));
    }

    #[tokio::test]
    async fn existing_deecodex_gateway_on_later_candidate_blocks_duplicate_instance() {
        let mut listeners = bind_consecutive_listeners(2).await;
        let preferred_port = listeners[0].local_addr().unwrap().port();
        let deecodex_port = listeners[1].local_addr().unwrap().port();
        let deecodex_listener = listeners.pop().unwrap();
        let occupied_listener = listeners.pop().unwrap();
        let occupied_responder = tokio::spawn(serve_health_once(
            occupied_listener,
            r#"{"status":"ok","service":"another-service"}"#,
        ));
        let deecodex_responder = tokio::spawn(serve_health_once(
            deecodex_listener,
            r#"{"status":"ok","service":"deecodex","version":"4.0.9"}"#,
        ));

        let error = bind_gateway("127.0.0.1", preferred_port, true)
            .await
            .unwrap_err()
            .to_string();
        occupied_responder.await.unwrap();
        deecodex_responder.await.unwrap();

        assert!(error.contains(&format!("候选端口 {deecodex_port}")));
        assert!(error.contains("已有 DEX AI 网关运行"));
    }

    #[test]
    fn non_loopback_and_high_ports_do_not_expand_unsafely() {
        assert_eq!(candidate_ports(4446, false), vec![4446]);
        assert_eq!(candidate_ports(65534, true), vec![65534, 65535]);
    }

    #[test]
    fn runtime_endpoint_roundtrip_and_pid_guarded_cleanup() {
        let dir = temp_dir("runtime");
        let endpoint = GatewayRuntimeEndpoint {
            schema_version: 1,
            pid: 12345,
            host: "127.0.0.1".into(),
            port: 4447,
            preferred_port: 4446,
            port_auto: true,
            auto_selected: true,
            selection_reason: Some("测试".into()),
        };

        write_runtime_endpoint(&dir, &endpoint).unwrap();
        assert_eq!(read_runtime_endpoint(&dir).unwrap(), Some(endpoint));
        assert!(!clear_runtime_endpoint(&dir, 99999).unwrap());
        assert!(runtime_endpoint_path(&dir).exists());
        assert!(clear_runtime_endpoint(&dir, 12345).unwrap());
        assert!(!runtime_endpoint_path(&dir).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn stale_runtime_endpoint_is_removed_when_process_is_gone() {
        let dir = temp_dir("stale-runtime");
        let endpoint = GatewayRuntimeEndpoint {
            schema_version: 1,
            pid: u32::MAX,
            host: "127.0.0.1".into(),
            port: 4447,
            preferred_port: 4446,
            port_auto: true,
            auto_selected: true,
            selection_reason: Some("测试".into()),
        };

        write_runtime_endpoint(&dir, &endpoint).unwrap();

        assert_eq!(read_live_runtime_endpoint(&dir).unwrap(), None);
        assert!(!runtime_endpoint_path(&dir).exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
