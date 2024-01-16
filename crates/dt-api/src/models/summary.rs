use std::{collections::HashMap, fmt::Display};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::Link;

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Gender {
    Female,
    Male,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Copy)]
#[serde(transparent)]
pub struct CharacterId(pub Uuid);

impl Display for CharacterId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Character {
    pub id: CharacterId,
    pub name: String,
    pub gender: Gender,
    pub archetype: String,
    pub specialization: String,
    pub level: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Email {
    pub verified: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinkedAccounts {
    pub steam: String,
    pub twitch: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketingPreferences {
    pub newsletter_subscribe: bool,
    pub opt_in: bool,
    pub terms_agreed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    #[serde(rename = "_links")]
    pub links: HashMap<String, Link>,
    pub username: String,
    pub name: String,
    pub discriminator: String,
    pub allow_rename: bool,
    pub characters: Vec<Character>,
    pub email: Email,
    pub linked_accounts: LinkedAccounts,
    pub marketing_preferences: MarketingPreferences,
}
