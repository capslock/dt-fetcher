use std::{
    collections::HashMap, future::IntoFuture, net::SocketAddr, path::PathBuf, sync::Arc,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use auth::{AuthData, AuthManager};
use axum::{
    body::Body,
    extract::{FromRef, Path, State},
    http::{Request, Response, StatusCode},
    routing::{get, put},
    Json, Router,
};
use clap::Parser;
use dt_api::models::{MasterData, Store, Summary};
use figment::{providers::Format, Figment};
use futures::stream::{FuturesOrdered, StreamExt};
use futures_util::future;
use tokio::sync::RwLock;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{error, metadata::LevelFilter, Span};
use tracing::{info, instrument};
use tracing_subscriber::{prelude::*, EnvFilter};
use uuid::Uuid;

use crate::{
    auth::{get_auth, put_auth},
    store::store,
    store::store_single,
};

mod auth;
mod store;

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

#[derive(Debug, Clone)]
struct AccountData {
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
                Ok(s) => Some((c.id.clone(), s)),
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
                Ok(s) => Some((c.id.clone(), s)),
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
}

#[derive(Debug, Clone)]
struct AppData {
    api: dt_api::Api,
    accounts: Accounts,
    auth_data: AuthData,
}

impl FromRef<AppData> for AuthData {
    fn from_ref(state: &AppData) -> Self {
        state.auth_data.clone()
    }
}

impl FromRef<AppData> for dt_api::Api {
    fn from_ref(state: &AppData) -> Self {
        state.api.clone()
    }
}

impl FromRef<AppData> for Accounts {
    fn from_ref(state: &AppData) -> Self {
        state.accounts.clone()
    }
}

fn init_logging(use_systemd: bool) -> Result<()> {
    let registry = tracing_subscriber::registry();
    let layer = {
        #[cfg(target_os = "linux")]
        if log_to_systemd && libsystemd::daemon::booted() {
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
    let auth_manager = AuthManager::new(api.clone(), accounts.clone());

    let app_data = if let Some(auth) = args.auth {
        let auth = Figment::new()
            .merge(figment::providers::Json::file(auth))
            .extract()?;
        info!("Refreshing auth");

        let auth = api.refresh_auth(&auth).await?;

        info!("Fetching data");

        accounts
            .insert(auth.sub.clone(), AccountData::fetch(&api, &auth).await?)
            .await;

        AppData {
            api,
            accounts,
            auth_data: auth_manager.auth_data(),
        }
    } else {
        AppData {
            api,
            accounts,
            auth_data: auth_manager.auth_data(),
        }
    };

    let auth_data = app_data.auth_data.clone();

    let app = Router::new()
        .route("/store", get(store_single))
        .route("/summary", get(summary_single))
        .route("/master_data", get(master_data_single))
        .route("/store/:id", get(store))
        .route("/summary/:id", get(summary))
        .route("/master_data/:id", get(master_data))
        .route("/auth/:id", put(put_auth))
        .route("/auth/:id", get(get_auth))
        .with_state(app_data)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|_request: &Request<Body>| tracing::info_span!("http-request"))
                .on_request(|request: &Request<Body>, _span: &Span| {
                    tracing::info!(method = %request.method(), path = %request.uri().path(), "got request")
                })
                .on_response(|_response: &Response<Body>, latency: Duration, _span: &Span| {
                tracing::info!("response generated in {:?}", latency)
            })
        ).layer(CorsLayer::permissive());

    info!("Starting server");

    let listener = tokio::net::TcpListener::bind(args.listen_addr).await?;

    let serve_task = tokio::spawn(axum::serve(listener, app).into_future());
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

#[instrument(skip(state))]
async fn summary(
    Path(id): Path<Uuid>,
    State(state): State<AppData>,
) -> Result<Json<Summary>, StatusCode> {
    if let Some(account_data) = state.accounts.get(&id).await {
        info!("Returning cached summary");
        Ok(Json(account_data.summary.read().await.clone()))
    } else {
        refresh_summary(&id, state).await
    }
}

#[instrument(skip(state))]
async fn summary_single(State(state): State<AppData>) -> Result<Json<Summary>, StatusCode> {
    let auth = state.auth_data.get_single().await;
    if let Some(auth) = auth {
        summary(Path(auth.sub), State(state)).await
    } else {
        error!("Failed to find account data");
        Err(StatusCode::NOT_FOUND)
    }
}

#[instrument(skip(state))]
async fn refresh_summary(account_id: &Uuid, state: AppData) -> Result<Json<Summary>, StatusCode> {
    let api = &state.api;
    let account_data = if let Some(account_data) = state.accounts.get(account_id).await {
        account_data
    } else {
        error!(sid = ?account_id, "Failed to find account data");
        return Err(StatusCode::NOT_FOUND);
    };
    if let Some(auth_data) = state.auth_data.get(account_id).await {
        let new_summary = api.get_summary(&auth_data).await;
        if let Ok(new_summary) = new_summary {
            let mut summary = account_data.summary.write().await;
            *summary = new_summary.clone();
            Ok(Json(new_summary))
        } else {
            error!(error = %new_summary.unwrap_err(), "Failed to get summary");
            Err(StatusCode::NOT_FOUND)
        }
    } else {
        error!(sid = ?account_id, "Failed to find auth data");
        Err(StatusCode::NOT_FOUND)
    }
}

#[instrument(skip(state))]
async fn master_data(
    Path(id): Path<Uuid>,
    State(state): State<AppData>,
) -> Result<Json<MasterData>, StatusCode> {
    if let Some(account_data) = state.accounts.get(&id).await {
        info!("Returning cached master data");
        Ok(Json(account_data.master_data.read().await.clone()))
    } else {
        error!("Failed to find account data");
        Err(StatusCode::NOT_FOUND)
    }
}

#[instrument(skip(state))]
async fn master_data_single(State(state): State<AppData>) -> Result<Json<MasterData>, StatusCode> {
    let auth = state.auth_data.get_single().await;
    if let Some(auth) = auth {
        master_data(Path(auth.sub), State(state)).await
    } else {
        error!("Failed to find account data");
        Err(StatusCode::NOT_FOUND)
    }
}
