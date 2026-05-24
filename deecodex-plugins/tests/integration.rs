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

fn templates_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates")
}

fn copy_fixture_dir(src: &std::path::Path, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).expect("创建测试插件目录失败");
    for entry in std::fs::read_dir(src).expect("读取 fixture 失败") {
        let entry = entry.expect("读取 fixture 条目失败");
        let target = dst.join(entry.file_name());
        if entry.path().is_dir() {
            copy_fixture_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).expect("复制 fixture 文件失败");
        }
    }
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
    assert_eq!(manifest.kind, "tool");
    assert_eq!(manifest.features.len(), 1);
    assert!(manifest.features[0].params_schema.contains_key("message"));
    assert_eq!(manifest.dex_tools.len(), 3);
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
fn test_templates_parse_and_validate() {
    let dir = templates_dir();
    let mut count = 0;
    for entry in std::fs::read_dir(&dir).expect("读取 templates 失败") {
        let entry = entry.expect("读取 template 失败");
        if !entry.path().join("plugin.json").exists() {
            continue;
        }
        let manifest = PluginManifest::from_dir(&entry.path()).expect("解析模板 manifest 失败");
        assert!(manifest.validate().is_ok());
        for feature in &manifest.features {
            for action in feature.params_schema.keys() {
                assert!(
                    feature.methods.contains_key(action),
                    "params_schema 动作必须在 methods 中声明: {} / {}",
                    manifest.id,
                    action
                );
            }
        }
        count += 1;
    }
    assert!(
        count >= 3,
        "至少应包含 node-tool、python-datasource、node-automation 模板"
    );
}

#[test]
fn test_templates_include_sdk_helpers() {
    let dir = templates_dir();
    assert!(
        dir.join("node-tool").join("deecodex-plugin.js").exists(),
        "Node 工具模板应包含本地 SDK helper"
    );
    assert!(
        dir.join("node-automation")
            .join("deecodex-plugin.js")
            .exists(),
        "Node 自动化模板应包含本地 SDK helper"
    );
    assert!(
        dir.join("python-datasource")
            .join("deecodex_plugin.py")
            .exists(),
        "Python 数据源模板应包含本地 SDK helper"
    );
}

#[test]
fn test_manifest_dex_tools_parse_and_validate() {
    let json = r#"{
        "id": "tool-plugin",
        "name": "Tool Plugin",
        "version": "1.0.0",
        "description": "DEX tool provider",
        "author": "unit test",
        "entry": { "runtime": "node", "script": "main.js" },
        "dex_tools": [{
            "name": "echo_status",
            "description": "回显状态",
            "level": 1,
            "method": "echo.status",
            "capability": "plugins.dynamic",
            "parameters": {
                "type": "object",
                "properties": { "message": { "type": "string" } },
                "required": ["message"]
            }
        }]
    }"#;
    let manifest: PluginManifest = serde_json::from_str(json).unwrap();
    assert!(manifest.validate().is_ok());
    assert_eq!(manifest.dex_tools.len(), 1);
    assert_eq!(manifest.dex_tools[0].name, "echo_status");
    assert_eq!(manifest.dex_tools[0].method, "echo.status");
    assert_eq!(manifest.dex_tools[0].level, 1);
}

