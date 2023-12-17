use std::time::Duration;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use models::{Character, CurrencyType};
use serde::{Deserialize, Serialize};
use serde_with::{
    formats::Strict, serde_as, skip_serializing_none, DurationSeconds, TimestampMilliSeconds,
};

pub mod models;

#[skip_serializing_none]
#[serde_as]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Auth {
    pub access_token: String,
    pub account_name: String,
    #[serde_as(as = "DurationSeconds<u64>")]
    pub expires_in: Duration,
    #[serde_as(as = "Option<TimestampMilliSeconds<i64, Strict>>")]
    pub refresh_at: Option<DateTime<Utc>>,
    pub refresh_token: String,
    pub sub: String,
}

pub struct Api {
    client: reqwest::Client,
    auth: Auth,
}

impl Api {
    pub fn new(auth: Auth) -> Self {
        let client = reqwest::Client::new();
        Self { client, auth }
    }

    pub async fn get_summary(&self) -> Result<models::Summary> {
        let url = format!(
            "https://bsp-td-prod.atoma.cloud/web/{}/summary",
            self.auth.sub
        );
        let res = self
            .client
            .get(&url)
            .bearer_auth(&self.auth.access_token)
            .send()
            .await?;
        if res.status().is_success() {
            let account_data = res.json::<models::Summary>().await?;
            Ok(account_data)
        } else {
            let error = res.json::<serde_json::Value>().await?;
            Err(anyhow!(
                "Failed to get summary {}: {}",
                self.auth.sub,
                error
            ))
        }
    }

    pub async fn get_store(
        &self,
        currency_type: CurrencyType,
        character: &Character,
    ) -> Result<models::Store> {
        let url = format!(
            "https://bsp-td-prod.atoma.cloud/store/storefront/{}_store_{}",
            currency_type, character.archetype
        );
        let res = self
            .client
            .get(&url)
            .bearer_auth(&self.auth.access_token)
            .query(&[
                ("accountId", self.auth.sub.clone()),
                ("personal", "true".to_string()),
                ("characterId", character.id.clone()),
            ])
            .send()
            .await?;
        if res.status().is_success() {
            let store = res.json::<models::Store>().await?;
            Ok(store)
        } else {
            let error = res.json::<serde_json::Value>().await?;
            return Err(anyhow!(
                "Failed to get store {} {}: {}",
                currency_type,
                character.archetype,
                error
            ));
        }
    }

    pub async fn get_master_data(&self) -> Result<models::MasterData> {
        let url = format!("https://bsp-td-prod.atoma.cloud/master-data/meta/items",);
        let res = self
            .client
            .get(&url)
            .bearer_auth(&self.auth.access_token)
            .send()
            .await?;
        if res.status().is_success() {
            let master_data = res.json::<models::MasterData>().await?;
            Ok(master_data)
        } else {
            let error = res.json::<serde_json::Value>().await?;
            Err(anyhow!("Failed to get master data: {}", error))
        }
    }

    pub async fn refresh_auth(&mut self) -> Result<Auth> {
        let url = "https://bsp-auth-prod.atoma.cloud/queue/refresh";
        let res = self
            .client
            .get(url)
            .bearer_auth(&self.auth.refresh_token)
            .send()
            .await?;
        if res.status().is_success() {
            let auth = res.json::<Auth>().await?;
            self.auth = auth.clone();
            Ok(auth)
        } else {
            let error = res.json::<serde_json::Value>().await?;
            Err(anyhow!("Failed to refresh auth: {}", error))
        }
    }
}
