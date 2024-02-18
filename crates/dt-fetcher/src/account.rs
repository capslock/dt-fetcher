use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use chrono::{DateTime, Utc};
use dt_api::models::{AccountId, CharacterId, MasterData, Store};
use futures::stream::{FuturesOrdered, StreamExt};
use tokio::sync::RwLock;
use tracing::error;
use tracing::{info, instrument};

#[derive(Debug, Clone)]
pub(crate) struct AccountData {
    pub last_updated: DateTime<Utc>,
    pub summary: Arc<RwLock<dt_api::models::Summary>>,
    pub marks_store: Arc<RwLock<HashMap<CharacterId, dt_api::models::Store>>>,
    pub credits_store: Arc<RwLock<HashMap<CharacterId, dt_api::models::Store>>>,
    pub master_data: Arc<RwLock<dt_api::models::MasterData>>,
}

impl AccountData {
    pub fn new(
        summary: dt_api::models::Summary,
        marks_store: HashMap<CharacterId, Store>,
        credits_store: HashMap<CharacterId, Store>,
        master_data: MasterData,
    ) -> Self {
        Self {
            last_updated: Utc::now(),
            summary: Arc::new(RwLock::new(summary)),
            marks_store: Arc::new(RwLock::new(marks_store)),
            credits_store: Arc::new(RwLock::new(credits_store)),
            master_data: Arc::new(RwLock::new(master_data)),
        }
    }

    #[instrument]
    pub async fn fetch(api: &dt_api::Api, auth: &dt_api::Auth) -> Result<AccountData> {
        let summary = api.get_summary(auth).await?;

        info!(
            "Fetching stores for {} characters",
            summary.characters.len()
        );

        let marks_store = summary
            .characters
            .iter()
            .map(|c| api.get_store(auth, dt_api::models::CurrencyType::Marks, c))
            .collect::<FuturesOrdered<_>>()
            .collect::<Vec<_>>();

        let credits_store = summary
            .characters
            .iter()
            .map(|c| api.get_store(auth, dt_api::models::CurrencyType::Credits, c))
            .collect::<FuturesOrdered<_>>()
            .collect::<Vec<_>>();

        let (marks_store, credits_store) = tokio::join!(marks_store, credits_store);

        let marks_store = summary
            .characters
            .iter()
            .zip(marks_store.into_iter())
            .filter_map(|(c, s)| match s {
                Ok(s) => Some((c.id, s)),
                Err(e) => {
                    error!("Failed to get marks store: {}", e);
                    None
                }
            })
            .collect::<HashMap<CharacterId, Store>>();

        let credits_store = summary
            .characters
            .iter()
            .zip(credits_store.into_iter())
            .filter_map(|(c, s)| match s {
                Ok(s) => Some((c.id, s)),
                Err(e) => {
                    error!("Failed to get credits store: {}", e);
                    None
                }
            })
            .collect::<HashMap<CharacterId, Store>>();

        let master_data = api.get_master_data(auth).await?;

        Ok(Self::new(summary, marks_store, credits_store, master_data))
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Accounts(Arc<RwLock<HashMap<AccountId, AccountData>>>);

impl Accounts {
    #[instrument]
    pub async fn get(&self, id: &AccountId) -> Option<AccountData> {
        self.0.read().await.get(id).cloned()
    }

    #[instrument]
    pub async fn insert(&self, id: AccountId, data: AccountData) {
        self.0.write().await.insert(id, data);
    }

    #[instrument]
    pub async fn update_timestamp(&self, id: &AccountId) {
        if let Some(account_data) = self.0.write().await.get_mut(id) {
            account_data.last_updated = Utc::now();
        }
    }

    #[instrument]
    pub async fn timestamp(&self, id: &AccountId) -> Option<DateTime<Utc>> {
        if let Some(account_data) = self.0.read().await.get(id) {
            return Some(account_data.last_updated);
        }
        None
    }
}
