use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use dt_api::models::AccountId;
use tracing::{error, instrument};

use super::{AuthData, AuthStorage};

#[instrument(skip(state))]
pub(crate) async fn put_auth<T: AuthStorage>(
    Path(id): Path<AccountId>,
    State(state): State<AuthData<T>>,
    Json(auth): Json<dt_api::Auth>,
) -> StatusCode {
    let result = state.contains(&id);
    if let Ok(true) = result {
        return StatusCode::OK;
    } else if let Err(e) = result {
        error!("Failed to check if auth exists: {}", e);
        return StatusCode::INTERNAL_SERVER_ERROR;
    } else if let Err(e) = state.add_auth(auth).await {
        error!("Failed to add auth: {}", e);
        return StatusCode::INTERNAL_SERVER_ERROR;
    }
    return StatusCode::CREATED;
}

#[instrument(skip(state))]
pub(crate) async fn get_auth<T: AuthStorage>(
    Path(id): Path<AccountId>,
    State(state): State<AuthData<T>>,
) -> StatusCode {
    let result = state.contains(&id);
    if let Ok(true) = result {
        StatusCode::OK
    } else if let Err(e) = result {
        error!("Failed to check if auth exists: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        error!("Auth not found");
        StatusCode::NOT_FOUND
    }
}
