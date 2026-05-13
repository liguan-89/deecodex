use anyhow::{Context, Result};
use dashmap::DashMap;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::info;

use crate::manifest::PluginManifest;
use crate::process::{self, resolve_default_model, PluginProcessHandle};
use crate::protocol::{AccountInfo, AccountStatus, PluginEvent, PluginInfo, PluginState};
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

        let reqwest_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("构建 reqwest client 失败");

        Self {
            data_dir,
            store: Arc::new(RwLock::new(store)),
            instances: Arc::new(DashMap::new()),
            events_tx,
            llm_base_url,
            reqwest_client,
        }
    }

    // ── 公开方法 ──────────────────────────────────────────────────────────

    /// 安装插件：从 .zip 文件或目录安装插件，写入注册表
    pub async fn install(&self, archive_path: &Path) -> Result<PluginManifest> {
        let is_dir = archive_path.is_dir();
        let manifest = if is_dir {
            PluginManifest::from_dir(archive_path)?
        } else {
            let file = std::fs::File::open(archive_path).context("无法打开插件包文件")?;
            let mut archive = zip::ZipArchive::new(file).context("无法解析插件包 (zip)")?;

            let plugin_json_bytes = archive
                .by_name("plugin.json")
                .with_context(|| format!("插件包中缺少 plugin.json: {}", archive_path.display()))
                .and_then(|mut entry| {
                    let mut buf = Vec::new();
                    std::io::Read::read_to_end(&mut entry, &mut buf)?;
                    Ok(buf)
                })?;

            PluginManifest::from_zip_entry(archive_path, "plugin.json", &plugin_json_bytes)?
        };

        let install_dir = self.data_dir.join("plugins").join(&manifest.id);
        if install_dir.exists() {
            anyhow::bail!("插件 '{}' 已安装，请先卸载", manifest.id);
        }

        if is_dir {
            // 目录安装：递归复制
            copy_dir_recursive(archive_path, &install_dir)?;
        } else {
            // Zip 安装：解压
            let file = std::fs::File::open(archive_path).context("无法打开插件包文件")?;
            let mut archive = zip::ZipArchive::new(file).context("无法解析插件包 (zip)")?;

            std::fs::create_dir_all(&install_dir)
                .with_context(|| format!("无法创建目录: {}", install_dir.display()))?;

            for i in 0..archive.len() {
                let mut entry = archive.by_index(i)?;
                // 安全检查：防止路径穿越
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
        }

        // 更新注册表
        {
            let mut store = self.store.write().await;
            store.add_plugin(manifest.clone());
            store.save(&self.data_dir)?;
        }

        // 初始化实例状态
        self.instances.insert(
            manifest.id.clone(),
            InstanceState {
                handle: None,
                state: PluginState::Installed,
                restart_count: 0,
                accounts: Vec::new(),
            },
        );

        info!(plugin_id = %manifest.id, "插件安装完成");
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

        {
            let mut store = self.store.write().await;
            store.remove_plugin(plugin_id);
            store.save(&self.data_dir)?;
        }

        self.instances.remove(plugin_id);

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
            (record.manifest.clone(), record.config.clone())
        };

        let install_dir = self.data_dir.join("plugins").join(plugin_id);

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

        let mut handle = process::spawn_plugin(
            &manifest,
            &install_dir,
            &self.data_dir,
            &self.llm_base_url,
            &config,
            self.events_tx.clone(),
        )
        .await?;

        // 发送 initialize 握手
        let init_result = handle
            .send_request(
                "initialize",
                Some(serde_json::json!({
                    "deecodex_version": env!("CARGO_PKG_VERSION"),
                    "data_dir": self.data_dir.to_string_lossy(),
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

                let mut instance = self.instances.get_mut(plugin_id).context("实例状态丢失")?;
                instance.state = PluginState::Running;
                instance.handle = Some(handle);
                instance.restart_count = 0;

                // 后台监控：进程退出时更新状态
                if let Some(rx) = exit_rx {
                    let instances = self.instances.clone();
                    let pid = plugin_id.to_string();
                    tokio::spawn(async move {
                        let _ = rx.await;
                        if let Some(mut inst) = instances.get_mut(&pid) {
                            inst.state = PluginState::Error;
                            inst.handle = None;
                        }
                        tracing::warn!(plugin_id = %pid, "插件进程意外退出");
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
                anyhow::bail!("插件初始化失败: {err_msg}");
            }
            Err(e) => {
                self.set_state(plugin_id, PluginState::Error);
                // 清理失败的进程
                process::shutdown_plugin(handle).await.ok();
                anyhow::bail!("插件初始化超时或失败: {e}");
            }
        }
    }

    /// 停止插件：shutdown notification → 等待 → kill
    pub async fn stop(&self, plugin_id: &str) -> Result<()> {
        let mut instance = self
            .instances
            .get_mut(plugin_id)
            .with_context(|| format!("插件 '{}' 未在运行", plugin_id))?;

        if let Some(handle) = instance.handle.take() {
            process::shutdown_plugin(handle).await?;
        }

        instance.state = PluginState::Stopped;
        instance.accounts.clear();

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
                let accounts = extract_accounts(&record.config, &runtime_accounts);

                PluginInfo {
                    id: record.manifest.id.clone(),
                    name: record.manifest.name.clone(),
                    version: record.manifest.version.clone(),
                    description: record.manifest.description.clone(),
                    author: record.manifest.author.clone(),
                    state: instance_state,
                    accounts,
                    permissions: record.manifest.permissions.clone(),
                    installed_at: record.installed_at,
                    config: record.config.clone(),
                    config_schema: record.manifest.config_schema.clone(),
                }
            })
            .collect()
    }

    /// 更新插件配置（持久化 + 热推送）
    pub async fn update_config(&self, plugin_id: &str, config: Value) -> Result<()> {
        {
            let mut store = self.store.write().await;
            store.update_config(plugin_id, config.clone())?;
            store.save(&self.data_dir)?;
        }

        // 如果插件正在运行，推送配置变更（不跨 .await 持有 DashMap Ref）
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

        info!(plugin_id = %plugin_id, "配置更新完成");
        Ok(())
    }

    /// 订阅插件事件（供 deecodex 集成层转发到前端）
    pub fn subscribe_events(&self) -> broadcast::Receiver<PluginEvent> {
        self.events_tx.subscribe()
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

    fn set_state(&self, plugin_id: &str, state: PluginState) {
        if let Some(mut instance) = self.instances.get_mut(plugin_id) {
            instance.state = state;
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

/// 从插件配置中提取账号列表，与运行时状态合并
fn extract_accounts(config: &Value, runtime: &[AccountInfo]) -> Vec<AccountInfo> {
    let Some(accounts_obj) = config.get("accounts").and_then(|v| v.as_object()) else {
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
