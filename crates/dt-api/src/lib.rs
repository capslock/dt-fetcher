use std::time::Duration;

use chrono::{DateTime, Utc};
use models::{Character, CurrencyType};
use serde::{Deserialize, Serialize};
use serde_with::{
    formats::Strict, serde_as, skip_serializing_none, DurationSeconds, TimestampMilliSeconds,
};
use thiserror::Error;
use tracing::{debug, info, instrument};
use uuid::Uuid;

pub mod models;

#[derive(Debug, Error)]
#[error(transparent)]
pub struct Error(#[from] ApiError);

#[derive(Error, Debug)]
enum ApiError {
    #[error("Request failed")]
    Reqwest(#[from] reqwest::Error),
    #[error("Failed to get summary for {sub}: {status}: {error}")]
    GetSummary {
        status: reqwest::StatusCode,
        error: serde_json::Value,
        sub: Uuid,
    },
    #[error("Failed to get store {currency_type} {archetype}: {status}: {error}")]
    GetStore {
        status: reqwest::StatusCode,
        error: serde_json::Value,
        currency_type: CurrencyType,
        archetype: String,
    },
    #[error("Failed to get master data: {status}: {error}")]
    GetMasterData {
        status: reqwest::StatusCode,
        error: serde_json::Value,
    },
    #[error("Failed to refresh auth: {status}: {error}")]
    RefreshAuth {
        status: reqwest::StatusCode,
        error: serde_json::Value,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

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
            .await
            .map_err(ApiError::Reqwest)?;
        if res.status().is_success() {
            let account_data = res
                .json::<models::Summary>()
                .await
                .map_err(ApiError::Reqwest)?;
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
            return Err(ApiError::GetSummary {
                status,
                error,
                sub: auth.sub,
            }
            .into());
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
            .await
            .map_err(ApiError::Reqwest)?;
        if res.status().is_success() {
            let store = res
                .json::<models::Store>()
                .await
                .map_err(ApiError::Reqwest)?;
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
            return Err(ApiError::GetStore {
                status,
                error,
                currency_type,
                archetype: character.archetype.clone(),
            }
            .into());
        }
    }

    #[instrument(skip(self))]
    pub async fn get_master_data(&self, auth: &Auth) -> Result<models::MasterData> {
        let url = "https://bsp-td-prod.atoma.cloud/master-data/meta/items";
        debug!(url = ?url, "Getting master data");
        let res = self
            .client
            .get(url)
            .bearer_auth(&auth.access_token)
            .send()
            .await
            .map_err(ApiError::Reqwest)?;
        if res.status().is_success() {
            let master_data = res
                .json::<models::MasterData>()
                .await
                .map_err(ApiError::Reqwest)?;
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
            Err(ApiError::GetMasterData { status, error }.into())
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
            .await
            .map_err(ApiError::Reqwest)?;
        if res.status().is_success() {
            let auth = res.json::<Auth>().await.map_err(ApiError::Reqwest)?;
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
            Err(ApiError::RefreshAuth { status, error }.into())
        }
    }
}
