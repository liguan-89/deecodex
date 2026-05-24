use anyhow::{Context, Result};
use dashmap::DashMap;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex, RwLock};
use tracing::info;

use crate::manifest::PluginManifest;
use crate::process::{self, resolve_default_model, PluginProcessHandle};
use crate::protocol::{
    AccountInfo, AccountStatus, PluginAssetInfo, PluginAssetPaths, PluginEvent, PluginEventRecord,
    PluginInfo, PluginInstallPreview, PluginPermissionChange, PluginPermissionInfo, PluginState,
};
use crate::rpc::{JsonRpcMessage, JsonRpcNotification};
use crate::store::PluginStore;
use tokio::io::AsyncWriteExt;

/// 插件运行时实例的内部状态
struct InstanceState {
    handle: Option<PluginProcessHandle>,
    state: PluginState,
    restart_count: u32,
    accounts: Vec<AccountInfo>,
}

pub struct PluginManager {
    data_dir: PathBuf,
    store: Arc<RwLock<PluginStore>>,
    instances: Arc<DashMap<String, InstanceState>>,
    events_tx: broadcast::Sender<PluginEvent>,
    event_log: Arc<Mutex<VecDeque<PluginEventRecord>>>,
    llm_base_url: String,
    reqwest_client: reqwest::Client,
}

impl PluginManager {
    /// 创建插件管理器
    ///
    /// * `data_dir` — 数据目录（如 `~/.deecodex/`），插件安装到此目录下
    /// * `llm_base_url` — deecodex HTTP 服务地址（如 `http://127.0.0.1:4446`）
    pub fn new(data_dir: PathBuf, llm_base_url: String) -> Self {
        // 确保 data_dir 是绝对路径，避免进程 current_dir 不一致导致路径错误
        let data_dir = if data_dir.is_relative() {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(&data_dir)
        } else {
            data_dir
        };
        // 规范化（去除 .. 和 .）
        let data_dir = std::path::absolute(&data_dir).unwrap_or(data_dir);

        let store = PluginStore::load(&data_dir);
        let (events_tx, _) = broadcast::channel(256);
        let event_log = Arc::new(Mutex::new(VecDeque::with_capacity(512)));
        spawn_event_recorder(events_tx.subscribe(), event_log.clone());

        let reqwest_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("构建 reqwest client 失败");

        Self {
            data_dir,
            store: Arc::new(RwLock::new(store)),
            instances: Arc::new(DashMap::new()),
            events_tx,
            event_log,
            llm_base_url,
            reqwest_client,
        }
    }

    // ── 公开方法 ──────────────────────────────────────────────────────────

    /// 预览插件安装：解析清单、来源指纹和权限风险，不写入注册表。
    pub async fn preview_install(&self, archive_path: &Path) -> Result<PluginInstallPreview> {
        let manifest = read_manifest_from_path(archive_path)?;
        let install_dir = self.data_dir.join("plugins").join(&manifest.id);
        let asset_dir = self.plugin_asset_root(&manifest.id);
        let permission_details = permission_details(&manifest.permissions);
        let permission_risk = aggregate_permission_risk(&permission_details);
        let (already_installed, existing_version, previous_source_hash, permission_changes) = {
            let store = self.store.read().await;
            if let Some(existing) = store.get_plugin(&manifest.id) {
                (
                    true,
                    Some(existing.manifest.version.clone()),
                    Some(existing.source_hash.clone()),
                    permission_changes(&existing.manifest.permissions, &manifest.permissions),
                )
            } else {
                (
                    install_dir.exists(),
                    None,
                    None,
                    permission_changes(&[], &manifest.permissions),
                )
            }
        };
        Ok(PluginInstallPreview {
            manifest,
            already_installed,
            existing_version,
            previous_source_hash,
            install_dir: install_dir.to_string_lossy().to_string(),
            asset_dir: asset_dir.to_string_lossy().to_string(),
            source_path: archive_path.to_string_lossy().to_string(),
            source_hash: source_hash(archive_path)?,
            permission_risk,
            permission_details,
            permission_changes,
        })
    }

    /// 安装插件：从 .zip 文件或目录安装插件，写入注册表
    pub async fn install(&self, archive_path: &Path) -> Result<PluginManifest> {
        let is_dir = archive_path.is_dir();
        let manifest = read_manifest_from_path(archive_path)?;
        let source_path = archive_path.to_string_lossy().to_string();
        let source_hash = source_hash(archive_path)?;

        let install_dir = self.data_dir.join("plugins").join(&manifest.id);
        if install_dir.exists() {
            anyhow::bail!("插件 '{}' 已安装，请先卸载", manifest.id);
        }

        install_plugin_files(archive_path, &install_dir, is_dir)?;
        self.ensure_plugin_asset_dirs(&manifest.id)?;

        // 更新注册表
        {
            let mut store = self.store.write().await;
            store.add_plugin(manifest.clone(), source_path, source_hash);
            store.save(&self.data_dir)?;
        }

        // 初始化实例状态
        self.instances.insert(
            manifest.id.clone(),
            InstanceState {
                handle: None,
                state: PluginState::Stopped,
                restart_count: 0,
                accounts: Vec::new(),
            },
        );

        let _ = self.events_tx.send(PluginEvent::Log {
            plugin_id: manifest.id.clone(),
            level: "info".into(),
            message: "插件已安装".into(),
        });
        info!(plugin_id = %manifest.id, "插件安装完成");
        Ok(manifest)
    }