#[test]
fn test_manifest_dex_tools_reject_invalid_level_and_name() {
    let bad_level = r#"{
        "id": "bad-tool-plugin",
        "name": "Bad Tool Plugin",
        "version": "1.0.0",
        "description": "bad",
        "author": "unit test",
        "entry": { "runtime": "node", "script": "main.js" },
        "dex_tools": [{ "name": "bad", "description": "bad", "level": 9, "method": "bad.run" }]
    }"#;
    let manifest: PluginManifest = serde_json::from_str(bad_level).unwrap();
    assert!(manifest.validate().is_err());

    let bad_name = r#"{
        "id": "bad-name-plugin",
        "name": "Bad Name Plugin",
        "version": "1.0.0",
        "description": "bad",
        "author": "unit test",
        "entry": { "runtime": "node", "script": "main.js" },
        "dex_tools": [{ "name": "bad.name", "description": "bad", "level": 1, "method": "bad.run" }]
    }"#;
    let manifest: PluginManifest = serde_json::from_str(bad_name).unwrap();
    assert!(manifest.validate().is_err());

    let bad_capability = r#"{
        "id": "bad-capability-plugin",
        "name": "Bad Capability Plugin",
        "version": "1.0.0",
        "description": "bad",
        "author": "unit test",
        "entry": { "runtime": "node", "script": "main.js" },
        "dex_tools": [{ "name": "bad", "description": "bad", "level": 1, "method": "bad.run", "capability": "core.system" }]
    }"#;
    let manifest: PluginManifest = serde_json::from_str(bad_capability).unwrap();
    assert!(manifest.validate().is_err());
}

#[test]
fn test_manifest_dex_tools_allow_plugin_scoped_capability() {
    let json = r#"{
        "id": "tool-plugin",
        "name": "Tool Plugin",
        "version": "1.0.0",
        "description": "DEX tool provider",
        "author": "unit test",
        "entry": { "runtime": "node", "script": "main.js" },
        "dex_tools": [{ "name": "echo", "description": "echo", "level": 0, "method": "echo.run", "capability": "plugin.tool-plugin" }]
    }"#;
    let manifest: PluginManifest = serde_json::from_str(json).unwrap();
    assert!(manifest.validate().is_ok());
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

    store.add_plugin(manifest.clone(), "fixture".into(), "sha256:test".into());
    assert_eq!(store.plugins.len(), 1);

    let found = store.get_plugin("test");
    assert!(found.is_some());
    assert_eq!(found.unwrap().manifest.id, "test");

    store.update_config("test", json!({"key": "val"})).unwrap();
    let updated = store.get_plugin("test").unwrap();
    assert_eq!(updated.config["key"], "val");

    store
        .upsert_account_asset("test", "acct-1", json!({"name": "Account 1"}))
        .unwrap();
    let updated = store.get_plugin("test").unwrap();
    assert_eq!(updated.account_assets["acct-1"]["name"], "Account 1");

    let removed = store.remove_plugin("test");
    assert!(removed.is_some());
    assert!(store.plugins.is_empty());

    store.save(&dir_path).unwrap();
}

#[test]
fn test_store_migrates_legacy_config_accounts_to_account_assets() {
    let dir = data_dir();
    let dir_path = dir.path().to_path_buf();
    std::fs::write(
        dir_path.join("plugins.json"),
        r#"{
          "plugins": [{
            "manifest": {
              "id": "legacy",
              "name": "Legacy",
              "version": "1.0.0",
              "description": "legacy",
              "author": "unit test",
              "entry": { "runtime": "node", "script": "index.js" }
            },
            "config": {
              "base_url": "https://example.com",
              "accounts": {
                "acct": { "name": "Legacy Account" }
              }
            },
            "enabled": true,
            "source_path": "fixture",
            "source_hash": "sha256:test",
            "installed_at": 1
          }]
        }"#,
    )
    .expect("写入旧注册表失败");

    let store = deecodex_plugin_host::store::PluginStore::load(&dir_path);
    let record = store.get_plugin("legacy").expect("旧插件应加载");
    assert_eq!(record.config["base_url"], "https://example.com");
    assert!(record.config.get("accounts").is_none());
    assert_eq!(record.account_assets["acct"]["name"], "Legacy Account");
}

// ── 集成测试（需要 node） ─────────────────────────────────────────────────────

