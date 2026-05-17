use serde_json::Value;

use crate::ServerManager;

use super::dex_registry::{plugin_function_name, DexToolDef};

async fn plugin_manager(
    manager: &ServerManager,
) -> Result<std::sync::Arc<deecodex_plugin_host::PluginManager>, String> {
    let guard = manager.plugin_manager.lock().await;
    guard
        .as_ref()
        .cloned()
        .ok_or_else(|| "插件管理器未初始化".to_string())
}

pub(crate) async fn plugin_tool_defs(manager: &ServerManager) -> Vec<DexToolDef> {
    let pm = match plugin_manager(manager).await {
        Ok(pm) => pm,
        Err(e) => {
            tracing::warn!(error = %e, "插件管理器不可用，跳过动态 DEX 工具");
            return Vec::new();
        }
    };

    let plugins = pm.list().await;
    let mut defs = Vec::new();
    for plugin in plugins {
        for tool in plugin.dex_tools {
            let capability = if tool.capability.is_empty() {
                "plugins.dynamic".to_string()
            } else {
                tool.capability.clone()
            };
            defs.push(DexToolDef {
                name: plugin_function_name(&plugin.id, &tool.name),
                tauri_cmd: "dex_plugin_tool".to_string(),
                level: tool.level,
                confirm: if tool.level >= 3 {
                    Some(format!(
                        "确定要执行插件工具 {} / {} 吗？",
                        plugin.name, tool.name
                    ))
                } else {
                    None
                },
                description: format!("{}：{}", plugin.name, tool.description),
                parameters: tool.parameters.clone(),
                capability,
                source: "plugin".to_string(),
                plugin_id: Some(plugin.id.clone()),
                plugin_method: Some(tool.method.clone()),
            });
        }
    }
    defs
}

pub(crate) async fn execute_plugin_tool(
    manager: &ServerManager,
    tool: &DexToolDef,
    args: Value,
) -> Result<Value, String> {
    let plugin_id = tool
        .plugin_id
        .as_deref()
        .ok_or_else(|| "插件工具缺少 plugin_id".to_string())?;
    let method = tool
        .plugin_method
        .as_deref()
        .ok_or_else(|| "插件工具缺少 method".to_string())?;
    let pm = plugin_manager(manager).await?;
    if !pm.is_running(plugin_id) {
        if tool.level <= 1 {
            pm.start(plugin_id).await.map_err(|e| e.to_string())?;
        } else {
            return Err(format!(
                "插件 '{}' 未运行。L2/L3 插件工具需要先启动插件后再执行。",
                plugin_id
            ));
        }
    }
    pm.send_request(plugin_id, method, Some(args))
        .await
        .map_err(|e| e.to_string())
}
