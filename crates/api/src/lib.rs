pub mod adapter;
pub mod jsonrpc;

pub use adapter::DefaultAPIAdapter;
pub use jsonrpc::web3_types::SyncStatus;
use once_cell::sync::Lazy;

use parking_lot::RwLock;
pub static SYNC_STATUS: Lazy<RwLock<SyncStatus>> =
    Lazy::new(|| RwLock::new(Default::default()));