    /// 更新插件包：覆盖插件文件和 manifest，保留配置、启用状态和连接资产。
    pub async fn update_package(&self, archive_path: &Path) -> Result<PluginManifest> {
        let is_dir = archive_path.is_dir();
        let manifest = read_manifest_from_path(archive_path)?;
        let source_path = archive_path.to_string_lossy().to_string();
        let source_hash = source_hash(archive_path)?;

        {
            let store = self.store.read().await;
            store
                .get_plugin(&manifest.id)
                .with_context(|| format!("插件 '{}' 未安装，无法更新", manifest.id))?;
        }

        let plugins_dir = self.data_dir.join("plugins");
        let install_dir = plugins_dir.join(&manifest.id);
        let staging_dir = plugins_dir.join(format!(
            ".updating-{}-{}",
            manifest.id,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));
        if staging_dir.exists() {
            std::fs::remove_dir_all(&staging_dir)
                .with_context(|| format!("无法清理临时目录: {}", staging_dir.display()))?;
        }
        install_plugin_files(archive_path, &staging_dir, is_dir)?;

        if self.is_running(&manifest.id) {
            self.stop(&manifest.id).await?;
        }

        if install_dir.exists() {
            std::fs::remove_dir_all(&install_dir)
                .with_context(|| format!("无法删除旧插件目录: {}", install_dir.display()))?;
        }
        std::fs::rename(&staging_dir, &install_dir).with_context(|| {
            format!(
                "无法替换插件目录: {} -> {}",
                staging_dir.display(),
                install_dir.display()
            )
        })?;
        self.ensure_plugin_asset_dirs(&manifest.id)?;

        {
            let mut store = self.store.write().await;
            store.update_plugin_package(manifest.clone(), source_path, source_hash)?;
            store.save(&self.data_dir)?;
        }

        self.instances.insert(
            manifest.id.clone(),
            InstanceState {
                handle: None,
                state: PluginState::Stopped,
                restart_count: 0,
                accounts: Vec::new(),
            },
        );

        let _ = self.events_tx.send(PluginEvent::Log {
            plugin_id: manifest.id.clone(),
            level: "info".into(),
            message: "插件包已更新".into(),
        });
        info!(plugin_id = %manifest.id, version = %manifest.version, "插件更新完成");
        Ok(manifest)
    }

    /// 卸载插件：停止进程 → 删除目录 → 更新注册表
    pub async fn uninstall(&self, plugin_id: &str) -> Result<()> {
        // 先停止
        if self.is_running(plugin_id) {
            self.stop(plugin_id).await?;
        }

        let install_dir = self.data_dir.join("plugins").join(plugin_id);
        if install_dir.exists() {
            std::fs::remove_dir_all(&install_dir)
                .with_context(|| format!("无法删除目录: {}", install_dir.display()))?;
        }
        let asset_dir = self.plugin_asset_root(plugin_id);
        if asset_dir.exists() {
            std::fs::remove_dir_all(&asset_dir)
                .with_context(|| format!("无法删除插件资产目录: {}", asset_dir.display()))?;
        }

        {
            let mut store = self.store.write().await;
            store.remove_plugin(plugin_id);
            store.save(&self.data_dir)?;
        }

        self.instances.remove(plugin_id);

        let _ = self.events_tx.send(PluginEvent::Log {
            plugin_id: plugin_id.into(),
            level: "info".into(),
            message: "插件已卸载".into(),
        });
        info!(plugin_id = %plugin_id, "插件卸载完成");
        Ok(())
    }

