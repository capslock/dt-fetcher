use std::{
    collections::{BinaryHeap, HashMap},
    sync::Arc,
    time::{Duration, SystemTime},
};

use anyhow::{anyhow, Context as _, Result};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use dt_api::Auth;
use futures_util::future::{self, Either};
use tokio::sync::{
    mpsc::{channel, Receiver, Sender},
    RwLock,
};
use tracing::{error, info, instrument, warn};
use uuid::Uuid;

use crate::{AccountData, Accounts};

const REFRESH_BUFFER: Duration = Duration::from_secs(300);

#[derive(PartialEq, Eq)]
struct RefreshAuth {
    id: Uuid,
    refresh_at: DateTime<Utc>,
}

impl RefreshAuth {
    fn new(auth: &Auth) -> Self {
        Self {
            id: auth.sub,
            refresh_at: auth.refresh_at.unwrap_or(DateTime::from(SystemTime::now())),
        }
    }
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

#[derive(Debug)]
pub(crate) enum AuthCommand {
    NewAuth(Auth),
    Shutdown,
}

#[derive(Debug, Clone)]
pub(crate) struct AuthData {
    auths: Arc<RwLock<HashMap<Uuid, Auth>>>,
    tx: Sender<AuthCommand>,
}

#[derive(Debug)]
pub(crate) struct AuthManager {
    api: dt_api::Api,
    auth_data: AuthData,
    accounts: Accounts,
    rx: Receiver<AuthCommand>,
}

impl AuthManager {
    #[instrument(skip_all)]
    pub fn new(api: dt_api::Api, accounts: Accounts) -> Self {
        let (tx, rx) = channel(100);
        AuthManager {
            auth_data: AuthData {
                auths: Arc::new(RwLock::new(HashMap::new())),
                tx,
            },
            rx,
            api,
            accounts,
        }
    }

    #[instrument(skip_all)]
    pub fn auth_data(&self) -> AuthData {
        self.auth_data.clone()
    }

    #[instrument(skip_all)]
    pub async fn start(mut self) -> Result<()> {
        let mut auths: BinaryHeap<RefreshAuth> = BinaryHeap::new();
        loop {
            let sleep = if let Some(refresh_auth) = auths.peek() {
                let duration = (refresh_auth.refresh_at - DateTime::from(SystemTime::now()))
                    .max(chrono::Duration::zero())
                    .to_std()
                    .expect("Duration was less than 0");
                info!(
                    duration = ?duration,
                    refresh_at = ?refresh_auth.refresh_at,
                    "Sleeping until next auth refresh");
                Either::Left(tokio::time::sleep(
                    (refresh_auth.refresh_at - DateTime::from(SystemTime::now()))
                        .max(chrono::Duration::zero())
                        .to_std()
                        .expect("Duration was less than 0"),
                ))
            } else {
                info!("No auths, sleeping.");
                Either::Right(future::pending())
            };
            tokio::select! {
                command = self.rx.recv() => match command {
                    Some(AuthCommand::NewAuth(auth)) => {
                        info!(auth = ?auth, "Adding new auth");
                        auths.push(RefreshAuth::new(&auth));
                        if let Ok(account) = AccountData::fetch(&self.api, &auth).await {
                            info!(sub = ?auth.sub, "Adding new account data");
                            self.accounts.insert(auth.sub.clone(), account).await;
                        } else {
                            error!(auth = ?auth, "Failed to fetch account data");
                        }
                        self.auth_data.auths.write().await.insert(auth.sub, auth);
                    }
                    Some(AuthCommand::Shutdown) => {
                        info!("Shutting down auth manager");
                        return Ok(())
                    }
                    None => {
                        warn!("Auth manager channel closed");
                        return Err(anyhow!("Auth manager channel closed"));
                    }
                },
                _ = sleep => {
                    if let Err(e) = self.refresh_auth(&mut auths).await {
                        error!(error = %e, "Failed to refresh auth");
                    }
                }
            }
        }
    }

    #[instrument(skip_all)]
    async fn refresh_auth(&mut self, auths: &mut BinaryHeap<RefreshAuth>) -> Result<()> {
        if let Some(mut refresh_auth) = auths.pop() {
            if let Some(auth) = self.auth_data.auths.write().await.get_mut(&refresh_auth.id) {
                info!(sub = ?refresh_auth.id, "Refreshing auth");
                *auth = self
                    .api
                    .refresh_auth(&*auth)
                    .await
                    .context("failed to refresh auth")?;
                (*auth).refresh_at = Some(
                    DateTime::from(SystemTime::now())
                        + auth.expires_in.saturating_sub(REFRESH_BUFFER),
                );
                refresh_auth.refresh_at = auth.refresh_at.expect("auth refresh_at is None");
                auths.push(refresh_auth);
                info!(auth = ?*auth, "Auth refreshed");
            } else {
                info!(sub = ?refresh_auth.id, "Auth not found, removing");
            }
        }
        Ok(())
    }
}

impl AuthData {
    #[instrument(skip(self))]
    pub async fn add_auth(&self, auth: Auth) -> Result<()> {
        Ok(self
            .tx
            .send(AuthCommand::NewAuth(auth))
            .await
            .context("Failed to send auth")?)
    }

    #[instrument(skip(self))]
    pub async fn get(&self, id: &Uuid) -> Option<Auth> {
        self.auths.read().await.get(id).cloned()
    }

    pub async fn get_single(&self) -> Option<Auth> {
        self.auths.read().await.values().next().cloned()
    }

    #[instrument(skip(self))]
    pub async fn shutdown(&self) -> Result<()> {
        Ok(self
            .tx
            .send(AuthCommand::Shutdown)
            .await
            .context("Failed to send shutdown")?)
    }
}

#[instrument(skip(state))]
pub(crate) async fn put_auth(
    Path(id): Path<Uuid>,
    State(state): State<AuthData>,
    Json(auth): Json<dt_api::Auth>,
) -> StatusCode {
    if let Some(_) = state.auths.read().await.get(&id) {
        return StatusCode::OK;
    } else if let Err(e) = state.add_auth(auth).await {
        error!("Failed to add auth: {}", e);
        return StatusCode::INTERNAL_SERVER_ERROR;
    }
    return StatusCode::CREATED;
}

#[instrument(skip(state))]
pub(crate) async fn get_auth(Path(id): Path<Uuid>, State(state): State<AuthData>) -> StatusCode {
    if let Some(_) = state.auths.read().await.get(&id) {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}
