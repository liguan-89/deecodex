use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::accounts::{now_secs, Account, AccountClientKind};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientProfile {
    pub kind: AccountClientKind,
    pub slug: String,
    pub label: String,
    pub description: String,
    pub icon: String,
    pub requires_deecodex_proxy: bool,
    pub config_path_hint: String,
    pub default_provider: String,
    pub default_base_url: String,
    pub default_model: String,
    pub capability_labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientCommandStatus {
    pub installed: bool,
    pub command: String,
    pub version: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientOperationReport {
    pub ok: bool,
    pub client_kind: AccountClientKind,
    pub dry_run: bool,
    pub message: String,
    pub command: ClientCommandStatus,
    pub config_path: Option<String>,
    pub env_path: Option<String>,
    pub backup_path: Option<String>,
    pub applied_at: Option<u64>,
    pub diff: Vec<String>,
    pub diagnostics: Vec<ClientDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientDiagnostic {
    pub level: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientImportCandidate {
    pub client_kind: AccountClientKind,
    pub name: String,
    pub provider: String,
    pub upstream: String,
    pub api_key: String,
    pub default_model: String,
    pub client_options: HashMap<String, Value>,
    pub source_path: Option<String>,
    pub warnings: Vec<String>,
}

struct FileLock {
    path: PathBuf,
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn get_client_profiles() -> Vec<ClientProfile> {
    vec![
        ClientProfile {
            kind: AccountClientKind::Codex,
            slug: "codex".into(),
            label: "Codex".into(),
            description: "通过 deecodex 本地代理接入 Responses API，支持模型映射、视觉胶水和请求历史".into(),
            icon: "codex".into(),
            requires_deecodex_proxy: true,
            config_path_hint: "~/.codex/config.toml".into(),
            default_provider: "openrouter".into(),
            default_base_url: "https://openrouter.ai/api/v1".into(),
            default_model: "gpt-5.5".into(),
            capability_labels: vec!["代理翻译".into(), "模型映射".into(), "视觉胶水".into()],
        },
        ClientProfile {
            kind: AccountClientKind::ClaudeCode,
            slug: "claude_code".into(),
            label: "Claude Code".into(),
            description: "写入 Claude Code settings env，适合 Anthropic 官方或 Anthropic 兼容入口".into(),
            icon: "claude".into(),
            requires_deecodex_proxy: false,
            config_path_hint: "~/.claude/settings.json".into(),
            default_provider: "anthropic".into(),
            default_base_url: "https://api.anthropic.com".into(),
            default_model: "claude-sonnet-4-5".into(),
            capability_labels: vec!["本地配置".into(), "Anthropic Key".into()],
        },
        ClientProfile {
            kind: AccountClientKind::Openclaw,
            slug: "openclaw".into(),
            label: "OpenClaw".into(),
            description: "通过 openclaw config 的 dry-run/validate 管理 OpenClaw Gateway 配置".into(),
            icon: "openclaw".into(),
            requires_deecodex_proxy: false,
            config_path_hint: "~/.openclaw/openclaw.json".into(),
            default_provider: "openai-compatible".into(),
            default_base_url: "https://openrouter.ai/api/v1".into(),
            default_model: "anthropic/claude-sonnet-4.5".into(),
            capability_labels: vec!["CLI 校验".into(), "SecretRef".into()],
        },
        ClientProfile {
            kind: AccountClientKind::Hermes,
            slug: "hermes".into(),
            label: "Hermes".into(),
            description: "写入 Hermes config.yaml 与 .env，支持 OpenRouter/Anthropic/OpenAI-compatible 模型".into(),
            icon: "hermes".into(),
            requires_deecodex_proxy: false,
            config_path_hint: "~/.hermes/config.yaml".into(),
            default_provider: "openrouter".into(),
            default_base_url: "https://openrouter.ai/api/v1".into(),
            default_model: "anthropic/claude-sonnet-4".into(),
            capability_labels: vec!["YAML 配置".into(), ".env 密钥".into()],
        },
        ClientProfile {
            kind: AccountClientKind::GenericClient,
            slug: "generic_client".into(),
            label: "通用客户端".into(),
            description: "生成 OpenAI-compatible 环境变量模板，供 opencode、cline、roo、continue 等客户端复用".into(),
            icon: "generic".into(),
            requires_deecodex_proxy: false,
            config_path_hint: "~/.deecodex/client-env".into(),
            default_provider: "custom".into(),
            default_base_url: "https://api.example.com/v1".into(),
            default_model: "gpt-5".into(),
            capability_labels: vec!["OpenAI 兼容".into(), "Env 模板".into()],
        },
    ]
}

pub fn profile_for_kind(kind: &AccountClientKind) -> ClientProfile {
    get_client_profiles()
        .into_iter()
        .find(|profile| &profile.kind == kind)
        .unwrap_or_else(|| get_client_profiles().remove(0))
}

pub fn discover_client_accounts() -> Vec<ClientImportCandidate> {
    let mut out = Vec::new();
    if let Some(candidate) = discover_claude_account() {
        out.push(candidate);
    }
    if let Some(candidate) = discover_openclaw_account() {
        out.push(candidate);
    }
    if let Some(candidate) = discover_hermes_account() {
        out.push(candidate);
    }
    if let Some(candidate) = discover_generic_client_account() {
        out.push(candidate);
    }
    out
}

pub fn status(account: &Account) -> ClientOperationReport {
    let command = command_status_for(&account.client_kind);
    let (config_path, env_path) = resolve_paths(account);
    let mut diagnostics = base_diagnostics(account, &command, config_path.as_deref());
    if account.client_kind == AccountClientKind::Codex {
        diagnostics.push(info(
            "codex_proxy",
            "Codex 账号由 deecodex 代理管理，不写入外部客户端配置",
        ));
    }
    ClientOperationReport {
        ok: diagnostics.iter().all(|d| d.level != "error"),
        client_kind: account.client_kind.clone(),
        dry_run: true,
        message: "客户端状态已检查".into(),
        command,
        config_path: config_path.map(|p| p.to_string_lossy().to_string()),
        env_path: env_path.map(|p| p.to_string_lossy().to_string()),
        backup_path: None,
        applied_at: account.last_applied_at,
        diff: Vec::new(),
        diagnostics,
    }
}

pub fn apply(account: &mut Account, dry_run: bool) -> Result<ClientOperationReport> {
    if account.client_kind == AccountClientKind::Codex {
        return Err(anyhow!("Codex 账号通过 deecodex 代理应用，请使用账号切换"));
    }

    let mut report = match account.client_kind {
        AccountClientKind::ClaudeCode => apply_claude(account, dry_run)?,
        AccountClientKind::Openclaw => apply_openclaw(account, dry_run)?,
        AccountClientKind::Hermes => apply_hermes(account, dry_run)?,
        AccountClientKind::GenericClient => apply_generic_client(account, dry_run)?,
        AccountClientKind::Codex => unreachable!(),
    };

    report.ok = report.diagnostics.iter().all(|d| d.level != "error");
    if report.ok && !dry_run {
        let now = now_secs();
        account.last_applied_at = Some(now);
        account.last_check = Some(crate::accounts::ClientCheckRecord {
            ok: true,
            checked_at: now,
            message: report.message.clone(),
            details: serde_json::to_value(&report).unwrap_or_else(|_| json!({})),
        });
        account.translate_enabled = false;
        account.endpoints.clear();
    }
    Ok(report)
}

fn apply_claude(account: &Account, dry_run: bool) -> Result<ClientOperationReport> {
    let command = command_status_for(&account.client_kind);
    let config_path = configured_path(account, "config_path")
        .unwrap_or_else(|| home_path(&[".claude", "settings.json"]));
    let current = read_json_object(&config_path)?;
    let mut next = current.clone();
    let env = ensure_json_object_path(&mut next, &["env"]);
    set_json_string(env, "ANTHROPIC_API_KEY", &account.api_key);
    set_json_string(env, "ANTHROPIC_BASE_URL", &account.upstream);
    set_json_string(env, "ANTHROPIC_MODEL", &account.default_model);
    merge_client_env(env, account);

    let diff = diff_json(&current, &next);
    let mut backup_path = None;
    if !dry_run {
        backup_path = write_json_file_with_backup(&config_path, &next)?;
    }

    let mut diagnostics = base_diagnostics(account, &command, Some(&config_path));
    if account.api_key.trim().is_empty() {
        diagnostics.push(error("empty_key", "Claude Code API Key 为空"));
    }
    Ok(report(
        account,
        ReportDraft {
            dry_run,
            message: "Claude Code 配置已准备".into(),
            command,
            config_path: Some(config_path),
            env_path: None,
            backup_path,
            diff,
            diagnostics,
        },
    ))
}

fn apply_openclaw(account: &Account, dry_run: bool) -> Result<ClientOperationReport> {
    let command = command_status_for(&account.client_kind);
    let config_path =
        openclaw_config_path().unwrap_or_else(|| home_path(&[".openclaw", "openclaw.json"]));
    let mut diagnostics = base_diagnostics(account, &command, Some(&config_path));
    let mut diff = vec![
        format!("provider: {}", redact_for_diff(&account.provider)),
        format!("base_url: {}", redact_for_diff(&account.upstream)),
        format!("model: {}", redact_for_diff(&account.default_model)),
    ];
    let env_name =
        client_option_string(account, "api_key_env").unwrap_or_else(|| "OPENAI_API_KEY".into());
    diff.push(format!("api_key: ${{{env_name}}}"));
    diff.push(format!(
        "openclaw_model: {}",
        openclaw_model_ref(&account.default_model)
    ));

    if !command.installed {
        diagnostics.push(error(
            "cli_missing",
            "未检测到 openclaw CLI，无法执行官方 dry-run/validate",
        ));
        return Ok(report(
            account,
            ReportDraft {
                dry_run,
                message: "OpenClaw CLI 不可用".into(),
                command,
                config_path: Some(config_path),
                env_path: None,
                backup_path: None,
                diff,
                diagnostics,
            },
        ));
    }

    let batch = openclaw_batch(account, &env_name);
    let batch_text = batch.to_string();
    let mut dry_cmd = openclaw_command(account, &env_name);
    let dry = dry_cmd
        .args([
            "config",
            "set",
            "--batch-json",
            &batch_text,
            "--dry-run",
            "--json",
        ])
        .output();
    match dry {
        Ok(output) if output.status.success() => {
            diagnostics.push(info("dry_run_ok", "OpenClaw dry-run 通过"));
        }
        Ok(output) => {
            diagnostics.push(error(
                "dry_run_failed",
                &format!(
                    "OpenClaw dry-run 失败: {}",
                    truncate(&String::from_utf8_lossy(&output.stderr), 240)
                ),
            ));
            return Ok(report(
                account,
                ReportDraft {
                    dry_run,
                    message: "OpenClaw dry-run 未通过".into(),
                    command,
                    config_path: Some(config_path),
                    env_path: None,
                    backup_path: None,
                    diff,
                    diagnostics,
                },
            ));
        }
        Err(err) => {
            diagnostics.push(error(
                "dry_run_error",
                &format!("OpenClaw dry-run 启动失败: {err}"),
            ));
            return Ok(report(
                account,
                ReportDraft {
                    dry_run,
                    message: "OpenClaw dry-run 异常".into(),
                    command,
                    config_path: Some(config_path),
                    env_path: None,
                    backup_path: None,
                    diff,
                    diagnostics,
                },
            ));
        }
    }

    let mut backup_path = None;
    if !dry_run {
        let _lock = acquire_lock(&config_path)?;
        backup_path = backup_file(&config_path)?;
        let mut apply_cmd = openclaw_command(account, &env_name);
        let apply = apply_cmd
            .args(["config", "set", "--batch-json", &batch_text, "--json"])
            .output();
        match apply {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                diagnostics.push(error(
                    "apply_failed",
                    &format!(
                        "OpenClaw 写入失败: {}",
                        truncate(&String::from_utf8_lossy(&output.stderr), 240)
                    ),
                ));
            }
            Err(err) => diagnostics.push(error(
                "apply_error",
                &format!("OpenClaw 写入启动失败: {err}"),
            )),
        }
        let mut validate_cmd = openclaw_command(account, &env_name);
        let validate = validate_cmd.args(["config", "validate", "--json"]).output();
        match validate {
            Ok(output) if output.status.success() => {
                diagnostics.push(info("validate_ok", "OpenClaw 配置校验通过"))
            }
            Ok(output) => diagnostics.push(error(
                "validate_failed",
                &format!(
                    "OpenClaw 配置校验失败: {}",
                    truncate(&String::from_utf8_lossy(&output.stderr), 240)
                ),
            )),
            Err(err) => diagnostics.push(error(
                "validate_error",
                &format!("OpenClaw 校验启动失败: {err}"),
            )),
        }
        if has_errors(&diagnostics) {
            match restore_backup(&config_path, backup_path.as_deref()) {
                Ok(()) => diagnostics.push(info("rollback_ok", "OpenClaw 配置已回滚到写入前状态")),
                Err(err) => diagnostics.push(error(
                    "rollback_failed",
                    &format!("OpenClaw 配置回滚失败: {err}"),
                )),
            }
        }
    }

    Ok(report(
        account,
        ReportDraft {
            dry_run,
            message: "OpenClaw 配置已准备".into(),
            command,
            config_path: Some(config_path),
            env_path: None,
            backup_path,
            diff,
            diagnostics,
        },
    ))
}

fn apply_hermes(account: &Account, dry_run: bool) -> Result<ClientOperationReport> {
    let command = command_status_for(&account.client_kind);
    let config_path =
        hermes_config_path().unwrap_or_else(|| home_path(&[".hermes", "config.yaml"]));
    let env_path = hermes_env_path().unwrap_or_else(|| home_path(&[".hermes", ".env"]));
    let current_config = fs::read_to_string(&config_path).unwrap_or_default();
    let mut yaml = if current_config.trim().is_empty() {
        serde_yaml::Value::Mapping(Default::default())
    } else {
        serde_yaml::from_str(&current_config)
            .unwrap_or_else(|_| serde_yaml::Value::Mapping(Default::default()))
    };
    set_yaml_path(
        &mut yaml,
        &["model"],
        serde_yaml::Value::String(account.default_model.clone()),
    );
    set_yaml_path(
        &mut yaml,
        &["provider"],
        serde_yaml::Value::String(account.provider.clone()),
    );
    set_yaml_path(
        &mut yaml,
        &["base_url"],
        serde_yaml::Value::String(account.upstream.clone()),
    );

    let next_config = serde_yaml::to_string(&yaml)?;
    let mut env_map = read_env_file(&env_path)?;
    let key_name = hermes_key_name(account);
    if !account.api_key.trim().is_empty() {
        env_map.insert(key_name.clone(), account.api_key.clone());
    }
    let next_env = render_env_file(&env_map);
    let mut diff = diff_text("config.yaml", &current_config, &next_config);
    diff.extend(diff_text(
        ".env",
        &redact_env_text(&fs::read_to_string(&env_path).unwrap_or_default()),
        &redact_env_text(&next_env),
    ));
    let mut backup_path = None;
    if !dry_run {
        backup_path = write_two_text_files_with_backup(
            &config_path,
            &next_config,
            None,
            &env_path,
            &next_env,
            Some(0o600),
        )?;
    }

    let mut diagnostics = base_diagnostics(account, &command, Some(&config_path));
    if account.default_model.trim().is_empty() {
        diagnostics.push(error("empty_model", "Hermes 默认模型为空"));
    }
    if account.api_key.trim().is_empty() {
        diagnostics.push(error(
            "empty_key",
            &format!("Hermes 密钥为空，应写入 {key_name}"),
        ));
    }
    if command.installed && !dry_run {
        match Command::new("hermes").args(["config", "check"]).output() {
            Ok(output) if output.status.success() => {
                diagnostics.push(info("config_check_ok", "Hermes config check 通过"))
            }
            Ok(output) => diagnostics.push(warn(
                "config_check_failed",
                &format!(
                    "Hermes config check 返回异常: {}",
                    truncate(&String::from_utf8_lossy(&output.stderr), 240)
                ),
            )),
            Err(err) => diagnostics.push(warn(
                "config_check_error",
                &format!("Hermes config check 启动失败: {err}"),
            )),
        }
    }

    Ok(report(
        account,
        ReportDraft {
            dry_run,
            message: "Hermes 配置已准备".into(),
            command,
            config_path: Some(config_path),
            env_path: Some(env_path),
            backup_path,
            diff,
            diagnostics,
        },
    ))
}

fn apply_generic_client(account: &Account, dry_run: bool) -> Result<ClientOperationReport> {
    let command = command_status_for(&account.client_kind);
    let path = configured_path(account, "config_path")
        .unwrap_or_else(|| home_path(&[".deecodex", "client-env"]));
    let current = fs::read_to_string(&path).unwrap_or_default();
    let mut env = read_env_text(&current);
    env.insert("OPENAI_BASE_URL".into(), account.upstream.clone());
    env.insert("OPENAI_API_KEY".into(), account.api_key.clone());
    env.insert("OPENAI_MODEL".into(), account.default_model.clone());
    let next = render_env_file(&env);
    let diff = diff_text(
        "client-env",
        &redact_env_text(&current),
        &redact_env_text(&next),
    );
    let mut backup_path = None;
    if !dry_run {
        backup_path = write_text_file_with_backup(&path, &next, Some(0o600))?;
    }
    let diagnostics = base_diagnostics(account, &command, Some(&path));
    Ok(report(
        account,
        ReportDraft {
            dry_run,
            message: "通用客户端环境变量模板已准备".into(),
            command,
            config_path: Some(path),
            env_path: None,
            backup_path,
            diff,
            diagnostics,
        },
    ))
}

struct ReportDraft {
    dry_run: bool,
    message: String,
    command: ClientCommandStatus,
    config_path: Option<PathBuf>,
    env_path: Option<PathBuf>,
    backup_path: Option<PathBuf>,
    diff: Vec<String>,
    diagnostics: Vec<ClientDiagnostic>,
}

fn report(account: &Account, draft: ReportDraft) -> ClientOperationReport {
    ClientOperationReport {
        ok: draft.diagnostics.iter().all(|d| d.level != "error"),
        client_kind: account.client_kind.clone(),
        dry_run: draft.dry_run,
        message: draft.message,
        command: draft.command,
        config_path: draft.config_path.map(|p| p.to_string_lossy().to_string()),
        env_path: draft.env_path.map(|p| p.to_string_lossy().to_string()),
        backup_path: draft.backup_path.map(|p| p.to_string_lossy().to_string()),
        applied_at: if draft.dry_run {
            None
        } else {
            Some(now_secs())
        },
        diff: draft.diff,
        diagnostics: draft.diagnostics,
    }
}

fn command_status_for(kind: &AccountClientKind) -> ClientCommandStatus {
    let command = match kind {
        AccountClientKind::Codex => "codex",
        AccountClientKind::ClaudeCode => "claude",
        AccountClientKind::Openclaw => "openclaw",
        AccountClientKind::Hermes => "hermes",
        AccountClientKind::GenericClient => "",
    };
    if command.is_empty() {
        return ClientCommandStatus {
            installed: true,
            command: "env-file".into(),
            version: Some("无需 CLI".into()),
            error: None,
        };
    }
    match Command::new(command).arg("--version").output() {
        Ok(output) if output.status.success() => ClientCommandStatus {
            installed: true,
            command: command.into(),
            version: Some(first_line(&output.stdout)),
            error: None,
        },
        Ok(output) => ClientCommandStatus {
            installed: false,
            command: command.into(),
            version: None,
            error: Some(first_line(&output.stderr)),
        },
        Err(err) => ClientCommandStatus {
            installed: false,
            command: command.into(),
            version: None,
            error: Some(err.to_string()),
        },
    }
}

fn resolve_paths(account: &Account) -> (Option<PathBuf>, Option<PathBuf>) {
    match account.client_kind {
        AccountClientKind::ClaudeCode => (
            Some(
                configured_path(account, "config_path")
                    .unwrap_or_else(|| home_path(&[".claude", "settings.json"])),
            ),
            None,
        ),
        AccountClientKind::Openclaw => (
            Some(
                openclaw_config_path()
                    .unwrap_or_else(|| home_path(&[".openclaw", "openclaw.json"])),
            ),
            None,
        ),
        AccountClientKind::Hermes => (
            Some(hermes_config_path().unwrap_or_else(|| home_path(&[".hermes", "config.yaml"]))),
            Some(hermes_env_path().unwrap_or_else(|| home_path(&[".hermes", ".env"]))),
        ),
        AccountClientKind::GenericClient => (
            Some(
                configured_path(account, "config_path")
                    .unwrap_or_else(|| home_path(&[".deecodex", "client-env"])),
            ),
            None,
        ),
        AccountClientKind::Codex => (Some(home_path(&[".codex", "config.toml"])), None),
    }
}

fn discover_claude_account() -> Option<ClientImportCandidate> {
    let config_path = home_path(&[".claude", "settings.json"]);
    let config = read_json_object(&config_path).ok()?;
    let env = config.get("env").and_then(Value::as_object)?;
    let api_key = env_string(env, "ANTHROPIC_API_KEY").unwrap_or_default();
    let upstream =
        env_string(env, "ANTHROPIC_BASE_URL").unwrap_or_else(|| "https://api.anthropic.com".into());
    let default_model =
        env_string(env, "ANTHROPIC_MODEL").unwrap_or_else(|| "claude-sonnet-4-5".into());
    if api_key.is_empty() && default_model.is_empty() && upstream.is_empty() {
        return None;
    }
    let mut client_options = HashMap::new();
    client_options.insert(
        "config_path".into(),
        Value::String(config_path.to_string_lossy().to_string()),
    );
    client_options.insert(
        "api_key_env".into(),
        Value::String("ANTHROPIC_API_KEY".into()),
    );
    Some(ClientImportCandidate {
        client_kind: AccountClientKind::ClaudeCode,
        name: "Claude Code · Anthropic".into(),
        provider: "anthropic".into(),
        upstream,
        api_key,
        default_model,
        client_options,
        source_path: Some(config_path.to_string_lossy().to_string()),
        warnings: Vec::new(),
    })
}

fn discover_openclaw_account() -> Option<ClientImportCandidate> {
    let config_path =
        openclaw_config_path().unwrap_or_else(|| home_path(&[".openclaw", "openclaw.json"]));
    let config = read_json_object(&config_path).ok()?;
    let provider_map = config
        .pointer("/models/providers/deecodex")
        .or_else(|| first_object_value(config.pointer("/models/providers")?))?;
    let upstream = provider_map
        .get("baseUrl")
        .and_then(Value::as_str)
        .unwrap_or("https://openrouter.ai/api/v1")
        .to_string();
    let api_adapter = provider_map
        .get("api")
        .and_then(Value::as_str)
        .unwrap_or("openai-completions");
    let provider = if api_adapter == "anthropic-messages" {
        "anthropic".into()
    } else {
        crate::providers::guess_provider(&upstream).to_string()
    };
    let (api_key, api_key_env) = secret_value_and_env(provider_map.get("apiKey"));
    let model_from_defaults = config
        .pointer("/agents/defaults/model")
        .and_then(Value::as_str)
        .and_then(|value| value.strip_prefix("deecodex/").or(Some(value)))
        .map(str::to_string);
    let model_from_provider = provider_map
        .get("models")
        .and_then(Value::as_array)
        .and_then(|models| models.first())
        .and_then(|model| model.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let default_model = model_from_defaults
        .or(model_from_provider)
        .unwrap_or_else(|| "anthropic/claude-sonnet-4.5".into());
    let mut client_options = HashMap::new();
    client_options.insert(
        "config_path".into(),
        Value::String(config_path.to_string_lossy().to_string()),
    );
    if let Some(env_name) = api_key_env.clone() {
        client_options.insert("api_key_env".into(), Value::String(env_name));
    }
    let mut warnings = Vec::new();
    if api_key.is_empty() {
        warnings.push("OpenClaw 使用 SecretRef，当前环境没有解析到对应 Key".into());
    }
    Some(ClientImportCandidate {
        client_kind: AccountClientKind::Openclaw,
        name: "OpenClaw · deecodex".into(),
        provider,
        upstream,
        api_key,
        default_model,
        client_options,
        source_path: Some(config_path.to_string_lossy().to_string()),
        warnings,
    })
}

fn discover_hermes_account() -> Option<ClientImportCandidate> {
    let config_path =
        hermes_config_path().unwrap_or_else(|| home_path(&[".hermes", "config.yaml"]));
    let env_path = hermes_env_path().unwrap_or_else(|| home_path(&[".hermes", ".env"]));
    let config_text = fs::read_to_string(&config_path).ok()?;
    let yaml: serde_yaml::Value = serde_yaml::from_str(&config_text).ok()?;
    let provider = yaml_string(&yaml, &["provider"]).unwrap_or_else(|| "openrouter".into());
    let upstream = yaml_string(&yaml, &["base_url"]).unwrap_or_else(|| {
        if provider == "anthropic" {
            "https://api.anthropic.com".into()
        } else {
            "https://openrouter.ai/api/v1".into()
        }
    });
    let default_model =
        yaml_string(&yaml, &["model"]).unwrap_or_else(|| "anthropic/claude-sonnet-4".into());
    let env_map = read_env_file(&env_path).unwrap_or_default();
    let api_key_env = hermes_key_name_for_provider(&provider);
    let api_key = env_map.get(&api_key_env).cloned().unwrap_or_default();
    let mut client_options = HashMap::new();
    client_options.insert(
        "config_path".into(),
        Value::String(config_path.to_string_lossy().to_string()),
    );
    client_options.insert("api_key_env".into(), Value::String(api_key_env.clone()));
    Some(ClientImportCandidate {
        client_kind: AccountClientKind::Hermes,
        name: format!("Hermes · {provider}"),
        provider,
        upstream,
        api_key,
        default_model,
        client_options,
        source_path: Some(config_path.to_string_lossy().to_string()),
        warnings: Vec::new(),
    })
}

fn discover_generic_client_account() -> Option<ClientImportCandidate> {
    let config_path = home_path(&[".deecodex", "client-env"]);
    let current = fs::read_to_string(&config_path).ok()?;
    let env = read_env_text(&current);
    let upstream = env.get("OPENAI_BASE_URL").cloned().unwrap_or_default();
    let api_key = env.get("OPENAI_API_KEY").cloned().unwrap_or_default();
    let default_model = env.get("OPENAI_MODEL").cloned().unwrap_or_default();
    if upstream.is_empty() && api_key.is_empty() && default_model.is_empty() {
        return None;
    }
    let mut client_options = HashMap::new();
    client_options.insert(
        "config_path".into(),
        Value::String(config_path.to_string_lossy().to_string()),
    );
    client_options.insert("api_key_env".into(), Value::String("OPENAI_API_KEY".into()));
    Some(ClientImportCandidate {
        client_kind: AccountClientKind::GenericClient,
        name: "通用客户端 · OpenAI compatible".into(),
        provider: crate::providers::guess_provider(&upstream).to_string(),
        upstream,
        api_key,
        default_model,
        client_options,
        source_path: Some(config_path.to_string_lossy().to_string()),
        warnings: Vec::new(),
    })
}

fn base_diagnostics(
    account: &Account,
    command: &ClientCommandStatus,
    path: Option<&Path>,
) -> Vec<ClientDiagnostic> {
    let mut out = Vec::new();
    if !command.installed {
        out.push(warn(
            "cli_missing",
            &format!(
                "未检测到 {} CLI: {}",
                command.command,
                command.error.clone().unwrap_or_default()
            ),
        ));
    }
    if account.upstream.trim().is_empty() && account.client_kind != AccountClientKind::Codex {
        out.push(error("empty_base_url", "目标客户端 Base URL 为空"));
    }
    if account.default_model.trim().is_empty() && account.client_kind != AccountClientKind::Codex {
        out.push(warn(
            "empty_model",
            "默认模型为空，客户端可能使用自身默认值",
        ));
    }
    if let Some(path) = path {
        if let Some(parent) = path.parent() {
            if parent.exists() {
                out.push(info(
                    "config_dir_exists",
                    &format!("配置目录存在: {}", parent.display()),
                ));
            } else {
                out.push(warn(
                    "config_dir_missing",
                    &format!("配置目录将被创建: {}", parent.display()),
                ));
            }
        }
    }
    out
}

fn info(code: &str, message: &str) -> ClientDiagnostic {
    ClientDiagnostic {
        level: "info".into(),
        code: code.into(),
        message: message.into(),
    }
}

fn warn(code: &str, message: &str) -> ClientDiagnostic {
    ClientDiagnostic {
        level: "warning".into(),
        code: code.into(),
        message: message.into(),
    }
}

fn error(code: &str, message: &str) -> ClientDiagnostic {
    ClientDiagnostic {
        level: "error".into(),
        code: code.into(),
        message: message.into(),
    }
}

fn first_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

fn configured_path(account: &Account, key: &str) -> Option<PathBuf> {
    client_option_string(account, key).map(|value| expand_tilde(&value))
}

fn client_option_string(account: &Account, key: &str) -> Option<String> {
    account
        .client_options
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn env_string(map: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    map.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn first_object_value(value: &Value) -> Option<&Value> {
    value.as_object()?.values().find(|item| item.is_object())
}

fn secret_value_and_env(value: Option<&Value>) -> (String, Option<String>) {
    match value {
        Some(Value::String(secret)) => (secret.clone(), None),
        Some(Value::Object(map)) => {
            let env_name = map.get("id").and_then(Value::as_str).map(str::to_string);
            let secret = env_name
                .as_deref()
                .and_then(|name| std::env::var(name).ok())
                .unwrap_or_default();
            (secret, env_name)
        }
        _ => (String::new(), None),
    }
}

fn yaml_string(root: &serde_yaml::Value, path: &[&str]) -> Option<String> {
    let mut current = root;
    for key in path {
        current = current.get(*key)?;
    }
    current
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn home_path(parts: &[&str]) -> PathBuf {
    let mut path = crate::config::home_dir().unwrap_or_else(|| PathBuf::from("."));
    for part in parts {
        path.push(part);
    }
    path
}

fn expand_tilde(value: &str) -> PathBuf {
    if value == "~" {
        return crate::config::home_dir().unwrap_or_else(|| PathBuf::from(value));
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return crate::config::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(rest);
    }
    PathBuf::from(value)
}

fn openclaw_config_path() -> Option<PathBuf> {
    let output = Command::new("openclaw")
        .args(["config", "file"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let line = first_line(&output.stdout);
    if line.is_empty() {
        None
    } else {
        Some(expand_tilde(&line))
    }
}

fn hermes_config_path() -> Option<PathBuf> {
    let output = Command::new("hermes")
        .args(["config", "path"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let line = first_line(&output.stdout);
    if line.is_empty() {
        None
    } else {
        Some(expand_tilde(&line))
    }
}

fn hermes_env_path() -> Option<PathBuf> {
    let output = Command::new("hermes")
        .args(["config", "env-path"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let line = first_line(&output.stdout);
    if line.is_empty() {
        None
    } else {
        Some(expand_tilde(&line))
    }
}

fn read_json_object(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let content = fs::read_to_string(path)
        .with_context(|| format!("读取 JSON 配置失败: {}", path.display()))?;
    if content.trim().is_empty() {
        return Ok(json!({}));
    }
    let value: Value = serde_json::from_str(&content)
        .with_context(|| format!("解析 JSON 配置失败: {}", path.display()))?;
    if value.is_object() {
        Ok(value)
    } else {
        Ok(json!({}))
    }
}

fn ensure_json_object_path<'a>(
    root: &'a mut Value,
    path: &[&str],
) -> &'a mut serde_json::Map<String, Value> {
    let mut current = root;
    for key in path {
        if !current.is_object() {
            *current = json!({});
        }
        let obj = current.as_object_mut().expect("json object just created");
        current = obj.entry((*key).to_string()).or_insert_with(|| json!({}));
    }
    if !current.is_object() {
        *current = json!({});
    }
    current.as_object_mut().expect("json object just created")
}

fn set_json_string(map: &mut serde_json::Map<String, Value>, key: &str, value: &str) {
    if !value.trim().is_empty() {
        map.insert(key.into(), Value::String(value.into()));
    }
}

fn merge_client_env(map: &mut serde_json::Map<String, Value>, account: &Account) {
    if let Some(env) = account.client_options.get("env").and_then(Value::as_object) {
        for (key, value) in env {
            if let Some(text) = value.as_str() {
                map.insert(key.clone(), Value::String(text.to_string()));
            }
        }
    }
}

fn write_json_file(path: &Path, value: &Value) -> Result<()> {
    let text = serde_json::to_string_pretty(value)?;
    write_text_file(path, &(text + "\n"), None)
}

fn write_json_file_with_backup(path: &Path, value: &Value) -> Result<Option<PathBuf>> {
    let _lock = acquire_lock(path)?;
    let backup_path = backup_file(path)?;
    match write_json_file(path, value) {
        Ok(()) => Ok(backup_path),
        Err(err) => {
            restore_backup(path, backup_path.as_deref())
                .with_context(|| format!("写入失败后回滚 JSON 配置失败: {}", path.display()))?;
            Err(err).with_context(|| format!("写入 JSON 配置失败，已回滚: {}", path.display()))
        }
    }
}

fn write_text_file(path: &Path, value: &str, mode: Option<u32>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, value)?;
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

fn write_text_file_with_backup(
    path: &Path,
    value: &str,
    mode: Option<u32>,
) -> Result<Option<PathBuf>> {
    let _lock = acquire_lock(path)?;
    let backup_path = backup_file(path)?;
    match write_text_file(path, value, mode) {
        Ok(()) => Ok(backup_path),
        Err(err) => {
            restore_backup(path, backup_path.as_deref())
                .with_context(|| format!("写入失败后回滚文本配置失败: {}", path.display()))?;
            Err(err).with_context(|| format!("写入文本配置失败，已回滚: {}", path.display()))
        }
    }
}

fn write_two_text_files_with_backup(
    first_path: &Path,
    first_value: &str,
    first_mode: Option<u32>,
    second_path: &Path,
    second_value: &str,
    second_mode: Option<u32>,
) -> Result<Option<PathBuf>> {
    let _first_lock = acquire_lock(first_path)?;
    let _second_lock = acquire_lock(second_path)?;
    let first_backup = backup_file(first_path)?;
    let second_backup = backup_file(second_path)?;
    let result = write_text_file(first_path, first_value, first_mode)
        .and_then(|_| write_text_file(second_path, second_value, second_mode));
    if let Err(err) = result {
        restore_backup(first_path, first_backup.as_deref())
            .with_context(|| format!("写入失败后回滚配置失败: {}", first_path.display()))?;
        restore_backup(second_path, second_backup.as_deref())
            .with_context(|| format!("写入失败后回滚配置失败: {}", second_path.display()))?;
        return Err(err).with_context(|| {
            format!(
                "写入配置失败，已回滚: {}, {}",
                first_path.display(),
                second_path.display()
            )
        });
    }
    Ok(first_backup)
}

fn acquire_lock(path: &Path) -> Result<FileLock> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let lock_path = path.with_extension(format!(
        "{}deecodex.lock",
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| format!("{ext}."))
            .unwrap_or_default()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
        .with_context(|| format!("配置文件正在被其他进程写入: {}", lock_path.display()))?;
    writeln!(file, "{}", std::process::id())?;
    Ok(FileLock { path: lock_path })
}

fn backup_file(path: &Path) -> Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let backup = path.with_extension(format!(
        "{}deecodex.bak.{}",
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| format!("{ext}."))
            .unwrap_or_default(),
        now_secs()
    ));
    fs::copy(path, &backup)?;
    Ok(Some(backup))
}

fn restore_backup(path: &Path, backup: Option<&Path>) -> Result<()> {
    match backup {
        Some(backup_path) => {
            fs::copy(backup_path, path)?;
        }
        None => {
            if path.exists() {
                fs::remove_file(path)?;
            }
        }
    }
    Ok(())
}

fn has_errors(diagnostics: &[ClientDiagnostic]) -> bool {
    diagnostics.iter().any(|d| d.level == "error")
}

fn diff_json(before: &Value, after: &Value) -> Vec<String> {
    let before = redact_json(before);
    let after = redact_json(after);
    diff_text(
        "json",
        &serde_json::to_string_pretty(&before).unwrap_or_default(),
        &serde_json::to_string_pretty(&after).unwrap_or_default(),
    )
}

fn diff_text(label: &str, before: &str, after: &str) -> Vec<String> {
    if before == after {
        return vec![format!("{label}: 无变化")];
    }

    let before_lines: Vec<String> = before
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    let after_lines: Vec<String> = after
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    let mut out = vec![
        format!("{label}: 旧长度 {} 字符", before.chars().count()),
        format!("{label}: 新长度 {} 字符", after.chars().count()),
    ];
    for line in before_lines
        .iter()
        .filter(|line| !after_lines.contains(line))
        .take(6)
    {
        out.push(format!("{label}: - {}", truncate(line, 120)));
    }
    for line in after_lines
        .iter()
        .filter(|line| !before_lines.contains(line))
        .take(6)
    {
        out.push(format!("{label}: + {}", truncate(line, 120)));
    }
    let omitted = before_lines
        .iter()
        .filter(|line| !after_lines.contains(line))
        .skip(6)
        .count()
        + after_lines
            .iter()
            .filter(|line| !before_lines.contains(line))
            .skip(6)
            .count();
    if omitted > 0 {
        out.push(format!("{label}: 另有 {omitted} 行变化"));
    }
    out
}

fn redact_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    if is_secret_key(key) {
                        (
                            key.clone(),
                            Value::String(mask_secret(value.as_str().unwrap_or(""))),
                        )
                    } else {
                        (key.clone(), redact_json(value))
                    }
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact_json).collect()),
        other => other.clone(),
    }
}

fn redact_for_diff(value: &str) -> String {
    if value.len() > 96 {
        truncate(value, 96)
    } else {
        value.into()
    }
}

fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("key")
        || key.contains("token")
        || key.contains("secret")
        || key.contains("password")
}

fn mask_secret(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 8 {
        "****".into()
    } else {
        let prefix: String = chars.iter().take(4).collect();
        let suffix: String = chars[chars.len() - 4..].iter().collect();
        format!("{prefix}****{suffix}")
    }
}

fn read_env_file(path: &Path) -> Result<HashMap<String, String>> {
    Ok(read_env_text(&fs::read_to_string(path).unwrap_or_default()))
}

fn read_env_text(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            Some((key.trim().into(), value.trim().trim_matches('"').into()))
        })
        .collect()
}

fn render_env_file(values: &HashMap<String, String>) -> String {
    let mut keys: Vec<_> = values.keys().collect();
    keys.sort();
    let mut out = String::new();
    for key in keys {
        let value = values.get(key).cloned().unwrap_or_default();
        out.push_str(key);
        out.push('=');
        out.push_str(&shell_quote(&value));
        out.push('\n');
    }
    out
}

fn shell_quote(value: &str) -> String {
    if value.chars().all(|ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':' | '$' | '{' | '}')
    }) {
        value.into()
    } else {
        format!("\"{}\"", value.replace('"', "\\\""))
    }
}

fn redact_env_text(text: &str) -> String {
    let env = read_env_text(text);
    let redacted: HashMap<String, String> = env
        .into_iter()
        .map(|(key, value)| {
            if is_secret_key(&key) {
                (key, mask_secret(&value))
            } else {
                (key, value)
            }
        })
        .collect();
    render_env_file(&redacted)
}

fn openclaw_batch(account: &Account, env_name: &str) -> Value {
    let model = account.default_model.trim();
    json!([
        {
            "path": "models.providers.deecodex",
            "value": {
                "baseUrl": account.upstream.clone(),
                "apiKey": {"provider": "default", "source": "env", "id": env_name},
                "auth": "api-key",
                "api": openclaw_api_adapter(&account.provider),
                "models": [{"id": model, "name": model}]
            }
        },
        {
            "path": "agents.defaults.model",
            "value": openclaw_model_ref(model)
        }
    ])
}

fn openclaw_command(account: &Account, env_name: &str) -> Command {
    let mut command = Command::new("openclaw");
    if !account.api_key.trim().is_empty() {
        command.env(env_name, account.api_key.trim());
    }
    command
}

fn openclaw_api_adapter(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "anthropic-messages",
        "google-ai" | "gemini" => "google-generative-ai",
        _ => "openai-completions",
    }
}

