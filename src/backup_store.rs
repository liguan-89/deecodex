use anyhow::{Context, Result};
use serde_json::Value;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// 本地备份存储，将会话数据序列化为 JSON 文件保存到指定目录。
pub struct BackupStore {
    backup_dir: PathBuf,
}

impl BackupStore {
    pub fn new(backup_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&backup_dir)
            .with_context(|| format!("创建备份目录失败: {}", backup_dir.display()))?;
        Ok(Self { backup_dir })
    }

    /// 写入备份文件，返回访问 token。
    /// token 格式：{timestamp}-{uuid16进}
    pub fn write_backup(
        &self,
        session_id: &str,
        session_type: &str,
        data: &Value,
    ) -> Result<String> {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let token = format!("{}-{}", ts, Uuid::new_v4().simple());
        let path = self.path_for(&token);

        let payload = serde_json::json!({
            "token": token,
            "session_id": session_id,
            "session_type": session_type,
            "data": data,
        });

        let json = serde_json::to_vec_pretty(&payload)
            .with_context(|| "序列化备份数据失败")?;
        std::fs::write(&path, json)
            .with_context(|| format!("写入备份文件失败: {}", path.display()))?;

        Ok(token)
    }

    /// 根据 token 读取备份数据。
    pub fn read_backup(&self, token: &str) -> Result<Value> {
        let path = self.path_for(token);
        if !path.exists() {
            anyhow::bail!("备份文件不存在: {}", token);
        }
        let json = std::fs::read_to_string(&path)
            .with_context(|| format!("读取备份文件失败: {}", path.display()))?;
        serde_json::from_str(&json)
            .with_context(|| format!("解析备份文件失败: {}", path.display()))
    }

    /// 删除指定 token 的备份文件。
    pub fn delete_backup(&self, token: &str) -> Result<()> {
        let path = self.path_for(token);
        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("删除备份文件失败: {}", path.display()))?;
        }
        Ok(())
    }

    /// 根据 token 计算备份文件路径（仅保留字母数字和 -_ 字符）。
    pub fn path_for(&self, token: &str) -> PathBuf {
        let safe: String = token
            .chars()
            .filter(|ch| ch.is_alphanumeric() || *ch == '-' || *ch == '_')
            .collect();
        self.backup_dir.join(format!("{}.json", safe))
    }
}
