use anyhow::{Context, Result};
use dashmap::DashMap;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
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
    generation: u64,
    accounts: Vec<AccountInfo>,
}

const MAX_RESTART_ATTEMPTS: u32 = 3;
const RESTART_BACKOFF_MS: [u64; MAX_RESTART_ATTEMPTS as usize] = [250, 1_000, 3_000];

#[derive(Clone)]
pub struct PluginManager {
    data_dir: PathBuf,
    host_version: String,
    store: Arc<RwLock<PluginStore>>,
    instances: Arc<DashMap<String, InstanceState>>,
    events_tx: broadcast::Sender<PluginEvent>,
    event_log: Arc<Mutex<VecDeque<PluginEventRecord>>>,
    llm_base_url: Arc<RwLock<String>>,
    reqwest_client: reqwest::Client,
    gateway_ready: Arc<AtomicBool>,
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
            host_version: String::new(),
            store: Arc::new(RwLock::new(store)),
            instances: Arc::new(DashMap::new()),
            events_tx,
            event_log,
            llm_base_url: Arc::new(RwLock::new(llm_base_url)),
            reqwest_client,
            gateway_ready: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_host_version(mut self, host_version: impl Into<String>) -> Self {
        self.host_version = host_version.into();
        self
    }

    // ── 公开方法 ──────────────────────────────────────────────────────────

    /// 更新插件宿主后续 LLM 调用使用的网关地址。
    ///
    /// 已启动插件的 stdout 请求循环与 PluginManager 共享同一份地址，
    /// 因此端口自动选择后无需重启插件进程。
    pub async fn set_llm_base_url(&self, llm_base_url: String) {
        *self.llm_base_url.write().await = llm_base_url;
    }

    pub async fn llm_base_url(&self) -> String {
        self.llm_base_url.read().await.clone()
    }

    /// 预览插件安装：解析清单、来源指纹和权限风险，不写入注册表。
    pub async fn preview_install(&self, archive_path: &Path) -> Result<PluginInstallPreview> {
        self.preview_install_inner(archive_path, true).await
    }

    /// 市场列表用轻量预览：未安装插件无需计算 source_hash（更新判断只在已安装时用到），
    /// 避免对每个插件目录递归做 SHA-256 拖慢市场加载。
    pub async fn preview_install_light(&self, archive_path: &Path) -> Result<PluginInstallPreview> {
        self.preview_install_inner(archive_path, false).await
    }

