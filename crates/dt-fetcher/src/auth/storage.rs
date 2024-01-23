use std::path::Path;

use anyhow::{Context, Result};
use dyn_clone::DynClone;
use im::HashMap;
use tracing::instrument;

use dt_api::{models::AccountId, Auth};

pub trait AuthStorage: Send + Sync + DynClone + 'static {
    fn get(&self, id: AccountId) -> Result<Option<Auth>>;

    fn get_single(&self) -> Result<Option<AccountId>>;

    fn contains(&self, id: &AccountId) -> Result<bool>;

    fn insert(&mut self, id: AccountId, auth: Auth) -> Result<()>;

    fn iter(&self) -> ErasedAuthStorageIter;
}

dyn_clone::clone_trait_object!(AuthStorage);

#[derive(Debug, Clone, Default)]
pub struct InMemoryAuthStorage {
    auths: HashMap<AccountId, Auth>,
}

pub struct InMemoryAuthStorageIter {
    inner: im::hashmap::ConsumingIter<(AccountId, Auth)>,
}

impl InMemoryAuthStorageIter {
    fn new(auths: &HashMap<AccountId, Auth>) -> Self {
        Self {
            inner: auths.clone().into_iter(),
        }
    }
}

impl Iterator for InMemoryAuthStorageIter {
    type Item = Result<(AccountId, Auth)>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(id, auth)| Ok((id, auth)))
    }
}

impl AuthStorage for InMemoryAuthStorage {
    #[instrument(skip(self))]
    fn get(&self, id: AccountId) -> Result<Option<Auth>> {
        Ok(self.auths.get(&id).cloned())
    }

    #[instrument(skip(self))]
    fn get_single(&self) -> Result<Option<AccountId>> {
        Ok(self.auths.keys().next().copied())
    }

    #[instrument(skip(self))]
    fn contains(&self, id: &AccountId) -> Result<bool> {
        Ok(self.auths.contains_key(id))
    }

    #[instrument(skip(self))]
    fn insert(&mut self, id: AccountId, auth: Auth) -> Result<()> {
        self.auths.insert(id, auth);
        Ok(())
    }

    fn iter(&self) -> ErasedAuthStorageIter {
        InMemoryAuthStorageIter::new(&self.auths).into()
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

pub struct SledDbAuthStorageIter {
    inner: sled::Iter,
}

impl SledDbAuthStorageIter {
    fn new(db: &sled::Db) -> Self {
        Self { inner: db.iter() }
    }
}

impl Iterator for SledDbAuthStorageIter {
    type Item = Result<(AccountId, Auth)>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|result| {
            let (id, auth) = result.expect("Failed to get key/value pair");
            Ok((
                AccountId(uuid::Uuid::from_slice(&id).context("Failed to deserialize uuid")?),
                postcard::from_bytes(&auth).context("Failed to deserialize auth")?,
            ))
        })
    }
}

impl AuthStorage for SledDbAuthStorage {
    fn get(&self, id: AccountId) -> Result<Option<Auth>> {
        let result = self.db.get(id.0.as_bytes()).context("Failed to get auth")?;
        result
            .map(|auth| postcard::from_bytes::<Auth>(&auth).context("Failed to deserialize auth"))
            .transpose()
    }

    fn get_single(&self) -> Result<Option<AccountId>> {
        let result = self.db.first().context("Failed to get auth")?;
        result
            .map(|(id, _)| {
                uuid::Uuid::from_slice(&id)
                    .context("Failed to deserialize uuid")
                    .map(AccountId)
            })
            .transpose()
    }

    fn contains(&self, id: &AccountId) -> Result<bool> {
        self.db
            .contains_key(id.0.as_bytes())
            .context("Failed to get auth")
    }

    fn insert(&mut self, id: AccountId, auth: Auth) -> Result<()> {
        self.db
            .insert(
                id.0.as_bytes(),
                postcard::to_vec::<Auth, 1024>(&auth)
                    .context("Failed to serialize auth")?
                    .as_slice(),
            )
            .context("Failed to insert")?;
        self.db.flush().context("Failed to flush")?;
        Ok(())
    }

    fn iter(&self) -> ErasedAuthStorageIter {
        SledDbAuthStorageIter::new(&self.db).into()
    }
}

type ErasedAuthStorageIter = Box<dyn Iterator<Item = Result<(AccountId, Auth)>> + Send>;

impl From<InMemoryAuthStorageIter> for ErasedAuthStorageIter {
    fn from(value: InMemoryAuthStorageIter) -> Self {
        Box::new(value)
    }
}

impl From<SledDbAuthStorageIter> for ErasedAuthStorageIter {
    fn from(value: SledDbAuthStorageIter) -> Self {
        Box::new(value)
    }
}

#[derive(Clone)]
pub struct ErasedAuthStorage(Box<dyn AuthStorage>);

impl AuthStorage for ErasedAuthStorage {
    fn get(&self, id: AccountId) -> Result<Option<Auth>> {
        self.0.get(id)
    }

    fn get_single(&self) -> Result<Option<AccountId>> {
        self.0.get_single()
    }

    fn contains(&self, id: &AccountId) -> Result<bool> {
        self.0.contains(id)
    }

    fn insert(&mut self, id: AccountId, auth: Auth) -> Result<()> {
        self.0.insert(id, auth)
    }

    fn iter(&self) -> ErasedAuthStorageIter {
        Box::new(self.0.iter())
    }
}

impl From<InMemoryAuthStorage> for ErasedAuthStorage {
    fn from(value: InMemoryAuthStorage) -> Self {
        Self(Box::new(value))
    }
}

impl From<SledDbAuthStorage> for ErasedAuthStorage {
    fn from(value: SledDbAuthStorage) -> Self {
        Self(Box::new(value))
    }
}
