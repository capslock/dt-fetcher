use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_with::{formats::Strict, serde_as, skip_serializing_none, TimestampMilliSeconds};
use uuid::Uuid;

use crate::models::Link;

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CurrencyType {
    Marks,
    Credits,
}

impl std::fmt::Display for CurrencyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CurrencyType::Marks => write!(f, "marks"),
            CurrencyType::Credits => write!(f, "credits"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub id: Uuid,
    pub name: String,
    pub generation: i32,
    pub layout_ref: Option<String>,
    pub valid_from: String,
    pub valid_to: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Amount {
    pub amount: i32,
    #[serde(rename = "type")]
    pub amount_type: CurrencyType,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Price {
    pub amount: Amount,
    pub id: Uuid,
    pub priority: i32,
    pub price_formula: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entitlement {
    pub id: Uuid,
    pub limit: i32,
    #[serde(rename = "type")]
    pub entitlement_type: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stat {
    pub name: String,
    pub value: f64,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trait {
    pub id: String,
    pub rarity: i32,
    pub value: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Perk {
    pub id: String,
    pub rarity: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Override {
    pub ver: i32,
    pub rarity: i32,
    #[serde(rename = "characterLevel")]
    pub character_level: i32,
    #[serde(rename = "itemLevel")]
    pub item_level: i32,
    #[serde(rename = "baseItemLevel")]
    pub base_item_level: i32,
    pub traits: Vec<Trait>,
    pub perks: Vec<Perk>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WeaponOverride {
    #[serde(flatten)]
    pub overrides: Override,
    pub base_stats: Vec<Stat>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
#[serde(deny_unknown_fields)]
pub enum Overrides {
    Weapon(WeaponOverride),
    Gadget(Override),
    RandomItem { slots: Vec<String> },
    None {},
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Description {
    pub id: String,
    pub gear_id: Uuid,
    pub rotation: String,
    #[serde(rename = "type")]
    pub description_type: String,
    pub properties: HashMap<String, serde_json::Value>,
    pub overrides: Overrides,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sku {
    pub id: Uuid,
    pub display_priority: i32,
    pub internal_name: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub asset_id: String,
    pub tags: Vec<String>,
    pub dlc_req: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    pub offer_id: Uuid,
    pub sku: Sku,
    pub entitlement: Entitlement,
    pub price: Price,
    pub state: String,
    pub description: Description,
    pub media: Vec<serde_json::Value>,
}

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Store {
    #[serde(rename = "_links")]
    pub links: HashMap<String, Link>,
    pub catalog: Catalog,
    pub name: String,
    pub public: Vec<Offer>,
    pub personal: Vec<Offer>,
    pub rerolls_this_rotation: i32,
    #[serde_as(as = "TimestampMilliSeconds<String, Strict>")]
    pub current_rotation_end: DateTime<Utc>,
}
