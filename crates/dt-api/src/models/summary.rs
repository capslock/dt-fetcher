use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::models::Link;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Gender {
    Female,
    Male,
}

#[derive(Serialize, Deserialize)]
pub struct Character {
    pub id: String,
    pub name: String,
    pub gender: Gender,
    pub archetype: String,
    pub specialization: String,
    pub level: u32,
}

#[derive(Serialize, Deserialize)]
pub struct Email {
    pub verified: bool,
}

#[derive(Serialize, Deserialize)]
pub struct LinkedAccounts {
    pub steam: String,
    pub twitch: String,
}

#[derive(Serialize, Deserialize)]
pub struct MarketingPreferences {
    pub newsletter_subscribe: bool,
    pub opt_in: bool,
    pub terms_agreed: bool,
}

#[derive(Serialize, Deserialize)]
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
