use std::{
    collections::BinaryHeap,
    time::{Duration, SystemTime},
};

use anyhow::{anyhow, bail, Context as _, Result};
use chrono::{DateTime, Utc};
use dt_api::{models::AccountId, Auth};
use futures_util::future::{self, Either};
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, instrument, warn};

use crate::account::{AccountData, Accounts};

use super::AuthStorage;

const REFRESH_BUFFER: Duration = Duration::from_secs(300);

#[derive(PartialEq, Eq)]
struct RefreshAuth {
    id: AccountId,
    refresh_at: DateTime<Utc>,
}

impl RefreshAuth {
    fn new(auth: &Auth) -> Self {
        Self {
            id: auth.sub,
            refresh_at: auth.refresh_at.unwrap_or(
                DateTime::from(SystemTime::now()) + auth.expires_in.saturating_sub(REFRESH_BUFFER),
            ),
        }
    }
}

impl PartialOrd for RefreshAuth {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
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
}

#[derive(Debug)]
pub(crate) struct AuthManager<T: AuthStorage + Clone> {
    api: dt_api::Api,
    auth_data: AuthData<T>,
    accounts: Accounts,
    rx: Receiver<AuthCommand>,
}

impl<T: AuthStorage + Default + Clone> AuthManager<T> {
    #[instrument(skip_all)]
    pub fn new(api: dt_api::Api, accounts: Accounts) -> Self {
        let (tx, rx) = channel(100);
        AuthManager {
            auth_data: AuthData {
                auths: Default::default(),
                tx,
            },
            rx,
            api,
            accounts,
        }
    }
}

impl<T: AuthStorage + Clone> AuthManager<T> {
    #[instrument(skip_all)]
    pub fn new_with_storage(api: dt_api::Api, accounts: Accounts, storage: T) -> Self {
        let (tx, rx) = channel(100);
        AuthManager {
            auth_data: AuthData { auths: storage, tx },
            rx,
            api,
            accounts,
        }
    }

    #[instrument(skip_all)]
    pub fn auth_data(&self) -> AuthData<T> {
        self.auth_data.clone()
    }

    #[instrument(skip(self, auths))]
    async fn insert_new_auth(
        &mut self,
        auths: &mut BinaryHeap<RefreshAuth>,
        auth: Auth,
    ) -> Result<()> {
        info!(auth = ?auth, "Adding new auth");
        if self.auth_data.contains(&auth.sub)? {
            error!(auth = ?auth, "Auth already exists");
            bail!("Auth already exists");
        }
        Self::insert_new_refresh_auth(auths, &auth).await;
        Self::populate_account_data(&self.api, &mut self.accounts, &auth).await?;
        if let Err(e) = self.auth_data.insert(auth.sub, auth).await {
            error!(error = %e, "Failed to insert auth");
            Err(e).context("Failed to insert auth")?;
        }

        Ok(())
    }

    async fn insert_new_refresh_auth(auths: &mut BinaryHeap<RefreshAuth>, auth: &Auth) {
        auths.push(RefreshAuth::new(auth));
    }

    #[instrument(skip(api, accounts))]
    async fn populate_account_data(
        api: &dt_api::Api,
        accounts: &mut Accounts,
        auth: &Auth,
    ) -> Result<()> {
        if let Ok(account) = AccountData::fetch(api, auth).await {
            info!(sub = ?auth.sub, "Adding new account data");
            accounts.insert(auth.sub, account).await;
        } else {
            error!(auth = ?auth, "Failed to fetch account data");
            bail!("Failed to fetch account data");
        }
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn start(mut self, token: CancellationToken) -> Result<()> {
        let mut auths: BinaryHeap<RefreshAuth> = BinaryHeap::new();
        for auth in self.auth_data.auths.iter() {
            match auth {
                Ok((_, auth)) => {
                    if auth.expired(REFRESH_BUFFER) {
                        warn!(sub = ?auth.sub, "Auth expired, removing");
                        self.auth_data.auths.remove(&auth.sub)?;
                    } else {
                        info!(sub = ?auth.sub, "Adding auth");
                        Self::insert_new_refresh_auth(&mut auths, &auth).await;
                        Self::populate_account_data(&self.api, &mut self.accounts, &auth).await?;
                    }
                }
                Err(e) => {
                    error!(error = %e, "Failed to get auth");
                }
            }
        }
        let mut shutdown = false;
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
                info!("No auths, sleeping");
                Either::Right(future::pending())
            };
            tokio::select! {
                command = self.rx.recv() => match command {
                    Some(AuthCommand::NewAuth(auth)) => self.insert_new_auth(&mut auths, auth).await?,
                    None => {
                        if shutdown {
                            info!("Auth manager channel closed");
                            return Ok(());
                        }
                        warn!("Auth manager channel closed");
                        return Err(anyhow!("Auth manager channel closed"));
                    }
                },
                _ = token.cancelled() => {
                    info!("Shutting down auth manager");
                    shutdown = true;
                    self.rx.close();
                }
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
        if let Some(refresh_auth) = auths.pop() {
            if let Some(auth) = self.auth_data.get(refresh_auth.id)? {
                info!(sub = ?refresh_auth.id, "Refreshing auth");
                let mut auth = self
                    .api
                    .refresh_auth(&auth)
                    .await
                    .context("failed to refresh auth")?;
                let refresh_auth = RefreshAuth::new(&auth);
                auth.refresh_at = Some(refresh_auth.refresh_at);
                info!(auth = ?auth, "Auth refreshed");
                if let Err(e) = self.auth_data.insert(refresh_auth.id, auth).await {
                    error!(error = %e, "Failed to insert auth, removing");
                    self.auth_data.auths.remove(&refresh_auth.id)?;
                    return Err(e);
                }
                auths.push(refresh_auth);
            } else {
                warn!(sub = ?refresh_auth.id, "Auth not found, removing");
                self.auth_data.auths.remove(&refresh_auth.id)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AuthData<A: AuthStorage> {
    auths: A,
    tx: Sender<AuthCommand>,
}

impl<T: AuthStorage> AuthData<T> {
    #[instrument(skip(self))]
    pub async fn add_auth(&self, auth: Auth) -> Result<()> {
        self.tx
            .send(AuthCommand::NewAuth(auth))
            .await
            .context("Failed to send auth")
    }

    #[instrument(skip(self))]
    pub fn get(&self, id: AccountId) -> Result<Option<Auth>> {
        self.auths.get(id)
    }

    #[instrument(skip(self))]
    pub fn get_single(&self) -> Result<Option<AccountId>> {
        self.auths.get_single()
    }

    #[instrument(skip(self))]
    pub fn contains(&self, id: &AccountId) -> Result<bool> {
        self.auths.contains(id)
    }

    #[instrument(skip(self))]
    async fn insert(&mut self, id: AccountId, auth: Auth) -> Result<()> {
        self.auths.insert(id, auth)
    }
}
