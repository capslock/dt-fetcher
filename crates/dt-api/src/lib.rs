use models::{Character, CurrencyType};
use serde::{Deserialize, Serialize};

pub mod models;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Auth {
    pub access_token: String,
    pub account_name: String,
    pub expires_in: u32,
    pub refresh_at: u32,
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

    pub async fn get_summary(&self) -> Result<models::Summary, reqwest::Error> {
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
        let account_data = res.json::<models::Summary>().await?;
        Ok(account_data)
    }

    pub async fn get_store(
        &self,
        currency_type: CurrencyType,
        character: Character,
    ) -> Result<models::Store, reqwest::Error> {
        let url = format!(
            "https://bsp-td-prod.atoma.cloud/store/storefront/{}_store_{}/store",
            currency_type, character.archetype
        );
        let res = self
            .client
            .get(&url)
            .bearer_auth(&self.auth.access_token)
            .query(&[
                ("accountId", self.auth.sub.clone()),
                ("personal", "true".to_string()),
                ("characterId", character.id),
            ])
            .send()
            .await?;
        let store = res.json::<models::Store>().await?;
        Ok(store)
    }

    pub async fn get_master_data(&self) -> Result<models::MasterData, reqwest::Error> {
        let url = format!(
            "https://bsp-td-prod.atoma.cloud/web/{}/masterdata",
            self.auth.sub
        );
        let res = self
            .client
            .get(&url)
            .bearer_auth(&self.auth.access_token)
            .send()
            .await?;
        let master_data = res.json::<models::MasterData>().await?;
        Ok(master_data)
    }

    pub async fn refresh_auth(&mut self) -> Result<Auth, reqwest::Error> {
        let url = "https://bsp-auth-prod.atoma.cloud/queue/refresh";
        let res = self
            .client
            .get(url)
            .bearer_auth(&self.auth.refresh_token)
            .send()
            .await?;
        let auth = res.json::<Auth>().await?;
        self.auth = auth.clone();
        Ok(auth)
    }
}
