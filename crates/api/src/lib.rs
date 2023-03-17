#![deny(warnings)]
#![cfg_attr(feature = "benchmark", allow(warnings))]

pub mod adapter;
pub mod jsonrpc;

pub use adapter::DefaultAPIAdapter;
pub use jsonrpc::{run_jsonrpc_server, web3_types::SyncStatus, ServerHandlers};

use once_cell::sync::Lazy;
use parking_lot::RwLock;
static SYNC_STATUS: Lazy<RwLock<SyncStatus>> =
    Lazy::new(|| RwLock::new(Default::default()));

pub fn set_node_sync_status(s: SyncStatus) {
    *SYNC_STATUS.write() = s;
}