fn has_node() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn create_storage_plugin(root: &std::path::Path, plugin_id: &str, permissions: &[&str]) -> PathBuf {
    let plugin_dir = root.join(plugin_id);
    std::fs::create_dir_all(&plugin_dir).expect("创建 storage 插件目录失败");
    let manifest = serde_json::json!({
        "id": plugin_id,
        "name": "Storage Test",
        "version": "1.0.0",
        "description": "受控资产 API 测试插件",
        "author": "unit test",
        "entry": { "runtime": "node", "script": "index.js" },
        "permissions": permissions,
        "features": [{
            "id": "storage",
            "kind": "tool",
            "label": "Storage",
            "methods": { "run": "storage.run" }
        }]
    });
    std::fs::write(
        plugin_dir.join("plugin.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .expect("写入 storage manifest 失败");
    std::fs::write(
        plugin_dir.join("index.js"),
        r#"
const readline = require('readline');

let nextHostRequestId = 1000;
const pending = new Map();
const rl = readline.createInterface({ input: process.stdin });

rl.on('line', line => {
  let msg;
  try { msg = JSON.parse(line.trim()); } catch { return; }
  if (!msg || msg.jsonrpc !== '2.0') return;
  if (msg.id !== undefined && !msg.method) {
    const waiter = pending.get(msg.id);
    if (!waiter) return;
    pending.delete(msg.id);
    if (msg.error) waiter.reject(new Error(msg.error.message || 'host error'));
    else waiter.resolve(msg.result || {});
    return;
  }
  if (msg.id !== undefined && msg.method) handleRequest(msg);
  else if (msg.method === 'shutdown') setTimeout(() => process.exit(0), 20);
});

async function handleRequest(req) {
  try {
    if (req.method === 'initialize') {
      respond(req.id, { ok: true });
      return;
    }
    if (req.method === 'storage.run') {
      const action = req.params?.action || 'full';
      if (action === 'traversal') {
        respond(req.id, await capture('assets.read', { path: '../outside.txt' }));
        return;
      }
      if (action === 'write') {
        respond(req.id, await capture('assets.write', { path: 'state.txt', content: 'hello' }));
        return;
      }
      await hostRequest('assets.write', { path: 'state.txt', content: 'hello' });
      const read = await hostRequest('assets.read', { path: 'state.txt' });
      await hostRequest('cache.write', { path: 'memo.txt', content: 'cached' });
      await hostRequest('secrets.set', { key: 'token.txt', content: 'secret-token' });
      const secret = await hostRequest('secrets.get', { key: 'token.txt' });
      respond(req.id, { ok: true, read, secret });
      return;
    }
    respondError(req.id, -32601, 'Method not found: ' + req.method);
  } catch (error) {
    respondError(req.id, -32603, String(error.message || error));
  }
}

function hostRequest(method, params) {
  const id = nextHostRequestId++;
  process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    setTimeout(() => {
      if (!pending.has(id)) return;
      pending.delete(id);
      reject(new Error('timeout: ' + method));
    }, 5000);
  });
}

async function capture(method, params) {
  try {
    const result = await hostRequest(method, params);
    return { ok: true, result };
  } catch (error) {
    return { ok: false, error: String(error.message || error) };
  }
}

function respond(id, result) {
  process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id, result }) + '\n');
}

function respondError(id, code, message) {
  process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id, error: { code, message } }) + '\n');
}
"#,
    )
    .expect("写入 storage 插件脚本失败");
    plugin_dir
}

#[tokio::test]
async fn test_plugin_install_preview() {
    let dir = data_dir();
    let manager = PluginManager::new(dir.path().to_path_buf(), "http://127.0.0.1:4446".into());

    let preview = manager
        .preview_install(&echo_plugin_dir())
        .await
        .expect("预览失败");
    assert_eq!(preview.manifest.id, "echo-test");
    assert!(!preview.already_installed);
    assert_eq!(preview.permission_risk, "medium");
    assert!(!preview.source_hash.is_empty());

    manager.install(&echo_plugin_dir()).await.expect("安装失败");
    let preview_after_install = manager
        .preview_install(&echo_plugin_dir())
        .await
        .expect("预览失败");
    assert!(preview_after_install.already_installed);
}