    /// 启动插件：spawn 子进程 → initialize 握手
    pub async fn start(&self, plugin_id: &str) -> Result<()> {
        let (manifest, config) = {
            let store = self.store.read().await;
            let record = store
                .get_plugin(plugin_id)
                .with_context(|| format!("插件 '{plugin_id}' 未安装"))?;
            if !record.enabled {
                anyhow::bail!("插件 '{plugin_id}' 已停用，请先启用后再启动");
            }
            (
                record.manifest.clone(),
                runtime_config(&record.config, &record.account_assets),
            )
        };

        let install_dir = self.data_dir.join("plugins").join(plugin_id);
        let asset_paths = self.ensure_plugin_asset_dirs(plugin_id)?;

        // 更新状态为 Starting
        {
            let mut instance =
                self.instances
                    .entry(plugin_id.into())
                    .or_insert_with(|| InstanceState {
                        handle: None,
                        state: PluginState::Starting,
                        restart_count: 0,
                        accounts: Vec::new(),
                    });
            instance.state = PluginState::Starting;
        }

        let mut handle = match process::spawn_plugin(
            &manifest,
            &install_dir,
            &asset_paths,
            &self.llm_base_url,
            &config,
            self.events_tx.clone(),
        )
        .await
        {
            Ok(handle) => handle,
            Err(error) => {
                self.set_state(plugin_id, PluginState::Error);
                let _ = self.events_tx.send(PluginEvent::Error {
                    plugin_id: plugin_id.into(),
                    message: format!("插件启动失败: {error}"),
                });
                return Err(error);
            }
        };

        // 发送 initialize 握手
        let init_result = handle
            .send_request(
                "initialize",
                Some(serde_json::json!({
                    "deecodex_version": env!("CARGO_PKG_VERSION"),
                    "data_dir": self.data_dir.to_string_lossy(),
                    "plugin_data_dir": &asset_paths.data_dir,
                    "plugin_cache_dir": &asset_paths.cache_dir,
                    "plugin_secrets_dir": &asset_paths.secrets_dir,
                    "asset_paths": &asset_paths,
                    "llm_base_url": self.llm_base_url,
                    "config": config,
                })),
            )
            .await;

        match init_result {
            Ok(resp) if resp.error.is_none() => {
                // 发送 initialized 通知
                let _ = handle.send_notification("initialized", None).await;

                // 从 handle 取出 exit_rx 用于进程退出监控
                let exit_rx = handle.exit_rx.take();

                {
                    let mut instance = self.instances.get_mut(plugin_id).context("实例状态丢失")?;
                    instance.state = PluginState::Running;
                    instance.handle = Some(handle);
                    instance.restart_count = 0;
                }

                // 后台监控：进程退出时更新状态
                if let Some(rx) = exit_rx {
                    let instances = self.instances.clone();
                    let events_tx = self.events_tx.clone();
                    let pid = plugin_id.to_string();
                    tokio::spawn(async move {
                        let _ = rx.await;
                        if let Some(mut inst) = instances.get_mut(&pid) {
                            if inst.handle.is_some() {
                                inst.state = PluginState::Error;
                                inst.handle = None;
                                let _ = events_tx.send(PluginEvent::Error {
                                    plugin_id: pid.clone(),
                                    message: "插件进程意外退出".into(),
                                });
                                tracing::warn!(plugin_id = %pid, "插件进程意外退出");
                            }
                        } else {
                            tracing::warn!(plugin_id = %pid, "插件进程退出但实例状态已不存在");
                        }
                    });
                }

                // 后台监控：插件状态事件，同步账号状态到 Instance.accounts
                {
                    let instances = self.instances.clone();
                    let pid = plugin_id.to_string();
                    let mut events_rx = self.events_tx.subscribe();
                    tokio::spawn(async move {
                        loop {
                            match events_rx.recv().await {
                                Ok(PluginEvent::StatusChanged {
                                    plugin_id: event_pid,
                                    account_id,
                                    status,
                                }) if event_pid == pid => {
                                    if let Some(mut inst) = instances.get_mut(&pid) {
                                        if let Some(account) = inst
                                            .accounts
                                            .iter_mut()
                                            .find(|a| a.account_id == account_id)
                                        {
                                            account.status = status;
                                        } else {
                                            inst.accounts.push(AccountInfo {
                                                account_id: account_id.clone(),
                                                name: account_id.clone(),
                                                status,
                                                last_active_at: None,
                                            });
                                        }
                                    }
                                }
                                Ok(_) => {}
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                    continue
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                            }
                        }
                    });
                }

                let _ = self.events_tx.send(PluginEvent::Log {
                    plugin_id: plugin_id.into(),
                    level: "info".into(),
                    message: "插件已启动".into(),
                });

                info!(plugin_id = %plugin_id, "插件启动完成");
                Ok(())
            }
            Ok(resp) => {
                let err_msg = resp
                    .error
                    .map(|e| e.message)
                    .unwrap_or_else(|| "未知错误".into());
                self.set_state(plugin_id, PluginState::Error);
                let _ = self.events_tx.send(PluginEvent::Error {
                    plugin_id: plugin_id.into(),
                    message: format!("插件初始化失败: {err_msg}"),
                });
                anyhow::bail!("插件初始化失败: {err_msg}");
            }
            Err(e) => {
                self.set_state(plugin_id, PluginState::Error);
                // 清理失败的进程
                process::shutdown_plugin(handle).await.ok();
                let _ = self.events_tx.send(PluginEvent::Error {
                    plugin_id: plugin_id.into(),
                    message: format!("插件初始化超时或失败: {e}"),
                });
                anyhow::bail!("插件初始化超时或失败: {e}");
            }
        }
    }

