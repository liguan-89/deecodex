use deecodex_plugin_host::{PluginManager, PluginManifest, PluginState};
use std::path::PathBuf;
use tempfile::TempDir;
fn data_dir() -> TempDir {
    tempfile::tempdir().expect("创建临时目录失败")
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn echo_plugin_dir() -> PathBuf {
    fixtures_dir().join("echo-plugin")
}

// ── 单元测试 ──────────────────────────────────────────────────────────────────

#[test]
fn test_parse_manifest() {
    let manifest = PluginManifest::from_dir(&echo_plugin_dir()).expect("解析 manifest 失败");
    assert_eq!(manifest.id, "echo-test");
    assert_eq!(manifest.name, "Echo Test Plugin");
    assert_eq!(manifest.version, "1.0.0");
    assert_eq!(manifest.entry.runtime, "node");
    assert_eq!(manifest.entry.script, "index.js");
}

#[test]
fn test_manifest_validation_empty_id() {
    let json = r#"{
        "id": "",
        "name": "Test",
        "version": "1.0.0",
        "description": "test",
        "author": "tester",
        "entry": { "runtime": "node", "script": "main.js" }
    }"#;
    let result: Result<PluginManifest, _> = serde_json::from_str(json);
    assert!(result.is_ok());
    let manifest = result.unwrap();
    assert!(manifest.validate().is_err());
}

#[test]
fn test_manifest_validation_bad_runtime() {
    let json = r#"{
        "id": "test",
        "name": "Test",
        "version": "1.0.0",
        "description": "test",
        "author": "tester",
        "entry": { "runtime": "deno", "script": "main.ts" }
    }"#;
    let manifest: PluginManifest = serde_json::from_str(json).unwrap();
    assert!(manifest.validate().is_err());
}

#[test]
fn test_manifest_validation_good() {
    let json = r#"{
        "id": "test-plugin",
        "name": "Test Plugin",
        "version": "2.0.0",
        "description": "A test plugin",
        "author": "unit test",
        "entry": {
            "runtime": "python",
            "script": "main.py",
            "args": ["--debug"]
        },
        "permissions": ["http", "llm.call"],
        "min_deecodex_version": "1.0.0"
    }"#;
    let manifest: PluginManifest = serde_json::from_str(json).unwrap();
    assert!(manifest.validate().is_ok());
    assert_eq!(manifest.entry.runtime, "python");
    assert_eq!(manifest.entry.args.len(), 1);
    assert_eq!(manifest.permissions.len(), 2);
}

#[test]
fn test_rpc_message_roundtrip() {
    use deecodex_plugin_host::rpc::{
        JsonRpcMessage, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse,
    };

    // Request
    let req = JsonRpcRequest::new(42, "test.method", Some(serde_json::json!({"key": "value"})));
    let msg = JsonRpcMessage::Request(req);
    let line = msg.to_line();
    let parsed = JsonRpcMessage::from_line(&line).unwrap();
    match parsed {
        JsonRpcMessage::Request(r) => {
            assert_eq!(r.id, 42);
            assert_eq!(r.method, "test.method");
        }
        _ => panic!("应为 Request"),
    }

    // Response
    let resp = JsonRpcResponse::success(42, serde_json::json!({"ok": true}));
    let msg = JsonRpcMessage::Response(resp);
    let line = msg.to_line();
    let parsed = JsonRpcMessage::from_line(&line).unwrap();
    match parsed {
        JsonRpcMessage::Response(r) => {
            assert_eq!(r.id, 42);
            assert_eq!(r.result, Some(serde_json::json!({"ok": true})));
        }
        _ => panic!("应为 Response"),
    }

    // Notification
    let notif = JsonRpcNotification::new("log", Some(serde_json::json!({"level": "info"})));
    let msg = JsonRpcMessage::Notification(notif);
    let line = msg.to_line();
    let parsed = JsonRpcMessage::from_line(&line).unwrap();
    match parsed {
        JsonRpcMessage::Notification(n) => {
            assert_eq!(n.method, "log");
        }
        _ => panic!("应为 Notification"),
    }
}

#[test]
fn test_rpc_parse_empty_line() {
    let result = deecodex_plugin_host::rpc::JsonRpcMessage::from_line("");
    assert!(result.is_none());
}

#[test]
fn test_rpc_parse_invalid_json() {
    let result = deecodex_plugin_host::rpc::JsonRpcMessage::from_line("not json");
    assert!(result.is_none());
}

