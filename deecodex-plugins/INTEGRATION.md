# 集成指南：将 deecodex-plugin-host 合并到 deecodex

## 概述

`deecodex-plugin-host` 是独立的插件宿主运行时，当前位于 `deecodex-plugins/` 目录下，与 deecodex 主项目零耦合。本文档记录将其合并到 deecodex 的步骤。

## 合并步骤

### 1. 将 crate 加入 workspace

在 deecodex 根 `Cargo.toml` 的 `[workspace]` 中添加：

```toml
[workspace]
members = ["deecodex-gui", "deecodex-plugins"]
```

然后在 deecodex 的 `Cargo.toml` 添加依赖（可选 feature flag）：

```toml
[dependencies]
deecodex-plugin-host = { path = "../deecodex-plugins", optional = true }

[features]
default = []
plugins = ["deecodex-plugin-host"]
```

### 2. 在 `src/handlers.rs` 中集成

在 `AppState` 中添加字段（feature gated）：

```rust
#[cfg(feature = "plugins")]
pub plugin_manager: std::sync::Arc<deecodex_plugin_host::PluginManager>,
```

在 `build_router()` 中追加路由：

```rust
#[cfg(feature = "plugins")]
let router = router
    .route("/api/plugins", get(handle_list_plugins))
    .route("/api/plugins/install", post(handle_install_plugin))
    .route("/api/plugins/:plugin_id", delete(handle_uninstall_plugin))
    .route("/api/plugins/:plugin_id/start", post(handle_start_plugin))
    .route("/api/plugins/:plugin_id/stop", post(handle_stop_plugin))
    .route("/api/plugins/:plugin_id/config", put(handle_update_plugin_config))
    .route("/api/plugins/:plugin_id/qrcode/:account_id", get(handle_get_plugin_qrcode));
```

Handler 实现示例：

```rust
async fn handle_list_plugins(
    State(state): State<AppState>,
) -> Json<Vec<deecodex_plugin_host::PluginInfo>> {
    #[cfg(feature = "plugins")]
    {
        return Json(state.plugin_manager.list().await);
    }
    #[cfg(not(feature = "plugins"))]
    {
        return Json(vec![]);
    }
}
```

### 3. 在 `deecodex-gui/src/lib.rs` 中集成

在 `ServerManager` 中添加字段：

```rust
#[cfg(feature = "plugins")]
pub plugin_manager: Mutex<Option<std::sync::Arc<deecodex_plugin_host::PluginManager>>>,
```

在 `generate_handler!` 中注册命令：

```rust
#[cfg(feature = "plugins")]
commands::list_plugins,
#[cfg(feature = "plugins")]
commands::start_plugin,
#[cfg(feature = "plugins")]
commands::stop_plugin,
// ... 等其他命令
```

在 `start_service_inner` 中创建 PluginManager：

```rust
#[cfg(feature = "plugins")]
{
    let plugin_manager = std::sync::Arc::new(
        deecodex_plugin_host::PluginManager::new(
            data_dir.clone(),
            format!("http://127.0.0.1:{}", port),
        )
    );
    let _ = manager.plugin_manager.lock().unwrap().replace(plugin_manager);
}
```

在 `stop_service_inner` 中停止所有插件：

```rust
#[cfg(feature = "plugins")]
{
    if let Some(pm) = manager.plugin_manager.lock().unwrap().as_ref() {
        for plugin in pm.list().await {
            if plugin.state == deecodex_plugin_host::PluginState::Running {
                let _ = pm.stop(&plugin.id).await;
            }
        }
    }
}
```

### 4. 在 `deecodex-gui/src/commands.rs` 中添加命令

```rust
#[cfg(feature = "plugins")]
#[tauri::command]
pub async fn list_plugins(
    manager: State<'_, ServerManager>,
) -> Result<serde_json::Value, String> {
    let pm = manager.plugin_manager.lock().unwrap();
    let pm = pm.as_ref().ok_or("插件管理器未初始化")?;
    let list = pm.list().await;
    serde_json::to_value(list).map_err(|e| format!("序列化失败: {e}"))
}

#[cfg(feature = "plugins")]
#[tauri::command]
pub async fn start_plugin(
    manager: State<'_, ServerManager>,
    plugin_id: String,
) -> Result<serde_json::Value, String> {
    let pm = manager.plugin_manager.lock().unwrap();
    let pm = pm.as_ref().ok_or("插件管理器未初始化")?;
    pm.start(&plugin_id).await.map_err(|e| format!("启动失败: {e}"))?;
    Ok(serde_json::json!({"ok": true}))
}

// ... 其他命令类似
```

### 5. 在前端 `index.html` 中添加插件管理面板

在侧边栏添加导航按钮：

```html
<button class="nav-item" data-panel="plugins" onclick="switchPanel('plugins')">
  <span class="icon">⬡</span> 插件管理
</button>
```

添加渲染函数：

```javascript
function renderPlugins() {
  return `
    <div class="panel">
      <h2>插件管理</h2>
      <div id="pluginList"></div>
      <button onclick="installPlugin()">+ 安装插件</button>
    </div>
  `;
}
```

### 6. 编译

```bash
# 不含插件
cargo build --release

# 含插件
cargo build --release --features plugins
```

## 文件放置

微信插件 `deecodex-weixin` 可打包为 zip 随 deecodex 发布，放在 `deecodex-gui/icons/` 同级目录，用户通过 GUI「安装插件」按钮安装。
