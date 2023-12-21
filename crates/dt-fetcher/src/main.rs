use std::{
    collections::{BinaryHeap, HashMap},
    future::IntoFuture,
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime},
};

use anyhow::{anyhow, Context, Result};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{Request, Response, StatusCode},
    routing::{get, put},
    Json, Router,
};
use chrono::{DateTime, Utc};
use clap::Parser;
use dt_api::models::{MasterData, Store, Summary};
use figment::{providers::Format, Figment};
use futures::stream::{FuturesOrdered, FuturesUnordered, StreamExt};
use tokio::sync::{Mutex, RwLock};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{debug, error, metadata::LevelFilter, warn, Span};
use tracing::{info, instrument};
use tracing_subscriber::{prelude::*, EnvFilter};
use uuid::Uuid;

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

#[derive(Debug)]
struct AccountData {
    auth: RwLock<dt_api::Auth>,
    summary: RwLock<dt_api::models::Summary>,
    marks_store: RwLock<HashMap<Uuid, dt_api::models::Store>>,
    credits_store: RwLock<HashMap<Uuid, dt_api::models::Store>>,
    master_data: RwLock<dt_api::models::MasterData>,
}

#[derive(Debug)]
struct AppData {
    api: RwLock<dt_api::Api>,
    account_data: RwLock<HashMap<Uuid, AccountData>>,
    new_auth: Mutex<Vec<Uuid>>,
}

#[derive(PartialEq, Eq)]
struct RefreshAuth {
    id: Uuid,
    refresh_at: DateTime<Utc>,
}

impl PartialOrd for RefreshAuth {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        other.refresh_at.partial_cmp(&self.refresh_at)
    }
}

impl Ord for RefreshAuth {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.refresh_at.cmp(&self.refresh_at)
    }
}

#[instrument(skip_all)]
async fn get_new_auths(
    app_data: Arc<AppData>,
    iter: impl Iterator<Item = Uuid>,
) -> Vec<RefreshAuth> {
    iter.map(|id| {
        info!("Adding new auth {}", id);
        let app_data = app_data.clone();
        async move { get_new_auth(app_data, id).await }
    })
    .collect::<FuturesUnordered<_>>()
    .collect::<Vec<_>>()
    .await
    .into_iter()
    .filter_map(|v| v)
    .collect::<Vec<_>>()
}

#[instrument(skip(app_data))]
async fn get_new_auth(app_data: Arc<AppData>, id: Uuid) -> Option<RefreshAuth> {
    if let Some(refresh_auth) = app_data.account_data.read().await.get(&id).map(|v| async {
        RefreshAuth {
            refresh_at: v.auth.read().await.refresh_at.unwrap_or_default(),
            id: id.clone(),
        }
    }) {
        let refresh_auth = refresh_auth.await;
        info!(sub = ?refresh_auth.id, "Got auth");
        Some(refresh_auth)
    } else {
        warn!("Failed to find account data");
        None
    }
}

#[instrument(skip_all)]
async fn refresh_auth(app_data: Arc<AppData>) -> Result<()> {
    let auths = get_new_auths(
        app_data.clone(),
        app_data.account_data.read().await.keys().copied(),
    )
    .await;
    let mut auths = BinaryHeap::from_iter(auths);
    loop {
        let mut new_auth = app_data.new_auth.lock().await;
        auths.extend(get_new_auths(app_data.clone(), new_auth.iter().copied()).await);
        new_auth.clear();
        drop(new_auth);

        let duration = if let Some(refresh_auth) = auths.peek() {
            (refresh_auth.refresh_at - DateTime::from(SystemTime::now())).to_std()?
        } else {
            Duration::from_secs(300)
        };

        if duration.as_secs() > 0 {
            info!("Refreshing auth in {:?}", duration);
            tokio::time::sleep(duration).await;
        }
        if let Some(mut refresh_auth) = auths.peek_mut() {
            info!(sub = ?refresh_auth.id, "Refreshing auth");
            let account_data = app_data.account_data.read().await;
            let mut auth = account_data[&refresh_auth.id].auth.write().await;
            *auth = app_data
                .api
                .read()
                .await
                .refresh_auth(&*auth)
                .await
                .context("failed to refresh auth")?;
            (*auth).refresh_at = Some(
                DateTime::from(SystemTime::now())
                    + auth.expires_in.saturating_sub(Duration::from_secs(300)),
            );
            refresh_auth.refresh_at = auth.refresh_at.expect("auth refresh_at is None");
            info!(auth = ?*auth, "Auth refreshed");
        }
    }
}

