mod api;
mod executor;
mod storage;

use ethereum_types::{H160, U256};
use once_cell::sync::{Lazy, OnceCell};
use std::str::FromStr;

pub use api::APIAdapter;
pub use executor::{ApplyBackend, Backend, Executor, ExecutorAdapter};
pub use storage::{BlockStorage, Storage, TxStorage};

pub static CONSTANT_ADDR: OnceCell<H160> = OnceCell::new();
pub static BALANCE_SLOT: OnceCell<U256> = OnceCell::new();

pub static SYSTEM_ADDR: Lazy<H160> =
    Lazy::new(|| H160::from_str("0x0000000000000000000000000000000000002000").unwrap());

pub struct SystemContract {
    pub gas_price: U256,
    pub address: H160,
    pub code: Vec<u8>,
}
