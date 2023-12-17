use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::models::Link;

#[derive(Copy, Clone, Serialize, Deserialize)]
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

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub id: String,
    pub name: String,
    pub generation: i32,
    pub layout_ref: Option<String>,
    pub valid_from: String,
    pub valid_to: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Amount {
    pub amount: i32,
    #[serde(rename = "type")]
    pub amount_type: CurrencyType,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Price {
    pub amount: Amount,
    pub id: String,
    pub priority: i32,
    pub price_formula: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entitlement {
    pub id: String,
    pub limit: i32,
    #[serde(rename = "type")]
    pub entitlement_type: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Stat {
    pub name: String,
    pub value: f64,
}

#[skip_serializing_none]
#[derive(Clone, Serialize, Deserialize)]
pub struct Trait {
    pub id: String,
    pub rarity: i32,
    pub value: Option<f64>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Perk {
    pub id: String,
    pub rarity: i32,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(untagged)]
pub enum Override {
    Override {
        ver: i32,
        rarity: i32,
        character_level: i32,
        item_level: i32,
        base_item_level: i32,
        traits: Vec<Trait>,
        perks: Vec<Perk>,
        base_stats: Vec<Stat>,
    },
    None {},
}

#[skip_serializing_none]
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Description {
    pub id: String,
    pub gear_id: String,
    pub rotation: String,
    #[serde(rename = "type")]
    pub description_type: String,
    pub properties: HashMap<String, serde_json::Value>,
    pub overrides: Override,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sku {
    pub id: String,
    pub display_priority: i32,
    pub internal_name: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub asset_id: String,
    pub tags: Vec<String>,
    pub dlc_req: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    pub offer_id: String,
    pub sku: Sku,
    pub entitlement: Entitlement,
    pub price: Price,
    pub state: String,
    pub description: Description,
    pub media: Vec<serde_json::Value>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Store {
    #[serde(rename = "_links")]
    pub links: HashMap<String, Link>,
    pub catalog: Catalog,
    pub name: String,
    pub public: Vec<Offer>,
    pub personal: Vec<Offer>,
    pub rerolls_this_rotation: i32,
    pub current_rotation_end: String,
}