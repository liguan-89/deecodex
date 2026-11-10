use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::accounts::{generate_id, now_secs};

pub const RELAY_CHANNEL_STORE_VERSION: u32 = 1;
const RELAY_CHANNELS_FILE: &str = "relay_channels.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RelayChannelStatus {
    #[default]
    Active,
    CoolingDown,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RelayChannel {
    pub id: String,
    pub name: String,
    pub url: String,
    pub api_key: String,
    #[serde(default = "default_client_surface")]
    pub client_surface: String,
    #[serde(default)]
    pub image_generation_enabled: bool,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub priority: i64,
    #[serde(default = "default_weight")]
    pub weight: u32,
    #[serde(default)]
    pub status: RelayChannelStatus,
    #[serde(default)]
    pub last_error: String,
    #[serde(default)]
    pub next_retry_after: Option<u64>,
    #[serde(default)]
    pub success: u64,
    #[serde(default)]
    pub failed: u64,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
}

fn default_enabled() -> bool {
    true
}

fn default_weight() -> u32 {
    1
}

fn default_client_surface() -> String {
    "desktop".into()
}

pub fn normalize_client_surface(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "cli" => "cli".into(),
        _ => "desktop".into(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayChannelStore {
    #[serde(default = "default_store_version")]
    pub version: u32,
    #[serde(default)]
    pub active_id: Option<String>,
    #[serde(default)]
    pub active_by_surface: HashMap<String, String>,
    #[serde(default)]
    pub channels: Vec<RelayChannel>,
}

fn default_store_version() -> u32 {
    RELAY_CHANNEL_STORE_VERSION
}

impl Default for RelayChannelStore {
    fn default() -> Self {
        Self {
            version: RELAY_CHANNEL_STORE_VERSION,
            active_id: None,
            active_by_surface: HashMap::new(),
            channels: Vec::new(),
        }
    }
}

impl RelayChannelStore {
    pub fn normalize(&mut self) {
        self.version = RELAY_CHANNEL_STORE_VERSION;
        let now = now_secs();
        for channel in &mut self.channels {
            if channel.id.trim().is_empty() {
                channel.id = generate_id();
            }
            if channel.name.trim().is_empty() {
                channel.name = "中转渠道".into();
            }
            channel.url = normalize_relay_base_url(&channel.url);
            channel.api_key = channel.api_key.trim().to_string();
            channel.client_surface = normalize_client_surface(&channel.client_surface);
            if channel.weight == 0 {
                channel.weight = 1;
            }
            if channel.created_at == 0 {
                channel.created_at = now;
            }
            if channel.updated_at == 0 {
                channel.updated_at = channel.created_at;
            }
            if channel.next_retry_after.is_some_and(|retry| retry <= now)
                && channel.status == RelayChannelStatus::CoolingDown
            {
                channel.status = RelayChannelStatus::Active;
                channel.next_retry_after = None;
            }
        }
        self.channels
            .retain(|channel| !channel.url.is_empty() && !channel.api_key.is_empty());
        let mut normalized_active_by_surface = HashMap::new();
        for (surface, active_id) in std::mem::take(&mut self.active_by_surface) {
            let surface = normalize_client_surface(&surface);
            let active_valid = self
                .channels
                .iter()
                .any(|channel| channel.id == active_id && channel.client_surface == surface);
            if active_valid {
                normalized_active_by_surface.insert(surface, active_id);
            }
        }
        if let Some(active_id) = self.active_id.as_deref() {
            if let Some(channel) = self.channels.iter().find(|channel| channel.id == active_id) {
                normalized_active_by_surface
                    .entry(channel.client_surface.clone())
                    .or_insert_with(|| active_id.to_string());
            }
        }
        for surface in ["cli", "desktop"] {
            let active_valid = normalized_active_by_surface
                .get(surface)
                .is_some_and(|active_id| {
                    self.channels.iter().any(|channel| {
                        channel.id == *active_id && channel.client_surface == surface
                    })
                });
            if !active_valid {
                if let Some(channel) = self
                    .channels
                    .iter()
                    .find(|channel| channel.client_surface == surface)
                {
                    normalized_active_by_surface.insert(surface.to_string(), channel.id.clone());
                }
            }
        }
        self.active_by_surface = normalized_active_by_surface;
        self.active_id = self
            .active_by_surface
            .get("desktop")
            .cloned()
            .or_else(|| self.active_by_surface.values().next().cloned());
    }

    pub fn select(&self, now: u64, cursor: u64) -> Option<RelayChannel> {
        self.select_for_surface("desktop", now, cursor)
    }

    pub fn active_id_for_surface(&self, surface: &str) -> Option<&str> {
        let surface = normalize_client_surface(surface);
        self.active_by_surface.get(&surface).map(String::as_str)
    }

    pub fn select_for_surface(&self, surface: &str, now: u64, cursor: u64) -> Option<RelayChannel> {
        let surface = normalize_client_surface(surface);
        let mut candidates = self
            .channels
            .iter()
            .filter(|channel| channel.client_surface == surface)
            .filter(|channel| channel.enabled)
            .filter(|channel| !channel.url.trim().is_empty() && !channel.api_key.trim().is_empty())
            .filter(|channel| channel.status != RelayChannelStatus::Error)
            .filter(|channel| channel.next_retry_after.unwrap_or(0) <= now)
            .cloned()
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return None;
        }

        if let Some(active_id) = self.active_id_for_surface(&surface) {
            if let Some(active) = candidates.iter().find(|channel| channel.id == active_id) {
                return Some(active.clone());
            }
        }

        candidates.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| left.id.cmp(&right.id))
        });
        let max_priority = candidates.first()?.priority;
        let top = candidates
            .into_iter()
            .filter(|channel| channel.priority == max_priority)
            .collect::<Vec<_>>();
        let total_weight = top
            .iter()
            .map(|channel| u64::from(channel.weight.max(1)))
            .sum::<u64>();
        let mut slot = if total_weight == 0 {
            0
        } else {
            cursor % total_weight
        };
        for channel in top {
            let weight = u64::from(channel.weight.max(1));
            if slot < weight {
                return Some(channel);
            }
            slot = slot.saturating_sub(weight);
        }
        None
    }
}

