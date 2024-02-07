use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::models::Link;

/// Player items
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlayerItems {
    pub href: String,
    pub version: String,
}

/// Master data model
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MasterData {
    #[serde(rename = "_links")]
    pub links: HashMap<String, Link>,
    pub player_items: PlayerItems,
}
