use std::{
    collections::BinaryHeap,
    sync::Arc,
    time::{Duration, SystemTime},
};

use anyhow::{Context as _, Result};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use futures::{stream::FuturesUnordered, StreamExt as _};
use tracing::{error, info, instrument, warn};
use uuid::Uuid;

use crate::{build_account_data, AppData};

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
pub(crate) async fn refresh_auth(app_data: Arc<AppData>) -> Result<()> {
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

#[instrument(skip(state))]
pub(crate) async fn put_auth(
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
pub(crate) async fn get_auth(
    Path(id): Path<Uuid>,
    State(state): State<Arc<AppData>>,
) -> StatusCode {
    if let Some(_) = state.account_data.read().await.get(&id) {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}
