use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::warn;

use crate::accounts::{with_account_store, Account, AccountStore};

#[derive(Clone)]
pub struct RuntimeFeedbackSink {
    data_dir: Arc<PathBuf>,
    account_store: Arc<RwLock<AccountStore>>,
    active_account: Arc<RwLock<Account>>,
    account_id: String,
}

pub struct RuntimeFeedbackRecord {
    pub account_id: String,
    pub model: String,
    pub status_code: u16,
    pub message: String,
    pub retry_after_secs: Option<u64>,
}

impl RuntimeFeedbackSink {
    pub fn new(
        data_dir: Arc<PathBuf>,
        account_store: Arc<RwLock<AccountStore>>,
        active_account: Arc<RwLock<Account>>,
        account_id: String,
    ) -> Self {
        Self {
            data_dir,
            account_store,
            active_account,
            account_id,
        }
    }

    pub async fn success(&self, model: &str) {
        self.record(model, 200, String::new(), None).await;
    }

    pub async fn failure(
        &self,
        model: &str,
        status_code: u16,
        message: impl Into<String>,
        retry_after_secs: Option<u64>,
    ) {
        self.record(model, status_code, message.into(), retry_after_secs)
            .await;
    }

    pub async fn record(
        &self,
        model: &str,
        status_code: u16,
        message: String,
        retry_after_secs: Option<u64>,
    ) {
        record_runtime_result(
            self.data_dir.clone(),
            self.account_store.clone(),
            self.active_account.clone(),
            RuntimeFeedbackRecord {
                account_id: self.account_id.clone(),
                model: model.to_string(),
                status_code,
                message,
                retry_after_secs,
            },
        )
        .await;
    }
}

pub async fn record_runtime_result(
    data_dir: Arc<PathBuf>,
    account_store: Arc<RwLock<AccountStore>>,
    active_account: Arc<RwLock<Account>>,
    record: RuntimeFeedbackRecord,
) {
    let now = crate::accounts::now_secs();
    let success = (200..300).contains(&record.status_code);
    let message_for_persist = record.message.clone();
    let mut active_update = None;
    {
        let mut store = account_store.write().await;
        if let Some(account) = store
            .accounts
            .iter_mut()
            .find(|candidate| candidate.id == record.account_id)
        {
            if success {
                account.record_runtime_success(&record.model, now);
            } else {
                account.record_runtime_failure(
                    &record.model,
                    record.status_code,
                    record.message.clone(),
                    record.retry_after_secs,
                    now,
                );
            }
            active_update = Some(account.clone());
        }
    }

    if let Some(account) = active_update {
        let should_update_active = active_account.read().await.id == account.id;
        if should_update_active {
            *active_account.write().await = account;
        }
    }

    if let Err(err) = with_account_store(data_dir.as_ref(), |store| {
        if let Some(account) = store
            .accounts
            .iter_mut()
            .find(|candidate| candidate.id == record.account_id)
        {
            if success {
                account.record_runtime_success(&record.model, now);
            } else {
                account.record_runtime_failure(
                    &record.model,
                    record.status_code,
                    message_for_persist,
                    record.retry_after_secs,
                    now,
                );
            }
        }
        Ok(())
    }) {
        warn!("保存账号运行态失败: {err}");
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use serde_json::json;
    use tokio::sync::RwLock;

    use super::*;
    use crate::accounts::{
        generate_id, load_accounts_checked, save_accounts, AccountRuntimeStatus,
        ACCOUNT_STORE_VERSION,
    };

    fn test_data_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("deecodex-{label}-{}", generate_id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn test_account(id: &str) -> Account {
        serde_json::from_value(json!({
            "id": id,
            "name": format!("Router {id}"),
            "provider": "openai",
            "client_kind": "codex",
            "client_surface": "desktop",
            "upstream": "https://api.example.com/v1",
            "api_key": format!("token-{id}"),
            "endpoints": [{
                "id": format!("ep-{id}"),
                "name": "Responses",
                "kind": "open_ai_responses",
                "base_url": "https://api.example.com/v1"
            }]
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn record_runtime_result_updates_memory_active_and_file() {
        let dir = test_data_dir("runtime-feedback");
        let account = test_account("a1");
        let store = AccountStore {
            version: ACCOUNT_STORE_VERSION,
            accounts: vec![account.clone()],
            active_id: Some(account.id.clone()),
            active_account_id: Some(account.id.clone()),
            active_endpoint_id: Some("ep-a1".into()),
            active_by_surface: HashMap::new(),
        };
        save_accounts(&dir, &store).unwrap();

        let data_dir = Arc::new(dir.clone());
        let account_store = Arc::new(RwLock::new(store));
        let active_account = Arc::new(RwLock::new(account));

        record_runtime_result(
            data_dir.clone(),
            account_store.clone(),
            active_account.clone(),
            RuntimeFeedbackRecord {
                account_id: "a1".into(),
                model: "gpt-5".into(),
                status_code: 429,
                message: "quota".into(),
                retry_after_secs: Some(90),
            },
        )
        .await;

        {
            let store = account_store.read().await;
            let account = store
                .accounts
                .iter()
                .find(|account| account.id == "a1")
                .unwrap();
            assert_eq!(
                account.runtime_state.status,
                AccountRuntimeStatus::QuotaExceeded
            );
            assert_eq!(account.runtime_state.failed, 1);
            assert!(account.runtime_state.model_states.contains_key("gpt-5"));
        }
        assert_eq!(
            active_account.read().await.runtime_state.status,
            AccountRuntimeStatus::QuotaExceeded
        );
        let loaded = load_accounts_checked(&dir).unwrap();
        assert_eq!(loaded.accounts[0].runtime_state.failed, 1);

        record_runtime_result(
            data_dir,
            account_store.clone(),
            active_account,
            RuntimeFeedbackRecord {
                account_id: "a1".into(),
                model: "gpt-5".into(),
                status_code: 200,
                message: String::new(),
                retry_after_secs: None,
            },
        )
        .await;

        let store = account_store.read().await;
        let account = store
            .accounts
            .iter()
            .find(|account| account.id == "a1")
            .unwrap();
        assert_eq!(account.runtime_state.status, AccountRuntimeStatus::Active);
        assert_eq!(account.runtime_state.success, 1);
        assert!(account.runtime_state.model_states.contains_key("gpt-5"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
