use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::manifest::PluginManifest;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginStore {
    #[serde(default)]
    pub plugins: Vec<PluginRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginRecord {
    pub manifest: PluginManifest,
    #[serde(default)]
    pub config: serde_json::Value,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub installed_at: u64,
}

fn default_enabled() -> bool {
    true
}

impl PluginStore {
    pub fn load(data_dir: &Path) -> Self {
        let path = Self::store_path(data_dir);
        match std::fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, data_dir: &Path) -> Result<()> {
        let path = Self::store_path(data_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("无法创建目录: {}", parent.display()))?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)
            .with_context(|| format!("无法写入注册表: {}", path.display()))?;
        Ok(())
    }

    pub fn add_plugin(&mut self, manifest: PluginManifest) {
        self.plugins.retain(|p| p.manifest.id != manifest.id);
        self.plugins.push(PluginRecord {
            manifest,
            config: serde_json::Value::Object(serde_json::Map::new()),
            enabled: true,
            installed_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        });
    }

    pub fn remove_plugin(&mut self, plugin_id: &str) -> Option<PluginRecord> {
        if let Some(idx) = self.plugins.iter().position(|p| p.manifest.id == plugin_id) {
            Some(self.plugins.remove(idx))
        } else {
            None
        }
    }

    pub fn get_plugin(&self, plugin_id: &str) -> Option<&PluginRecord> {
        self.plugins.iter().find(|p| p.manifest.id == plugin_id)
    }

    pub fn get_plugin_mut(&mut self, plugin_id: &str) -> Option<&mut PluginRecord> {
        self.plugins.iter_mut().find(|p| p.manifest.id == plugin_id)
    }

    pub fn update_config(&mut self, plugin_id: &str, config: serde_json::Value) -> Result<()> {
        let record = self
            .get_plugin_mut(plugin_id)
            .with_context(|| format!("插件 '{plugin_id}' 未安装"))?;
        record.config = config;
        Ok(())
    }

    pub fn set_enabled(&mut self, plugin_id: &str, enabled: bool) -> Result<()> {
        let record = self
            .get_plugin_mut(plugin_id)
            .with_context(|| format!("插件 '{plugin_id}' 未安装"))?;
        record.enabled = enabled;
        Ok(())
    }

    pub fn install_dir(&self, data_dir: &Path, plugin_id: &str) -> PathBuf {
        data_dir.join("plugins").join(plugin_id)
    }

    fn store_path(data_dir: &Path) -> PathBuf {
        data_dir.join("plugins.json")
    }
}
