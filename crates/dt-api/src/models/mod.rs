use std::fmt::Display;

use serde::{Deserialize, Serialize};

mod summary;
pub use summary::*;

mod store;
pub use store::*;

mod master_data;
pub use master_data::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Link {
    pub href: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Hash, Copy)]
#[serde(transparent)]
pub struct AccountId(pub uuid::Uuid);

impl Display for AccountId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