pub fn normalize_relay_base_url(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return String::new();
    }
    let Ok(mut url) = Url::parse(trimmed) else {
        return trimmed.to_string();
    };

    let mut segments = url
        .path_segments()
        .map(|segments| {
            segments
                .filter(|segment| !segment.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if segments.last().is_some_and(|segment| {
        segment.eq_ignore_ascii_case("completions")
            && segments
                .get(segments.len().saturating_sub(2))
                .is_some_and(|previous| previous.eq_ignore_ascii_case("chat"))
    }) {
        segments.pop();
        segments.pop();
    } else if segments
        .last()
        .is_some_and(|segment| segment.eq_ignore_ascii_case("responses"))
    {
        segments.pop();
    }
    if !segments
        .last()
        .is_some_and(|segment| segment.eq_ignore_ascii_case("v1"))
    {
        segments.push("v1");
    }
    url.set_path(&segments.join("/"));
    url.set_query(None);
    url.set_fragment(None);
    url.as_str().trim_end_matches('/').to_string()
}

pub fn relay_channels_path(data_dir: &Path) -> PathBuf {
    data_dir.join(RELAY_CHANNELS_FILE)
}

pub fn load_store(data_dir: &Path) -> RelayChannelStore {
    let path = relay_channels_path(data_dir);
    let mut store = std::fs::read_to_string(&path)
        .ok()
        .and_then(|content| serde_json::from_str::<RelayChannelStore>(&content).ok())
        .unwrap_or_default();
    store.normalize();
    store
}

pub fn save_store(data_dir: &Path, mut store: RelayChannelStore) -> Result<()> {
    store.normalize();
    crate::config::ensure_private_directory(data_dir)
        .with_context(|| format!("创建中转渠道数据目录失败: {}", data_dir.display()))?;
    let path = relay_channels_path(data_dir);
    let content = serde_json::to_string_pretty(&store)?;
    std::fs::write(&path, content)
        .with_context(|| format!("写入中转渠道失败: {}", path.display()))?;
    crate::config::set_private_file_permissions(&path)
        .with_context(|| format!("收紧中转渠道文件权限失败: {}", path.display()))
}

pub fn upsert_channel(
    data_dir: &Path,
    id: Option<&str>,
    name: &str,
    url: &str,
    api_key: &str,
    client_surface: &str,
    set_active: bool,
) -> Result<RelayChannelStore> {
    let mut store = load_store(data_dir);
    let now = now_secs();
    let normalized_url = normalize_relay_base_url(url);
    let normalized_key = api_key.trim().to_string();
    let client_surface = normalize_client_surface(client_surface);
    let channel_name = if name.trim().is_empty() {
        "DEX AI 中转".to_string()
    } else {
        name.trim().to_string()
    };
    let target_index = id
        .and_then(|id| store.channels.iter().position(|channel| channel.id == id))
        .or_else(|| {
            store.channels.iter().position(|channel| {
                channel.url.trim_end_matches('/') == normalized_url
                    && channel.api_key == normalized_key
            })
        });
    let channel_id = if let Some(index) = target_index {
        let channel = &mut store.channels[index];
        channel.name = channel_name;
        channel.url = normalized_url;
        channel.api_key = normalized_key;
        channel.client_surface = client_surface.clone();
        channel.enabled = true;
        channel.status = RelayChannelStatus::Active;
        channel.last_error.clear();
        channel.next_retry_after = None;
        channel.updated_at = now;
        channel.id.clone()
    } else {
        let id = generate_id();
        store.channels.push(RelayChannel {
            id: id.clone(),
            name: channel_name,
            url: normalized_url,
            api_key: normalized_key,
            client_surface: client_surface.clone(),
            image_generation_enabled: false,
            enabled: true,
            priority: 0,
            weight: 1,
            status: RelayChannelStatus::Active,
            last_error: String::new(),
            next_retry_after: None,
            success: 0,
            failed: 0,
            created_at: now,
            updated_at: now,
        });
        id
    };
    let surface_has_active = store.active_id_for_surface(&client_surface).is_some();
    if set_active || !surface_has_active {
        store
            .active_by_surface
            .insert(client_surface.clone(), channel_id);
    }
    save_store(data_dir, store)?;
    Ok(load_store(data_dir))
}

pub fn set_active(data_dir: &Path, id: &str) -> Result<RelayChannelStore> {
    let mut store = load_store(data_dir);
    let Some(channel) = store.channels.iter().find(|channel| channel.id == id) else {
        anyhow::bail!("中转渠道不存在: {id}");
    };
    store
        .active_by_surface
        .insert(channel.client_surface.clone(), id.to_string());
    save_store(data_dir, store)?;
    Ok(load_store(data_dir))
}

pub fn record_success(data_dir: &Path, id: &str) -> Result<()> {
    let mut store = load_store(data_dir);
    let now = now_secs();
    if let Some(channel) = store.channels.iter_mut().find(|channel| channel.id == id) {
        channel.status = RelayChannelStatus::Active;
        channel.last_error.clear();
        channel.next_retry_after = None;
        channel.success = channel.success.saturating_add(1);
        channel.updated_at = now;
        save_store(data_dir, store)?;
    }
    Ok(())
}

pub fn record_failure(
    data_dir: &Path,
    id: &str,
    status_code: u16,
    message: &str,
    retry_after_secs: Option<u64>,
) -> Result<()> {
    let mut store = load_store(data_dir);
    let now = now_secs();
    if let Some(channel) = store.channels.iter_mut().find(|channel| channel.id == id) {
        channel.failed = channel.failed.saturating_add(1);
        channel.last_error = if message.trim().is_empty() {
            format!("HTTP {status_code}")
        } else {
            message.chars().take(500).collect()
        };
        channel.updated_at = now;
        match status_code {
            401 | 403 => {
                channel.status = RelayChannelStatus::Error;
                channel.next_retry_after = None;
            }
            429 => {
                channel.status = RelayChannelStatus::CoolingDown;
                channel.next_retry_after = Some(now + retry_after_secs.unwrap_or(300));
            }
            500..=599 | 408 => {
                channel.status = RelayChannelStatus::CoolingDown;
                channel.next_retry_after = Some(now + retry_after_secs.unwrap_or(60));
            }
            _ => {
                channel.status = RelayChannelStatus::Active;
                channel.next_retry_after = None;
            }
        }
        save_store(data_dir, store)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "deecodex-relay-{name}-{}-{}",
            std::process::id(),
            now_secs()
        ))
    }

    #[test]
    fn relay_store_upserts_and_selects_active_channel() {
        let dir = temp_dir("select");
        let store = upsert_channel(
            &dir,
            None,
            "主渠道",
            "https://api.dexai.vip/",
            "sk-a",
            "desktop",
            true,
        )
        .unwrap();
        assert_eq!(store.channels.len(), 1);
        assert_eq!(
            store.active_id.as_deref(),
            Some(store.channels[0].id.as_str())
        );
        let selected = load_store(&dir).select(now_secs(), 0).unwrap();
        assert_eq!(selected.url, "https://api.dexai.vip/v1");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn relay_base_url_normalization_adds_v1_and_trims_endpoint_paths() {
        assert_eq!(
            normalize_relay_base_url("https://api.dexai.vip"),
            "https://api.dexai.vip/v1"
        );
        assert_eq!(
            normalize_relay_base_url("https://api.dexai.vip/v1/chat/completions"),
            "https://api.dexai.vip/v1"
        );
        assert_eq!(
            normalize_relay_base_url("https://proxy.example.com/openai"),
            "https://proxy.example.com/openai/v1"
        );
        assert_eq!(
            normalize_relay_base_url("https://proxy.example.com/openai/v1/"),
            "https://proxy.example.com/openai/v1"
        );
    }

    #[test]
    fn relay_store_selects_active_channel_by_surface() {
        let dir = temp_dir("surface");
        let cli = upsert_channel(
            &dir,
            None,
            "CLI 渠道",
            "https://cli.example.com",
            "sk-cli",
            "cli",
            true,
        )
        .unwrap()
        .active_by_surface["cli"]
            .clone();
        let desktop_store = upsert_channel(
            &dir,
            None,
            "桌面渠道",
            "https://desktop.example.com",
            "sk-desktop",
            "desktop",
            true,
        )
        .unwrap();
        let desktop = desktop_store.active_by_surface["desktop"].clone();

        let store = load_store(&dir);
        assert_eq!(
            store.select_for_surface("cli", now_secs(), 0).unwrap().id,
            cli
        );
        assert_eq!(
            store
                .select_for_surface("desktop", now_secs(), 0)
                .unwrap()
                .id,
            desktop
        );
        assert_ne!(cli, desktop);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn relay_store_cools_down_failed_channel() {
        let dir = temp_dir("cooldown");
        let store = upsert_channel(
            &dir,
            None,
            "主渠道",
            "https://api.dexai.vip",
            "sk-a",
            "desktop",
            true,
        )
        .unwrap();
        let id = store.channels[0].id.clone();
        record_failure(&dir, &id, 429, "rate limited", Some(10)).unwrap();
        let store = load_store(&dir);
        assert!(store.select(now_secs(), 0).is_none());
        assert_eq!(store.channels[0].status, RelayChannelStatus::CoolingDown);
        let _ = std::fs::remove_dir_all(dir);
    }
}