    /// 停止插件：shutdown notification → 等待 → kill
    pub async fn stop(&self, plugin_id: &str) -> Result<()> {
        let handle = {
            let mut instance = self
                .instances
                .get_mut(plugin_id)
                .with_context(|| format!("插件 '{}' 未在运行", plugin_id))?;
            let handle = instance.handle.take();
            instance.state = PluginState::Stopped;
            instance.accounts.clear();
            handle
        };

        if let Some(handle) = handle {
            process::shutdown_plugin(handle).await?;
        }

        let _ = self.events_tx.send(PluginEvent::Log {
            plugin_id: plugin_id.into(),
            level: "info".into(),
            message: "插件已停止".into(),
        });

        info!(plugin_id = %plugin_id, "插件停止完成");
        Ok(())
    }

    /// 返回所有已安装插件的信息列表
    pub async fn list(&self) -> Vec<PluginInfo> {
        let store = self.store.read().await;
        store
            .plugins
            .iter()
            .map(|record| {
                let instance_state = self
                    .instances
                    .get(&record.manifest.id)
                    .map(|i| i.state.clone())
                    .unwrap_or(PluginState::Stopped);

                // 从 store 配置中提取 accounts，与运行时状态合并
                let runtime_accounts = self
                    .instances
                    .get(&record.manifest.id)
                    .map(|i| i.accounts.clone())
                    .unwrap_or_default();
                let accounts =
                    extract_accounts(&record.account_assets, &record.config, &runtime_accounts);
                let permission_details = permission_details(&record.manifest.permissions);
                let assets = self.plugin_asset_info(&record.manifest.id, accounts.len());

                PluginInfo {
                    id: record.manifest.id.clone(),
                    name: record.manifest.name.clone(),
                    version: record.manifest.version.clone(),
                    description: record.manifest.description.clone(),
                    author: record.manifest.author.clone(),
                    kind: record.manifest.kind.clone(),
                    tags: record.manifest.tags.clone(),
                    features: record.manifest.features.clone(),
                    state: instance_state,
                    enabled: record.enabled,
                    accounts,
                    account: record.manifest.account.clone(),
                    permissions: record.manifest.permissions.clone(),
                    permission_risk: aggregate_permission_risk(&permission_details),
                    permission_details,
                    installed_at: record.installed_at,
                    source_path: record.source_path.clone(),
                    source_hash: record.source_hash.clone(),
                    config: record.config.clone(),
                    config_schema: record.manifest.config_schema.clone(),
                    dex_tools: record.manifest.dex_tools.clone(),
                    assets,
                }
            })
            .collect()
    }

    /// 更新插件配置（持久化 + 热推送）
    pub async fn update_config(&self, plugin_id: &str, mut config: Value) -> Result<()> {
        let effective_config = {
            let mut store = self.store.write().await;
            let account_assets = take_legacy_accounts(&mut config);
            store.update_config(plugin_id, config.clone())?;
            if let Some(accounts) = account_assets {
                store.update_account_assets(plugin_id, accounts)?;
            }
            let record = store
                .get_plugin(plugin_id)
                .with_context(|| format!("插件 '{plugin_id}' 未安装"))?;
            let effective_config = runtime_config(&record.config, &record.account_assets);
            store.save(&self.data_dir)?;
            effective_config
        };

        // 如果插件正在运行，推送配置变更（不跨 .await 持有 DashMap Ref）
        let stdin_to_notify = self
            .instances
            .get(plugin_id)
            .and_then(|inst| inst.handle.as_ref().map(|h| h.stdin_clone()));

        if let Some(stdin) = stdin_to_notify {
            let notif = JsonRpcNotification::new(
                "config.update",
                Some(serde_json::json!({ "config": effective_config })),
            );
            let msg = JsonRpcMessage::Notification(notif);
            let line = msg.to_line() + "\n";
            let mut guard = stdin.lock().await;
            guard.write_all(line.as_bytes()).await.ok();
            guard.flush().await.ok();
        }

        info!(plugin_id = %plugin_id, "配置更新完成");
        Ok(())
    }

    /// 新增或更新插件连接资产。账号资产独立于普通 config，避免连接资料混进配置表单。
    pub async fn upsert_account_asset(
        &self,
        plugin_id: &str,
        account_id: &str,
        value: Value,
    ) -> Result<()> {
        let effective_config = {
            let mut store = self.store.write().await;
            store.upsert_account_asset(plugin_id, account_id, value)?;
            let record = store
                .get_plugin(plugin_id)
                .with_context(|| format!("插件 '{plugin_id}' 未安装"))?;
            let effective_config = runtime_config(&record.config, &record.account_assets);
            store.save(&self.data_dir)?;
            effective_config
        };
        self.push_config_update(plugin_id, effective_config).await;
        Ok(())
    }