fn openclaw_model_ref(model: &str) -> String {
    format!("deecodex/{}", model.trim())
}

fn hermes_key_name(account: &Account) -> String {
    if let Some(name) = client_option_string(account, "api_key_env") {
        return name;
    }
    hermes_key_name_for_provider(&account.provider)
}

fn hermes_key_name_for_provider(provider: &str) -> String {
    match provider {
        "openrouter" => "OPENROUTER_API_KEY",
        "anthropic" => "ANTHROPIC_API_KEY",
        "openai" => "OPENAI_API_KEY",
        "minimax" => "MINIMAX_API_KEY",
        _ => "OPENAI_API_KEY",
    }
    .into()
}

fn set_yaml_path(root: &mut serde_yaml::Value, path: &[&str], value: serde_yaml::Value) {
    if path.is_empty() {
        *root = value;
        return;
    }
    if !matches!(root, serde_yaml::Value::Mapping(_)) {
        *root = serde_yaml::Value::Mapping(Default::default());
    }
    let mut current = root;
    for key in &path[..path.len() - 1] {
        let map = match current {
            serde_yaml::Value::Mapping(map) => map,
            _ => unreachable!(),
        };
        current = map
            .entry(serde_yaml::Value::String((*key).into()))
            .or_insert_with(|| serde_yaml::Value::Mapping(Default::default()));
        if !matches!(current, serde_yaml::Value::Mapping(_)) {
            *current = serde_yaml::Value::Mapping(Default::default());
        }
    }
    if let serde_yaml::Value::Mapping(map) = current {
        map.insert(
            serde_yaml::Value::String(path[path.len() - 1].into()),
            value,
        );
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in value.chars().enumerate() {
        if idx >= max_chars {
            out.push_str("...");
            return out;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_profiles_include_requested_clients() {
        let slugs: Vec<_> = get_client_profiles().into_iter().map(|p| p.slug).collect();
        assert!(slugs.contains(&"codex".to_string()));
        assert!(slugs.contains(&"claude_code".to_string()));
        assert!(slugs.contains(&"openclaw".to_string()));
        assert!(slugs.contains(&"hermes".to_string()));
        assert!(slugs.contains(&"generic_client".to_string()));
    }

    #[test]
    fn redaction_masks_secret_values() {
        let value =
            json!({"env":{"ANTHROPIC_API_KEY":"sk-abcdef123456","ANTHROPIC_MODEL":"sonnet"}});
        let redacted = redact_json(&value);
        assert_eq!(redacted["env"]["ANTHROPIC_API_KEY"], "sk-a****3456");
        assert_eq!(redacted["env"]["ANTHROPIC_MODEL"], "sonnet");
    }

    #[test]
    fn diff_text_summarizes_changed_lines() {
        let diff = diff_text("env", "OPENAI_MODEL=old\n", "OPENAI_MODEL=new\n");

        assert!(diff.iter().any(|line| line == "env: - OPENAI_MODEL=old"));
        assert!(diff.iter().any(|line| line == "env: + OPENAI_MODEL=new"));
    }

    #[test]
    fn env_redaction_masks_keys_in_diff() {
        let before = redact_env_text("OPENAI_API_KEY=sk-before123456\nOPENAI_MODEL=old\n");
        let after = redact_env_text("OPENAI_API_KEY=sk-afterabcdef\nOPENAI_MODEL=new\n");
        let diff = diff_text("env", &before, &after).join("\n");

        assert!(diff.contains("OPENAI_MODEL=new"));
        assert!(!diff.contains("sk-before123456"));
        assert!(!diff.contains("sk-afterabcdef"));
        assert!(diff.contains("****"));
    }

    #[test]
    fn env_renderer_roundtrips_simple_values() {
        let mut values = HashMap::new();
        values.insert(
            "OPENAI_BASE_URL".into(),
            "https://api.example.com/v1".into(),
        );
        values.insert("OPENAI_API_KEY".into(), "sk-test".into());
        let rendered = render_env_file(&values);
        let parsed = read_env_text(&rendered);
        assert_eq!(parsed.get("OPENAI_API_KEY"), Some(&"sk-test".to_string()));
    }

    #[test]
    fn restore_backup_restores_existing_file_and_removes_new_file() {
        let dir = std::env::temp_dir().join(format!(
            "deecodex-client-rollback-{}-{}",
            std::process::id(),
            now_secs()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let backup = dir.join("settings.json.deecodex.bak.test");
        fs::write(&path, "new").unwrap();
        fs::write(&backup, "old").unwrap();

        restore_backup(&path, Some(backup.as_path())).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "old");

        let created = dir.join("created.env");
        fs::write(&created, "temporary").unwrap();
        restore_backup(&created, None).unwrap();
        assert!(!created.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mask_secret_is_unicode_safe() {
        assert_eq!(mask_secret("钥匙abcdef1234"), "钥匙ab****1234");
    }

    #[test]
    fn openclaw_batch_uses_schema_supported_paths() {
        let account: Account = serde_json::from_value(json!({
            "id": "oc",
            "name": "OpenClaw",
            "provider": "openrouter",
            "client_kind": "openclaw",
            "upstream": "https://openrouter.ai/api/v1",
            "api_key": "sk-test",
            "default_model": "anthropic/claude-sonnet-4.5"
        }))
        .unwrap();
        let batch = openclaw_batch(&account, "OPENROUTER_API_KEY");
        assert_eq!(batch[0]["path"], "models.providers.deecodex");
        assert_eq!(batch[0]["value"]["api"], "openai-completions");
        assert_eq!(
            batch[0]["value"]["models"][0]["id"],
            "anthropic/claude-sonnet-4.5"
        );
        assert_eq!(batch[1]["path"], "agents.defaults.model");
        assert_eq!(batch[1]["value"], "deecodex/anthropic/claude-sonnet-4.5");
    }
}
