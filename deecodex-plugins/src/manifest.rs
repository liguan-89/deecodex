use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub entry: PluginEntry,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub config_schema: Option<serde_json::Value>,
    #[serde(default)]
    pub min_deecodex_version: Option<String>,
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
        Ok(())
    }

    pub fn install_dir_name(&self) -> String {
        self.id.clone()
    }
}