    /// 删除插件连接资产，运行中的插件会收到合成后的配置更新。
    pub async fn remove_account_asset(&self, plugin_id: &str, account_id: &str) -> Result<()> {
        let effective_config = {
            let mut store = self.store.write().await;
            store.remove_account_asset(plugin_id, account_id)?;
            let record = store
                .get_plugin(plugin_id)
                .with_context(|| format!("插件 '{plugin_id}' 未安装"))?;
            let effective_config = runtime_config(&record.config, &record.account_assets);
            store.save(&self.data_dir)?;
            effective_config
        };
        self.push_config_update(plugin_id, effective_config).await;
        Ok(())
    }

    /// 启用或停用插件。停用只影响宿主调度，不删除插件文件或配置。
    pub async fn set_enabled(&self, plugin_id: &str, enabled: bool) -> Result<()> {
        if !enabled && self.is_running(plugin_id) {
            self.stop(plugin_id).await?;
        }

        {
            let mut store = self.store.write().await;
            store.set_enabled(plugin_id, enabled)?;
            store.save(&self.data_dir)?;
        }

        let _ = self.events_tx.send(PluginEvent::Log {
            plugin_id: plugin_id.into(),
            level: "info".into(),
            message: if enabled {
                "插件已启用".into()
            } else {
                "插件已停用".into()
            },
        });

        info!(plugin_id = %plugin_id, enabled = %enabled, "插件启用状态已更新");
        Ok(())
    }

    /// 清空插件缓存目录。只清理 cache，不触碰长期数据、密钥和连接资产。
    pub async fn clear_cache(&self, plugin_id: &str) -> Result<PluginAssetInfo> {
        let account_count = {
            let store = self.store.read().await;
            let record = store
                .get_plugin(plugin_id)
                .with_context(|| format!("插件 '{plugin_id}' 未安装"))?;
            account_asset_count(&record.account_assets, &record.config)
        };
        let paths = self.ensure_plugin_asset_dirs(plugin_id)?;
        let deleted = clear_dir_contents(Path::new(&paths.cache_dir))?;
        let _ = self.events_tx.send(PluginEvent::AssetOperation {
            plugin_id: plugin_id.into(),
            scope: "cache".into(),
            action: "clear".into(),
            path: String::new(),
            ok: true,
        });
        info!(plugin_id = %plugin_id, deleted = %deleted, "插件缓存已清理");
        Ok(self.plugin_asset_info(plugin_id, account_count))
    }

    /// 订阅插件事件（供 deecodex 集成层转发到前端）
    pub fn subscribe_events(&self) -> broadcast::Receiver<PluginEvent> {
        self.events_tx.subscribe()
    }

    /// 返回最近插件事件，用于前端详情页排查插件运行状态。
    pub async fn recent_events(
        &self,
        plugin_id: Option<&str>,
        limit: usize,
    ) -> Vec<PluginEventRecord> {
        let limit = limit.clamp(1, 200);
        let log = self.event_log.lock().await;
        let mut events: Vec<PluginEventRecord> = log
            .iter()
            .rev()
            .filter(|record| plugin_id.map(|id| record.plugin_id == id).unwrap_or(true))
            .take(limit)
            .cloned()
            .collect();
        events.reverse();
        events
    }

