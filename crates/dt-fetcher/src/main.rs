use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result};
use axum::{
    body::Body,
    extract::{Query, State},
    http::{Request, Response, StatusCode},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use clap::Parser;
use dt_api::models::{MasterData, Store, Summary};
use figment::{providers::Format, Figment};
use futures::stream::{FuturesOrdered, StreamExt};
use tokio::sync::RwLock;
use tower_http::trace::TraceLayer;
use tracing::{error, metadata::LevelFilter, Span};
use tracing::{info, instrument};
use tracing_subscriber::{prelude::*, EnvFilter};

#[derive(Parser, Debug)]
struct Args {
    /// Path to auth json file
    #[arg(
        short,
        long,
        value_parser = clap::value_parser!(PathBuf),
        default_value = "auth.json"
    )]
    auth: PathBuf,
}

#[derive(Debug)]
struct AppData {
    api: RwLock<dt_api::Api>,
    summary: RwLock<dt_api::models::Summary>,
    marks_store: RwLock<HashMap<String, dt_api::models::Store>>,
    credits_store: RwLock<HashMap<String, dt_api::models::Store>>,
    master_data: RwLock<dt_api::models::MasterData>,
}

#[instrument]
async fn refresh_auth(app_data: Arc<AppData>, mut auth: dt_api::Auth) -> Result<()> {
    loop {
        let duration = if let Some(refresh_at) = auth.refresh_at {
            (refresh_at - DateTime::from(SystemTime::now())).to_std()?
        } else {
            auth.expires_in
                .checked_sub(Duration::from_secs(300))
                .unwrap_or_default()
        };

        if duration.as_secs() > 0 {
            info!("Refreshing auth in {:?}", duration);
            tokio::time::sleep(duration).await;
        }
        info!("Refreshing auth");
        auth = app_data.api.write().await.refresh_auth().await?;
        info!(
            "Auth refreshed, next refresh in {:?}",
            auth.expires_in.checked_sub(Duration::from_secs(300))
        );
    }
}

#[instrument]
async fn build_app_data(api: dt_api::Api) -> Result<Arc<AppData>> {
    let summary = api.get_summary().await?;

    let marks_store = summary
        .characters
        .iter()
        .map(|c| api.get_store(dt_api::models::CurrencyType::Marks, c))
        .collect::<FuturesOrdered<_>>()
        .collect::<Vec<_>>();

    let credits_store = summary
        .characters
        .iter()
        .map(|c| api.get_store(dt_api::models::CurrencyType::Credits, c))
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
                error!(character = ?c, "Failed to get marks store: {}", e);
                None
            }
        })
        .collect::<HashMap<String, Store>>();

    let credits_store = summary
        .characters
        .iter()
        .zip(credits_store.into_iter())
        .filter_map(|(c, s)| match s {
            Ok(s) => Some((c.id.clone(), s)),
            Err(e) => {
                error!(character = ?c, "Failed to get credits store: {}", e);
                None
            }
        })
        .collect::<HashMap<String, Store>>();

    let master_data = api.get_master_data().await?;

    Ok(Arc::new(AppData {
        api: RwLock::new(api),
        summary: RwLock::new(summary),
        marks_store: RwLock::new(marks_store),
        credits_store: RwLock::new(credits_store),
        master_data: RwLock::new(master_data),
    }))
}

fn init_logging() -> Result<()> {
    let registry = tracing_subscriber::registry();
    let layer = {
        #[cfg(target_os = "linux")]
        if libsystemd::daemon::booted() {
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
        tracing_subscriber::fmt::layer().pretty().with_target(true)
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
    init_logging().context("Failed to initialize logging")?;

    let auth: dt_api::Auth = Figment::new()
        .merge(figment::providers::Json::file(Args::parse().auth))
        .extract()?;

    let mut api = dt_api::Api::new(auth);

    let auth = api.refresh_auth().await?;

    let app_data = build_app_data(api).await?;
    let refresh_app_data = app_data.clone();

    tokio::spawn(async move { refresh_auth(refresh_app_data, auth) });

    let app = Router::new()
        .route("/store", get(store))
        .route("/summary", get(summary))
        .route("/master_data", get(master_data))
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
        );

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;

    axum::serve(listener, app).await?;

    Ok(())
}

#[instrument(skip(state))]
async fn summary(State(state): State<Arc<AppData>>) -> Json<Summary> {
    info!("Returning cached summary");
    Json(state.summary.read().await.clone())
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoreQuery {
    character_id: String,
    currency_type: dt_api::models::CurrencyType,
}

#[instrument(skip(state))]
async fn refresh_store(
    character_id: String,
    state: Arc<AppData>,
    currency_type: dt_api::models::CurrencyType,
) -> Result<Json<Store>, StatusCode> {
    let api = state.api.read().await;
    let summary = state.summary.read().await;
    let character = summary.characters.iter().find(|c| c.id == character_id);
    if let Some(character) = character {
        let store = api.get_store(currency_type, &character).await;
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
                        state
                            .marks_store
                            .write()
                            .await
                            .insert(character_id.clone(), store.clone());
                    }
                    dt_api::models::CurrencyType::Credits => {
                        state
                            .credits_store
                            .write()
                            .await
                            .insert(character_id.clone(), store.clone());
                    }
                }
                info!("Successfully fetched store");
                Ok(Json(store))
            }
        }
    } else {
        error!(character.id = %character_id, "Failed to find character");
        Err(StatusCode::NOT_FOUND)
    }
}

#[instrument(skip(state))]
async fn store(
    Query(StoreQuery {
        character_id,
        currency_type,
    }): Query<StoreQuery>,
    State(state): State<Arc<AppData>>,
) -> Result<Json<Store>, StatusCode> {
    let currency_store = match currency_type {
        dt_api::models::CurrencyType::Marks => {
            let marks_store = state.marks_store.read().await;
            marks_store
        }
        dt_api::models::CurrencyType::Credits => {
            let credits_store = state.credits_store.read().await;
            credits_store
        }
    };
    let char_store = currency_store.get(&character_id);
    if let Some(store) = char_store {
        if store.current_rotation_end <= DateTime::<Utc>::from(SystemTime::now()) {
            drop(currency_store);
            info!("Store is out of date, refreshing");
            return refresh_store(character_id, state, currency_type).await;
        }
        info!("Returning cached store");
        return Ok(Json(store.clone()));
    } else {
        drop(currency_store);
        info!("Trying to fetch store");
        return refresh_store(character_id, state, currency_type).await;
    }
}

#[instrument(skip(state))]
async fn master_data(State(state): State<Arc<AppData>>) -> Json<MasterData> {
    info!("Returning cached master data");
    Json(state.master_data.read().await.clone())
}