#[tokio::test]
async fn test_node_tool_template_sdk_lifecycle() {
    if !has_node() {
        eprintln!("跳过：未找到 node");
        return;
    }

    let dir = data_dir();
    let manager = PluginManager::new(dir.path().to_path_buf(), "http://127.0.0.1:4446".into());
    let template_dir = templates_dir().join("node-tool");

    manager.install(&template_dir).await.expect("安装模板失败");
    manager
        .start("example-node-tool")
        .await
        .expect("启动模板失败");
    let result = manager
        .send_request(
            "example-node-tool",
            "example.status",
            Some(serde_json::json!({})),
        )
        .await
        .expect("执行模板方法失败");

    assert_eq!(result["ok"], true);
    assert_eq!(result["message"], "ready");
    assert!(result["cache"]["ts"].as_i64().unwrap_or_default() > 0);

    let info = manager
        .list()
        .await
        .into_iter()
        .find(|plugin| plugin.id == "example-node-tool")
        .expect("找不到模板插件");
    assert!(
        info.assets.cache_bytes > 0,
        "模板 SDK 应通过受控缓存 API 写入缓存"
    );

    manager
        .stop("example-node-tool")
        .await
        .expect("停止模板失败");
}

#[tokio::test]
async fn test_plugin_enabled_state_blocks_start() {
    let dir = data_dir();
    let manager = PluginManager::new(dir.path().to_path_buf(), "http://127.0.0.1:4446".into());

    manager.install(&echo_plugin_dir()).await.expect("安装失败");
    manager
        .set_enabled("echo-test", false)
        .await
        .expect("停用失败");

    let list = manager.list().await;
    let info = list
        .iter()
        .find(|p| p.id == "echo-test")
        .expect("找不到插件");
    assert!(!info.enabled);

    let start_result = manager.start("echo-test").await;
    assert!(start_result.is_err());

    manager
        .set_enabled("echo-test", true)
        .await
        .expect("启用失败");
    let list = manager.list().await;
    let info = list
        .iter()
        .find(|p| p.id == "echo-test")
        .expect("找不到插件");
    assert!(info.enabled);
}

#[tokio::test]
async fn test_plugin_start_failure_records_event_and_error_state() {
    let dir = data_dir();
    let manager = PluginManager::new(dir.path().to_path_buf(), "http://127.0.0.1:4446".into());
    let source_dir = dir.path().join("broken-plugin-src");
    std::fs::create_dir_all(&source_dir).expect("创建坏插件目录失败");
    std::fs::write(
        source_dir.join("plugin.json"),
        r#"{
          "id": "broken-start",
          "name": "Broken Start",
          "version": "1.0.0",
          "description": "启动失败测试",
          "author": "unit test",
          "entry": { "runtime": "binary", "script": "missing-binary" },
          "permissions": []
        }"#,
    )
    .expect("写入坏插件 manifest 失败");

    manager.install(&source_dir).await.expect("安装失败");
    let result = manager.start("broken-start").await;
    assert!(result.is_err());
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    let list = manager.list().await;
    let info = list
        .iter()
        .find(|p| p.id == "broken-start")
        .expect("找不到插件");
    assert_eq!(info.state, PluginState::Error);

    let events = manager.recent_events(Some("broken-start"), 20).await;
    assert!(
        events.iter().any(|record| matches!(
            &record.event,
            deecodex_plugin_host::PluginEvent::Error { message, .. }
              if message.contains("插件启动失败")
        )),
        "启动失败应写入插件事件"
    );
}