    /// 向运行中的插件发送 JSON-RPC 请求并等待响应
    pub async fn send_request(
        &self,
        plugin_id: &str,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value> {
        let instance = self
            .instances
            .get(plugin_id)
            .with_context(|| format!("插件 '{}' 未在运行", plugin_id))?;
        let handle = instance
            .handle
            .as_ref()
            .with_context(|| format!("插件 '{}' 未在运行", plugin_id))?;
        let resp = handle.send_request(method, params).await?;
        match resp.error {
            Some(e) => anyhow::bail!("插件返回错误: {}", e.message),
            None => Ok(resp.result.unwrap_or(Value::Null)),
        }
    }

    // ── 内部方法 ───────────────────────────────────────────────────────────

    pub fn is_running(&self, plugin_id: &str) -> bool {
        self.instances
            .get(plugin_id)
            .map(|i| i.state == PluginState::Running)
            .unwrap_or(false)
    }

    pub async fn is_enabled(&self, plugin_id: &str) -> bool {
        let store = self.store.read().await;
        store
            .get_plugin(plugin_id)
            .map(|record| record.enabled)
            .unwrap_or(false)
    }

    fn set_state(&self, plugin_id: &str, state: PluginState) {
        if let Some(mut instance) = self.instances.get_mut(plugin_id) {
            instance.state = state;
        }
    }

    async fn push_config_update(&self, plugin_id: &str, config: Value) {
        let stdin_to_notify = self
            .instances
            .get(plugin_id)
            .and_then(|inst| inst.handle.as_ref().map(|h| h.stdin_clone()));

        if let Some(stdin) = stdin_to_notify {
            let notif = JsonRpcNotification::new(
                "config.update",
                Some(serde_json::json!({ "config": config })),
            );
            let msg = JsonRpcMessage::Notification(notif);
            let line = msg.to_line() + "\n";
            let mut guard = stdin.lock().await;
            guard.write_all(line.as_bytes()).await.ok();
            guard.flush().await.ok();
        }
    }

    fn plugin_asset_root(&self, plugin_id: &str) -> PathBuf {
        self.data_dir.join("plugin-assets").join(plugin_id)
    }

    fn plugin_asset_paths(&self, plugin_id: &str) -> PluginAssetPaths {
        let install_dir = self.data_dir.join("plugins").join(plugin_id);
        let asset_root = self.plugin_asset_root(plugin_id);
        PluginAssetPaths {
            install_dir: install_dir.to_string_lossy().to_string(),
            data_dir: asset_root.join("data").to_string_lossy().to_string(),
            cache_dir: asset_root.join("cache").to_string_lossy().to_string(),
            secrets_dir: asset_root.join("secrets").to_string_lossy().to_string(),
        }
    }

    fn ensure_plugin_asset_dirs(&self, plugin_id: &str) -> Result<PluginAssetPaths> {
        let paths = self.plugin_asset_paths(plugin_id);
        for path in [&paths.data_dir, &paths.cache_dir, &paths.secrets_dir] {
            std::fs::create_dir_all(path)
                .with_context(|| format!("无法创建插件资产目录: {path}"))?;
        }
        Ok(paths)
    }

    fn plugin_asset_info(&self, plugin_id: &str, account_count: usize) -> PluginAssetInfo {
        let paths = self.plugin_asset_paths(plugin_id);
        let data_bytes = dir_size(Path::new(&paths.data_dir)).unwrap_or(0);
        let cache_bytes = dir_size(Path::new(&paths.cache_dir)).unwrap_or(0);
        let secrets_bytes = dir_size(Path::new(&paths.secrets_dir)).unwrap_or(0);
        let secret_count = file_count(Path::new(&paths.secrets_dir)).unwrap_or(0);
        PluginAssetInfo {
            paths,
            data_bytes,
            cache_bytes,
            secrets_bytes,
            total_bytes: data_bytes
                .saturating_add(cache_bytes)
                .saturating_add(secrets_bytes),
            secret_count,
            account_count,
            lifecycle: "update_preserve_uninstall_delete".into(),
        }
    }

    /// LLM 调用转发到 deecodex（通过 /v1/responses SSE 流式）
    pub async fn forward_llm_call(
        &self,
        plugin_id: &str,
        request_id: u64,
        model: Option<String>,
        messages: Vec<Value>,
        system_prompt: Option<String>,
    ) -> Result<String> {
        let model_name = match model.as_deref() {
            Some(m) if m != "auto" && !m.is_empty() => m.to_string(),
            _ => resolve_default_model(&self.llm_base_url).await,
        };
        let mut payload = serde_json::json!({
            "model": model_name,
            "input": messages,
        });

        if let Some(sp) = system_prompt {
            payload["instructions"] = serde_json::Value::String(sp);
        }

        let url = format!("{}/v1/responses", self.llm_base_url);

        let response = self
            .reqwest_client
            .post(&url)
            .json(&payload)
            .header("Accept", "text/event-stream")
            .send()
            .await
            .with_context(|| format!("LLM 调用失败: {url}"))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("deecodex 返回错误 {status}: {body}");
        }

        // 提取插件 stdin 句柄（不跨 .await 持有 DashMap Ref）
        let plugin_stdin = self
            .instances
            .get(plugin_id)
            .and_then(|inst| inst.handle.as_ref().map(|h| h.stdin_clone()));

        let mut stream = response.bytes_stream();
        let mut full_text = String::new();
        let mut stream_idx: u64 = 0;

        use futures_util::StreamExt;
        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.with_context(|| "读取 SSE 流失败")?;

            // 简单 SSE 解析：查找 "data: " 前缀的行
            let chunk_str = String::from_utf8_lossy(&chunk);
            for line in chunk_str.lines() {
                let data = line.strip_prefix("data: ").unwrap_or(line);
                if data.is_empty() || data == "[DONE]" {
                    continue;
                }

                // 尝试解析 JSON，提取文本内容
                if let Ok(val) = serde_json::from_str::<Value>(data) {
                    if let Some(delta) = val["delta"].as_str() {
                        full_text.push_str(delta);
                    }
                    if let Some(content) = val["output_text"].as_str() {
                        full_text.push_str(content);
                    }
                    // 发送流式 chunk 给插件
                    if let Some(ref stdin) = plugin_stdin {
                        let notif = JsonRpcNotification::new(
                            "llm.stream_chunk",
                            Some(serde_json::json!({
                                "id": request_id,
                                "index": stream_idx,
                                "chunk": data,
                            })),
                        );
                        let msg = JsonRpcMessage::Notification(notif);
                        let line = msg.to_line() + "\n";
                        let mut guard = stdin.lock().await;
                        guard.write_all(line.as_bytes()).await.ok();
                        guard.flush().await.ok();
                    }
                    stream_idx += 1;
                }
            }
        }

