#![forbid(unsafe_code)]

use std::error::Error;
use std::io::Write as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_platform_app::Application;
use agent_platform_auth::{
    AGENTS_MANAGE, AGENTS_READ, CAPABILITIES_MANAGE, CAPABILITIES_READ, CredentialVerifier,
    DevelopmentVerifier, IdentityVerifier, TASKS_READ, TASKS_SUBMIT, TRIGGERS_MANAGE,
    TRIGGERS_READ, VerifiedAuthority,
};
use agent_platform_connectors::{InMemoryCatalog, OperationDescription};
use agent_platform_core::{SubjectId, TenantId};
use agent_platform_harness::UserModelRunner;
use agent_platform_http::{HttpState, router};
use clap::{Args, Parser, Subcommand};

const MAX_CATALOG_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Parser)]
#[command(name = "agent-platform", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the loopback development management service.
    Serve(Box<ServeOptions>),
    /// Write the exact `OpenAPI` document served at `/openapi.json`.
    Openapi {
        /// Print only the deterministic SHA-256 digest.
        #[arg(long)]
        digest: bool,
    },
}

#[derive(Debug, Args)]
struct ServeOptions {
    #[arg(long, default_value = "127.0.0.1:8090")]
    listen: SocketAddr,
    #[arg(long)]
    allow_insecure_dev_listener: bool,
    /// Identity service origin. When present, requests use production session authority.
    #[arg(long)]
    identity_origin: Option<String>,
    #[arg(long, default_value = "urn:b10x:agent-platform")]
    identity_audience: String,
    /// Identity-authenticated hosted Connectors API base used for attempt leases.
    #[arg(long)]
    connectors_api_base: Option<String>,
    /// Identity-authenticated Workspace service origin used for hosted coding-session turns.
    #[arg(long)]
    workspace_origin: Option<String>,
    /// Messages-compatible provider API prefix used by Harness.
    #[arg(long, default_value = "https://api.anthropic.com/v1")]
    model_endpoint_base: String,
    #[arg(long, default_value_t = 200_000)]
    model_context_window: u64,
    #[arg(long, default_value = "local")]
    tenant: String,
    #[arg(long, default_value = "human:developer")]
    subject: String,
    /// Synthetic or operator-owned Connector descriptions for the walking slice.
    #[arg(long)]
    connector_catalog: Option<PathBuf>,
    /// Credential-free durable state snapshot. Omit only for disposable development processes.
    #[arg(long)]
    state_path: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Command::Serve(options) => serve(&options).await,
        Command::Openapi { digest } => {
            if digest {
                println!("{}", agent_platform_openapi::document_sha256());
            } else {
                std::io::stdout().write_all(&agent_platform_openapi::document_bytes())?;
            }
            Ok(())
        }
    }
}

async fn serve(options: &ServeOptions) -> Result<(), Box<dyn Error>> {
    let identity_origin = options.identity_origin.as_deref();
    let connectors_api_base = options.connectors_api_base.as_deref();
    let workspace_origin = options.workspace_origin.as_deref();
    if workspace_origin.is_some() && identity_origin.is_none() {
        return Err("Workspace coding sessions require Identity production authority".into());
    }
    if identity_origin.is_none()
        && !options.listen.ip().is_loopback()
        && !options.allow_insecure_dev_listener
    {
        return Err(
            "the development verifier may bind only loopback; pass --allow-insecure-dev-listener for an isolated preview"
                .into(),
        );
    }
    if identity_origin.is_none() && !options.listen.ip().is_loopback() {
        eprintln!(
            "warning: exposing the fixed development bearer verifier on {}; this is not production authentication",
            options.listen
        );
    }
    let scopes = [
        AGENTS_MANAGE,
        AGENTS_READ,
        CAPABILITIES_MANAGE,
        CAPABILITIES_READ,
        TASKS_READ,
        TASKS_SUBMIT,
        TRIGGERS_MANAGE,
        TRIGGERS_READ,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let verifier: Arc<dyn CredentialVerifier> = if let Some(identity_origin) = identity_origin {
        let verifier = IdentityVerifier::new(identity_origin, &options.identity_audience, scopes)?;
        let verifier = if let Some(connectors_api_base) = connectors_api_base {
            verifier.with_connectors(connectors_api_base)?
        } else {
            verifier
        };
        let verifier = if let Some(workspace_origin) = workspace_origin {
            verifier.with_workspace(workspace_origin)?
        } else {
            verifier
        };
        Arc::new(verifier)
    } else {
        let token = std::env::var("AGENT_PLATFORM_DEV_BEARER_TOKEN")
            .map_err(|_| "AGENT_PLATFORM_DEV_BEARER_TOKEN is required")?;
        let authority = VerifiedAuthority::new(
            TenantId::new(options.tenant.clone())?,
            SubjectId::new(options.subject.clone())?,
            None,
            None,
            scopes,
        )?;
        let verifier = Arc::new(DevelopmentVerifier::new(&token, authority)?);
        drop(token);
        verifier
    };

    let descriptions = options
        .connector_catalog
        .as_deref()
        .map_or_else(|| Ok(Vec::new()), read_catalog)?;
    let catalog = Arc::new(InMemoryCatalog::new(descriptions));
    let app = if let Some(path) = &options.state_path {
        Application::open(catalog, path, now_ms())?
    } else {
        Application::new(catalog)
    };
    let mut http_state = HttpState::new(app, verifier);
    if connectors_api_base.is_some() {
        http_state = http_state.with_runner(UserModelRunner::new(
            &options.model_endpoint_base,
            options.model_context_window,
        )?);
    }
    let listener = tokio::net::TcpListener::bind(options.listen).await?;
    println!(
        "agent-platform development service listening on {}",
        options.listen
    );
    axum::serve(listener, router(http_state))
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

fn read_catalog(path: &Path) -> Result<Vec<OperationDescription>, Box<dyn Error>> {
    let metadata = std::fs::metadata(path)?;
    if !metadata.is_file() || metadata.len() > MAX_CATALOG_BYTES {
        return Err(format!(
            "connector catalog must be a regular file no larger than {MAX_CATALOG_BYTES} bytes"
        )
        .into());
    }
    let bytes = std::fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

async fn shutdown() {
    if tokio::signal::ctrl_c().await.is_err() {
        eprintln!("warning: could not install the shutdown signal handler");
    }
}
