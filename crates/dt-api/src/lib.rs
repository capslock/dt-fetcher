use std::time::Duration;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use models::{Character, CurrencyType};
use serde::{Deserialize, Serialize};
use serde_with::{
    formats::Strict, serde_as, skip_serializing_none, DurationSeconds, TimestampMilliSeconds,
};
use tracing::{debug, info, instrument};
use uuid::Uuid;

pub mod models;

#[skip_serializing_none]
#[serde_as]
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Auth {
    pub access_token: String,
    pub account_name: String,
    #[serde_as(as = "DurationSeconds<u64>")]
    pub expires_in: Duration,
    #[serde_as(as = "Option<TimestampMilliSeconds<i64, Strict>>")]
    pub refresh_at: Option<DateTime<Utc>>,
    pub refresh_token: String,
    pub sub: Uuid,
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Auth")
            .field("access_token", &"<REDACTED>")
            .field("account_name", &self.account_name)
            .field("expires_in", &self.expires_in)
            .field("refresh_at", &self.refresh_at)
            .field("refresh_token", &"<REDACTED>")
            .field("sub", &self.sub)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct Api {
    client: reqwest::Client,
}

impl Api {
    #[instrument]
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    #[instrument(skip(self))]
    pub async fn get_summary(&self, auth: &Auth) -> Result<models::Summary> {
        let url = format!("https://bsp-td-prod.atoma.cloud/web/{}/summary", auth.sub);
        debug!(url = ?url, "Getting summary");
        let res = self
            .client
            .get(&url)
            .bearer_auth(&auth.access_token)
            .send()
            .await?;
        if res.status().is_success() {
            let account_data = res.json::<models::Summary>().await?;
            info!("Got summary");
            debug!(summary = ?account_data);
            Ok(account_data)
        } else {
            let status = res.status();
            let error = res
                .json::<serde_json::Value>()
                .await
                .unwrap_or("No error details".into());
            tracing::error!(
                status = ?status,
                error = ?error,
                "Failed to get summary"
            );
            return Err(anyhow!(
                "Failed to get summary {}: {status}: {error}",
                auth.sub
            ));
        }
    }

    #[instrument(skip(self))]
    pub async fn get_store(
        &self,
        auth: &Auth,
        currency_type: CurrencyType,
        character: &Character,
    ) -> Result<models::Store> {
        let url = format!(
            "https://bsp-td-prod.atoma.cloud/store/storefront/{}_store_{}",
            currency_type, character.archetype
        );
        debug!(url = ?url, "Getting store");
        let res = self
            .client
            .get(&url)
            .bearer_auth(&auth.access_token)
            .query(&[
                ("accountId", auth.sub.to_string()),
                ("personal", "true".to_string()),
                ("characterId", character.id.to_string()),
            ])
            .send()
            .await?;
        if res.status().is_success() {
            let store = res.json::<models::Store>().await?;
            info!("Got store");
            debug!(store = ?store);
            Ok(store)
        } else {
            let status = res.status();
            let error = res
                .json::<serde_json::Value>()
                .await
                .unwrap_or("No error details".into());
            tracing::error!(
                status = ?status,
                error = ?error,
                "Failed to get store"
            );
            return Err(anyhow!(
                "Failed to get store {} {}: {status}: {error}",
                currency_type,
                character.archetype
            ));
        }
    }

    #[instrument(skip(self))]
    pub async fn get_master_data(&self, auth: &Auth) -> Result<models::MasterData> {
        let url = format!("https://bsp-td-prod.atoma.cloud/master-data/meta/items");
        debug!(url = ?url, "Getting master data");
        let res = self
            .client
            .get(&url)
            .bearer_auth(&auth.access_token)
            .send()
            .await?;
        if res.status().is_success() {
            let master_data = res.json::<models::MasterData>().await?;
            info!("Got master data");
            debug!(master_data = ?master_data);
            Ok(master_data)
        } else {
            let status = res.status();
            let error = res
                .json::<serde_json::Value>()
                .await
                .unwrap_or("No error details".into());
            tracing::error!(
                status = ?status,
                error = ?error,
                "Failed to get master data"
            );
            Err(anyhow!("Failed to get master data: {status}: {error}"))
        }
    }

    #[instrument(skip(self))]
    pub async fn refresh_auth(&self, auth: &Auth) -> Result<Auth> {
        let url = "https://bsp-auth-prod.atoma.cloud/queue/refresh";
        debug!(url = ?url, "Refreshing auth");
        let res = self
            .client
            .get(url)
            .bearer_auth(&auth.refresh_token)
            .send()
            .await?;
        if res.status().is_success() {
            let auth = res.json::<Auth>().await?;
            info!("Refreshed auth");
            debug!(auth = ?auth);
            Ok(auth)
        } else {
            let status = res.status();
            let error = res
                .json::<serde_json::Value>()
                .await
                .unwrap_or("No error details".into());
            tracing::error!(
                status = ?status,
                error = ?error,
                "Failed to refresh auth"
            );
            Err(anyhow!("Failed to refresh auth: {status}: {error}"))
        }
    }
}