        Ok(full_text)
    }
}

fn read_manifest_from_path(path: &Path) -> Result<PluginManifest> {
    if path.is_dir() {
        return PluginManifest::from_dir(path);
    }

    let file = std::fs::File::open(path).context("无法打开插件包文件")?;
    let mut archive = zip::ZipArchive::new(file).context("无法解析插件包 (zip)")?;
    let plugin_json_bytes = archive
        .by_name("plugin.json")
        .with_context(|| format!("插件包中缺少 plugin.json: {}", path.display()))
        .and_then(|mut entry| {
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut buf)?;
            Ok(buf)
        })?;

    PluginManifest::from_zip_entry(path, "plugin.json", &plugin_json_bytes)
}

fn spawn_event_recorder(
    mut events_rx: broadcast::Receiver<PluginEvent>,
    event_log: Arc<Mutex<VecDeque<PluginEventRecord>>>,
) {
    tokio::spawn(async move {
        let mut seq = 0_u64;
        loop {
            match events_rx.recv().await {
                Ok(event) => {
                    seq = seq.saturating_add(1);
                    let record = PluginEventRecord {
                        seq,
                        ts: unix_ts(),
                        plugin_id: event.plugin_id().to_string(),
                        event,
                    };
                    let mut log = event_log.lock().await;
                    if log.len() >= 512 {
                        log.pop_front();
                    }
                    log.push_back(record);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn unix_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn install_plugin_files(source: &Path, install_dir: &Path, is_dir: bool) -> Result<()> {
    if is_dir {
        copy_dir_recursive(source, install_dir)?;
        return Ok(());
    }

    let file = std::fs::File::open(source).context("无法打开插件包文件")?;
    let mut archive = zip::ZipArchive::new(file).context("无法解析插件包 (zip)")?;

    std::fs::create_dir_all(install_dir)
        .with_context(|| format!("无法创建目录: {}", install_dir.display()))?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        // 安全检查：防止路径穿越。
        let out_path = match entry.enclosed_name() {
            Some(name) => install_dir.join(name),
            None => continue,
        };

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut outfile = std::fs::File::create(&out_path)?;
            std::io::copy(&mut entry, &mut outfile)?;
        }
    }
    Ok(())
}

fn source_hash(path: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    if path.is_dir() {
        hash_dir(path, path, &mut hasher)?;
    } else {
        hash_file(path, &mut hasher)?;
    }
    let digest = hasher.finalize();
    Ok(hex_digest(&digest))
}

fn hash_dir(root: &Path, dir: &Path, hasher: &mut Sha256) -> Result<()> {
    let mut entries = std::fs::read_dir(dir)
        .with_context(|| format!("无法读取目录: {}", dir.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        if name == ".git" || name == ".DS_Store" {
            continue;
        }
        let relative = path.strip_prefix(root).unwrap_or(&path);
        hasher.update(relative.to_string_lossy().as_bytes());
        if path.is_dir() {
            hasher.update(b"/");
            hash_dir(root, &path, hasher)?;
        } else {
            hasher.update(b"\0");
            hash_file(&path, hasher)?;
        }
    }
    Ok(())
}

fn hash_file(path: &Path, hasher: &mut Sha256) -> Result<()> {
    let mut file =
        std::fs::File::open(path).with_context(|| format!("无法读取文件: {}", path.display()))?;
    let mut buf = [0_u8; 16 * 1024];
    loop {
        let len = std::io::Read::read(&mut file, &mut buf)?;
        if len == 0 {
            break;
        }
        hasher.update(&buf[..len]);
    }
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn permission_details(permissions: &[String]) -> Vec<PluginPermissionInfo> {
    permissions
        .iter()
        .map(|permission| {
            let (risk, description) = permission_risk(permission);
            PluginPermissionInfo {
                permission: permission.clone(),
                risk: risk.to_string(),
                description: description.to_string(),
            }
        })
        .collect()
}

fn permission_changes(
    old_permissions: &[String],
    new_permissions: &[String],
) -> Vec<PluginPermissionChange> {
    let mut changes = Vec::new();
    for permission in new_permissions {
        let change = if old_permissions.iter().any(|old| old == permission) {
            "unchanged"
        } else {
            "added"
        };
        let (risk, description) = permission_risk(permission);
        changes.push(PluginPermissionChange {
            permission: permission.clone(),
            change: change.to_string(),
            risk: risk.to_string(),
            description: description.to_string(),
        });
    }
    for permission in old_permissions {
        if new_permissions.iter().any(|new| new == permission) {
            continue;
        }
        let (risk, description) = permission_risk(permission);
        changes.push(PluginPermissionChange {
            permission: permission.clone(),
            change: "removed".to_string(),
            risk: risk.to_string(),
            description: description.to_string(),
        });
    }
    changes
}

fn aggregate_permission_risk(details: &[PluginPermissionInfo]) -> String {
    if details.iter().any(|item| item.risk == "high") {
        "high".into()
    } else if details.iter().any(|item| item.risk == "medium") {
        "medium".into()
    } else {
        "low".into()
    }
}

fn permission_risk(permission: &str) -> (&'static str, &'static str) {
    let value = permission.to_ascii_lowercase();
    if value.contains("shell")
        || value.contains("exec")
        || value.contains("process")
        || value.contains("system")
        || value.contains("fs.write")
        || value.contains("file.write")
    {
        return ("high", "可影响本机系统、进程或写入文件");
    }
    if value.contains("http")
        || value.contains("network")
        || value.contains("fs.read")
        || value.contains("file.read")
        || value.contains("llm")
        || value.contains("media")
        || value.contains("account")
        || value.contains("secret")
    {
        return ("medium", "会访问网络、模型、媒体、账号或读取本地数据");
    }
    ("low", "低风险或仅用于声明插件内部能力")
}

fn runtime_config(config: &Value, account_assets: &Value) -> Value {
    let mut config = config.clone();
    if account_assets.as_object().is_some() {
        if !config.is_object() {
            config = serde_json::json!({});
        }
        if let Some(obj) = config.as_object_mut() {
            obj.insert("accounts".into(), account_assets.clone());
        }
    }
    config
}

fn take_legacy_accounts(config: &mut Value) -> Option<Value> {
    config
        .as_object_mut()
        .and_then(|obj| obj.remove("accounts"))
        .filter(|value| value.as_object().is_some())
}

fn account_asset_count(account_assets: &Value, config: &Value) -> usize {
    account_assets
        .as_object()
        .or_else(|| config.get("accounts").and_then(|value| value.as_object()))
        .map(|accounts| accounts.len())
        .unwrap_or(0)
}

/// 从插件连接资产中提取账号列表，与运行时状态合并。config.accounts 仅作为旧注册表兜底。
fn extract_accounts(
    account_assets: &Value,
    config: &Value,
    runtime: &[AccountInfo],
) -> Vec<AccountInfo> {
    let accounts_obj = account_assets
        .as_object()
        .or_else(|| config.get("accounts").and_then(|v| v.as_object()));
    let Some(accounts_obj) = accounts_obj else {
        return Vec::new();
    };
    accounts_obj
        .iter()
        .map(|(id, val)| {
            let name = val
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or(id)
                .to_string();
            let status = runtime
                .iter()
                .find(|a| a.account_id == *id)
                .map(|a| a.status.clone())
                .unwrap_or(AccountStatus::Disconnected);
            AccountInfo {
                account_id: id.clone(),
                name,
                status,
                last_active_at: None,
            }
        })
        .collect()
}

fn dir_size(path: &Path) -> Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("无法读取路径: {}", path.display()))?;
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Ok(0);
    }

    let mut total = 0_u64;
    for entry in
        std::fs::read_dir(path).with_context(|| format!("无法读取目录: {}", path.display()))?
    {
        let entry = entry?;
        total = total.saturating_add(dir_size(&entry.path())?);
    }
    Ok(total)
}

fn file_count(path: &Path) -> Result<usize> {
    if !path.exists() {
        return Ok(0);
    }
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("无法读取路径: {}", path.display()))?;
    if metadata.is_file() {
        return Ok(1);
    }
    if !metadata.is_dir() {
        return Ok(0);
    }

    let mut total = 0_usize;
    for entry in
        std::fs::read_dir(path).with_context(|| format!("无法读取目录: {}", path.display()))?
    {
        let entry = entry?;
        total = total.saturating_add(file_count(&entry.path())?);
    }
    Ok(total)
}

fn clear_dir_contents(path: &Path) -> Result<usize> {
    std::fs::create_dir_all(path).with_context(|| format!("无法创建目录: {}", path.display()))?;
    let mut deleted = 0_usize;
    for entry in
        std::fs::read_dir(path).with_context(|| format!("无法读取目录: {}", path.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("无法读取路径: {}", path.display()))?;
        if metadata.file_type().is_symlink() || metadata.is_file() {
            std::fs::remove_file(&path)
                .with_context(|| format!("无法删除文件: {}", path.display()))?;
        } else if metadata.is_dir() {
            std::fs::remove_dir_all(&path)
                .with_context(|| format!("无法删除目录: {}", path.display()))?;
        } else {
            continue;
        }
        deleted += 1;
    }
    Ok(deleted)
}

/// 递归复制目录
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).with_context(|| format!("无法创建目录: {}", dst.display()))?;
    for entry in
        std::fs::read_dir(src).with_context(|| format!("无法读取目录: {}", src.display()))?
    {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)
                .with_context(|| format!("无法复制文件: {}", src_path.display()))?;
        }
    }
    Ok(())
}
