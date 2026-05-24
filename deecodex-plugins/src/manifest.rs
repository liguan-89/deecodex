use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    #[serde(default = "default_plugin_kind")]
    pub kind: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub features: Vec<PluginFeatureManifest>,
    pub entry: PluginEntry,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub config_schema: Option<serde_json::Value>,
    #[serde(default)]
    pub account: Option<PluginAccountManifest>,
    #[serde(default)]
    pub dex_tools: Vec<DexToolManifest>,
    #[serde(default)]
    pub min_deecodex_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginFeatureManifest {
    pub id: String,
    pub kind: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub methods: BTreeMap<String, String>,
    #[serde(default)]
    pub params_schema: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginAccountManifest {
    #[serde(default = "default_account_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub methods: PluginAccountMethods,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginAccountMethods {
    #[serde(default)]
    pub login: Option<String>,
    #[serde(default)]
    pub cancel_login: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub stop: Option<String>,
}

fn default_account_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DexToolManifest {
    pub name: String,
    pub description: String,
    #[serde(default = "default_parameters_schema")]
    pub parameters: serde_json::Value,
    #[serde(default)]
    pub level: u8,
    pub method: String,
    #[serde(default = "default_plugin_capability")]
    pub capability: String,
}

fn default_parameters_schema() -> serde_json::Value {
    serde_json::json!({ "type": "object", "properties": {}, "required": [] })
}

fn default_plugin_capability() -> String {
    "plugins.dynamic".to_string()
}

fn default_plugin_kind() -> String {
    "tool".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginEntry {
    pub runtime: String,
    pub script: String,
    #[serde(default)]
    pub args: Vec<String>,
}

impl PluginManifest {
    pub fn from_dir(dir: &Path) -> Result<Self> {
        let manifest_path = dir.join("plugin.json");
        let content = std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("无法读取插件清单: {}", manifest_path.display()))?;
        let manifest: PluginManifest = serde_json::from_str(&content)
            .with_context(|| format!("插件清单格式错误: {}", manifest_path.display()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn from_zip_entry(_zip_path: &Path, _entry_name: &str, json_bytes: &[u8]) -> Result<Self> {
        let manifest: PluginManifest =
            serde_json::from_slice(json_bytes).with_context(|| "插件清单格式错误".to_string())?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty() {
            anyhow::bail!("插件 id 不能为空");
        }
        if self.entry.script.is_empty() {
            anyhow::bail!("插件入口脚本不能为空");
        }
        if self.kind.trim().is_empty() {
            anyhow::bail!("插件 kind 不能为空");
        }
        let valid_runtimes = ["node", "python", "binary"];
        if !valid_runtimes.contains(&self.entry.runtime.as_str()) {
            anyhow::bail!(
                "不支持的运行时 '{}'，支持的运行时: {:?}",
                self.entry.runtime,
                valid_runtimes
            );
        }
        if let Some(ref min_ver) = self.min_deecodex_version {
            semver::Version::parse(min_ver)
                .with_context(|| format!("min_deecodex_version 格式无效: {min_ver}"))?;
        }
        for tool in &self.dex_tools {
            if tool.name.is_empty() {
                anyhow::bail!("dex_tools.name 不能为空");
            }
            if tool.method.is_empty() {
                anyhow::bail!("dex_tools.method 不能为空");
            }
            if tool.level > 3 {
                anyhow::bail!("dex_tools.level 只能是 0..3，当前: {}", tool.level);
            }
            if !tool
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                anyhow::bail!("dex_tools.name 只能包含 ASCII 字母、数字、下划线或短横线");
            }
            let allowed_plugin_capability = format!("plugin.{}", self.id);
            if tool.capability != "plugins.dynamic" && tool.capability != allowed_plugin_capability
            {
                anyhow::bail!(
                    "dex_tools.capability 只能是 plugins.dynamic 或 {}，当前: {}",
                    allowed_plugin_capability,
                    tool.capability
                );
            }
        }
        for feature in &self.features {
            if feature.id.trim().is_empty() {
                anyhow::bail!("features.id 不能为空");
            }
            if feature.kind.trim().is_empty() {
                anyhow::bail!("features.kind 不能为空");
            }
            if feature.label.trim().is_empty() {
                anyhow::bail!("features.label 不能为空");
            }
            if !feature
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
            {
                anyhow::bail!("features.id 只能包含 ASCII 字母、数字、下划线、短横线或点");
            }
            if !feature
                .kind
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                anyhow::bail!("features.kind 只能包含 ASCII 字母、数字、下划线或短横线");
            }
            for (name, method) in &feature.methods {
                if name.trim().is_empty() || method.trim().is_empty() {
                    anyhow::bail!("features.methods 不能包含空方法名或空插件 RPC 方法");
                }
            }
            for (name, schema) in &feature.params_schema {
                if name.trim().is_empty() {
                    anyhow::bail!("features.params_schema 不能包含空动作名");
                }
                let is_object_schema = schema
                    .get("type")
                    .and_then(|value| value.as_str())
                    .map(|value| value == "object")
                    .unwrap_or(false);
                if !is_object_schema {
                    anyhow::bail!("features.params_schema.{name} 必须是 object schema");
                }
            }
        }
        Ok(())
    }

    pub fn install_dir_name(&self) -> String {
        self.id.clone()
    }
}
