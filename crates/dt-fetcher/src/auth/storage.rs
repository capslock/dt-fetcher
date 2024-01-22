use std::{collections::HashMap, ops::Deref, sync::Arc};

use dt_api::{models::AccountId, Auth};
use futures_util::Future;
use tokio::sync::RwLock;
use tracing::instrument;

pub trait AuthStorage: Clone + Default + Send + Sync + 'static {
    fn get(
        &self,
        id: AccountId,
    ) -> impl Future<Output = Option<impl Deref<Target = Auth> + Send>> + Send;

    fn get_single(&self) -> impl Future<Output = Option<AccountId>> + Send;

    fn contains(&self, id: &AccountId) -> impl Future<Output = bool> + Send;

    fn insert(&self, id: AccountId, auth: Auth) -> impl Future<Output = ()> + Send;
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryAuthStorage {
    auths: Arc<RwLock<HashMap<AccountId, Auth>>>,
}

impl AuthStorage for InMemoryAuthStorage {
    #[instrument(skip(self))]
    async fn get<'a>(&'a self, id: AccountId) -> Option<impl Deref<Target = Auth>> {
        tokio::sync::RwLockReadGuard::try_map(self.auths.read().await, |auths| auths.get(&id)).ok()
    }

    #[instrument(skip(self))]
    async fn get_single(&self) -> Option<AccountId> {
        self.auths.read().await.keys().next().copied()
    }

    #[instrument(skip(self))]
    async fn contains(&self, id: &AccountId) -> bool {
        self.auths.read().await.contains_key(id)
    }

    #[instrument(skip(self))]
    async fn insert(&self, id: AccountId, auth: Auth) {
        self.auths.write().await.insert(id, auth);
    }
}
