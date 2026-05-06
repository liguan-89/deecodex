mod cache;
mod files;
mod handlers;
mod prompts;
mod session;
mod sse;
mod stream;
mod translate;
mod types;
mod utils;
mod vector_stores;

use std::io::{self, Write};

use anyhow::Result;
use clap::Parser;
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info};

#[derive(Parser, Debug)]
#[command(name = "deecodex", about = "Responses API <-> Chat Completions bridge")]
struct Args {
    #[arg(long, env = "CODEX_RELAY_PORT", default_value = "4444")]
    port: u16,

    #[arg(
        long,
        env = "CODEX_RELAY_UPSTREAM",
        default_value = "https://openrouter.ai/api/v1"
    )]
    upstream: String,

    #[arg(long, env = "CODEX_RELAY_API_KEY", default_value = "")]
    api_key: String,

    /// Client-facing bearer token required by local callers. Empty disables local auth.
    #[arg(long, env = "CODEX_RELAY_CLIENT_API_KEY", default_value = "")]
    client_api_key: String,

    #[arg(long, env = "CODEX_RELAY_MODEL_MAP", default_value = "{}")]
    model_map: String,

    #[arg(long, env = "CODEX_RELAY_MAX_BODY_MB", default_value = "100")]
    max_body_mb: usize,

    #[arg(long, env = "CODEX_RELAY_VISION_UPSTREAM", default_value = "")]
    vision_upstream: String,

    #[arg(long, env = "CODEX_RELAY_VISION_API_KEY", default_value = "")]
    vision_api_key: String,

    #[arg(long, env = "CODEX_RELAY_VISION_MODEL", default_value = "MiniMax-M1")]
    vision_model: String,

    #[arg(
        long,
        env = "CODEX_RELAY_VISION_ENDPOINT",
        default_value = "v1/coding_plan/vlm"
    )]
    vision_endpoint: String,

    #[arg(long, env = "CODEX_RELAY_CHINESE_THINKING", default_value = "false")]
    chinese_thinking: bool,

    #[arg(long, env = "CODEX_RELAY_PROMPTS_DIR", default_value = "prompts")]
    prompts_dir: std::path::PathBuf,
}

struct FlushWriter<W: Write>(W);

impl<W: Write> Write for FlushWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.0.write(buf)?;
        self.0.flush()?;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(|| FlushWriter(std::io::stderr()))
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "deecodex=info".into()),
        )
        .init();

    let args = Args::parse();

    let model_map: HashMap<String, String> = match serde_json::from_str(&args.model_map) {
        Ok(m) => m,
        Err(e) => {
            error!("Failed to parse CODEX_RELAY_MODEL_MAP: {e}");
            HashMap::new()
        }
    };

    info!("model map: {} entries", model_map.len());

    let upstream = handlers::validate_upstream(&args.upstream)?;

    let vision_upstream = if args.vision_upstream.is_empty() {
        None
    } else {
        Some(Arc::new(handlers::validate_upstream(
            &args.vision_upstream,
        )?))
    };
    if vision_upstream.is_some() {
        info!("vision upstream configured: {}", args.vision_upstream);
    }

    let state = handlers::AppState {
        sessions: crate::session::SessionStore::new(),
        client: Client::builder()
            .pool_idle_timeout(None)
            .pool_max_idle_per_host(4)
            .timeout(std::time::Duration::from_secs(300))
            .build()?,
        upstream: Arc::new(upstream),
        api_key: Arc::new(args.api_key),
        client_api_key: Arc::new(args.client_api_key),
        model_map: Arc::new(model_map),
        vision_upstream,
        vision_api_key: Arc::new(args.vision_api_key),
        vision_model: Arc::new(args.vision_model),
        vision_endpoint: Arc::new(args.vision_endpoint),
        start_time: std::time::Instant::now(),
        request_cache: crate::cache::RequestCache::default(),
        prompts: Arc::new(crate::prompts::PromptRegistry::new(&args.prompts_dir)),
        files: crate::files::FileStore::new(),
        vector_stores: crate::vector_stores::VectorStoreRegistry::new(),
        background_tasks: Arc::new(dashmap::DashMap::new()),
        chinese_thinking: args.chinese_thinking,
    };
    info!("local prompts registry: {}", args.prompts_dir.display());
    if args.chinese_thinking {
        info!("chinese thinking mode: enabled (system prompt will include Chinese instruction)");
    }
    if state.client_api_key.is_empty() {
        tracing::warn!(
            "client auth disabled because CODEX_RELAY_CLIENT_API_KEY and CODEX_RELAY_API_KEY are empty"
        );
    } else {
        info!("client auth enabled for /v1 API routes");
    }

    let max_bytes = args.max_body_mb * 1024 * 1024;
    let body_limit = axum::extract::DefaultBodyLimit::max(max_bytes);

    let app = handlers::build_router(state.clone()).layer(body_limit);

    let addr = format!("127.0.0.1:{}", args.port);
    info!(
        "listening {} -> {} | body:{}MB",
        addr,
        state.upstream.as_ref(),
        args.max_body_mb
    );

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