#[tokio::test]
async fn test_plugin_update_preserves_config_and_enabled_state() {
    let dir = data_dir();
    let manager = PluginManager::new(dir.path().to_path_buf(), "http://127.0.0.1:4446".into());

    manager.install(&echo_plugin_dir()).await.expect("安装失败");
    manager
        .update_config("echo-test", serde_json::json!({"echo_prefix": "PONG:"}))
        .await
        .expect("配置更新失败");
    manager
        .upsert_account_asset(
            "echo-test",
            "acct-1",
            serde_json::json!({"name": "Account 1"}),
        )
        .await
        .expect("连接资产写入失败");
    let before_update = manager.list().await;
    let before_info = before_update
        .iter()
        .find(|p| p.id == "echo-test")
        .expect("找不到插件");
    assert!(before_info.config.get("accounts").is_none());
    assert_eq!(before_info.assets.account_count, 1);
    let asset_file = PathBuf::from(&before_info.assets.paths.data_dir).join("state.txt");
    std::fs::write(&asset_file, "preserved").expect("写入插件资产失败");
    manager
        .set_enabled("echo-test", false)
        .await
        .expect("停用失败");

    let update_dir = dir.path().join("echo-update");
    copy_fixture_dir(&echo_plugin_dir(), &update_dir);
    let manifest_path = update_dir.join("plugin.json");
    let mut manifest_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    manifest_json["version"] = serde_json::json!("1.0.1");
    manifest_json["permissions"] = serde_json::json!(["llm.call", "fs.read"]);
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest_json).unwrap(),
    )
    .unwrap();

    let preview = manager
        .preview_install(&update_dir)
        .await
        .expect("预览失败");
    assert!(preview.already_installed);
    assert_eq!(preview.existing_version.as_deref(), Some("1.0.0"));
    assert!(preview
        .permission_changes
        .iter()
        .any(|item| item.permission == "fs.read" && item.change == "added"));

    manager.update_package(&update_dir).await.expect("更新失败");
    let list = manager.list().await;
    let info = list
        .iter()
        .find(|p| p.id == "echo-test")
        .expect("找不到插件");
    assert_eq!(info.version, "1.0.1");
    assert_eq!(info.config["echo_prefix"], "PONG:");
    assert!(!info.enabled);
    assert_eq!(info.accounts.len(), 1);
    assert_eq!(info.assets.account_count, 1);
    assert!(info.assets.data_bytes >= "preserved".len() as u64);
    assert!(asset_file.exists(), "更新插件不应删除资产目录");
}

#[tokio::test]
async fn test_plugin_uninstall_removes_isolated_assets() {
    let dir = data_dir();
    let manager = PluginManager::new(dir.path().to_path_buf(), "http://127.0.0.1:4446".into());

    manager.install(&echo_plugin_dir()).await.expect("安装失败");
    manager
        .upsert_account_asset(
            "echo-test",
            "acct-1",
            serde_json::json!({"name": "Account 1"}),
        )
        .await
        .expect("连接资产写入失败");
    let info = manager
        .list()
        .await
        .into_iter()
        .find(|p| p.id == "echo-test")
        .expect("找不到插件");
    let asset_root = PathBuf::from(&info.assets.paths.data_dir)
        .parent()
        .expect("资产根目录应存在")
        .to_path_buf();
    std::fs::write(asset_root.join("data").join("state.txt"), "delete me")
        .expect("写入插件资产失败");

    manager.uninstall("echo-test").await.expect("卸载失败");
    assert!(!asset_root.exists(), "卸载插件应清理隔离资产目录");
}

