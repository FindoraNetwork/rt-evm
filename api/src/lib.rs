pub mod adapter;
pub mod jsonrpc;

pub use adapter::DefaultAPIAdapter;
pub use jsonrpc::web3_types::SyncStatus;
use once_cell::sync::Lazy;

use rt_evm_model::types::SignedTransaction;
use std::sync::{
    mpsc::{channel, Sender},
    Mutex, RwLock,
};

pub static TXS_MANAGER: Lazy<Mutex<Sender<SignedTransaction>>> = Lazy::new(|| {
    Mutex::new({
        channel().0 // must be replaced during initial process!
    })
});

pub static SYNC_STATUS: Lazy<RwLock<SyncStatus>> =
    Lazy::new(|| RwLock::new(Default::default()));
