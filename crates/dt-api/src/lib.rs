use std::time::Duration;

use chrono::{DateTime, Utc};
use models::{AccountId, Character, CurrencyType};
use serde::{Deserialize, Serialize};
use serde_with::{
    formats::Strict, serde_as, skip_serializing_none, DurationSeconds, TimestampMilliSeconds,
};
use thiserror::Error;
use tracing::{debug, info, instrument};

pub mod models;

/// Errors that can occur when interacting with the API.
#[derive(Error, Debug)]
pub enum Error {
    /// An error occurred while sending a request to the API.
    #[error("Sending request failed")]
    RequestFailed(#[from] reqwest::Error),
    /// An error occurred while parsing the response from the API.
    #[error("Parsing response failed")]
    InvalidResponse(#[source] reqwest::Error),
    /// The server returned an error response when getting the summary.
    #[error("Failed to get summary for {sub}: {status}: {error}")]
    GetSummary {
        status: reqwest::StatusCode,
        error: serde_json::Value,
        sub: AccountId,
    },
    /// The server returned an error response when getting the store.
    #[error("Failed to get {currency_type} store for {archetype}: {status}: {error}")]
    GetStore {
        status: reqwest::StatusCode,
        error: serde_json::Value,
        currency_type: CurrencyType,
        archetype: String,
    },
    /// The server returned an error response when getting the master data.
    #[error("Failed to get master data: {status}: {error}")]
    GetMasterData {
        status: reqwest::StatusCode,
        error: serde_json::Value,
    },
    /// The server returned an error response when refreshing the auth.
    #[error("Failed to refresh auth: {status}: {error}")]
    RefreshAuth {
        status: reqwest::StatusCode,
        error: serde_json::Value,
    },
}

/// Result type for API operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Authentication token and account auth information.
#[skip_serializing_none]
#[serde_as]
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Auth {
    /// The JWT access token.
    pub access_token: String,
    /// The name of the account.
    pub account_name: String,
    /// The duration until the access token expires.
    #[serde_as(as = "DurationSeconds<u64>")]
    pub expires_in: Duration,
    /// The time when the access token should be refreshed.
    #[serde_as(as = "Option<TimestampMilliSeconds<i64, Strict>>")]
    pub refresh_at: Option<DateTime<Utc>>,
    /// The JWT refresh token.
    pub refresh_token: String,
    /// The subject of the JWT.
    pub sub: AccountId,
}

impl Auth {
    /// Returns true if the access token is expired.
    ///
    /// If the `refresh_at` field is not set, the token is considered expired.
    ///
    /// # Parameters
    ///
    /// - `buffer` - The buffer to use when checking if the token is expired.
    ///
    /// # Returns
    ///
    /// True if the token is expired, false otherwise.
    pub fn expired(&self, buffer: Duration) -> bool {
        self.refresh_at
            .map(|refresh_at| refresh_at <= Utc::now() + buffer)
            .unwrap_or(true)
    }
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

/// API client for interacting with the DT Api.
#[derive(Clone, Debug)]
pub struct Api {
    client: reqwest::Client,
}

impl Api {
    /// Creates a new API client.
    #[instrument]
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    /// Gets the summary for the account.
    ///
    /// # Parameters
    ///
    /// - `auth` - The authentication token.
    ///
    /// # Returns
    ///
    /// The summary for the account.
    ///
    /// # Errors
    ///
    /// An error is returned if the request fails or the server returns an error response.
    #[instrument(skip(self))]
    pub async fn get_summary(&self, auth: &Auth) -> Result<models::Summary> {
        let url = format!("https://bsp-td-prod.atoma.cloud/web/{}/summary", auth.sub.0);
        debug!(url = ?url, "Getting summary");
        let res = self
            .client
            .get(&url)
            .bearer_auth(&auth.access_token)
            .send()
            .await?;
        if res.status().is_success() {
            let account_data = res
                .json::<models::Summary>()
                .await
                .map_err(Error::InvalidResponse)?;
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
            return Err(Error::GetSummary {
                status,
                error,
                sub: auth.sub,
            });
        }
    }

    /// Gets the store for the character.
    ///
    /// # Parameters
    ///
    /// - `auth` - The authentication token.
    /// - `currency_type` - The type of currency to get the store for.
    /// - `character` - The character to get the store for.
    ///
    /// # Returns
    ///
    /// The store for the character.
    ///
    /// # Errors
    ///
    /// An error is returned if the request fails or the server returns an error response.
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
                ("characterId", character.id.0.to_string()),
            ])
            .send()
            .await?;
        if res.status().is_success() {
            let store = res
                .json::<models::Store>()
                .await
                .map_err(Error::InvalidResponse)?;
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
            return Err(Error::GetStore {
                status,
                error,
                currency_type,
                archetype: character.archetype.clone(),
            });
        }
    }

    /// Gets the master data.
    ///
    /// # Parameters
    ///
    /// - `auth` - The authentication token.
    ///
    /// # Returns
    ///
    /// The master data.
    ///
    /// # Errors
    ///
    /// An error is returned if the request fails or the server returns an error response.
    #[instrument(skip(self))]
    pub async fn get_master_data(&self, auth: &Auth) -> Result<models::MasterData> {
        let url = "https://bsp-td-prod.atoma.cloud/master-data/meta/items";
        debug!(url = ?url, "Getting master data");
        let res = self
            .client
            .get(url)
            .bearer_auth(&auth.access_token)
            .send()
            .await?;
        if res.status().is_success() {
            let master_data = res
                .json::<models::MasterData>()
                .await
                .map_err(Error::InvalidResponse)?;
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
            Err(Error::GetMasterData { status, error })
        }
    }

    /// Refreshes the authentication token.
    ///
    /// # Parameters
    ///
    /// - `auth` - The authentication token to refresh.
    ///
    /// # Returns
    ///
    /// The refreshed authentication token.
    ///
    /// # Errors
    ///
    /// An error is returned if the request fails or the server returns an error response.
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
            let auth = res.json::<Auth>().await.map_err(Error::InvalidResponse)?;
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
            Err(Error::RefreshAuth { status, error })
        }
    }
}
