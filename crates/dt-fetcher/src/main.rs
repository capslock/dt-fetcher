use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use clap::Parser;
use dt_api::models::{MasterData, Store, Summary};
use figment::{providers::Format, Figment};
use futures::stream::{FuturesOrdered, StreamExt};
use tokio::{sync::RwLock, time::Instant};

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

struct AppData {
    api: RwLock<dt_api::Api>,
    summary: RwLock<dt_api::models::Summary>,
    marks_store: RwLock<HashMap<String, dt_api::models::Store>>,
    credits_store: RwLock<HashMap<String, dt_api::models::Store>>,
    master_data: RwLock<dt_api::models::MasterData>,
}

fn time_until(time: u64) -> Duration {
    let expiration = UNIX_EPOCH + Duration::from_millis(time);
    expiration
        .duration_since(SystemTime::now())
        .unwrap_or_default()
}

async fn refresh_auth(app_data: Arc<AppData>, mut auth: dt_api::Auth) -> Result<()> {
    loop {
        let duration = if let Some(refresh_at) = auth.refresh_at {
            time_until(refresh_at)
        } else {
            Duration::from_secs(auth.expires_in.into()).saturating_sub(Duration::from_secs(300))
        };

        tokio::time::sleep_until(Instant::now() + duration).await;
        auth = app_data.api.write().await.refresh_auth().await?;
    }
}

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
        .map(|c| api.get_store(dt_api::models::CurrencyType::Marks, c))
        .collect::<FuturesOrdered<_>>()
        .collect::<Vec<_>>();

    let (marks_store, credits_store) = tokio::join!(marks_store, credits_store);

    let marks_store = summary
        .characters
        .iter()
        .zip(marks_store.into_iter())
        .filter_map(|(c, s)| s.ok().map(|s| (c.id.clone(), s)))
        .collect::<HashMap<String, Store>>();

    let credits_store = summary
        .characters
        .iter()
        .zip(credits_store.into_iter())
        .filter_map(|(c, s)| s.ok().map(|s| (c.id.clone(), s)))
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

#[tokio::main]
async fn main() -> Result<()> {
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
        .with_state(app_data);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;

    axum::serve(listener, app).await?;

    Ok(())
}

async fn summary(State(state): State<Arc<AppData>>) -> Json<Summary> {
    Json(state.summary.read().await.clone())
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoreQuery {
    character_id: String,
    currency_type: dt_api::models::CurrencyType,
}

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
        if let Err(e) = store {
            println!("Failed to get store {}: {}", character_id, e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        } else {
            let store = store.unwrap();
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
            return Ok(Json(store));
        }
    } else {
        println!("Failed to find character {}", character_id);
        return Err(StatusCode::NOT_FOUND);
    }
}

async fn store(
    Query(StoreQuery {
        character_id,
        currency_type,
    }): Query<StoreQuery>,
    State(state): State<Arc<AppData>>,
) -> Result<Json<Store>, StatusCode> {
    match currency_type {
        dt_api::models::CurrencyType::Marks => {
            let marks_store = state.marks_store.read().await;
            let store = marks_store.get(&character_id);
            if let Some(store) = store {
                if store
                    .current_rotation_end
                    .parse::<u64>()
                    .map(time_until)
                    .unwrap_or_default()
                    == Duration::from_secs(0)
                {
                    return refresh_store(
                        character_id,
                        state.clone(),
                        dt_api::models::CurrencyType::Marks,
                    )
                    .await;
                }
                return Ok(Json(store.clone()));
            } else {
                return refresh_store(
                    character_id,
                    state.clone(),
                    dt_api::models::CurrencyType::Marks,
                )
                .await;
            }
        }
        dt_api::models::CurrencyType::Credits => {
            let credits_store = state.credits_store.read().await;
            let store = credits_store.get(&character_id);
            if let Some(store) = store {
                if store
                    .current_rotation_end
                    .parse::<u64>()
                    .map(time_until)
                    .unwrap_or_default()
                    == Duration::from_secs(0)
                {
                    return refresh_store(
                        character_id,
                        state.clone(),
                        dt_api::models::CurrencyType::Credits,
                    )
                    .await;
                }
                return Ok(Json(store.clone()));
            } else {
                return refresh_store(
                    character_id,
                    state.clone(),
                    dt_api::models::CurrencyType::Credits,
                )
                .await;
            }
        }
    }
}

async fn master_data(State(state): State<Arc<AppData>>) -> Json<MasterData> {
    Json(state.master_data.read().await.clone())
}
