use std::path::Path;

use anyhow::{Context, Result};
use im::HashMap;
use tracing::instrument;

use dt_api::{models::AccountId, Auth};

pub trait AuthStorage: Send + Sync + 'static {
    type Iter: Iterator<Item = Result<(AccountId, Auth)>>;

    fn get(&self, id: AccountId) -> Result<Option<Auth>>;

    fn get_single(&self) -> Result<Option<AccountId>>;

    fn contains(&self, id: &AccountId) -> Result<bool>;

    fn insert(&mut self, id: AccountId, auth: Auth) -> Result<()>;

    fn iter(&self) -> Self::Iter;
}

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
    type Iter = InMemoryAuthStorageIter;

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

    fn iter(&self) -> Self::Iter {
        InMemoryAuthStorageIter::new(&self.auths)
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
                serde_json::from_slice(&auth).context("Failed to deserialize auth")?,
            ))
        })
    }
}

impl AuthStorage for SledDbAuthStorage {
    type Iter = SledDbAuthStorageIter;

    fn get(&self, id: AccountId) -> Result<Option<Auth>> {
        let result = self.db.get(id.0.as_bytes()).context("Failed to get auth")?;
        result
            .map(|auth| serde_json::from_slice::<Auth>(&auth).context("Failed to deserialize auth"))
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
                serde_json::to_vec(&auth).context("Failed to serialize auth")?,
            )
            .context("Failed to insert")?;
        self.db.flush().context("Failed to flush")?;
        Ok(())
    }

    fn iter(&self) -> Self::Iter {
        SledDbAuthStorageIter::new(&self.db)
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

pub enum Either<L, R> {
    Left(L),
    Right(R),
}

impl<L, R> Iterator for Either<L, R>
where
    L: Iterator,
    R: Iterator<Item = L::Item>,
{
    type Item = L::Item;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Either::Left(l) => l.next(),
            Either::Right(r) => r.next(),
        }
    }
}

impl AuthStorage for ErasedAuthStorage {
    type Iter = Either<InMemoryAuthStorageIter, SledDbAuthStorageIter>;

    fn get(&'_ self, id: AccountId) -> Result<Option<Auth>> {
        match self {
            ErasedAuthStorage::InMemoryAuthStorage(s) => s.get(id),
            ErasedAuthStorage::SledDbAuthStorage(s) => s.get(id),
        }
    }

    fn get_single(&self) -> Result<Option<AccountId>> {
        match self {
            ErasedAuthStorage::InMemoryAuthStorage(s) => s.get_single(),
            ErasedAuthStorage::SledDbAuthStorage(s) => s.get_single(),
        }
    }

    fn contains(&self, id: &AccountId) -> Result<bool> {
        match self {
            ErasedAuthStorage::InMemoryAuthStorage(s) => s.contains(id),
            ErasedAuthStorage::SledDbAuthStorage(s) => s.contains(id),
        }
    }

    fn insert(&mut self, id: AccountId, auth: Auth) -> Result<()> {
        match self {
            ErasedAuthStorage::InMemoryAuthStorage(s) => s.insert(id, auth),
            ErasedAuthStorage::SledDbAuthStorage(s) => s.insert(id, auth),
        }
    }

    fn iter(&self) -> Self::Iter {
        match self {
            ErasedAuthStorage::InMemoryAuthStorage(s) => Either::Left(s.iter()),
            ErasedAuthStorage::SledDbAuthStorage(s) => Either::Right(s.iter()),
        }
    }
}
