use std::time::SystemTime;

use anyhow::Result;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use dt_api::models::Store;
use tracing::{debug, error, info, instrument};
use uuid::Uuid;

use crate::server::{refresh_summary, AppData};

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoreQuery {
    character_id: Uuid,
    currency_type: dt_api::models::CurrencyType,
}

#[instrument(skip(state))]
async fn refresh_store(
    account_id: &Uuid,
    character_id: Uuid,
    state: AppData,
    currency_type: dt_api::models::CurrencyType,
) -> Result<Json<Store>, StatusCode> {
    let api = &state.api;
    let account_data = if let Some(account_data) = state.accounts.get(account_id).await {
        account_data
    } else {
        error!(sid = ?account_id, "Failed to find account data");
        return Err(StatusCode::NOT_FOUND);
    };
    let mut summary = account_data.summary.read().await;
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
                summary = account_data.summary.read().await;
                if let Some(character) = summary.characters.iter().find(|c| c.id == character_id) {
                    character
                } else {
                    error!(character.id = %character_id, "Failed to find character");
                    return Err(StatusCode::NOT_FOUND);
                }
            }
        };
    let auth_data = if let Some(auth_data) = state.auth_data.get(account_id).await {
        auth_data
    } else {
        error!(sid = ?account_id, "Failed to find auth data");
        return Err(StatusCode::NOT_FOUND);
    };
    let store = api.get_store(&auth_data, currency_type, character).await;
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
                    account_data
                        .marks_store
                        .write()
                        .await
                        .insert(character_id, store.clone());
                }
                dt_api::models::CurrencyType::Credits => {
                    account_data
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
pub(crate) async fn store(
    Path(id): Path<Uuid>,
    Query(StoreQuery {
        character_id,
        currency_type,
    }): Query<StoreQuery>,
    State(state): State<AppData>,
) -> Result<Json<Store>, StatusCode> {
    if let Some(account_data) = state.accounts.get(&id).await {
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
pub(crate) async fn store_single(
    query: Query<StoreQuery>,
    State(state): State<AppData>,
) -> Result<Json<Store>, StatusCode> {
    let auth = state.auth_data.get_single().await;
    if let Some(auth) = auth {
        store(Path(auth.sub), query, State(state)).await
    } else {
        error!("Failed to find account data");
        Err(StatusCode::NOT_FOUND)
    }
}