#[instrument]
async fn build_account_data(api: &dt_api::Api, auth: &dt_api::Auth) -> Result<AccountData> {
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

    Ok(AccountData {
        auth: RwLock::new(auth.clone()),
        summary: RwLock::new(summary),
        marks_store: RwLock::new(marks_store),
        credits_store: RwLock::new(credits_store),
        master_data: RwLock::new(master_data),
    })
}

#[instrument]
async fn build_app_data(api: dt_api::Api, auth: &dt_api::Auth) -> Result<Arc<AppData>> {
    Ok(Arc::new(AppData {
        account_data: RwLock::new(HashMap::from([(
            auth.sub.clone(),
            build_account_data(&api, auth).await?,
        )])),
        api: RwLock::new(api),
        new_auth: Default::default(),
    }))
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

    let app_data = if let Some(auth) = args.auth {
        let auth = Figment::new()
            .merge(figment::providers::Json::file(auth))
            .extract()?;
        info!("Refreshing auth");

        let auth = api.refresh_auth(&auth).await?;

        info!("Fetching data");

        build_app_data(api, &auth).await?
    } else {
        Arc::new(AppData {
            api: RwLock::new(api),
            account_data: Default::default(),
            new_auth: Default::default(),
        })
    };

    let refresh_auth_task = tokio::spawn(refresh_auth(app_data.clone()));

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

    info!("Listening on {}", args.listen_addr);

    match tokio::try_join!(refresh_auth_task, serve_task) {
        Ok((auth_res, serve_res)) => {
            auth_res?;
            serve_res?;
            Ok(())
        }
        Err(e) => Err(anyhow!("task failed: {e}")),
    }
}

#[instrument(skip(state))]
async fn summary(
    Path(id): Path<Uuid>,
    State(state): State<Arc<AppData>>,
) -> Result<Json<Summary>, StatusCode> {
    if let Some(account_data) = state.account_data.read().await.get(&id) {
        info!("Returning cached summary");
        Ok(Json(account_data.summary.read().await.clone()))
    } else {
        error!("Failed to find account data");
        Err(StatusCode::NOT_FOUND)
    }
}

