use std::{net::SocketAddr, path::PathBuf};

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use figment::{providers::Format, Figment};
use futures_util::future;
use tracing::info;
use tracing::metadata::LevelFilter;
use tracing_subscriber::{prelude::*, EnvFilter};

mod account;
mod auth;
mod server;

use auth::{AuthData, AuthManager};

use crate::{
    account::Accounts,
    auth::SledDbAuthStorage,
    auth::{ErasedAuthStorage, InMemoryAuthStorage},
};

#[derive(Parser, Debug)]
struct Args {
    /// Path to auth json file
    #[arg(
        long,
        value_parser = clap::value_parser!(PathBuf),
    )]
    auth: Option<PathBuf>,
    /// Host and port to listen on
    #[arg(
        long,
        value_parser = clap::value_parser!(SocketAddr),
        default_value = "0.0.0.0:3000"
    )]
    listen_addr: SocketAddr,
    /// Output logs directly to systemd
    #[arg(long, default_value = "false")]
    log_to_systemd: bool,
    /// Path to database
    #[arg(long, value_parser = clap::value_parser!(PathBuf))]
    db_path: Option<PathBuf>,
}

fn init_logging(use_systemd: bool) -> Result<()> {
    let registry = tracing_subscriber::registry();
    let layer = {
        #[cfg(target_os = "linux")]
        if use_systemd && libsystemd::daemon::booted() {
            tracing_journald::layer()
                .context("tracing_journald layer")?
                .boxed()
        } else {
            tracing_subscriber::fmt::layer()
                .pretty()
                .with_target(true)
                .boxed()
        }
        #[cfg(not(target_os = "linux"))]
        if use_systemd {
            return Err(anyhow!("Systemd logging is not supported on this platform"));
        } else {
            tracing_subscriber::fmt::layer().pretty().with_target(true)
        }
    };

    let filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env()
        .context("Failed to parse filter from env")?;

    registry.with(filter).with(layer).init();

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    init_logging(args.log_to_systemd).context("Failed to initialize logging")?;

    let api = dt_api::Api::new();

    let accounts = Accounts::default();

    let auth_storage = if let Some(db_path) = args.db_path {
        SledDbAuthStorage::new(db_path)?.into()
    } else {
        InMemoryAuthStorage::default().into()
    };

    let auth_manager = AuthManager::<ErasedAuthStorage>::new_with_storage(
        api.clone(),
        accounts.clone(),
        auth_storage,
    );

    if let Some(auth) = args.auth {
        let auth = Figment::new()
            .merge(figment::providers::Json::file(auth))
            .extract()?;

        auth_manager
            .auth_data()
            .add_auth(auth)
            .await
            .context("Failed to add auth")?;
    }

    let auth_data = auth_manager.auth_data();

    let server = server::Server::new(api, accounts, auth_data.clone(), args.listen_addr);

    info!("Starting server");

    let serve_task = tokio::spawn(server.start());
    let auth_task = tokio::spawn(auth_manager.start());
    let exit_task = tokio::spawn(async move {
        tokio::signal::ctrl_c()
            .await
            .context("ctrl_c handler failed")?;
        auth_data
            .shutdown()
            .await
            .context("sending shutdown signal failed")?;
        future::pending::<()>().await;
        Result::<()>::Ok(())
    });

    info!("Listening on {}", args.listen_addr);

    match tokio::select! {
        res = auth_task => res?.context("Auth manager failed"),
        res = serve_task => res?.context("Server failed"),
        res = exit_task => res?.context("Exit task failed"),
    } {
        Ok(_) => {
            info!("Exiting");
            Ok(())
        }
        Err(e) => Err(anyhow!("task failed: {e}")),
    }
}
