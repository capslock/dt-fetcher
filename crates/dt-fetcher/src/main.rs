use std::{collections::HashMap, net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::{anyhow, Context, Result};
use auth::{AuthData, AuthManager};
use chrono::{DateTime, Utc};
use clap::Parser;
use dt_api::models::{MasterData, Store};
use figment::{providers::Format, Figment};
use futures::stream::{FuturesOrdered, StreamExt};
use futures_util::future;
use tokio::sync::RwLock;
use tracing::{error, metadata::LevelFilter};
use tracing::{info, instrument};
use tracing_subscriber::{prelude::*, EnvFilter};
use uuid::Uuid;

mod auth;
mod server;

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

#[derive(Debug, Clone)]
struct AccountData {
    last_updated: DateTime<Utc>,
    summary: Arc<RwLock<dt_api::models::Summary>>,
    marks_store: Arc<RwLock<HashMap<Uuid, dt_api::models::Store>>>,
    credits_store: Arc<RwLock<HashMap<Uuid, dt_api::models::Store>>>,
    master_data: Arc<RwLock<dt_api::models::MasterData>>,
}

impl AccountData {
    fn new(
        summary: dt_api::models::Summary,
        marks_store: HashMap<Uuid, Store>,
        credits_store: HashMap<Uuid, Store>,
        master_data: MasterData,
    ) -> Self {
        Self {
            last_updated: Utc::now(),
            summary: Arc::new(RwLock::new(summary)),
            marks_store: Arc::new(RwLock::new(marks_store)),
            credits_store: Arc::new(RwLock::new(credits_store)),
            master_data: Arc::new(RwLock::new(master_data)),
        }
    }

    #[instrument]
    async fn fetch(api: &dt_api::Api, auth: &dt_api::Auth) -> Result<AccountData> {
        let summary = api.get_summary(auth).await?;

        info!(
            "Fetching stores for {} characters",
            summary.characters.len()
        );

        let marks_store = summary
            .characters
            .iter()
            .map(|c| api.get_store(auth, dt_api::models::CurrencyType::Marks, c))
            .collect::<FuturesOrdered<_>>()
            .collect::<Vec<_>>();

        let credits_store = summary
            .characters
            .iter()
            .map(|c| api.get_store(auth, dt_api::models::CurrencyType::Credits, c))
            .collect::<FuturesOrdered<_>>()
            .collect::<Vec<_>>();

        let (marks_store, credits_store) = tokio::join!(marks_store, credits_store);

        let marks_store = summary
            .characters
            .iter()
            .zip(marks_store.into_iter())
            .filter_map(|(c, s)| match s {
                Ok(s) => Some((c.id, s)),
                Err(e) => {
                    error!("Failed to get marks store: {}", e);
                    None
                }
            })
            .collect::<HashMap<Uuid, Store>>();

        let credits_store = summary
            .characters
            .iter()
            .zip(credits_store.into_iter())
            .filter_map(|(c, s)| match s {
                Ok(s) => Some((c.id, s)),
                Err(e) => {
                    error!("Failed to get credits store: {}", e);
                    None
                }
            })
            .collect::<HashMap<Uuid, Store>>();

        let master_data = api.get_master_data(auth).await?;

        Ok(Self::new(summary, marks_store, credits_store, master_data))
    }
}

#[derive(Debug, Clone, Default)]
struct Accounts(Arc<RwLock<HashMap<Uuid, AccountData>>>);

impl Accounts {
    #[instrument]
    async fn get(&self, id: &Uuid) -> Option<AccountData> {
        self.0.read().await.get(id).cloned()
    }

    #[instrument]
    async fn insert(&self, id: Uuid, data: AccountData) {
        self.0.write().await.insert(id, data);
    }

    #[instrument]
    async fn update_timestamp(&self, id: &Uuid) {
        if let Some(account_data) = self.0.write().await.get_mut(id) {
            account_data.last_updated = Utc::now();
        }
    }

    #[instrument]
    async fn timestamp(&self, id: &Uuid) -> Option<DateTime<Utc>> {
        if let Some(account_data) = self.0.read().await.get(id) {
            return Some(account_data.last_updated);
        }
        None
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    init_logging(args.log_to_systemd).context("Failed to initialize logging")?;

    let api = dt_api::Api::new();

    let accounts = Accounts::default();

    let auth_manager = AuthManager::new(api.clone(), accounts.clone());

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