#[tokio::test]
async fn test_plugin_controlled_storage_api_and_cache_clear() {
    if !has_node() {
        eprintln!("跳过：未找到 node");
        return;
    }

    let dir = data_dir();
    let plugin_dir = create_storage_plugin(
        dir.path(),
        "storage-ok",
        &["fs.read", "fs.write", "secrets"],
    );
    let manager = PluginManager::new(dir.path().to_path_buf(), "http://127.0.0.1:4446".into());

    manager.install(&plugin_dir).await.expect("安装失败");
    manager.start("storage-ok").await.expect("启动失败");

    let result = manager
        .send_request(
            "storage-ok",
            "storage.run",
            Some(serde_json::json!({ "action": "full" })),
        )
        .await
        .expect("执行 storage.run 失败");
    assert_eq!(result["ok"], true);
    assert_eq!(result["read"]["content"], "hello");
    assert_eq!(result["secret"]["value"], "secret-token");

    let info = manager
        .list()
        .await
        .into_iter()
        .find(|p| p.id == "storage-ok")
        .expect("找不到插件");
    assert!(info.assets.data_bytes >= "hello".len() as u64);
    assert!(info.assets.cache_bytes >= "cached".len() as u64);
    assert_eq!(info.assets.secret_count, 1);
    assert!(info.config.get("secrets").is_none(), "密钥不应混入普通配置");

    let assets_after_clear = manager
        .clear_cache("storage-ok")
        .await
        .expect("清理缓存失败");
    assert_eq!(assets_after_clear.cache_bytes, 0);
    assert!(assets_after_clear.data_bytes >= "hello".len() as u64);
    assert_eq!(assets_after_clear.secret_count, 1);

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let events = manager.recent_events(Some("storage-ok"), 50).await;
    assert!(
        events.iter().any(|record| matches!(
            &record.event,
            deecodex_plugin_host::PluginEvent::AssetOperation { scope, action, ok, .. }
                if scope == "cache" && action == "clear" && *ok
        )),
        "缓存清理应记录资产操作事件"
    );

    manager.stop("storage-ok").await.expect("停止失败");
}

#[tokio::test]
async fn test_plugin_controlled_storage_api_blocks_path_traversal() {
    if !has_node() {
        eprintln!("跳过：未找到 node");
        return;
    }

    let dir = data_dir();
    let plugin_dir = create_storage_plugin(dir.path(), "storage-traversal", &["fs.read"]);
    let manager = PluginManager::new(dir.path().to_path_buf(), "http://127.0.0.1:4446".into());

    manager.install(&plugin_dir).await.expect("安装失败");
    manager.start("storage-traversal").await.expect("启动失败");

    let result = manager
        .send_request(
            "storage-traversal",
            "storage.run",
            Some(serde_json::json!({ "action": "traversal" })),
        )
        .await
        .expect("执行 storage.run 失败");
    assert_eq!(result["ok"], false);
    assert!(
        result["error"]
            .as_str()
            .unwrap_or_default()
            .contains("路径"),
        "路径穿越应被拒绝: {result}"
    );

    manager.stop("storage-traversal").await.expect("停止失败");
}

#[tokio::test]
async fn test_plugin_controlled_storage_api_requires_permission() {
    if !has_node() {
        eprintln!("跳过：未找到 node");
        return;
    }

    let dir = data_dir();
    let plugin_dir = create_storage_plugin(dir.path(), "storage-no-permission", &[]);
    let manager = PluginManager::new(dir.path().to_path_buf(), "http://127.0.0.1:4446".into());

    manager.install(&plugin_dir).await.expect("安装失败");
    manager
        .start("storage-no-permission")
        .await
        .expect("启动失败");

    let result = manager
        .send_request(
            "storage-no-permission",
            "storage.run",
            Some(serde_json::json!({ "action": "write" })),
        )
        .await
        .expect("执行 storage.run 失败");
    assert_eq!(result["ok"], false);
    assert!(
        result["error"]
            .as_str()
            .unwrap_or_default()
            .contains("缺少插件权限"),
        "缺少权限应被拒绝: {result}"
    );

    manager
        .stop("storage-no-permission")
        .await
        .expect("停止失败");
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
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let events = manager.recent_events(Some("echo-test"), 20).await;
    assert!(
        events
            .iter()
            .any(|record| matches!(record.event, deecodex_plugin_host::PluginEvent::Log { .. })),
        "启动后应记录插件日志事件"
    );

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
