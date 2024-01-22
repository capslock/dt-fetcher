use std::{collections::HashMap, ops::Deref, path::Path, sync::Arc};

use anyhow::{Context, Result};
use futures_util::Future;
use tokio::sync::RwLock;
use tracing::instrument;

use dt_api::{models::AccountId, Auth};

pub trait AuthStorage: Clone + Send + Sync + 'static {
    type T<'a>: Deref<Target = Auth> + Send;

    fn get(&'_ self, id: AccountId) -> impl Future<Output = Result<Option<Self::T<'_>>>> + Send;

    fn get_single(&self) -> impl Future<Output = Result<Option<AccountId>>> + Send;

    fn contains(&self, id: &AccountId) -> impl Future<Output = Result<bool>> + Send;

    fn insert(&self, id: AccountId, auth: Auth) -> impl Future<Output = Result<()>> + Send;
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryAuthStorage {
    auths: Arc<RwLock<HashMap<AccountId, Auth>>>,
}

impl AuthStorage for InMemoryAuthStorage {
    type T<'a> = tokio::sync::RwLockReadGuard<'a, Auth>;

    #[instrument(skip(self))]
    async fn get<'a>(&'a self, id: AccountId) -> Result<Option<Self::T<'a>>> {
        Ok(
            tokio::sync::RwLockReadGuard::try_map(self.auths.read().await, |auths| auths.get(&id))
                .ok(),
        )
    }

    #[instrument(skip(self))]
    async fn get_single(&self) -> Result<Option<AccountId>> {
        Ok(self.auths.read().await.keys().next().copied())
    }

    #[instrument(skip(self))]
    async fn contains(&self, id: &AccountId) -> Result<bool> {
        Ok(self.auths.read().await.contains_key(id))
    }

    #[instrument(skip(self))]
    async fn insert(&self, id: AccountId, auth: Auth) -> Result<()> {
        self.auths.write().await.insert(id, auth);
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct SledDbAuthStorage {
    db: sled::Db,
}

impl SledDbAuthStorage {
    pub fn new<P: AsRef<Path>>(db: P) -> Result<Self> {
        Ok(Self {
            db: sled::open(db).context("Failed to open db")?,
        })
    }
}

impl AuthStorage for SledDbAuthStorage {
    type T<'a> = Arc<Auth>;

    async fn get(&'_ self, id: AccountId) -> Result<Option<Self::T<'_>>> {
        let result = self.db.get(id.0.as_bytes()).context("Failed to get auth")?;
        result
            .map(|auth| {
                serde_json::from_slice::<Auth>(&auth)
                    .context("Failed to deserialize auth")
                    .map(Arc::new)
            })
            .transpose()
    }

    async fn get_single(&self) -> Result<Option<AccountId>> {
        let result = self.db.first().context("Failed to get auth")?;
        result
            .map(|(id, _)| {
                uuid::Uuid::from_slice(&id)
                    .context("Failed to deserialize uuid")
                    .map(AccountId)
            })
            .transpose()
    }

    async fn contains(&self, id: &AccountId) -> Result<bool> {
        self.db
            .contains_key(id.0.as_bytes())
            .context("Failed to get auth")
    }

    async fn insert(&self, id: AccountId, auth: Auth) -> Result<()> {
        self.db
            .insert(
                id.0.as_bytes(),
                serde_json::to_vec(&auth).context("Failed to serialize auth")?,
            )
            .context("Failed to insert")?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum ErasedAuthStorage {
    InMemoryAuthStorage(InMemoryAuthStorage),
    SledDbAuthStorage(SledDbAuthStorage),
}

impl From<InMemoryAuthStorage> for ErasedAuthStorage {
    fn from(value: InMemoryAuthStorage) -> Self {
        ErasedAuthStorage::InMemoryAuthStorage(value)
    }
}

impl From<SledDbAuthStorage> for ErasedAuthStorage {
    fn from(value: SledDbAuthStorage) -> Self {
        ErasedAuthStorage::SledDbAuthStorage(value)
    }
}

pub struct ErasedAuth<'a>(Box<dyn Deref<Target = Auth> + 'a + Send>);

impl<'a> Deref for ErasedAuth<'a> {
    type Target = Auth;

    fn deref(self: &ErasedAuth<'a>) -> &Self::Target {
        &self.0
    }
}

impl AuthStorage for ErasedAuthStorage {
    type T<'a> = ErasedAuth<'a>;

    async fn get(&'_ self, id: AccountId) -> Result<Option<Self::T<'_>>> {
        match self {
            ErasedAuthStorage::InMemoryAuthStorage(s) => s
                .get(id)
                .await
                .map(|a| a.map(|a| ErasedAuth::<'_>(Box::new(a)))),
            ErasedAuthStorage::SledDbAuthStorage(s) => {
                s.get(id).await.map(|a| a.map(|a| ErasedAuth(Box::new(a))))
            }
        }
    }

    async fn get_single(&self) -> Result<Option<AccountId>> {
        match self {
            ErasedAuthStorage::InMemoryAuthStorage(s) => s.get_single().await,
            ErasedAuthStorage::SledDbAuthStorage(s) => s.get_single().await,
        }
    }

    async fn contains(&self, id: &AccountId) -> Result<bool> {
        match self {
            ErasedAuthStorage::InMemoryAuthStorage(s) => s.contains(id).await,
            ErasedAuthStorage::SledDbAuthStorage(s) => s.contains(id).await,
        }
    }

    async fn insert(&self, id: AccountId, auth: Auth) -> Result<()> {
        match self {
            ErasedAuthStorage::InMemoryAuthStorage(s) => s.insert(id, auth).await,
            ErasedAuthStorage::SledDbAuthStorage(s) => s.insert(id, auth).await,
        }
    }
}
