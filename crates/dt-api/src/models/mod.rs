use serde::{Deserialize, Serialize};

mod summary;
pub use summary::*;

mod store;
pub use store::*;

mod master_data;
pub use master_data::*;

#[derive(Clone, Serialize, Deserialize)]
pub struct Link {
    pub href: String,
}