#[instrument(skip(state))]
async fn summary_single(State(state): State<Arc<AppData>>) -> Result<Json<Summary>, StatusCode> {
    if let Some(account_id) = state.account_data.read().await.keys().next() {
        summary(Path(*account_id), State(state.clone())).await
    } else {
        error!("Failed to find account data");
        Err(StatusCode::NOT_FOUND)
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoreQuery {
    character_id: Uuid,
    currency_type: dt_api::models::CurrencyType,
}

#[instrument(skip(state))]
async fn refresh_summary(
    account_id: &Uuid,
    state: Arc<AppData>,
) -> Result<Json<Summary>, StatusCode> {
    let api = state.api.read().await;
    let account_data = state.account_data.read().await;
    let new_summary = api
        .get_summary(
            &*state.account_data.read().await[account_id]
                .auth
                .read()
                .await,
        )
        .await;
    if let Ok(new_summary) = new_summary {
        let mut summary = account_data[account_id].summary.write().await;
        *summary = new_summary.clone();
        Ok(Json(new_summary))
    } else {
        error!(error = %new_summary.unwrap_err(), "Failed to get summary");
        Err(StatusCode::NOT_FOUND)
    }
}

#[instrument(skip(state))]
async fn refresh_store(
    account_id: &Uuid,
    character_id: Uuid,
    state: Arc<AppData>,
    currency_type: dt_api::models::CurrencyType,
) -> Result<Json<Store>, StatusCode> {
    let api = state.api.read().await;
    let account_data = state.account_data.read().await;
    let mut summary = account_data[account_id].summary.read().await;
    let character =
        if let Some(character) = summary.characters.iter().find(|c| c.id == character_id) {
            character
        } else {
            info!("Failed to find character in summary, fetching new summary");
            drop(summary);
            if refresh_summary(account_id, state.clone()).await.is_err() {
                error!("Failed to refresh summary");
                return Err(StatusCode::NOT_FOUND);
            } else {
                summary = account_data[account_id].summary.read().await;
                if let Some(character) = summary.characters.iter().find(|c| c.id == character_id) {
                    character
                } else {
                    error!(character.id = %character_id, "Failed to find character");
                    return Err(StatusCode::NOT_FOUND);
                }
            }
        };
    let store = api
        .get_store(
            &*state.account_data.read().await[account_id]
                .auth
                .read()
                .await,
            currency_type,
            &character,
        )
        .await;
    match store {
        Err(e) => {
            error!(
                character.id = %character_id,
                error = %e,
                "Failed to get store"
            );
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
        Ok(store) => {
            match currency_type {
                dt_api::models::CurrencyType::Marks => {
                    account_data[account_id]
                        .marks_store
                        .write()
                        .await
                        .insert(character_id, store.clone());
                }
                dt_api::models::CurrencyType::Credits => {
                    account_data[account_id]
                        .credits_store
                        .write()
                        .await
                        .insert(character_id, store.clone());
                }
            }
            info!("Successfully fetched store");
            Ok(Json(store))
        }
    }
}

#[instrument(skip(state))]
async fn store(
    Path(id): Path<Uuid>,
    Query(StoreQuery {
        character_id,
        currency_type,
    }): Query<StoreQuery>,
    State(state): State<Arc<AppData>>,
) -> Result<Json<Store>, StatusCode> {
    if let Some(account_data) = state.account_data.read().await.get(&id) {
        let currency_store = match currency_type {
            dt_api::models::CurrencyType::Marks => account_data.marks_store.read().await,
            dt_api::models::CurrencyType::Credits => account_data.credits_store.read().await,
        };
        let char_store = currency_store.get(&character_id);
        if let Some(store) = char_store {
            if store.current_rotation_end <= DateTime::<Utc>::from(SystemTime::now()) {
                drop(currency_store);
                info!("Store is out of date, refreshing");
                refresh_store(&id, character_id, state.clone(), currency_type).await
            } else {
                debug!("Store valid until {:?}", store.current_rotation_end);
                info!("Returning cached store");
                Ok(Json(store.clone()))
            }
        } else {
            drop(currency_store);
            info!("Trying to fetch store");
            refresh_store(&id, character_id, state.clone(), currency_type).await
        }
    } else {
        error!("Failed to find account data");
        Err(StatusCode::NOT_FOUND)
    }
}

#[instrument(skip(state))]
async fn store_single(
    query: Query<StoreQuery>,
    State(state): State<Arc<AppData>>,
) -> Result<Json<Store>, StatusCode> {
    if let Some(account_id) = state.account_data.read().await.keys().next() {
        store(Path(*account_id), query, State(state.clone())).await
    } else {
        error!("Failed to find account data");
        Err(StatusCode::NOT_FOUND)
    }
}

#[instrument(skip(state))]
async fn master_data(
    Path(id): Path<Uuid>,
    State(state): State<Arc<AppData>>,
) -> Result<Json<MasterData>, StatusCode> {
    if let Some(account_data) = state.account_data.read().await.get(&id) {
        info!("Returning cached master data");
        Ok(Json(account_data.master_data.read().await.clone()))
    } else {
        error!("Failed to find account data");
        Err(StatusCode::NOT_FOUND)
    }
}

#[instrument(skip(state))]
async fn master_data_single(
    State(state): State<Arc<AppData>>,
) -> Result<Json<MasterData>, StatusCode> {
    if let Some(account_id) = state.account_data.read().await.keys().next() {
        master_data(Path(*account_id), State(state.clone())).await
    } else {
        error!("Failed to find account data");
        Err(StatusCode::NOT_FOUND)
    }
}

#[instrument(skip(state))]
async fn put_auth(
    Path(id): Path<Uuid>,
    State(state): State<Arc<AppData>>,
    Json(auth): Json<dt_api::Auth>,
) -> StatusCode {
    let mut account_data = state.account_data.write().await;
    if let Some(account_data) = account_data.get(&id) {
        info!("Updating auth");
        *account_data.auth.write().await = auth;
        StatusCode::OK
    } else {
        let api = state.api.read().await;
        match build_account_data(&api, &auth).await {
            Ok(new_account_data) => {
                info!("Adding new account data");
                account_data.insert(id, new_account_data);
                state.new_auth.lock().await.push(id);
                StatusCode::CREATED
            }
            Err(e) => {
                error!("Failed to build account data: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
}

#[instrument(skip(state))]
async fn get_auth(Path(id): Path<Uuid>, State(state): State<Arc<AppData>>) -> StatusCode {
    if let Some(_) = state.account_data.read().await.get(&id) {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}