    async fn preview_install_inner(
        &self,
        archive_path: &Path,
        compute_source_hash: bool,
    ) -> Result<PluginInstallPreview> {
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
            source_hash: if compute_source_hash {
                source_hash(archive_path)?
            } else {
                String::new()
            },
            permission_risk,
            permission_details,
            permission_changes,
        })
    }

    /// 安装插件：从 .zip 文件或目录安装插件，写入注册表
    pub async fn install(&self, archive_path: &Path) -> Result<PluginManifest> {
        self.install_internal(archive_path, None).await
    }

    /// 安装宿主内置第一方插件。调用方必须提供构建时固化的完整包 SHA-256。
    pub async fn install_trusted_first_party(
        &self,
        archive_path: &Path,
        expected_source_hash: &str,
    ) -> Result<PluginManifest> {
        self.install_internal(archive_path, Some(expected_source_hash))
            .await
    }

    async fn install_internal(
        &self,
        archive_path: &Path,
        expected_first_party_hash: Option<&str>,
    ) -> Result<PluginManifest> {
        let is_dir = archive_path.is_dir();
        let manifest = read_manifest_from_path(archive_path)?;
        let source_path = archive_path.to_string_lossy().to_string();
        let source_hash = source_hash(archive_path)?;
        validate_first_party_source(&manifest, &source_hash, expected_first_party_hash)?;

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
                generation: 0,
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
        self.update_package_internal(archive_path, None).await
    }

    /// 更新宿主内置第一方插件。调用方必须提供构建时固化的完整包 SHA-256。
    pub async fn update_trusted_first_party(
        &self,
        archive_path: &Path,
        expected_source_hash: &str,
    ) -> Result<PluginManifest> {
        self.update_package_internal(archive_path, Some(expected_source_hash))
            .await
    }

    async fn update_package_internal(
        &self,
        archive_path: &Path,
        expected_first_party_hash: Option<&str>,
    ) -> Result<PluginManifest> {
        let is_dir = archive_path.is_dir();
        let manifest = read_manifest_from_path(archive_path)?;
        let source_path = archive_path.to_string_lossy().to_string();
        let source_hash = source_hash(archive_path)?;
        validate_first_party_source(&manifest, &source_hash, expected_first_party_hash)?;

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
        let backup_dir = plugins_dir.join(format!(
            ".backup-{}-{}",
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
        if backup_dir.exists() {
            std::fs::remove_dir_all(&backup_dir)
                .with_context(|| format!("无法清理备份目录: {}", backup_dir.display()))?;
        }
        install_plugin_files(archive_path, &staging_dir, is_dir)?;

        if self.is_running(&manifest.id) {
            self.stop(&manifest.id).await?;
        }

        preserve_mobile_codex_keychain_hosts(&manifest, &install_dir, &staging_dir)?;
        // 保留 DEXAI Debug 运行时数据目录（.diagnose-data：netd 持久身份、
        // tailnet 节点 key、审计记录）。若更新时被删，netd 会丢失 tailnet 身份，
        // 重启后必须重新注册成新节点（换 IP / 短时无地址）。
        preserve_debug_runtime_data(&manifest, &install_dir, &staging_dir)?;
        if install_dir.exists() {
            std::fs::rename(&install_dir, &backup_dir).with_context(|| {
                format!(
                    "无法备份旧插件目录: {} -> {}",
                    install_dir.display(),
                    backup_dir.display()
                )
            })?;
        }
        if let Err(error) = std::fs::rename(&staging_dir, &install_dir) {
            if backup_dir.exists() {
                let _ = std::fs::rename(&backup_dir, &install_dir);
            }
            return Err(error).with_context(|| {
                format!(
                    "无法替换插件目录: {} -> {}",
                    staging_dir.display(),
                    install_dir.display()
                )
            });
        }
        self.ensure_plugin_asset_dirs(&manifest.id)?;

        {
            let mut store = self.store.write().await;
            store.update_plugin_package(manifest.clone(), source_path, source_hash)?;
            store.save(&self.data_dir)?;
        }
        if backup_dir.exists() {
            std::fs::remove_dir_all(&backup_dir)
                .with_context(|| format!("无法清理旧插件备份: {}", backup_dir.display()))?;
        }

        self.instances.insert(
            manifest.id.clone(),
            InstanceState {
                handle: None,
                state: PluginState::Stopped,
                restart_count: 0,
                generation: 0,
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
        self.start_internal(plugin_id, true).await
    }

    fn start_internal<'a>(
        &'a self,
        plugin_id: &'a str,
        reset_restart_count: bool,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
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
            if manifest.lifecycle.requires_gateway && !self.gateway_ready.load(Ordering::Acquire) {
                anyhow::bail!("插件 '{plugin_id}' 依赖 DEX 网关，当前网关尚未就绪");
            }

            let install_dir = self.data_dir.join("plugins").join(plugin_id);
            let asset_paths = self.ensure_plugin_asset_dirs(plugin_id)?;

            // 更新状态为 Starting
            let generation = {
                let mut instance =
                    self.instances
                        .entry(plugin_id.into())
                        .or_insert_with(|| InstanceState {
                            handle: None,
                            state: PluginState::Stopped,
                            restart_count: 0,
                            generation: 0,
                            accounts: Vec::new(),
                        });
                if instance.handle.is_some()
                    || matches!(instance.state, PluginState::Starting | PluginState::Running)
                {
                    anyhow::bail!("插件 '{plugin_id}' 已在运行或正在启动");
                }
                instance.state = PluginState::Starting;
                instance.generation = instance.generation.wrapping_add(1);
                if reset_restart_count {
                    instance.restart_count = 0;
                }
                instance.generation
            };

            let llm_base_url = self.llm_base_url().await;
            let mut handle = match process::spawn_plugin_with_shared_base_url(
                &manifest,
                &install_dir,
                &asset_paths,
                self.llm_base_url.clone(),
                &config,
                self.events_tx.clone(),
            )
            .await
            {
                Ok(handle) => handle,
                Err(error) => {
                    self.set_state_for_generation(plugin_id, generation, PluginState::Error);
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
                        "deecodex_version": self.host_version,
                        "data_dir": self.data_dir.to_string_lossy(),
                        "plugin_data_dir": &asset_paths.data_dir,
                        "plugin_cache_dir": &asset_paths.cache_dir,
                        "plugin_secrets_dir": &asset_paths.secrets_dir,
                        "asset_paths": &asset_paths,
                        "llm_base_url": llm_base_url,
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
                        let mut instance =
                            self.instances.get_mut(plugin_id).context("实例状态丢失")?;
                        if instance.generation != generation {
                            drop(instance);
                            process::shutdown_plugin(
                                handle,
                                Duration::from_millis(manifest.lifecycle.shutdown_timeout_ms),
                            )
                            .await
                            .ok();
                            anyhow::bail!("插件 '{plugin_id}' 启动结果已过期");
                        }
                        instance.state = PluginState::Running;
                        instance.handle = Some(handle);
                    }

                    // 后台监控：进程退出时更新状态
                    if let Some(rx) = exit_rx {
                        let manager = self.clone();
                        let pid = plugin_id.to_string();
                        tokio::spawn(async move {
                            let _ = rx.await;
                            manager.handle_unexpected_exit(pid, generation).await;
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
                                            if inst.generation != generation {
                                                break;
                                            }
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
                    self.set_state_for_generation(plugin_id, generation, PluginState::Error);
                    process::shutdown_plugin(
                        handle,
                        Duration::from_millis(manifest.lifecycle.shutdown_timeout_ms),
                    )
                    .await
                    .ok();
                    let _ = self.events_tx.send(PluginEvent::Error {
                        plugin_id: plugin_id.into(),
                        message: format!("插件初始化失败: {err_msg}"),
                    });
                    anyhow::bail!("插件初始化失败: {err_msg}");
                }
                Err(e) => {
                    self.set_state_for_generation(plugin_id, generation, PluginState::Error);
                    // 清理失败的进程
                    process::shutdown_plugin(
                        handle,
                        Duration::from_millis(manifest.lifecycle.shutdown_timeout_ms),
                    )
                    .await
                    .ok();
                    let _ = self.events_tx.send(PluginEvent::Error {
                        plugin_id: plugin_id.into(),
                        message: format!("插件初始化超时或失败: {e}"),
                    });
                    anyhow::bail!("插件初始化超时或失败: {e}");
                }
            }
        })
    }

    /// 停止插件：shutdown notification → 等待 → kill
    pub async fn stop(&self, plugin_id: &str) -> Result<()> {
        let shutdown_timeout = {
            let store = self.store.read().await;
            let record = store
                .get_plugin(plugin_id)
                .with_context(|| format!("插件 '{plugin_id}' 未安装"))?;
            Duration::from_millis(record.manifest.lifecycle.shutdown_timeout_ms)
        };
        let handle = {
            let mut instance = self
                .instances
                .get_mut(plugin_id)
                .with_context(|| format!("插件 '{}' 未在运行", plugin_id))?;
            let handle = instance.handle.take();
            instance.state = PluginState::Stopped;
            instance.generation = instance.generation.wrapping_add(1);
            instance.accounts.clear();
            handle
        };

        if let Some(handle) = handle {
            process::shutdown_plugin(handle, shutdown_timeout).await?;
        }

        let _ = self.events_tx.send(PluginEvent::Log {
            plugin_id: plugin_id.into(),
            level: "info".into(),
            message: "插件已停止".into(),
        });

        info!(plugin_id = %plugin_id, "插件停止完成");
        Ok(())
    }

    async fn handle_unexpected_exit(&self, plugin_id: String, generation: u64) {
        let (manifest, mut attempt) = {
            let store = self.store.read().await;
            let Some(record) = store.get_plugin(&plugin_id) else {
                return;
            };
            let Some(mut instance) = self.instances.get_mut(&plugin_id) else {
                return;
            };
            if instance.generation != generation || instance.handle.is_none() {
                return;
            }
            instance.state = PluginState::Error;
            instance.handle = None;
            instance.accounts.clear();
            instance.restart_count = instance.restart_count.saturating_add(1);
            (record.manifest.clone(), instance.restart_count)
        };

        let _ = self.events_tx.send(PluginEvent::Error {
            plugin_id: plugin_id.clone(),
            message: "插件进程意外退出".into(),
        });
        tracing::warn!(plugin_id = %plugin_id, generation, "插件进程意外退出");

        if manifest.lifecycle.restart_policy != "on_failure" {
            return;
        }

        let mut expected_generation = generation;
        loop {
            if attempt > MAX_RESTART_ATTEMPTS {
                let message =
                    format!("插件已达到最大自动重启次数 {MAX_RESTART_ATTEMPTS}，停止自动重启");
                let _ = self.events_tx.send(PluginEvent::Error {
                    plugin_id: plugin_id.clone(),
                    message: message.clone(),
                });
                tracing::error!(plugin_id = %plugin_id, %message);
                return;
            }

            let delay_ms = RESTART_BACKOFF_MS[(attempt - 1) as usize];
            let _ = self.events_tx.send(PluginEvent::Log {
                plugin_id: plugin_id.clone(),
                level: "warn".into(),
                message: format!(
                    "插件将在 {delay_ms}ms 后执行第 {attempt}/{MAX_RESTART_ATTEMPTS} 次自动重启"
                ),
            });
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;

            if !self
                .restart_is_allowed(&plugin_id, expected_generation)
                .await
            {
                return;
            }

            match Box::pin(self.start_internal(&plugin_id, false)).await {
                Ok(()) => {
                    let _ = self.events_tx.send(PluginEvent::Log {
                        plugin_id: plugin_id.clone(),
                        level: "info".into(),
                        message: format!("插件第 {attempt} 次自动重启成功"),
                    });
                    return;
                }
                Err(error) => {
                    tracing::error!(
                        plugin_id = %plugin_id,
                        attempt,
                        %error,
                        "插件自动重启失败"
                    );
                    let Some(mut instance) = self.instances.get_mut(&plugin_id) else {
                        return;
                    };
                    if !matches!(instance.state, PluginState::Error) {
                        return;
                    }
                    instance.restart_count = instance.restart_count.saturating_add(1);
                    attempt = instance.restart_count;
                    expected_generation = instance.generation;
                }
            }
        }
    }

    async fn restart_is_allowed(&self, plugin_id: &str, failed_generation: u64) -> bool {
        let manifest_allows_restart = {
            let store = self.store.read().await;
            let Some(record) = store.get_plugin(plugin_id) else {
                return false;
            };
            record.enabled
                && record.manifest.lifecycle.restart_policy == "on_failure"
                && (!record.manifest.lifecycle.requires_gateway || self.gateway_ready())
        };
        if !manifest_allows_restart {
            return false;
        }
        self.instances
            .get(plugin_id)
            .map(|instance| {
                instance.generation == failed_generation
                    && instance.handle.is_none()
                    && instance.state == PluginState::Error
            })
            .unwrap_or(false)
    }

    pub fn set_gateway_ready(&self, ready: bool) {
        self.gateway_ready.store(ready, Ordering::Release);
    }

    pub fn gateway_ready(&self) -> bool {
        self.gateway_ready.load(Ordering::Acquire)
    }

    pub async fn start_host_plugins(&self) -> Vec<(String, Result<(), String>)> {
        let plugin_ids = {
            let store = self.store.read().await;
            store
                .plugins
                .iter()
                .filter(|record| record.enabled)
                .filter(|record| record.manifest.first_party)
                .filter(|record| record.manifest.lifecycle.autostart == "host_ready")
                .map(|record| record.manifest.id.clone())
                .collect::<Vec<_>>()
        };
        let mut results = Vec::with_capacity(plugin_ids.len());
        for plugin_id in plugin_ids {
            if self.is_running(&plugin_id) {
                results.push((plugin_id, Ok(())));
                continue;
            }
            let result = self
                .start(&plugin_id)
                .await
                .map_err(|error| error.to_string());
            results.push((plugin_id, result));
        }
        results
    }

    pub async fn start_gateway_plugins(&self) -> Vec<(String, Result<(), String>)> {
        if !self.gateway_ready() {
            return Vec::new();
        }
        let plugin_ids = {
            let store = self.store.read().await;
            store
                .plugins
                .iter()
                .filter(|record| record.enabled)
                .filter(|record| record.manifest.lifecycle.autostart == "gateway_ready")
                .map(|record| record.manifest.id.clone())
                .collect::<Vec<_>>()
        };
        let mut results = Vec::with_capacity(plugin_ids.len());
        for plugin_id in plugin_ids {
            if self.is_running(&plugin_id) {
                results.push((plugin_id, Ok(())));
                continue;
            }
            let result = self
                .start(&plugin_id)
                .await
                .map_err(|error| error.to_string());
            results.push((plugin_id, result));
        }
        results
    }

    pub async fn stop_gateway_plugins(&self) -> Vec<(String, Result<(), String>)> {
        self.set_gateway_ready(false);
        let plugin_ids = {
            let store = self.store.read().await;
            store
                .plugins
                .iter()
                .filter(|record| record.manifest.lifecycle.requires_gateway)
                .map(|record| record.manifest.id.clone())
                .collect::<Vec<_>>()
        };
        let mut results = Vec::with_capacity(plugin_ids.len());
        for plugin_id in plugin_ids {
            if !self.is_starting_or_running(&plugin_id) {
                continue;
            }
            let result = self
                .stop(&plugin_id)
                .await
                .map_err(|error| error.to_string());
            results.push((plugin_id, result));
        }
        results
    }

    pub async fn stop_all(&self) -> Vec<(String, Result<(), String>)> {
        self.set_gateway_ready(false);
        let plugin_ids = self
            .instances
            .iter()
            .filter(|entry| matches!(entry.state, PluginState::Starting | PluginState::Running))
            .map(|entry| entry.key().clone())
            .collect::<Vec<_>>();
        let mut results = Vec::with_capacity(plugin_ids.len());
        for plugin_id in plugin_ids {
            let result = self
                .stop(&plugin_id)
                .await
                .map_err(|error| error.to_string());
            results.push((plugin_id, result));
        }
        results
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
                    first_party: record.manifest.first_party,
                    lifecycle: record.manifest.lifecycle.clone(),
                    features: record.manifest.features.clone(),
                    state: instance_state,
                    enabled: record.enabled,
                    accounts,
                    account: record.manifest.account.clone(),
                    auth: record.manifest.auth.clone(),
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

    fn is_starting_or_running(&self, plugin_id: &str) -> bool {
        self.instances
            .get(plugin_id)
            .map(|instance| matches!(instance.state, PluginState::Starting | PluginState::Running))
            .unwrap_or(false)
    }

    pub async fn is_enabled(&self, plugin_id: &str) -> bool {
        let store = self.store.read().await;
        store
            .get_plugin(plugin_id)
            .map(|record| record.enabled)
            .unwrap_or(false)
    }

    fn set_state_for_generation(&self, plugin_id: &str, generation: u64, state: PluginState) {
        if let Some(mut instance) = self.instances.get_mut(plugin_id) {
            if instance.generation == generation {
                instance.state = state;
            }
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
        let llm_base_url = self.llm_base_url().await;
        let model_name = match model.as_deref() {
            Some(m) if m != "auto" && !m.is_empty() => m.to_string(),
            _ => resolve_default_model(&llm_base_url).await,
        };
        let mut payload = serde_json::json!({
            "model": model_name,
            "input": messages,
        });

        if let Some(sp) = system_prompt {
            payload["instructions"] = serde_json::Value::String(sp);
        }

        let url = format!("{llm_base_url}/v1/responses");

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
            #[cfg(unix)]
            if let Some(mode) = entry.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(mode))?;
            }
        }
    }
    Ok(())
}

fn preserve_mobile_codex_keychain_hosts(
    manifest: &PluginManifest,
    install_dir: &Path,
    staging_dir: &Path,
) -> Result<()> {
    if !cfg!(target_os = "macos")
        || manifest.id != "deecodex-mobile-codex"
        || !manifest.first_party
        || !install_dir.is_dir()
    {
        return Ok(());
    }
    for name in ["mobile-codex-dex-plugin", "mobile-codex-companion"] {
        let source = install_dir.join(name);
        if source.is_file() {
            std::fs::copy(&source, staging_dir.join(name)).with_context(|| {
                format!("保留 Mobile Codex Keychain 宿主失败: {}", source.display())
            })?;
        }
    }
    Ok(())
}

/// 保留 DEXAI Debug 插件运行时数据目录（.diagnose-data），避免插件更新时
/// 丢失 netd 的 tailnet 持久身份（tailscaled.state / .mobile-codex-enrolled）与审计记录。
fn preserve_debug_runtime_data(
    manifest: &PluginManifest,
    install_dir: &Path,
    staging_dir: &Path,
) -> Result<()> {
    if manifest.id != "dexai-debug" || !manifest.first_party || !install_dir.is_dir() {
        return Ok(());
    }
    let source = install_dir.join(".diagnose-data");
    if !source.is_dir() {
        return Ok(());
    }
    let destination = staging_dir.join(".diagnose-data");
    copy_dir_recursive(&source, &destination).with_context(|| {
        format!(
            "保留 DEXAI Debug 运行时数据失败: {} -> {}",
            source.display(),
            destination.display()
        )
    })?;
    info!(
        plugin_id = %manifest.id,
        "插件更新时已保留 .diagnose-data 运行时数据"
    );
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

fn validate_first_party_source(
    manifest: &PluginManifest,
    actual_source_hash: &str,
    expected_first_party_hash: Option<&str>,
) -> Result<()> {
    if manifest.first_party {
        let expected = expected_first_party_hash
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .context("第一方插件只能通过宿主受信任安装通道安装")?;
        if expected != actual_source_hash {
            anyhow::bail!("第一方插件包完整性校验失败：期望 {expected}，实际 {actual_source_hash}");
        }
        return Ok(());
    }
    if expected_first_party_hash.is_some() {
        anyhow::bail!("受信任第一方安装通道拒绝非第一方插件");
    }
    Ok(())
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
