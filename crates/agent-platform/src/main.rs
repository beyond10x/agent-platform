#![forbid(unsafe_code)]

use std::error::Error;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_platform_app::Application;
use agent_platform_auth::{
    AGENTS_MANAGE, AGENTS_READ, CAPABILITIES_MANAGE, CAPABILITIES_READ, DevelopmentVerifier,
    TASKS_READ, TASKS_SUBMIT, TRIGGERS_MANAGE, TRIGGERS_READ, VerifiedAuthority,
};
use agent_platform_connectors::{InMemoryCatalog, OperationDescription};
use agent_platform_core::{SubjectId, TenantId};
use agent_platform_http::{HttpState, router};
use clap::{Parser, Subcommand};

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
    Serve {
        #[arg(long, default_value = "127.0.0.1:8090")]
        listen: SocketAddr,
        #[arg(long)]
        allow_insecure_dev_listener: bool,
        #[arg(long, default_value = "local")]
        tenant: String,
        #[arg(long, default_value = "human:developer")]
        subject: String,
        /// Synthetic or operator-owned Connector descriptions for the walking slice.
        #[arg(long)]
        connector_catalog: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Command::Serve {
            listen,
            allow_insecure_dev_listener,
            tenant,
            subject,
            connector_catalog,
        } => {
            serve(
                listen,
                allow_insecure_dev_listener,
                tenant,
                subject,
                connector_catalog.as_deref(),
            )
            .await
        }
    }
}

async fn serve(
    listen: SocketAddr,
    allow_insecure_dev_listener: bool,
    tenant: String,
    subject: String,
    connector_catalog: Option<&Path>,
) -> Result<(), Box<dyn Error>> {
    if !listen.ip().is_loopback() && !allow_insecure_dev_listener {
        return Err(
            "the development verifier may bind only loopback; pass --allow-insecure-dev-listener for an isolated preview"
                .into(),
        );
    }
    if !listen.ip().is_loopback() {
        eprintln!(
            "warning: exposing the fixed development bearer verifier on {listen}; this is not production authentication"
        );
    }
    let token = std::env::var("AGENT_PLATFORM_DEV_BEARER_TOKEN")
        .map_err(|_| "AGENT_PLATFORM_DEV_BEARER_TOKEN is required")?;
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
    .map(str::to_owned);
    let authority = VerifiedAuthority::new(
        TenantId::new(tenant)?,
        SubjectId::new(subject)?,
        None,
        None,
        scopes,
    )?;
    let verifier = Arc::new(DevelopmentVerifier::new(&token, authority)?);
    drop(token);

    let descriptions = connector_catalog.map_or_else(|| Ok(Vec::new()), read_catalog)?;
    let catalog = Arc::new(InMemoryCatalog::new(descriptions));
    let app = Application::new(catalog);
    let listener = tokio::net::TcpListener::bind(listen).await?;
    println!("agent-platform development service listening on {listen}");
    axum::serve(listener, router(HttpState::new(app, verifier)))
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
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
