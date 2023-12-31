use std::{net::SocketAddr, time::Duration};

use anyhow::Result;
use axum::{
    body::Body,
    extract::{FromRef, Path, State},
    http::{Request, Response, StatusCode},
    routing::{get, put},
    Json, Router,
};
use dt_api::models::{MasterData, Summary};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{error, Span};
use tracing::{info, instrument};
use uuid::Uuid;

use crate::auth::{get_auth, put_auth, AuthData};

mod store;
use store::{store, store_single};

#[derive(Debug, Clone)]
struct AppData {
    api: dt_api::Api,
    accounts: crate::Accounts,
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

impl FromRef<AppData> for crate::Accounts {
    fn from_ref(state: &AppData) -> Self {
        state.accounts.clone()
    }
}

pub struct Server {
    app: Router<()>,
    listen_addr: SocketAddr,
}

impl Server {
    pub fn new(
        api: dt_api::Api,
        accounts: crate::Accounts,
        auth_data: crate::AuthData,
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
async fn summary(
    Path(id): Path<Uuid>,
    State(state): State<AppData>,
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
