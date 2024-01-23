use std::{net::SocketAddr, time::Duration};

use anyhow::Result;
use axum::{
    body::Body,
    extract::{FromRef, Path, State},
    http::{Request, Response, StatusCode},
    routing::{get, put},
    Json, Router,
};
use dt_api::models::{AccountId, MasterData, Summary};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{error, Span};
use tracing::{info, instrument};

use crate::auth::{get_auth, put_auth, AuthData, AuthStorage};

mod store;
use store::{store, store_single};

#[derive(Debug, Clone)]
struct AppData<T: AuthStorage> {
    api: dt_api::Api,
    accounts: crate::account::Accounts,
    auth_data: AuthData<T>,
}

impl<T: AuthStorage> FromRef<AppData<T>> for AuthData<T> {
    fn from_ref(state: &AppData<T>) -> Self {
        state.auth_data.clone()
    }
}

impl<T: AuthStorage> FromRef<AppData<T>> for dt_api::Api {
    fn from_ref(state: &AppData<T>) -> Self {
        state.api.clone()
    }
}

impl<T: AuthStorage> FromRef<AppData<T>> for crate::account::Accounts {
    fn from_ref(state: &AppData<T>) -> Self {
        state.accounts.clone()
    }
}

pub struct Server {
    app: Router<()>,
    listen_addr: SocketAddr,
}

impl Server {
    pub fn new<T: AuthStorage>(
        api: dt_api::Api,
        accounts: crate::account::Accounts,
        auth_data: crate::AuthData<T>,
        listen_addr: SocketAddr,
    ) -> Self {
        let app_data = AppData {
            api,
            accounts,
            auth_data,
        };

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

        Self { app, listen_addr }
    }

    #[instrument(skip_all)]
    pub async fn start(self) -> Result<()> {
        let listener = tokio::net::TcpListener::bind(self.listen_addr).await?;

        axum::serve(listener, self.app).await?;

        Ok(())
    }
}

const SUMMARY_REFRESH_INTERVAL_MINS: i64 = 60;

#[instrument(skip(state))]
async fn summary<T: AuthStorage>(
    Path(id): Path<AccountId>,
    State(state): State<AppData<T>>,
) -> Result<Json<Summary>, StatusCode> {
    if let Some(account_data) = state.accounts.get(&id).await {
        if account_data.last_updated
            < chrono::Utc::now() - chrono::Duration::minutes(SUMMARY_REFRESH_INTERVAL_MINS)
        {
            info!("Summary out of date; refreshing");
            refresh_summary(&id, state).await
        } else {
            info!("Returning cached summary");
            Ok(Json(account_data.summary.read().await.clone()))
        }
    } else {
        info!("Account data not found, attempting to refresh");
        refresh_summary(&id, state).await
    }
}

#[instrument(skip(state))]
async fn summary_single<T: AuthStorage>(
    State(state): State<AppData<T>>,
) -> Result<Json<Summary>, StatusCode> {
    let account = state
        .auth_data
        .get_single()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if let Some(account) = account {
        summary(Path(account), State(state)).await
    } else {
        error!("Failed to find account data");
        Err(StatusCode::NOT_FOUND)
    }
}

#[instrument(skip(state))]
async fn refresh_summary<T: AuthStorage>(
    account_id: &AccountId,
    state: AppData<T>,
) -> Result<Json<Summary>, StatusCode> {
    let api = &state.api;
    let account_data = if let Some(account_data) = state.accounts.get(account_id).await {
        account_data
    } else {
        error!(sid = ?account_id, "Failed to find account data");
        return Err(StatusCode::NOT_FOUND);
    };
    if let Some(auth_data) = state
        .auth_data
        .get(*account_id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        let new_summary = api.get_summary(&auth_data).await;
        if let Ok(new_summary) = new_summary {
            let mut summary = account_data.summary.write().await;
            *summary = new_summary.clone();
            state.accounts.update_timestamp(account_id).await;
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
async fn master_data<T: AuthStorage>(
    Path(id): Path<AccountId>,
    State(state): State<AppData<T>>,
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
async fn master_data_single<T: AuthStorage>(
    State(state): State<AppData<T>>,
) -> Result<Json<MasterData>, StatusCode> {
    let account = state
        .auth_data
        .get_single()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if let Some(account) = account {
        master_data(Path(account), State(state)).await
    } else {
        error!("Failed to find account data");
        Err(StatusCode::NOT_FOUND)
    }
}