#[test]
fn test_store_crud() {
    use deecodex_plugin_host::store::PluginStore;
    use serde_json::json;

    let dir = data_dir();
    let dir_path = dir.path().to_path_buf();

    let mut store = PluginStore::load(&dir_path);
    assert!(store.plugins.is_empty());

    let manifest: PluginManifest = serde_json::from_str(
        r#"{"id":"test","name":"Test","version":"1.0.0","description":"t","author":"a","entry":{"runtime":"node","script":"main.js"}}"#,
    )
    .unwrap();

    store.add_plugin(manifest.clone());
    assert_eq!(store.plugins.len(), 1);

    let found = store.get_plugin("test");
    assert!(found.is_some());
    assert_eq!(found.unwrap().manifest.id, "test");

    store.update_config("test", json!({"key": "val"})).unwrap();
    let updated = store.get_plugin("test").unwrap();
    assert_eq!(updated.config["key"], "val");

    let removed = store.remove_plugin("test");
    assert!(removed.is_some());
    assert!(store.plugins.is_empty());

    store.save(&dir_path).unwrap();
}

// ── 集成测试（需要 node） ─────────────────────────────────────────────────────

fn has_node() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[tokio::test]
async fn test_echo_plugin_lifecycle() {
    if !has_node() {
        eprintln!("跳过：未找到 node");
        return;
    }

    let dir = data_dir();
    let manager = PluginManager::new(dir.path().to_path_buf(), "http://127.0.0.1:4446".into());

    // 安装
    let manifest = manager.install(&echo_plugin_dir()).await.expect("安装失败");
    assert_eq!(manifest.id, "echo-test");

    // 启动
    manager.start("echo-test").await.expect("启动失败");

    // 检查状态
    let list = manager.list().await;
    let info = list
        .iter()
        .find(|p| p.id == "echo-test")
        .expect("找不到插件");
    assert_eq!(info.state, PluginState::Running);

    // 停止
    manager.stop("echo-test").await.expect("停止失败");

    let list = manager.list().await;
    let info = list
        .iter()
        .find(|p| p.id == "echo-test")
        .expect("找不到插件");
    assert_eq!(info.state, PluginState::Stopped);

    // 卸载
    manager.uninstall("echo-test").await.expect("卸载失败");
    assert!(manager.list().await.iter().all(|p| p.id != "echo-test"));
}

#[tokio::test]
async fn test_echo_plugin_config_update() {
    if !has_node() {
        eprintln!("跳过：未找到 node");
        return;
    }

    let dir = data_dir();
    let manager = PluginManager::new(dir.path().to_path_buf(), "http://127.0.0.1:4446".into());

    manager.install(&echo_plugin_dir()).await.unwrap();
    manager.start("echo-test").await.unwrap();

    // 更新配置
    manager
        .update_config("echo-test", serde_json::json!({"echo_prefix": "PONG:"}))
        .await
        .unwrap();

    manager.stop("echo-test").await.unwrap();
    manager.uninstall("echo-test").await.unwrap();
}

#[tokio::test]
async fn test_event_subscription() {
    if !has_node() {
        eprintln!("跳过：未找到 node");
        return;
    }

    let dir = data_dir();
    let manager = PluginManager::new(dir.path().to_path_buf(), "http://127.0.0.1:4446".into());

    let mut rx = manager.subscribe_events();

    manager.install(&echo_plugin_dir()).await.unwrap();
    manager.start("echo-test").await.unwrap();

    // 等待启动事件
    let mut found_log = false;
    let mut found_status = false;
    let timeout = tokio::time::sleep(std::time::Duration::from_secs(5));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            event = rx.recv() => {
                match event.unwrap() {
                    deecodex_plugin_host::PluginEvent::Log { plugin_id, .. }
                        if plugin_id == "echo-test" => {
                        found_log = true;
                    }
                    deecodex_plugin_host::PluginEvent::StatusChanged { plugin_id, .. }
                        if plugin_id == "echo-test" => {
                        found_status = true;
                    }
                    _ => {}
                }
                if found_log && found_status {
                    break;
                }
            }
            _ = &mut timeout => {
                break;
            }
        }
    }

    assert!(found_log, "应该收到插件的 log 事件");
    assert!(found_status, "应该收到插件的 status 事件");

    manager.stop("echo-test").await.unwrap();
    manager.uninstall("echo-test").await.unwrap();
}
